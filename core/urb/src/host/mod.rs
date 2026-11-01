//! The native host (feature `native`, docs/rust-core.md, M7): the
//! sans-IO [`Core`] driven by a UDP socket (or the relay), a clock, an
//! HTTPS client for the roller and the comet stars, and a store.
//!
//! [`Host`] is the driving, and is the same whatever does the IO: it
//! hands the core each input and acts on each output, in order, through
//! an [`Io`]. [`net::NetIo`] is the real one. The trace replay
//! (tests/host.rs) is another, which feeds a trace's inputs through the
//! same driving and checks what comes out.
//!
//! What it keeps from the shim's host (shim/mesa.mjs):
//!
//! - outputs are acted on in the order the core gives them, so every
//!   commit is saved before the datagrams after it go;
//! - one timer: the host waits for the core's earliest deadline and then
//!   ticks the earliest timer due by then, by name (of two due at once,
//!   the one set first), one a turn;
//! - nothing is started or saved after a failed saveKey or transport
//!   start, and open then fails as they failed, closing a transport the
//!   core started and didn't close;
//! - a store that fails to commit is logged as `saving the engine's
//!   state failed: …`, and the engine goes on.

pub mod net;
pub mod store;

use crate::core::{Failure, Inbox, Lane, Output, Reply, Request};
use crate::js::{Js, Json};
use crate::{Core, Input};
use std::collections::{HashMap, VecDeque};
pub use store::{Given, Loaded, Store};

/// something the world brought, besides a timer
#[derive(Debug)]
pub enum Event {
    Packet {
        ip: u32,
        port: u16,
        bytes: Vec<u8>,
    },
    Fetched {
        id: u64,
        answer: Result<(u16, Vec<u8>), String>,
    },
}

/// why [`Io::wait`] came back
#[derive(Debug)]
pub enum Wake {
    Event(Event),
    /// the deadline it was given has come
    Timer,
    /// neither, after a short while: a host checks for a signal
    Idle,
}

/// What the host does to the world, and how the world answers.
pub trait Io {
    /// ms since 1970
    fn now(&mut self) -> u64;
    /// block until something comes, the deadline passes, or a short while
    fn wait(&mut self, deadline: Option<u64>) -> Wake;
    fn load(&mut self) -> Result<Loaded, String>;
    fn save_key(&mut self, keyfile: &str) -> Result<(), String>;
    fn commit(&mut self, rows: &[u8]) -> Result<(), String>;
    /// the transport starts, as ship
    fn start(&mut self, ship: &str) -> Result<(), String>;
    fn send(&mut self, lane: Lane, bytes: &[u8]);
    fn close(&mut self);
    /// an HTTPS request; its answer comes back as [`Event::Fetched`]
    fn fetch(
        &mut self,
        id: u64,
        url: &str,
        method: &str,
        headers: &[(String, String)],
        body: Option<&str>,
    );
    fn log(&mut self, line: &str);
    /// every output the host hands on to its caller (a result, an
    /// event, a note, an inbox poke), as it comes: for the replay
    fn tap(&mut self, _o: &Output) {}
}

/// The driving. Calls are made by id, and their results kept until
/// taken.
pub struct Host<I: Io> {
    core: Core,
    pub io: I,
    ids: u64,
    done: HashMap<u64, Result<Reply, Failure>>,
    /// furum's and forum's notes, as they came
    pub notes: VecDeque<(String, Js)>,
    /// publishing's saves and the rest of the core's events
    pub events: VecDeque<(String, Json)>,
    pub inbox: VecDeque<Inbox>,
    /// why a saveKey or the transport's start failed: nothing more is
    /// started or saved
    gate: Option<String>,
    opening: Option<u64>,
    started: bool,
    start_failed: bool,
    stopped: bool,
    /// why open failed, once it has: later calls fail with it
    dead: Option<Failure>,
}

impl<I: Io> Host<I> {
    /// a host on io, the core drawing entropy from entropy
    pub fn new(io: I, entropy: impl FnMut(usize) -> Vec<u8> + 'static) -> Host<I> {
        Host {
            core: Core::new(entropy),
            io,
            ids: 1 << 32,
            done: HashMap::new(),
            notes: VecDeque::new(),
            events: VecDeque::new(),
            inbox: VecDeque::new(),
            gate: None,
            opening: None,
            started: false,
            start_failed: false,
            stopped: false,
            dead: None,
        }
    }

    pub fn core(&self) -> &Core {
        &self.core
    }

    fn step(&mut self, input: Input) {
        let now = self.io.now();
        match self.core.step(now, input) {
            Ok(outs) => {
                for o in outs {
                    self.act(o);
                }
            }
            Err(e) => self.io.log(&e),
        }
    }

