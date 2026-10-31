//! The sans-IO core: libmesa's engine with mesa.mjs's reader and
//! grubbery client on it, as one state machine.
//!
//! It owns the engine and takes `now` and entropy from its host. Each
//! [`Input`] (an app request, a datagram, a timer's tick or a fetch's
//! answer) gives the [`Output`]s it caused: datagrams, rows to save,
//! fetches, log lines and the results of requests. It never opens a
//! socket, a file or a timer: its timers are named, as mesa.mjs names
//! them (`wake`, `point <ship>`, `roller <ship>`, `peek <wire>`, `poke
//! <wire>`, `keen <path>`, `close`), and the host fires each by its name
//! once it is due. [`Core::deadline`] is the earliest.
//!
//! The JS is the specification (docs/rust-core.md, rule 1). The requests
//! are trace.mjs's calls (open, read, close, peek, poke), with their
//! results and errors, word for word; the timeouts are the JS's (keen
//! 10 s then %yawn, peek and poke 30 s, the revision list kept 60 s), and
//! the outputs of each input come in the JS's order (see [`crate::rt`]).

use crate::codec::{self, Budget, MAX_NUMBER, Measured};
use crate::engine::{self, Engine, Journal, Ship, ship_text};
use crate::js::{self, Json, Object};
use crate::msg;
use crate::noun::{Noun, NounRef, Value};
use crate::rt::{self, Promise, Settled, Step, on_err, on_ok, resolve, spawn, then, wait, wait_ok};
use crate::url::{self, APP_BASE, DESK_BASE, Mount, PAGE_MARKS};
use crate::{Int, UrbError};
use mesa_sys as sys;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::rc::Rc;

/// +revs-ttl: how long a peer's revision list is kept
pub const REVS_TTL: u64 = 60_000;
/// +mesa-timeout: how long a keen waits before it is yawned
pub const KEEN_TIMEOUT: u64 = 10_000;
/// +remote-timeout: how long a peek or a poke waits
pub const PEEK_TIMEOUT: u64 = 30_000;
/// how long close() waits after the last save
pub const CLOSE_WAIT: u64 = 500;
/// the longest the wake timer is set for: waking early re-arms it
const WAKE_MAX: u64 = 60_000;
/// a key lookup's limit
const ROLLER_WAIT: u64 = 30_000;
pub const ROLLER: &str = "https://roller.urbit.org/v1/azimuth";
pub const STARS: &str = "https://bootstrap.urbit.org/comet-stars.jam";
const NONE: u64 = sys::MESA_NONE;
const ACK: sys::mesa_plea_result = sys::mesa_plea_result_MESA_ACK;

// ------------------------------------------------------------ the face

/// What threw, as the JS names it: an `UrbError` (with `later` where the
/// JS sets it), an `Error` or a `TypeError`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    UrbError,
    Error,
    TypeError,
}

/// An error, as the JS throws it: its kind and its message, word for
/// word.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub kind: Kind,
    pub message: String,
    pub later: bool,
}

impl Failure {
    pub fn urb(m: impl Into<String>) -> Failure {
        Failure {
            kind: Kind::UrbError,
            message: m.into(),
            later: false,
        }
    }

    pub fn error(m: impl Into<String>) -> Failure {
        Failure {
            kind: Kind::Error,
            message: m.into(),
            later: false,
        }
    }

    pub fn type_error(m: impl Into<String>) -> Failure {
        Failure {
            kind: Kind::TypeError,
            message: m.into(),
            later: false,
        }
    }

    fn later(mut self) -> Failure {
        self.later = true;
        self
    }

    /// as trace.mjs logs an error: `<its class>: <its message>`
    pub fn log_text(&self) -> String {
        format!("{:?}: {}", self.kind, self.message)
    }
}

impl From<UrbError> for Failure {
    fn from(e: UrbError) -> Failure {
        Failure {
            kind: Kind::UrbError,
            message: e.message,
            later: e.later,
        }
    }
}

impl From<crate::noun::TypeError> for Failure {
    fn from(e: crate::noun::TypeError) -> Failure {
        Failure::type_error(e.0)
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

/// where a datagram goes: a galaxy, which the host looks up, or an
/// address
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lane {
    Galaxy(u8),
    Ip(u32, u16),
}

impl Lane {
    /// as trace.mjs writes it: g<n>, or <ip>:<port>
    pub fn text(&self) -> String {
        match self {
            Lane::Galaxy(g) => format!("g{g}"),
            Lane::Ip(ip, port) => format!("{ip}:{port}"),
        }
    }
}

/// How a peek's noun comes back (trace.mjs's converters): a cord, a
/// file's text or a know entry's body, true, or a directory's files.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Conv {
    Cord,
    Text,
    Exists,
    Files,
}

/// A request from the app: trace.mjs's calls.
#[derive(Clone, Debug)]
pub enum Request {
    /// open as the identity in keyfile (a comet is mined with none), on
    /// the saved rows; fresh while a moon has never run, fake on a fake
    /// network
    Open {
        keyfile: Option<String>,
        rows: Option<Vec<u8>>,
        fresh: bool,
        fake: bool,
    },
    Close,
    Read(String),
    /// grub.peek: a file [folder name], or with name None the directory
    /// and all under it; wait in ms (30 s if None)
    Peek {
        ship: String,
        folder: String,
        name: Option<String>,
        conv: Conv,
        wait: Option<u64>,
    },
    /// grub.poke: [mark value] to the grub [folder name]
    Poke {
        ship: String,
        folder: String,
        name: String,
        mark: (String, String),
        value: Value<'static>,
    },
    /// grub.onInbox: hear the grubbery pokes other ships send ours
    Inbox(bool),
}

/// a page read: where it was found, its body, how it was read (keen or
/// peek) and the mark it was published under (none for /k/ and /t/)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Page {
    pub key: String,
    pub body: String,
    pub how: &'static str,
    pub mark: Option<String>,
}

/// a grubbery poke's answer
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Poked {
    pub ok: bool,
    pub error: Option<String>,
    pub later: bool,
}

/// what a peek gave, by its [`Conv`]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Peeked {
    Null,
    Text(String),
    True,
    Files(Vec<(Vec<String>, Option<String>)>),
}

/// A request's result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    Open {
        our: String,
    },
    /// close's, and onInbox's
    True,
    Read(Page),
    Poke(Poked),
    Peek(Peeked),
}

/// A grubbery poke to us, while the inbox is on
#[derive(Clone, Debug)]
pub struct Inbox {
    pub from: String,
    pub to: (String, String),
    pub mark: (String, String),
    pub noun: Noun,
}

/// What an input caused.
#[derive(Clone, Debug)]
pub enum Output {
    Send {
        lane: Lane,
        bytes: Vec<u8>,
    },
    /// rows to save, as glue.c's store hands them out, before any
    /// datagram after them goes
    Commit(Vec<u8>),
    /// a mined comet's keyfile, to keep
    SaveKey(String),
    /// the transport starts, for this ship
    Start(String),
    /// the transport closes
    TClose,
    Log(String),
    /// an HTTPS request, answered by [`Input::Fetched`] with its id
    Fetch {
        id: u64,
        url: String,
        method: String,
        headers: Vec<(String, String)>,
        body: Option<String>,
    },
    /// a request's result, by the id it was made with
    Done {
        call: u64,
        result: Result<Reply, Failure>,
    },
    Inbox(Inbox),
}

/// What the core hears.
#[derive(Clone, Debug)]
pub enum Input {
    Call {
        id: u64,
        req: Request,
    },
    Hear {
        ip: u32,
        port: u16,
        bytes: Vec<u8>,
    },
    /// the timer of this name, set for due, fires
    Tick {
        kind: String,
        due: u64,
    },
    /// a fetch's answer: its status and body, or why it failed
    Fetched {
        id: u64,
        answer: Result<(u16, Vec<u8>), String>,
    },
}

/// The core. Its state lives at a fixed address, which libmesa's hooks
/// are handed.
pub struct Core {
    w: Box<World>,
}