    fn act(&mut self, o: Output) {
        let open = self.gate.is_none();
        match o {
            Output::Send { lane, bytes } => {
                if open {
                    self.io.send(lane, &bytes);
                }
            }
            Output::Commit(rows) => {
                if open && let Err(e) = self.io.commit(&rows) {
                    self.io
                        .log(&format!("saving the engine's state failed: {e}"));
                }
            }
            Output::SaveKey(k) => {
                if open && let Err(e) = self.io.save_key(&k) {
                    self.gate = Some(e);
                }
            }
            Output::Start(ship) => {
                if open {
                    self.started = true;
                    if let Err(e) = self.io.start(&ship) {
                        self.start_failed = true;
                        self.gate = Some(e);
                    }
                }
            }
            Output::TClose => {
                if open {
                    self.stopped = true;
                    self.io.close();
                }
            }
            Output::Log(l) => self.io.log(&l),
            Output::Fetch {
                id,
                url,
                method,
                headers,
                body,
            } => self.io.fetch(id, &url, &method, &headers, body.as_deref()),
            Output::Done { call, mut result } => {
                if self.opening == Some(call) {
                    self.opening = None;
                    if let Some(g) = &self.gate {
                        result = Err(Failure::error(g.clone()));
                    }
                    if let Err(f) = &result {
                        if self.started && !self.start_failed && !self.stopped {
                            self.stopped = true;
                            self.io.close();
                        }
                        self.dead = Some(f.clone());
                    }
                }
                let o = Output::Done { call, result };
                self.io.tap(&o);
                let Output::Done { call, result } = o else {
                    unreachable!()
                };
                self.done.insert(call, result);
            }
            Output::Inbox(m) => {
                let o = Output::Inbox(m);
                self.io.tap(&o);
                let Output::Inbox(m) = o else { unreachable!() };
                self.inbox.push_back(m);
            }
            Output::Event { name, value } => {
                let o = Output::Event { name, value };
                self.io.tap(&o);
                let Output::Event { name, value } = o else {
                    unreachable!()
                };
                self.events.push_back((name, value));
            }
            Output::Note { name, value } => {
                let o = Output::Note { name, value };
                self.io.tap(&o);
                let Output::Note { name, value } = o else {
                    unreachable!()
                };
                self.notes.push_back((name, value));
            }
        }
    }

    /// make a call, under id: its result is taken with [`Host::take`]
    pub fn submit_as(&mut self, id: u64, req: Request) {
        if let Some(f) = &self.dead {
            self.done.insert(id, Err(f.clone()));
            return;
        }
        self.step(Input::Call { id, req });
    }

    /// make a call: its id
    pub fn submit(&mut self, req: Request) -> u64 {
        self.ids += 1;
        let id = self.ids;
        self.submit_as(id, req);
        id
    }

    /// open, on what the store holds (fake on a fake network), under id
    pub fn submit_open_as(&mut self, id: u64, fake: bool) {
        match self.io.load() {
            Err(e) => {
                self.done.insert(id, Err(Failure::error(e)));
            }
            Ok(l) => {
                self.gate = None;
                self.started = false;
                self.start_failed = false;
                self.stopped = false;
                self.opening = Some(id);
                self.submit_as(
                    id,
                    Request::Open {
                        keyfile: l.keyfile,
                        rows: l.rows.filter(|r| !r.is_empty()),
                        fresh: l.fresh,
                        fake,
                    },
                );
            }
        }
    }

    /// a call's result, once it has one
    pub fn take(&mut self, id: u64) -> Option<Result<Reply, Failure>> {
        self.done.remove(&id)
    }

    /// One turn: wait for the next thing (no later than limit), and hand
    /// it to the core, or tick the earliest timer due.
    pub fn turn_until(&mut self, limit: Option<u64>) {
        let deadline = match (self.core.deadline(), limit) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        match self.io.wait(deadline) {
            Wake::Event(Event::Packet { ip, port, bytes }) => {
                self.step(Input::Hear { ip, port, bytes })
            }
            Wake::Event(Event::Fetched { id, answer }) => self.step(Input::Fetched { id, answer }),
            Wake::Timer => {
                let now = self.io.now();
                if let Some((kind, due)) = self.core.due(now) {
                    self.step(Input::Tick { kind, due });
                }
            }
            Wake::Idle => {}
        }
    }

    pub fn turn(&mut self) {
        self.turn_until(None)
    }

    /// fire one named timer, whether or not it is due (a clock that
    /// fires early, as the JS's did in trace 5)
    pub fn tick(&mut self, kind: String, due: u64) {
        self.step(Input::Tick { kind, due });
    }

    /// run until the call id has its result
    pub fn wait_for(&mut self, id: u64) -> Result<Reply, Failure> {
        loop {
            if let Some(r) = self.take(id) {
                return r;
            }
            self.turn();
        }
    }

    /// run for ms, or until stop says so (checked each turn)
    pub fn run_for(&mut self, ms: u64, mut stop: impl FnMut(&mut Self) -> bool) {
        let end = self.io.now() + ms;
        while self.io.now() < end && !stop(self) {
            self.turn_until(Some(end));
        }
    }

    /// a call, run to its result
    pub fn call(&mut self, req: Request) -> Result<Reply, Failure> {
        let id = self.submit(req);
        self.wait_for(id)
    }

    /// open: our ship
    pub fn open(&mut self, fake: bool) -> Result<String, Failure> {
        self.ids += 1;
        let id = self.ids;
        self.submit_open_as(id, fake);
        match self.wait_for(id)? {
            Reply::Open { our } => Ok(our),
            r => Err(Failure::error(format!("open gave {r:?}"))),
        }
    }

    /// close: once everything committed is saved and sent, and the
    /// transport closed
    pub fn close(&mut self) -> Result<(), Failure> {
        self.call(Request::Close).map(|_| ())
    }
}