impl Core {
    /// a core drawing its entropy from `entropy` (n bytes a call)
    pub fn new(entropy: impl FnMut(usize) -> Vec<u8> + 'static) -> Core {
        Core {
            w: Box::new(World {
                now: Cell::new(0),
                entropy: RefCell::new(Box::new(entropy)),
                out: RefCell::new(Vec::new()),
                jobs: rt::Jobs::default(),
                timers: RefCell::new(Vec::new()),
                tid: Cell::new(0),
                fid: Cell::new(0),
                fetching: RefCell::new(Vec::new()),
                inst: RefCell::new(None),
                urb: RefCell::new(None),
            }),
        }
    }

    /// One input, at now: what it caused. Err if a tick names no timer
    /// set (the host's mistake, never another ship's).
    pub fn step(&mut self, now: u64, input: Input) -> Result<Vec<Output>, String> {
        let w = &*self.w;
        w.now.set(now);
        let r = match input {
            Input::Call { id, req } => {
                w.call(id, req);
                Ok(())
            }
            Input::Hear { ip, port, bytes } => {
                if let Some(i) = w.inst()
                    && i.eng.is_open()
                {
                    i.call(w, || {
                        let _ = i.eng.hear(ip, port, &bytes, now);
                    });
                }
                Ok(())
            }
            Input::Tick { kind, due } => {
                let t = {
                    let mut ts = w.timers.borrow_mut();
                    ts.iter()
                        .position(|t| t.kind == kind && t.due == due)
                        .map(|at| ts.remove(at))
                };
                match t {
                    Some(t) => {
                        (t.f)(w);
                        Ok(())
                    }
                    None => Err(format!("no timer {kind} is set for {due}")),
                }
            }
            Input::Fetched { id, answer } => {
                w.answer(id, answer);
                Ok(())
            }
        };
        w.jobs.drain(w);
        r.map(|()| std::mem::take(&mut *w.out.borrow_mut()))
    }

    /// the timers set: each one's name and when it is due, in the order
    /// they were set
    pub fn timers(&self) -> Vec<(String, u64)> {
        self.w
            .timers
            .borrow()
            .iter()
            .map(|t| (t.kind.clone(), t.due))
            .collect()
    }

    /// the earliest a timer is due: the host ticks it once now >= this
    pub fn deadline(&self) -> Option<u64> {
        self.w.timers.borrow().iter().map(|t| t.due).min()
    }

    /// the earliest timer due by now, to tick
    pub fn due(&self, now: u64) -> Option<(String, u64)> {
        let ts = self.w.timers.borrow();
        ts.iter()
            .filter(|t| now >= t.due)
            .min_by_key(|t| t.due)
            .map(|t| (t.kind.clone(), t.due))
    }
}

/// A hook's call as libmesa makes it, handed to the core by a test or a
/// fuzzer: a %grubbery poke from a ship (its mark and noun), a nack or an
/// ack, and a keen's answer. Keen starts a keen, as page() does, for one
/// to answer.
#[doc(hidden)]
pub enum Hook<'a> {
    Poke(&'a str, &'a str, NounRef<'a>),
    Done(&'a str, u64, Option<NounRef<'a>>),
    Sage(&'a str, &'a str, Option<NounRef<'a>>),
    Keen(&'a str, &'a str),
}

/// what the core waits on: its peeks' and pokes' wires, its keens'
/// paths, and the bones of its pokes' flows and then of its shared ones
#[doc(hidden)]
#[derive(Clone, Debug, Default)]
pub struct Waiting {
    pub peeks: Vec<String>,
    pub pokes: Vec<String>,
    pub keens: Vec<String>,
    pub bones: Vec<u64>,
}

impl Core {
    /// A hook, called as libmesa would (guarded, as the trampolines
    /// are), but outside an engine call: what it caused.
    #[doc(hidden)]
    pub fn hook(&mut self, now: u64, h: Hook<'_>) -> Vec<Output> {
        let w = &*self.w;
        w.now.set(now);
        let ship = |s: &str| engine::ship_parse(s).unwrap_or_default();
        match h {
            Hook::Poke(her, mark, noun) => {
                let m = Noun::atom(mark.as_bytes());
                engine::guard(w, ACK, |w| {
                    w.hook_poke(ship(her), 0, "grubbery", Some(m.borrow()), Some(noun))
                });
            }
            Hook::Done(her, bone, e) => engine::guard(w, (), |w| w.hook_done(ship(her), bone, e)),
            Hook::Sage(her, path, g) => engine::guard(w, (), |w| w.hook_sage(ship(her), path, g)),
            Hook::Keen(her, path) => {
                if let Some(i) = w.inst() {
                    let _ = keen(w, &i, ship(her), path.into());
                }
            }
        }
        w.jobs.drain(w);
        std::mem::take(&mut *w.out.borrow_mut())
    }

    #[doc(hidden)]
    pub fn waiting(&self) -> Waiting {
        let Some(i) = self.w.inst() else {
            return Waiting::default();
        };
        let mut bones: Vec<u64> = i.poking.borrow().keys().copied().collect();
        bones.extend(i.flows.borrow().values().copied());
        Waiting {
            peeks: i.peeks.borrow().iter().map(|(k, _)| k.clone()).collect(),
            pokes: i.pokes.borrow().iter().map(|(k, _)| k.clone()).collect(),
            keens: i.keens.borrow().keys().cloned().collect(),
            bones,
        }
    }
}

// ------------------------------------------------------------ the world

/// what a promise settles with: the values mesa.mjs's promises carry
#[derive(Clone)]
pub(crate) enum Val {
    Undef,
    Null,
    Bool(bool),
    Str(String),
    Obj(Rc<Object>),
    Promise(Promise),
    Err(Failure),
    /// a keen's answer, {mark, body}
    Gage(String, Option<String>),
    /// page()'s [body, how, mark], body None for none
    Page(Option<(String, &'static str, String)>),
    Reply(Reply),
    /// a fetch's Response
    Response(u16, Rc<Vec<u8>>),
    Bytes(Rc<Vec<u8>>),
    Files(Rc<Vec<(Vec<String>, Option<String>)>>),
    Inst(Rc<Inst>),
}

impl Val {
    fn is_null(&self) -> bool {
        matches!(self, Val::Null | Val::Undef)
    }

    fn failure(self) -> Failure {
        match self {
            Val::Err(f) => f,
            _ => Failure::error("a value was thrown"),
        }
    }
}

struct Timer {
    kind: String,
    due: u64,
    id: u64,
    f: Box<dyn FnOnce(&World)>,
}

/// trace.mjs's world and mesa.mjs's module: the clock, the entropy, the
/// timers, the fetches, and the engine open now.
pub(crate) struct World {
    now: Cell<u64>,
    entropy: RefCell<Box<dyn FnMut(usize) -> Vec<u8>>>,
    out: RefCell<Vec<Output>>,
    pub(crate) jobs: rt::Jobs,
    timers: RefCell<Vec<Timer>>,
    tid: Cell<u64>,
    fid: Cell<u64>,
    fetching: RefCell<Vec<(u64, Promise)>>,
    /// the module the hooks reach: the last one opened
    inst: RefCell<Option<Rc<Inst>>>,
    /// trace.mjs's app.urb: the engine the calls act on
    urb: RefCell<Option<Rc<Inst>>>,
}

impl Drop for World {
    fn drop(&mut self) {
        // reactions and timers hold the engine's state: let it go, and
        // close the engine, which points here
        self.jobs.clear();
        self.timers.borrow_mut().clear();
        for i in [self.inst.take(), self.urb.take()].into_iter().flatten() {
            i.shutdown();
        }
    }
}

impl World {
    fn now(&self) -> u64 {
        self.now.get()
    }

    fn emit(&self, o: Output) {
        self.out.borrow_mut().push(o);
    }

    pub(crate) fn log(&self, s: String) {
        self.emit(Output::Log(s));
    }

    fn entropy(&self, n: usize) -> Vec<u8> {
        (self.entropy.borrow_mut())(n)
    }

    fn inst(&self) -> Option<Rc<Inst>> {
        self.inst.borrow().clone()
    }

    /// setTimeout(f, ms, kind): its id
    fn set_timeout(&self, f: Box<dyn FnOnce(&World)>, ms: u64, kind: String) -> u64 {
        let id = self.tid.get() + 1;
        self.tid.set(id);
        let due = self.now() + ms;
        self.timers.borrow_mut().push(Timer { kind, due, id, f });
        id
    }

    fn clear_timeout(&self, id: u64) {
        self.timers.borrow_mut().retain(|t| t.id != id);
    }

    /// fetch(url, opts): a promise of its Response
    fn fetch(
        &self,
        url: &str,
        method: &str,
        headers: &[(&str, &str)],
        body: Option<String>,
    ) -> Promise {
        let id = self.fid.get() + 1;
        self.fid.set(id);
        self.emit(Output::Fetch {
            id,
            url: url.into(),
            method: method.into(),
            headers: headers
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            body,
        });
        let p = Promise::new();
        self.fetching.borrow_mut().push((id, p.clone()));
        p
    }

    /// a fetch's answer, as a Response would give it
    fn answer(&self, id: u64, answer: Result<(u16, Vec<u8>), String>) {
        let p = {
            let mut f = self.fetching.borrow_mut();
            let Some(at) = f.iter().position(|(i, _)| *i == id) else {
                return;
            };
            f.remove(at).1
        };
        match answer {
            Ok((status, body)) => resolve(self, &p, Ok(Val::Response(status, Rc::new(body)))),
            Err(e) => resolve(self, &p, Err(Val::Err(Failure::type_error(e)))),
        }
    }

    // -------------------------------------------------------- calls

    /// trace.mjs's call: the request's promise, or what it threw at once,
    /// and its result when it settles
    fn call(&self, id: u64, req: Request) {
        match self.request(req) {
            Err(e) => self.emit(Output::Done {
                call: id,
                result: Err(e),
            }),
            Ok(v) => {
                let p = Promise::of(v);
                then(
                    self,
                    &p,
                    Box::new(move |w, r| {
                        let result = match r {
                            Ok(Val::Reply(r)) => Ok(r),
                            // a peek's value, as its conv gave it
                            Ok(Val::Str(s)) => Ok(Reply::Peek(Peeked::Text(s))),
                            Ok(Val::Bool(true)) => Ok(Reply::Peek(Peeked::True)),
                            Ok(Val::Files(f)) => Ok(Reply::Peek(Peeked::Files((*f).clone()))),
                            Ok(Val::Null) => Ok(Reply::Peek(Peeked::Null)),
                            Ok(_) => Err(Failure::error("a call gave no result")),
                            Err(e) => Err(e.failure()),
                        };
                        w.emit(Output::Done { call: id, result });
                        Ok(Val::Undef)
                    }),
                );
            }
        }
    }

    fn urb(&self, what: &str) -> Result<Rc<Inst>, Failure> {
        self.urb.borrow().clone().ok_or_else(|| {
            Failure::type_error(format!("Cannot read properties of null (reading '{what}')"))
        })
    }

    /// trace.mjs's OPS
    fn request(&self, req: Request) -> Result<Val, Failure> {
        match req {
            Request::Open {
                keyfile,
                rows,
                fresh,
                fake,
            } => Ok(Val::Promise(spawn(self, move |w| {
                if w.urb.borrow().is_some() {
                    return Step::Throw(Val::Err(Failure::error("trace: one engine at a time")));
                }
                let p = open(w, keyfile, rows, fresh, fake);
                wait_ok(Val::Promise(p), |w, v| {
                    let Val::Inst(i) = v else {
                        return Step::Throw(Val::Err(Failure::error("open gave no engine")));
                    };
                    let our = i.our.borrow().clone();
                    *w.urb.borrow_mut() = Some(i);
                    Step::Ret(Val::Reply(Reply::Open { our }))
                })
            }))),
            Request::Close => Ok(Val::Promise(spawn(self, |w| {
                let Some(i) = w.urb.take() else {
                    return Step::Throw(Val::Err(Failure::type_error(
                        "Cannot read properties of null (reading 'close')",
                    )));
                };
                let p = close(w, i);
                wait_ok(Val::Promise(p), |w, _| {
                    // the engine is gone: its timers would only touch it
                    w.timers.borrow_mut().clear();
                    Step::Ret(Val::Reply(Reply::True))
                })
            }))),
            Request::Read(url) => {
                let i = self.urb("read")?;
                Ok(Val::Promise(read(self, &i, url)))
            }
            Request::Peek {
                ship,
                folder,
                name,
                conv,
                wait,
            } => {
                let i = self.urb("grub")?;
                let her = ship_of(&ship)?;
                let read = Read {
                    dir: name.is_none(),
                    conv: match conv {
                        Conv::Cord => Conv2::Cord,
                        Conv::Text => Conv2::Text,
                        Conv::Exists => Conv2::Exists,
                        Conv::Files => Conv2::Files,
                    },
                };
                let p = peek(
                    self,
                    &i,
                    her,
                    &folder,
                    name.as_deref(),
                    read,
                    wait.unwrap_or(PEEK_TIMEOUT),
                );
                Ok(Val::Promise(p))
            }
            Request::Poke {
                ship,
                folder,
                name,
                mark,
                value,
            } => {
                let i = self.urb("grub")?;
                let her = ship_of(&ship)?;
                Ok(Val::Promise(grub_poke(
                    self, &i, her, &folder, &name, mark, value,
                )))
            }
            Request::Inbox(on) => {
                let i = self.urb("grub")?;
                i.inbox.set(on);
                Ok(Val::Reply(Reply::True))
            }
        }
    }

    // -------------------------------------------------------- hooks

    /// hook: save what a commit changed, before the datagrams it released
    pub(crate) fn store_commit(&self, rows: Vec<u8>) -> Result<i32, Failure> {
        let Some(i) = self.inst() else {
            return Ok(0);
        };
        let saved = i.saved.borrow().clone();
        let a = on_ok(self, &saved, move |w, _| {
            w.emit(Output::Commit(rows));
            Ok(Val::Undef)
        });
        let b = on_err(self, &a, |w, e| {
            w.log(format!(
                "saving the engine's state failed: {}",
                e.failure().message
            ));
            Ok(Val::Undef)
        });
        *i.saved.borrow_mut() = b;
        Ok(0)
    }

    /// hook: send one datagram, once what was committed before it is saved
    pub(crate) fn hook_send(&self, kind: u32, gal: u8, ip: u32, port: u16, bytes: Vec<u8>) {
        let Some(i) = self.inst() else {
            return;
        };
        let lane = if kind == sys::mesa_lane_kind_MESA_LANE_GALAXY {
            Some(Lane::Galaxy(gal))
        } else if kind == sys::mesa_lane_kind_MESA_LANE_IP && ip != 0 && port != 0 {
            Some(Lane::Ip(ip, port))
        } else {
            None
        };
        if let Some(lane) = lane {
            let saved = i.saved.borrow().clone();
            on_ok(self, &saved, move |w, _| {
                w.emit(Output::Send { lane, bytes });
                Ok(Val::Undef)
            });
        }
    }

    /// hook: find a ship's keys. A fake network's come from names, once
    /// the engine call that asked has returned; a real one's from the
    /// roller.
    pub(crate) fn hook_need_point(&self, who: Ship) -> Result<(), Failure> {
        let Some(i) = self.inst() else {
            return Ok(());
        };
        if i.fake {
            let i2 = i.clone();
            self.set_timeout(
                Box::new(move |w| {
                    i2.call(w, || {
                        let _ = i2.eng.point_fake(who);
                    })
                }),
                0,
                format!("point {}", ship_text(who)),
            );
            return Ok(());
        }
        if !i.asking.borrow_mut().insert(who) {
            return Ok(());
        }
        let body = engine::point_request(who);
        // given up on after 30 s by the host's fetch; the timer is the
        // engine's, so a trace has it
        let t = self.set_timeout(
            Box::new(|_| {}),
            ROLLER_WAIT,
            format!("roller {}", ship_text(who)),
        );
        let f = self.fetch(
            ROLLER,
            "POST",
            &[("content-type", "application/json")],
            Some(body),
        );
        // .then(r => r.text())
        let a = on_ok(self, &f, |_, r| match r {
            Val::Response(_, body) => Ok(Val::Promise(Promise::of(Val::Str(js::utf8(&body))))),
            _ => Err(Val::Err(Failure::type_error("r.text is not a function"))),
        });
        // .finally(() => clearTimeout(t))
        let b = rt::finally(self, &a, move |w| w.clear_timeout(t));
        // .catch(e => (log(...), null))
        let c = on_err(self, &b, move |w, e| {
            w.log(format!(
                "azimuth lookup of {} failed: {}",
                ship_text(who),
                e.failure().message
            ));
            Ok(Val::Null)
        });
        on_ok(self, &c, move |w, js| {
            i.asking.borrow_mut().remove(&who);
            i.call(w, || {
                let _ = match &js {
                    Val::Str(s) => i.eng.point(who, Some(s.as_bytes())),
                    _ => i.eng.point(who, None),
                };
            });
            Ok(Val::Undef)
        });
        Ok(())
    }

    /// hook: a poke to us. %grubbery-transfer answers our peeks and
    /// pokes; a %grubbery-load is another ship's grubbery reading our
    /// files (%peek, %keep, %drop) or a grub there writing to ours.
    pub(crate) fn hook_poke(
        &self,
        her: Ship,
        bone: u64,
        agent: &str,
        mark: Option<NounRef<'_>>,
        noun: Option<NounRef<'_>>,
    ) -> Result<sys::mesa_plea_result, Failure> {
        let _ = bone;
        let Some(i) = self.inst() else {
            return Ok(ACK);
        };
        if agent != "grubbery" {
            return Ok(ACK);
        }
        let none = || Failure::urb("another ship's noun has none where a noun goes");
        let mark = mark.ok_or_else(none)?.text()?;
        let n = noun.ok_or_else(none)?;
        if mark == "grubbery-load" {
            // nobody waits on it: one we can't read is logged
            codec::measure(n)?;
            // [[wire dest] %poke [blot noun]], dest a rail [%& [path name]]
            let dest = n.head()?.tail()?;
            let act = n.tail()?;
            let tag = act.head()?.text()?;
            if ["peek", "keep", "drop"].contains(&tag.as_str()) {
                self.serve_load(&i, her, n.head()?.head()?, dest, &tag)?;
                return Ok(ACK);
            }
            if tag != "poke" {
                return Ok(ACK);
            }
            if i.inbox.get() && dest.head()?.int(MAX_NUMBER)?.is_zero() {
                let blot = act.tail()?.head()?;
                let msg = Inbox {
                    from: ship_text(her),
                    to: (
                        codec::path_of(dest.tail()?.head()?)?,
                        dest.tail()?.tail()?.text()?,
                    ),
                    mark: (codec::path_of(blot.head()?)?, blot.tail()?.text()?),
                    noun: act.tail()?.tail()?.to_noun(),
                };
                self.emit(Output::Inbox(msg));
            }
            // say it was taken in: the sender waits for this %pack
            let bone = i.flow_to(her)?;
            i.poke(
                her,
                bone,
                "grubbery",
                "grubbery-transfer",
                msg::pack(Value::Noun(n.head()?.head()?)),
            )?;
            return Ok(ACK);
        }
        if mark != "grubbery-transfer" {
            return Ok(ACK);
        }
        let wire = codec::path_of(n.head()?)?;
        let got = (|| -> Result<(), Failure> {
            let mut b = Budget::new();
            let m = codec::measure_in(n, &mut b)?;
            let tag = m.tail()?.head()?.noun().text()?;
            self.transfer(&i, &wire, &tag, m.tail()?.tail()?, her, n.head()?, &mut b)
        })();
        if let Err(e) = got {
            // what waits on this wire fails with it: for /want, every
            // %want out to the ship that sent it
            let (p, st) = (i.poke_at(&wire), i.peek_at(&wire));
            let wants = if wire == "/want" {
                i.wants(her)
            } else {
                Vec::new()
            };
            if let Some(p) = p {
                p.finish(
                    self,
                    &i,
                    Poked {
                        ok: false,
                        error: Some(e.message),
                        later: false,
                    },
                );
            } else if let Some(st) = st {
                st.fail(self, &i, e);
            } else if !wants.is_empty() {
                for w in wants {
                    w.fail(self, &i, e.clone());
                }
            } else {
                self.log(format!("{}: {}", ship_text(her), e.message));
            }
        }
        Ok(ACK)
    }

    /// a %grubbery-transfer [wire tag r], measured from b
    #[allow(clippy::too_many_arguments)]
    fn transfer(
        &self,
        i: &Rc<Inst>,
        wire: &str,
        tag: &str,
        r: Measured<'_>,
        her: Ship,
        wire_n: NounRef<'_>,
        b: &mut Budget,
    ) -> Result<(), Failure> {
        if let Some(p) = i.poke_at(wire) {
            // [%pack err=(unit tang)]: the grub took the poke in, or not
            let err = if tag == "pack" { r.tail().ok() } else { None };
            let res = match err {
                Some(e) => Poked {
                    ok: false,
                    error: Some(codec::tang_text(e, b)?),
                    later: false,
                },
                None => Poked {
                    ok: tag == "pack",
                    error: None,
                    later: false,
                },
            };
            p.finish(self, i, res);
            return Ok(());
        }
        if wire == "/want" {
            // +process-want answers on the fixed wire /want: match the
            // content to the snap that asked for it, of the ship that sent it
            let wants = i.wants(her);
            for st in &wants {
                let v = if tag == "data" {
                    let got = {
                        let snap = st.snap.borrow();
                        let snap = snap.as_ref().expect("a want has its snap");
                        st.read.read(snap.borrow(), r.noun())
                    };
                    match got {
                        Ok(v) => v,
                        Err(e) => {
                            st.fail(self, i, e);
                            continue;
                        }
                    }
                } else {
                    Val::Null
                };
                if !v.is_null() {
                    st.finish(self, i, v);
                }
            }
            if tag == "miss" && !wants.is_empty() {
                // the pin expired
                wants[0].finish(self, i, Val::Null);
            }
            return Ok(());
        }
        let Some(st) = i.peek_at(wire) else {
            return Ok(());
        };
        if tag == "snap" {
            // [dest snap-id snap=(unit [cass pace refs]) data=(unit silo)]
            let rest = r.noun().tail()?.tail()?;
            let snap = codec::unit(rest.head()?);
            let data = codec::unit(rest.tail()?);
            if let (Some(snap), None) = (snap, data) {
                // too big to inline: ask for the content
                let id = r.noun().tail()?.head()?.int(64)?;
                *st.snap.borrow_mut() = Some(snap.to_noun());
                i.poke(
                    her,
                    st.bone.get().unwrap_or(NONE),
                    "grubbery",
                    "grubbery-transfer",
                    msg::want(
                        Value::Noun(wire_n),
                        Value::Noun(r.noun().head()?),
                        Value::Int(id),
                    ),
                )?;
                return Ok(());
            }
            let v = match (snap, data) {
                (Some(s), Some(d)) => st.read.read(s, d)?,
                _ => Val::Null,
            };
            st.finish(self, i, v);
            return Ok(());
        }
        // %veto, %miss
        st.finish(self, i, Val::Null);
        Ok(())
    }

    /// A %peek, %keep or %drop of one of our files, in a hook: answered
    /// on our flow to the asker's %grubbery, as grubbery's +process-peek
    /// and +keep do. We serve no files yet (M4), so a peek is answered
    /// with nothing and a keep with an empty wave.
    fn serve_load(
        &self,
        i: &Rc<Inst>,
        her: Ship,
        wire: NounRef<'_>,
        dest: NounRef<'_>,
        tag: &str,
    ) -> Result<(), Failure> {
        let at = if dest.head()?.int(MAX_NUMBER)?.is_zero() {
            Some((
                codec::path_of(dest.tail()?.head()?)?,
                dest.tail()?.tail()?.text()?,
            ))
        } else {
            None
        };
        if tag == "drop" {
            return Ok(());
        }
        if tag == "peek" {
            let id = Int::from_le(&self.entropy(16));
            let bone = i.flow_to(her)?;
            return i.poke(
                her,
                bone,
                "grubbery",
                "grubbery-transfer",
                msg::snap(
                    Value::Noun(wire),
                    Value::Noun(dest),
                    Value::Int(id),
                    None,
                    None,
                ),
            );
        }
        // the first wave answers the keep, even for nothing
        let bone = i.flow_to(her)?;
        let name = at.map(|a| a.1).unwrap_or_default();
        i.poke(
            her,
            bone,
            "grubbery",
            "grubbery-intake",
            msg::intake(
                Value::Noun(wire),
                Value::Noun(dest),
                msg::wave_of(None, &name),
            ),
        )
    }

    /// hook: our poke was acked, or nacked
    pub(crate) fn hook_done(
        &self,
        her: Ship,
        bone: u64,
        error: Option<NounRef<'_>>,
    ) -> Result<(), Failure> {
        let Some(i) = self.inst() else {
            return Ok(());
        };
        let st = i.poking.borrow_mut().remove(&bone);
        if let Some(st) = st {
            // a grubbery poke's own flow: closed once answered
            let _ = i.eng.flow_cork(her, bone);
            if let Some(e) = error {
                st.finish(
                    self,
                    &i,
                    Poked {
                        ok: false,
                        error: Some(codec::tang(e)),
                        later: false,
                    },
                );
            }
            return Ok(());
        }
        let Some(e) = error else {
            return Ok(());
        };
        self.log(format!("nack from {}: {}", ship_text(her), codec::tang(e)));
        let all: Vec<Rc<PeekSt>> = i.peeks.borrow().iter().map(|(_, s)| s.clone()).collect();
        for st in all {
            if st.bone.get() == Some(bone) {
                st.finish(self, &i, Val::Null);
            }
        }
        Ok(())
    }

    /// hook: the answer to a keen
    pub(crate) fn hook_sage(
        &self,
        _her: Ship,
        path: &str,
        gage: Option<NounRef<'_>>,
    ) -> Result<(), Failure> {
        let Some(i) = self.inst() else {
            return Ok(());
        };
        let Some(k) = i.keens.borrow().get(path).cloned() else {
            return Ok(());
        };
        let Some(g) = gage else {
            k.settle(self, &i, Ok(Val::Null));
            return Ok(());
        };
        let got = (|| -> Result<Val, Failure> {
            let g = codec::measure(g)?.noun();
            Ok(Val::Gage(g.head()?.text()?, codec::cord(g.tail()?)?))
        })();
        k.settle(self, &i, got.map_err(Val::Err));
        Ok(())
    }

    /// hook: a read of our namespace; we serve none yet (M4)
    pub(crate) fn hook_peek(&self, _path: &str) -> Option<Noun> {
        None
    }
}

/// shipOf: a ship from its @p, or `not a ship`
fn ship_of(text: &str) -> Result<Ship, Failure> {
    engine::ship_parse(text).ok_or_else(|| Failure::urb(format!("not a ship: {text}")))
}

fn check(err: i32, what: &str) -> Result<(), Failure> {
    if err != 0 {
        return Err(Failure::error(format!("{what} failed: mesa error {err}")));
    }
    Ok(())
}

// ------------------------------------------------------------ the module

/// how a peek reads what it got: a file or a directory, and its conv
#[derive(Clone, Copy)]
enum Conv2 {
    /// a /pub/revs: its $json object, or {} for anything else
    Revs,
    Cord,
    Text,
    Exists,
    Files,
}

#[derive(Clone, Copy)]
struct Read {
    dir: bool,
    conv: Conv2,
}

impl Read {
    fn conv(&self, n: NounRef<'_>) -> Result<Val, Failure> {
        Ok(match self.conv {
            Conv2::Revs => match codec::json(n)? {
                Json::Obj(o) => Val::Obj(Rc::new(o)),
                _ => Val::Obj(Rc::new(Object::default())),
            },
            Conv2::Cord => codec::cord(n)?.map_or(Val::Null, Val::Str),
            Conv2::Text => {
                if n.is_cell() {
                    let h = n.head()?;
                    if h.is_cell() {
                        Val::Null
                    } else {
                        Val::Str(h.text()?)
                    }
                } else {
                    Val::Str(n.text()?)
                }
            }
            Conv2::Exists => Val::Bool(true),
            Conv2::Files => return Err(Failure::type_error("files.map is not a function")),
        })
    }

    /// what a %snap and its silo hold, as this read reads it; Null if
    /// the silo lacks it
    fn read(&self, snap: NounRef<'_>, silo: NounRef<'_>) -> Result<Val, Failure> {
        if self.dir {
            let conv = self.conv;
            let got = codec::ball_of(snap, silo, |files| -> Result<Val, Failure> {
                match conv {
                    Conv2::Files => {
                        let mut out = Vec::new();
                        for (p, n) in files {
                            out.push((p, if n.is_cell() { None } else { Some(n.text()?) }));
                        }
                        Ok(Val::Files(Rc::new(out)))
                    }
                    Conv2::Exists => Ok(Val::Bool(true)),
                    _ => Err(Failure::type_error("n.text is not a function")),
                }
            })?;
            return got.unwrap_or(Ok(Val::Null));
        }
        let got = codec::file_of(snap, silo, |n| self.conv(n))?;
        got.unwrap_or(Ok(Val::Null))
    }
}

struct PeekSt {
    wire: String,
    read: Read,
    snap: RefCell<Option<Noun>>,
    done: Cell<bool>,
    her: Ship,
    bone: Cell<Option<u64>>,
    timer: Cell<u64>,
    p: Promise,
}

impl PeekSt {
    fn settle(&self, w: &World, i: &Inst, v: Settled) {
        if self.done.get() {
            return;
        }
        self.done.set(true);
        w.clear_timeout(self.timer.get());
        i.peeks.borrow_mut().retain(|(k, _)| *k != self.wire);
        drop(self.snap.borrow_mut().take());
        resolve(w, &self.p, v);
    }

    /// Null: refused, nothing there, or no answer
    fn finish(&self, w: &World, i: &Inst, v: Val) {
        self.settle(w, i, Ok(v));
    }

    /// an answer we could not read
    fn fail(&self, w: &World, i: &Inst, e: Failure) {
        self.settle(w, i, Err(Val::Err(e)));
    }
}

struct PokeSt {
    wire: String,
    done: Cell<bool>,
    timer: Cell<u64>,
    p: Promise,
}

impl PokeSt {
    fn finish(&self, w: &World, i: &Inst, r: Poked) {
        if self.done.get() {
            return;
        }
        self.done.set(true);
        w.clear_timeout(self.timer.get());
        i.pokes.borrow_mut().retain(|(k, _)| *k != self.wire);
        resolve(w, &self.p, Ok(Val::Reply(Reply::Poke(r))));
    }
}

struct KeenSt {
    path: String,
    timer: Cell<u64>,
    p: Promise,
}

impl KeenSt {
    /// done (Ok) or fail (Err)
    fn settle(&self, w: &World, i: &Inst, v: Settled) {
        w.clear_timeout(self.timer.get());
        i.keens.borrow_mut().remove(&self.path);
        resolve(w, &self.p, v);
    }
}

/// One mesa.mjs open(): the engine and the reader's bookkeeping.
pub(crate) struct Inst {
    eng: Engine,
    journal: Box<Journal>,
    fake: bool,
    // wires never repeat across runs: a publisher can deliver a reply
    // meant for an earlier run late, and it must not match ours
    nonce: String,
    wires: Cell<u64>,
    timer: Cell<Option<u64>>,
    // each commit's rows are saved before the datagrams it released go out
    saved: RefCell<Promise>,
    our: RefCell<String>,
    peeks: RefCell<Vec<(String, Rc<PeekSt>)>>,
    pokes: RefCell<Vec<(String, Rc<PokeSt>)>>,
    // bone -> that poke: each has a flow of its own
    poking: RefCell<HashMap<u64, Rc<PokeSt>>>,
    keens: RefCell<HashMap<String, Rc<KeenSt>>>,
    // our one flow to each ship's %grubbery, for peeks and answers
    flows: RefCell<HashMap<Ship, u64>>,
    // where each ship runs lattice
    bases: RefCell<HashMap<String, &'static str>>,
    // each ship's revision list, and when it was asked for
    revs: RefCell<HashMap<String, (u64, Rc<Object>)>>,
    asking: RefCell<HashSet<Ship>>,
    inbox: Cell<bool>,
}

impl Inst {
    fn shutdown(&self) {
        self.eng.close();
        // what waits on it holds it: let go
        self.peeks.borrow_mut().clear();
        self.pokes.borrow_mut().clear();
        self.poking.borrow_mut().clear();
        self.keens.borrow_mut().clear();
    }

    fn next_wire(&self) -> u64 {
        let n = self.wires.get() + 1;
        self.wires.set(n);
        n
    }

    fn peek_at(&self, wire: &str) -> Option<Rc<PeekSt>> {
        self.peeks
            .borrow()
            .iter()
            .find(|(k, _)| k == wire)
            .map(|(_, s)| s.clone())
    }

    fn poke_at(&self, wire: &str) -> Option<Rc<PokeSt>> {
        self.pokes
            .borrow()
            .iter()
            .find(|(k, _)| k == wire)
            .map(|(_, s)| s.clone())
    }

    /// the peeks waiting on a %want's content from her
    fn wants(&self, her: Ship) -> Vec<Rc<PeekSt>> {
        self.peeks
            .borrow()
            .iter()
            .map(|(_, s)| s.clone())
            .filter(|s| s.snap.borrow().is_some() && !s.done.get() && s.her == her)
            .collect()
    }

    /// after every engine call: wake the engine at its next deadline
    fn arm(self: &Rc<Self>, w: &World) {
        if let Some(t) = self.timer.take() {
            w.clear_timeout(t);
        }
        let d = self.eng.deadline();
        if d == NONE {
            return;
        }
        let wait = d.saturating_sub(w.now()).min(WAKE_MAX);
        let me = self.clone();
        let id = w.set_timeout(
            Box::new(move |w| {
                if w.now() >= me.eng.deadline() {
                    me.call(w, || {
                        let _ = me.eng.wake(w.now());
                    });
                } else {
                    me.arm(w);
                }
            }),
            wait,
            "wake".into(),
        );
        self.timer.set(Some(id));
    }

    fn call<R>(self: &Rc<Self>, w: &World, f: impl FnOnce() -> R) -> R {
        let r = f();
        self.arm(w);
        r
    }

    /// engine calls that commit together
    fn txn<R>(
        self: &Rc<Self>,
        w: &World,
        f: impl FnOnce() -> Result<R, Failure>,
    ) -> Result<R, Failure> {
        self.call(w, || {
            check(self.eng.txn_begin()?, "txn_begin")?;
            let r = f().and_then(|r| {
                check(self.eng.txn_commit()?, "txn_commit")?;
                Ok(r)
            });
            if r.is_err() {
                self.eng.txn_abort();
            }
            r
        })
    }

    fn poke(
        &self,
        her: Ship,
        bone: u64,
        agent: &str,
        mark: &str,
        value: Value<'_>,
    ) -> Result<(), Failure> {
        let n = Noun::of(&value)?;
        check(self.eng.poke(her, bone, agent, mark, n)?, "poke")
    }

    /// our one flow to her %grubbery, for peeks and answers
    fn flow_to(&self, her: Ship) -> Result<u64, Failure> {
        if let Some(&b) = self.flows.borrow().get(&her) {
            return Ok(b);
        }
        let bone = self.eng.flow_open(her)?;
        if bone == NONE {
            return Err(Failure::error("flow_open failed"));
        }
        self.flows.borrow_mut().insert(her, bone);
        Ok(bone)
    }
}

impl Drop for Inst {
    fn drop(&mut self) {
        self.eng.close();
    }
}

// ------------------------------------------------------------ opening

/// mesa.mjs's open(): the module, the identity (a comet mined if there
/// is none), the transport, and the engine on the saved rows
fn open(
    w: &World,
    keyfile: Option<String>,
    rows: Option<Vec<u8>>,
    fresh: bool,
    fake: bool,
) -> Promise {
    // the module is instantiated: a wait
    spawn(w, move |_| {
        wait(Val::Undef, move |w, _| {
            let world = w as *const World;
            let nonce: String = w.entropy(4).iter().map(|b| format!("{b:02x}")).collect();
            let i = Rc::new(Inst {
                eng: Engine::none(),
                journal: Journal::new(world),
                fake,
                nonce,
                wires: Cell::new(0),
                timer: Cell::new(None),
                saved: RefCell::new(Promise::of(Val::Undef)),
                our: RefCell::new(String::new()),
                peeks: RefCell::new(Vec::new()),
                pokes: RefCell::new(Vec::new()),
                poking: RefCell::new(HashMap::new()),
                keens: RefCell::new(HashMap::new()),
                flows: RefCell::new(HashMap::new()),
                bases: RefCell::new(HashMap::new()),
                revs: RefCell::new(HashMap::new()),
                asking: RefCell::new(HashSet::new()),
                inbox: Cell::new(false),
            });
            if let Some(old) = w.inst.replace(Some(i.clone()))
                && !w.urb.borrow().as_ref().is_some_and(|u| Rc::ptr_eq(u, &old))
            {
                old.shutdown();
            }
            // await store.load()
            wait(Val::Undef, move |w, _| match keyfile {
                Some(k) => keyed(w, i, k, rows, fresh),
                None => {
                    // 403 without a user-agent, outside a browser
                    let f = w.fetch(STARS, "GET", &[("user-agent", "urb-fetch")], None);
                    wait_ok(Val::Promise(f), move |_, res| {
                        let Val::Response(status, body) = res else {
                            return Step::Throw(Val::Err(Failure::type_error("no Response")));
                        };
                        if !(200..300).contains(&status) {
                            return Step::Throw(Val::Err(Failure::error(format!(
                                "{STARS}: {status}"
                            ))));
                        }
                        // await res.arrayBuffer()
                        wait_ok(Val::Bytes(body), move |w, jam| {
                            let Val::Bytes(jam) = jam else {
                                return Step::Throw(Val::Err(Failure::error("no body")));
                            };
                            let eny: [u8; 64] = w.entropy(64).try_into().unwrap_or([0; 64]);
                            let Some(k) = engine::mine_comet(&jam, &eny) else {
                                return Step::Throw(Val::Err(Failure::error(
                                    "mining a comet failed",
                                )));
                            };
                            w.emit(Output::SaveKey(k.clone()));
                            // await store.saveKey(keyfile)
                            wait(Val::Undef, move |w, _| keyed(w, i, k, None, fresh))
                        })
                    })
                }
            })
        })
    })
}

/// open(), once it has a keyfile
fn keyed(w: &World, i: Rc<Inst>, keyfile: String, rows: Option<Vec<u8>>, fresh: bool) -> Step {
    let Some((us, _, _)) = engine::keyfile_ship(&keyfile) else {
        return Step::Throw(Val::Err(Failure::error("the saved keyfile is not one")));
    };
    let our = ship_text(us);
    *i.our.borrow_mut() = our.clone();
    // the transport is up before the engine opens: opening can send
    w.emit(Output::Start(our.clone()));
    wait(Val::Undef, move |w, _| {
        let rows = rows.filter(|r| !r.is_empty());
        if let Some(r) = &rows
            && i.journal.load(r).is_err()
        {
            return Step::Throw(Val::Err(Failure::error("store_load failed: mesa error -1")));
        }
        // a fake ship is rebooted fresh, so it may always start from empty
        let breached = i.fake || (fresh && rows.is_none());
        let store = i.journal.store();
        let err = i.call(w, || unsafe {
            i.eng.open(
                &keyfile,
                w.now(),
                breached,
                i.fake,
                w as *const World,
                store,
            )
        });
        if err == sys::mesa_err_MESA_E_FRESH {
            w.emit(Output::TClose);
            return Step::Throw(Val::Err(Failure::error(format!(
                "{our} has run before, and its state here is empty. Starting it from nothing \
                 would replay old messages, so breach it on its parent (|moon-breach) and add \
                 its new key."
            ))));
        }
        if let Err(e) = check(err, "open") {
            return Step::Throw(Val::Err(e));
        }
        Step::Ret(Val::Inst(i))
    })
}

/// close(): once everything committed is saved and sent, and 500 ms more
fn close(w: &World, i: Rc<Inst>) -> Promise {
    spawn(w, move |w| {
        if let Some(t) = i.timer.take() {
            w.clear_timeout(t);
        }
        let saved = i.saved.borrow().clone();
        wait(Val::Promise(saved), move |w, _| {
            let p = Promise::new();
            let p2 = p.clone();
            w.set_timeout(
                Box::new(move |w| resolve(w, &p2, Ok(Val::Undef))),
                CLOSE_WAIT,
                "close".into(),
            );
            wait(Val::Promise(p), move |w, _| {
                w.emit(Output::TClose);
                i.shutdown();
                Step::Ret(Val::Undef)
            })
        })
    })
}

// ------------------------------------------------------------ the reader

fn throw(f: Failure) -> Step {
    Step::Throw(Val::Err(f))
}

fn rejected(w: &World, e: Val) -> Promise {
    let p = Promise::new();
    resolve(w, &p, Err(e));
    p
}

/// Grubbery's cross-ship peek of one file [folder name], or with name
/// None of the directory and all under it (a deep peek). Null after
/// wait ms.
fn peek(
    w: &World,
    i: &Rc<Inst>,
    her: Ship,
    folder: &str,
    name: Option<&str>,
    read: Read,
    wait: u64,
) -> Promise {
    let wire = format!("/urb-fetch/{}/{}", i.nonce, i.next_wire());
    let p = Promise::new();
    let st = Rc::new(PeekSt {
        wire: wire.clone(),
        read,
        snap: RefCell::new(None),
        done: Cell::new(false),
        her,
        bone: Cell::new(None),
        timer: Cell::new(0),
        p: p.clone(),
    });
    let (st2, i2) = (st.clone(), i.clone());
    st.timer.set(w.set_timeout(
        Box::new(move |w| st2.finish(w, &i2, Val::Null)),
        wait,
        format!("peek {wire}"),
    ));
    i.peeks.borrow_mut().push((wire.clone(), st.clone()));
    let sent = i.txn(w, || {
        let bone = i.flow_to(her)?;
        st.bone.set(Some(bone));
        i.poke(
            her,
            bone,
            "grubbery",
            "grubbery-load",
            msg::peek(&wire, folder, name),
        )
    });
    if let Err(e) = sent {
        resolve(w, &p, Err(Val::Err(e)));
    }
    p
}

/// grubbery's cross-ship poke, on a flow of its own: {ok} once the grub
/// took it in, or {ok: false, error}: refused, or no answer in time
fn grub_poke(
    w: &World,
    i: &Rc<Inst>,
    her: Ship,
    folder: &str,
    name: &str,
    mark: (String, String),
    value: Value<'static>,
) -> Promise {
    let wire = format!("/grub-poke/{}/{}", i.nonce, i.next_wire());
    let p = Promise::new();
    let st = Rc::new(PokeSt {
        wire: wire.clone(),
        done: Cell::new(false),
        timer: Cell::new(0),
        p: p.clone(),
    });
    let (st2, i2) = (st.clone(), i.clone());
    st.timer.set(w.set_timeout(
        Box::new(move |w| {
            st2.finish(
                w,
                &i2,
                Poked {
                    ok: false,
                    error: Some(format!("{} did not answer", ship_text(her))),
                    later: true,
                },
            )
        }),
        PEEK_TIMEOUT,
        format!("poke {wire}"),
    ));
    i.pokes.borrow_mut().push((wire.clone(), st.clone()));
    let sent = i.txn(w, || {
        let bone = i.eng.flow_open(her)?;
        if bone == NONE {
            return Err(Failure::error("flow_open failed"));
        }
        i.poking.borrow_mut().insert(bone, st.clone());
        i.poke(
            her,
            bone,
            "grubbery",
            "grubbery-load",
            msg::poke(&wire, folder, name, (&mark.0, &mark.1), value),
        )
    });
    if let Err(e) = sent {
        resolve(w, &p, Err(Val::Err(e)));
    }
    p
}

/// One keen at a time per path; readers asking the same share it. A
/// miss never answers: after 10 s it is yawned, and gives Null.
fn keen(w: &World, i: &Rc<Inst>, her: Ship, path: String) -> Result<Promise, Failure> {
    if let Some(k) = i.keens.borrow().get(&path) {
        return Ok(k.p.clone());
    }
    let p = Promise::new();
    let k = Rc::new(KeenSt {
        path: path.clone(),
        timer: Cell::new(0),
        p: p.clone(),
    });
    let (i2, p2, path2) = (i.clone(), p.clone(), path.clone());
    k.timer.set(w.set_timeout(
        Box::new(move |w| {
            // stop asking, as +keen-page-raw %yawns
            i2.keens.borrow_mut().remove(&path2);
            let _ = i2.txn(w, || i2.eng.yawn(her, &path2));
            resolve(w, &p2, Ok(Val::Null));
        }),
        KEEN_TIMEOUT,
        format!("keen {path}"),
    ));
    i.keens.borrow_mut().insert(path.clone(), k);
    i.txn(w, || check(i.eng.keen(her, &path)?, "keen"))?;
    Ok(p)
}

/// A peer's /pub/revs, or Null if it has none there. One we can't read
/// is an empty list, as lattice's +fetch-peer-revs reads it.
fn peek_revs(w: &World, i: &Rc<Inst>, her: Ship, base: &str) -> Promise {
    let p = peek(
        w,
        i,
        her,
        &format!("{base}/pub"),
        Some("revs"),
        Read {
            dir: false,
            conv: Conv2::Revs,
        },
        PEEK_TIMEOUT,
    );
    on_err(w, &p, |w, e| match &e {
        Val::Err(f) if f.kind == Kind::UrbError => Ok(Val::Obj(Rc::new(Object::default()))),
        _ => Ok(Val::Promise(rejected(w, e))),
    })
}

/// +peer-base: where the peer runs lattice, asked, not guessed
fn base(w: &World, i: &Rc<Inst>, her: Ship, who: String) -> Promise {
    let i = i.clone();
    spawn(w, move |w| {
        if let Some(b) = i.bases.borrow().get(&who) {
            return Step::Ret(Val::Str(b.to_string()));
        }
        base_try(w, i, her, who, 0)
    })
}

fn base_try(w: &World, i: Rc<Inst>, her: Ship, who: String, k: usize) -> Step {
    let bases = [DESK_BASE, APP_BASE];
    if k == bases.len() {
        return base_done(i, who);
    }
    let b = bases[k];
    let p = peek_revs(w, &i, her, b);
    wait_ok(Val::Promise(p), move |w, got| {
        if got.is_null() {
            return base_try(w, i, her, who, k + 1);
        }
        let list = match got {
            Val::Obj(o) => o,
            _ => Rc::new(Object::default()),
        };
        i.bases.borrow_mut().insert(who.clone(), b);
        i.revs.borrow_mut().insert(who.clone(), (w.now(), list));
        base_done(i, who)
    })
}

fn base_done(i: Rc<Inst>, who: String) -> Step {
    match i.bases.borrow().get(&who) {
        Some(b) => Step::Ret(Val::Str(b.to_string())),
        // later: a ship catching up on a comet that moved answers in a
        // minute or two
        None => throw(
            Failure::urb(format!(
                "{who} answered no lattice peek (offline, or no lattice)"
            ))
            .later(),
        ),
    }
}

/// +peer-revs-cached
fn rev_list(w: &World, i: &Rc<Inst>, her: Ship, who: String) -> Promise {
    let i = i.clone();
    spawn(w, move |w| {
        let p = base(w, &i, her, who.clone());
        wait_ok(Val::Promise(p), move |w, b| {
            let Val::Str(b) = b else {
                return throw(Failure::error("no base"));
            };
            let r = i.revs.borrow().get(&who).cloned();
            match r {
                // now() - r.at > REVS_TTL, in the JS's numbers
                Some((at, list)) if (w.now() as i128 - at as i128) <= REVS_TTL as i128 => {
                    Step::Ret(Val::Obj(list))
                }
                _ => {
                    // {at: now(), list: await ...}: at is read first
                    let at = w.now();
                    let p = peek_revs(w, &i, her, &b);
                    wait_ok(Val::Promise(p), move |_, got| {
                        let list = match got {
                            Val::Obj(o) => o,
                            _ => Rc::new(Object::default()),
                        };
                        i.revs.borrow_mut().insert(who, (at, list.clone()));
                        Step::Ret(Val::Obj(list))
                    })
                }
            }
        })
    })
}

/// a JS value's truthiness
fn truthy(j: Option<&Json>) -> bool {
    match j {
        None | Some(Json::Null) => false,
        Some(Json::Bool(b)) => *b,
        Some(Json::Num(n)) => *n != 0.0 && !n.is_nan(),
        Some(Json::Str(s)) => !s.is_empty(),
        Some(Json::Arr(_) | Json::Obj(_)) => true,
    }
}

/// String(x) of a JS value (None is undefined)
fn js_string(j: Option<&Json>) -> String {
    match j {
        None => "undefined".into(),
        Some(Json::Null) => "null".into(),
        Some(Json::Bool(b)) => b.to_string(),
        Some(Json::Num(n)) => js::number_text(*n),
        Some(Json::Str(s)) => s.clone(),
        Some(Json::Arr(a)) => a
            .iter()
            .map(|x| match x {
                Json::Null => String::new(),
                x => js_string(Some(x)),
            })
            .collect::<Vec<_>>()
            .join(","),
        Some(Json::Obj(_)) => "[object Object]".into(),
    }
}

/// +read-page-body-rev: keen the listed id, else peek the vault file.
/// [body, how, mark]. The vault file is %gmi whatever the page's format.
fn page(w: &World, i: &Rc<Inst>, her: Ship, who: String, rel: Vec<String>) -> Promise {
    let i = i.clone();
    spawn(w, move |w| {
        let key = format!("/{}", rel.join("/"));
        let p = rev_list(w, &i, her, who.clone());
        wait_ok(Val::Promise(p), move |w, list| {
            let Val::Obj(list) = list else {
                return throw(Failure::error("no list"));
            };
            let ent = list.get(&key);
            if !truthy(ent) {
                return vault(w, i, her, who, key);
            }
            let id = match ent {
                Some(Json::Obj(o)) => js_string(o.get("id")),
                _ => js_string(None),
            };
            let k = match keen(w, &i, her, format!("/g/x/1/grubbery//1/pub/page{key}/{id}")) {
                Ok(k) => k,
                Err(e) => return throw(e),
            };
            wait_ok(Val::Promise(k), move |w, got| {
                if let Val::Gage(mark, Some(body)) = &got
                    && PAGE_MARKS.contains(&mark.as_str())
                {
                    return Step::Ret(Val::Page(Some((body.clone(), "keen", mark.clone()))));
                }
                // a miss, or a %del tombstone: the list is stale
                i.revs.borrow_mut().remove(&who);
                vault(w, i, her, who, key)
            })
        })
    })
}

/// page()'s vault peek
fn vault(w: &World, i: Rc<Inst>, her: Ship, who: String, key: String) -> Step {
    let p = base(w, &i, her, who);
    wait_ok(Val::Promise(p), move |w, b| {
        let Val::Str(b) = b else {
            return throw(Failure::error("no base"));
        };
        let read = Read {
            dir: false,
            conv: Conv2::Cord,
        };
        let p = peek(
            w,
            &i,
            her,
            &format!("{b}/pub/vault{key}"),
            Some("gmi"),
            read,
            PEEK_TIMEOUT,
        );
        wait_ok(Val::Promise(p), |_, body| match body {
            Val::Str(s) => Step::Ret(Val::Page(Some((s, "peek", "gmi".into())))),
            _ => Step::Ret(Val::Page(None)),
        })
    })
}

/// a file in the publisher's own tree, which it shares with whom it
/// chooses: text, or a know entry's body
fn read_private(
    w: &World,
    i: &Rc<Inst>,
    her: Ship,
    url: String,
    folder: String,
    name: String,
) -> Promise {
    let i = i.clone();
    spawn(w, move |w| {
        let read = Read {
            dir: false,
            conv: Conv2::Text,
        };
        let p = peek(w, &i, her, &folder, Some(&name), read, PEEK_TIMEOUT);
        wait_ok(Val::Promise(p), move |_, body| match body {
            Val::Str(body) => Step::Ret(Val::Reply(Reply::Read(Page {
                key: format!("{folder}/{name}"),
                body,
                how: "peek",
                mark: None,
            }))),
            _ => throw(Failure::urb(format!(
                "{url}: nothing there, or not shared with {}",
                i.our.borrow()
            ))),
        })
    })
}

/// +read-page-or-index for one url
fn read(w: &World, i: &Rc<Inst>, url: String) -> Promise {
    let i = i.clone();
    spawn(w, move |w| {
        let u = match url::de_urb(&url) {
            Ok(u) => u,
            Err(e) => return throw(e.into()),
        };
        let her = match ship_of(&u.ship) {
            Ok(h) => h,
            Err(e) => return throw(e),
        };
        let who = u.ship.clone();
        let rel = u.rel;
        match u.mount {
            Mount::P => throw(Failure::urb(format!(
                "{url} is a live Lattice page, which only Lattice shows"
            ))),
            Mount::Forum { .. } => throw(Failure::urb(format!(
                "{url} is a forum board: forum.mjs reads it"
            ))),
            Mount::K => {
                if rel.is_empty() {
                    return throw(Failure::urb(format!("{url} names no know entry")));
                }
                let p = base(w, &i, her, who);
                wait_ok(Val::Promise(p), move |w, b| {
                    let Val::Str(b) = b else {
                        return throw(Failure::error("no base"));
                    };
                    let folder = format!("{b}/know/vault/{}", rel.join("/"));
                    Step::Ret(Val::Promise(read_private(
                        w,
                        &i,
                        her,
                        url,
                        folder,
                        "entry".into(),
                    )))
                })
            }
            Mount::T => {
                let Some((name, dirs)) = rel.split_last() else {
                    return throw(Failure::urb(format!(
                        "{url} is the publisher's whole tree, not a file"
                    )));
                };
                let folder = format!("/{}", dirs.join("/"));
                let name = name.clone();
                Step::Ret(Val::Promise(read_private(w, &i, her, url, folder, name)))
            }
            Mount::N => {
                let p = rev_list(w, &i, her, who.clone());
                wait_ok(Val::Promise(p), move |w, list| {
                    let Val::Obj(list) = list else {
                        return throw(Failure::error("no list"));
                    };
                    let hit = |p: &[String]| list.get(&format!("/{}", p.join("/"))).is_some();
                    let mut dex = rel.clone();
                    dex.push("index".into());
                    let tries = if rel.is_empty() || rel.last().map(String::as_str) == Some("index")
                    {
                        vec![rel]
                    } else if !hit(&rel) && hit(&dex) {
                        vec![dex]
                    } else if !list.is_empty() && !hit(&rel) {
                        vec![rel]
                    } else {
                        vec![rel, dex]
                    };
                    try_pages(w, i, her, who, url, tries, 0)
                })
            }
        }
    })
}

fn try_pages(
    w: &World,
    i: Rc<Inst>,
    her: Ship,
    who: String,
    url: String,
    tries: Vec<Vec<String>>,
    k: usize,
) -> Step {
    let Some(p) = tries.get(k).cloned() else {
        return throw(Failure::urb(format!("{url}: not published")));
    };
    let key = format!("/{}", p.join("/"));
    let pg = page(w, &i, her, who.clone(), p);
    wait_ok(Val::Promise(pg), move |w, got| match got {
        Val::Page(Some((body, how, mark))) => Step::Ret(Val::Reply(Reply::Read(Page {
            key,
            body,
            how,
            mark: Some(mark),
        }))),
        _ => try_pages(w, i, her, who, url, tries, k + 1),
    })
}
