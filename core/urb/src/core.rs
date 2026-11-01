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
//! are trace.mjs's calls (open, read, close, peek, poke, pub.mjs's, and
//! furum.mjs's and forum.mjs's in apps), with their
//! results and errors, word for word; the timeouts are the JS's (keen
//! 10 s then %yawn, peek and poke 30 s, the revision list kept 60 s), and
//! the outputs of each input come in the JS's order (see [`crate::rt`]).

use crate::codec::{self, Budget, MAX_NUMBER, Measured};
use crate::engine::{self, Engine, Journal, Ship, ship_text};
use crate::js::{self, Js, Json, Object};
use crate::msg::{self, File};
use crate::noun::{self, Noun, NounRef, Value};
use crate::publish::{self, PubState};
use crate::rt::{self, Promise, Settled, Step, on_err, on_ok, resolve, spawn, then, wait, wait_ok};
use crate::url::{self, APP_BASE, DESK_BASE, Mount, PAGE_MARKS};
use crate::{Int, UrbError};

mod apps;
pub use apps::{ForumCall, FurumCall};
use apps::{ForumSt, Listener};
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
/// a cap on the keeps we answer, so a ship can't grow our state without
/// bound; past it a keep gets its first wave and no news
pub const MAX_KEEPERS: usize = 256;
/// and each ship's share of them
pub const PER_SHIP: usize = 64;
/// how long close() waits after the last save
pub const CLOSE_WAIT: u64 = 500;
/// the longest the wake timer is set for: waking early re-arms it
const WAKE_MAX: u64 = 60_000;
/// a key lookup's limit
const ROLLER_WAIT: u64 = 30_000;
pub const ROLLER: &str = "https://roller.urbit.org/v1/azimuth";
pub const STARS: &str = "https://bootstrap.urbit.org/comet-stars.jam";
const NONE: u64 = sys::MESA_NONE;
/// reader.go's lateWait: how long a revision list's peek its caller gave
/// up on still waits for its answer (Request::Gateway only)
pub const LATE_WAIT: u64 = 300_000;
const ACK: sys::mesa_plea_result = sys::mesa_plea_result_MESA_ACK;

// ------------------------------------------------------------ the face

/// What threw, as the JS names it: an `UrbError` (with `later` where the
/// JS sets it), an `Error` or a `TypeError`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    UrbError,
    Error,
    TypeError,
    RangeError,
    SyntaxError,
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

    pub fn range_error(m: impl Into<String>) -> Failure {
        Failure {
            kind: Kind::RangeError,
            message: m.into(),
            later: false,
        }
    }

    pub fn syntax_error(m: impl Into<String>) -> Failure {
        Failure {
            kind: Kind::SyntaxError,
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
    /// publish(urb, {pages, keepers, save}): serve the pages, from
    /// meta.pub as it was saved; each save is an [`Output::Event`]
    /// `pub.save`
    Pub {
        pages: Option<Object>,
        keepers: Option<Json>,
    },
    /// pub.put(name, body, mark): mark None is gmi
    PubPut {
        name: String,
        body: String,
        mark: Option<String>,
    },
    /// pub.remove(name)
    PubRemove(String),
    /// pub.pages()
    PubPages,
    /// log what other ships ask of what publish() serves, as
    /// wasm/pub-cli.mjs does by wrapping grub.serve and scry.serve: each
    /// file looked up, each change of the keeps and each scry is a
    /// [`Output::Log`] in pub-cli's words. For the native command line;
    /// no trace sets it.
    Watch(bool),
    /// Read as the Go gateway's reader (reader.go) reads, where it
    /// differs from mesa.mjs's, for the native gateway (M8), whose HTTP
    /// answers must stay the Go gateway's: a base probe nobody answers
    /// ends the probe, and each failure has the Go's words (see
    /// [`gateway_status`]); a revision list nobody answers keeps the
    /// stale one; a list's answer that comes after its caller gave up
    /// is kept, for [`LATE_WAIT`]; a list's entry with no id is no
    /// entry. No trace sets it.
    Gateway(bool),
    /// furum(urb, host)'s calls
    Furum {
        host: String,
        call: FurumCall,
    },
    /// forum(urb, host)'s calls
    Forum {
        host: String,
        call: ForumCall,
    },
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
#[derive(Clone, Debug, PartialEq)]
pub enum Reply {
    Open {
        our: String,
    },
    /// close's, and onInbox's
    True,
    Read(Page),
    Poke(Poked),
    Peek(Peeked),
    /// a JS value: pub.put's record (or null), pub.remove's boolean,
    /// pub.pages()
    Json(Json),
    /// what a furum or forum call gives
    Js(Js),
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
    /// an app's event: `pub.save` hands {pages, keepers} to keep
    Event {
        name: String,
        value: Json,
    },
    /// an app's note: `furum.note` hands {from, title, body}, and
    /// `forum.note` {host, board, reason, at}
    Note {
        name: String,
        value: Js,
    },
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
                publ: RefCell::new(None),
                forums: RefCell::new(HashMap::new()),
                watch: Cell::new(false),
                gateway: Cell::new(false),
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
/// ack, a keen's answer, and another ship's read of our namespace (what
/// it would be answered with is given as a [`Output::Log`] of its jam's
/// length, or none). Keen starts a keen, as page() does, for one to
/// answer.
#[doc(hidden)]
pub enum Hook<'a> {
    Poke(&'a str, &'a str, NounRef<'a>),
    Done(&'a str, u64, Option<NounRef<'a>>),
    Sage(&'a str, &'a str, Option<NounRef<'a>>),
    Keen(&'a str, &'a str),
    Peek(&'a str),
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
            Hook::Peek(path) => {
                if let Some(n) = engine::guard(w, None, |w| w.hook_peek(path)) {
                    w.log(format!("scry: {} bytes", n.jam().len()));
                }
            }
        }
        w.jobs.drain(w);
        std::mem::take(&mut *w.out.borrow_mut())
    }

    /// the revision list cached for a ship, as (page, id), if any
    #[doc(hidden)]
    pub fn cached_revs(&self, who: &str) -> Option<Vec<(String, String)>> {
        let i = self.w.inst()?;
        let revs = i.revs.borrow();
        let (_, list) = revs.get(who)?;
        Some(
            list.entries()
                .into_iter()
                .map(|(k, v)| {
                    let id = match v {
                        Json::Obj(o) => js::string(o.get("id")),
                        _ => String::new(),
                    };
                    (k.to_string(), id)
                })
                .collect(),
        )
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
    /// an app's result
    Js(Rc<Js>),
    /// what an app's conv read
    Any(Rc<dyn std::any::Any>),
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
    /// trace.mjs's app.pub: what publish() gave
    publ: RefCell<Option<Rc<Pub>>>,
    /// trace.mjs's app.forums: forum(urb, host) for each host
    forums: RefCell<HashMap<String, Rc<ForumSt>>>,
    /// Request::Watch
    watch: Cell<bool>,
    /// Request::Gateway
    pub(crate) gateway: Cell<bool>,
}

impl Drop for World {
    fn drop(&mut self) {
        // reactions and timers hold the engine's state: let it go, and
        // close the engine, which points here
        self.jobs.clear();
        self.timers.borrow_mut().clear();
        self.publ.take();
        self.forums.borrow_mut().clear();
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
                            Ok(Val::Js(j)) => Ok(Reply::Js((*j).clone())),
                            Ok(Val::Any(a)) => match a.downcast_ref::<crate::forum::board::Board>()
                            {
                                Some(b) => Ok(Reply::Js(b.js())),
                                None => Err(Failure::error("a call gave no result")),
                            },
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

    fn publ(&self, what: &str) -> Result<Rc<Pub>, Failure> {
        self.publ.borrow().clone().ok_or_else(|| {
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
                w.publ.take();
                w.forums.borrow_mut().clear();
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
                Ok(Val::Promise(grub_poke_with(
                    self,
                    &i,
                    her,
                    &folder,
                    &name,
                    mark,
                    Ok(value),
                )))
            }
            Request::Inbox(on) => {
                let i = self.urb("grub")?;
                *i.inbox.borrow_mut() = on.then_some(Listener::Emit);
                Ok(Val::Reply(Reply::True))
            }
            Request::Furum { host, call } => self.furum(host, call),
            Request::Forum { host, call } => self.forum(host, call),
            Request::Pub { pages, keepers } => {
                let i = self.urb("grub")?;
                *self.publ.borrow_mut() = Some(Pub::publish(&i, PubState::new(pages, keepers)));
                Ok(Val::Reply(Reply::True))
            }
            Request::PubPut { name, body, mark } => {
                let p = self.publ("put")?;
                let r = p.put(self, &name, &body, mark.as_deref().unwrap_or("gmi"))?;
                Ok(Val::Reply(Reply::Json(r)))
            }
            Request::PubRemove(name) => {
                let p = self.publ("remove")?;
                let r = p.remove(self, &name)?;
                Ok(Val::Reply(Reply::Json(Json::Bool(r))))
            }
            Request::PubPages => {
                let p = self.publ("pages")?;
                let r = Json::Obj(p.st.borrow().pages.clone());
                Ok(Val::Reply(Reply::Json(r)))
            }
            Request::Watch(on) => {
                self.watch.set(on);
                Ok(Val::Reply(Reply::True))
            }
            Request::Gateway(on) => {
                self.gateway.set(on);
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
            let m = codec::measure(n)?;
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
            let listener = i.inbox.borrow().clone();
            if let Some(l) = listener
                && dest.head()?.int(MAX_NUMBER)?.is_zero()
            {
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
                self.hear_inbox(&l, msg, m.tail()?.tail()?.tail()?);
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
                        st.read.read(snap.borrow(), r)
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
                // v !== null: an app's undefined finishes it
                if !matches!(v, Val::Null) {
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
            let rest = r.tail()?.tail()?;
            let snap = codec::unit(rest.head()?.noun());
            let data = rest.tail()?.tail().ok();
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
    /// and +keep do. Only files the host serves are ever answered with
    /// content.
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
        let f = match &at {
            Some((folder, name)) => i.file(self, folder, name)?,
            None => None,
        };
        let ship = ship_text(her);
        let k = at.as_ref().map(|(fo, n)| format!("{ship} {fo} {n}"));
        if tag == "drop" {
            if let Some(k) = k
                && i.server.borrow_mut().keepers.remove(&k)
            {
                i.keepers_changed(self);
            }
            return Ok(());
        }
        if tag == "peek" {
            let (snap, data) = match &f {
                Some(f) if f.value.is_some() => {
                    let (s, d) = msg::snap_of(f)?;
                    (Some(s), Some(d))
                }
                _ => (None, None),
            };
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
                    snap,
                    data,
                ),
            );
        }
        // A keep may wait for a page not yet published, or one taken
        // down and put up again, as lattice's reader keeps it. So that one
        // ship can't fill every slot, each ship has PER_SHIP of them,
        // inside MAX_KEEPERS; a keep held already is never evicted, and
        // one held already and unchanged isn't saved again.
        if let (Some(_), Some(k), Some((folder, name))) = (&f, &k, &at) {
            let room = {
                let s = i.server.borrow();
                s.keepers.get(k).is_some()
                    || (s.keepers.len() < MAX_KEEPERS && s.keepers.kept_by(&ship) < PER_SHIP)
            };
            if room {
                let mut o = Object::default();
                o.set("ship".into(), Json::Str(ship.clone()));
                let segs = codec::segments(wire)?;
                o.set(
                    "wire".into(),
                    Json::Arr(segs.into_iter().map(Json::Str).collect()),
                );
                o.set("folder".into(), Json::Str(folder.clone()));
                o.set("name".into(), Json::Str(name.clone()));
                let kept = Json::Obj(o);
                // JSON.stringify(keepers.get(k)) !== JSON.stringify(kept)
                let same = i
                    .server
                    .borrow()
                    .keepers
                    .get(k)
                    .is_some_and(|old| js::stringify(old) == js::stringify(&kept));
                if !same {
                    i.server.borrow_mut().keepers.set(k.clone(), kept);
                    i.keepers_changed(self);
                }
            }
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
                msg::wave_of(f.as_ref(), &name)?,
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

    /// hook: a read of our namespace, a %keen from another ship
    pub(crate) fn hook_peek(&self, path: &str) -> Result<Option<Noun>, Failure> {
        let Some(i) = self.inst() else {
            return Ok(None);
        };
        let scries = i.server.borrow().scries.clone();
        match scries {
            Some(f) => {
                let got = f(self, path)?;
                if self.watch.get() {
                    let what = if got.is_some() { "answered" } else { "nothing" };
                    self.log(format!("scry {path}: {what}"));
                }
                match got {
                    Some(v) => Ok(Some(Noun::of(&v)?)),
                    None => Ok(None),
                }
            }
            None => Ok(None),
        }
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
#[derive(Clone)]
enum Conv2 {
    /// a /pub/revs: its $json object, or {} for anything else
    Revs,
    Cord,
    Text,
    Exists,
    Files,
    /// an app's own reading of a file (furum's and forum's)
    File(FileConv),
    /// an app's own reading of a directory's files
    Dir(DirConv),
}

/// a file's noun, read during its peek's hook: a noun is only borrowed
pub(crate) type FileConv = Rc<dyn for<'a> Fn(Measured<'a>) -> Result<Val, Failure>>;
/// a directory's files, [path, noun], read during its peek's hook
pub(crate) type DirConv =
    Rc<dyn for<'a> Fn(Vec<(Vec<String>, Measured<'a>)>) -> Result<Val, Failure>>;

#[derive(Clone)]
struct Read {
    dir: bool,
    conv: Conv2,
}

impl Read {
    fn conv(&self, m: Measured<'_>) -> Result<Val, Failure> {
        let n = m.noun();
        Ok(match &self.conv {
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
            Conv2::File(f) => return f(m),
            Conv2::Files | Conv2::Dir(_) => {
                return Err(Failure::type_error("files.map is not a function"));
            }
        })
    }

    /// what a %snap and its silo hold, as this read reads it; Null if
    /// the silo lacks it
    fn read(&self, snap: NounRef<'_>, silo: Measured<'_>) -> Result<Val, Failure> {
        if self.dir {
            let conv = self.conv.clone();
            let got = codec::ball_of(snap, silo.noun(), |files| -> Result<Val, Failure> {
                match conv {
                    Conv2::Files => {
                        let mut out = Vec::new();
                        for (p, n) in files {
                            let n = n.noun();
                            out.push((p, if n.is_cell() { None } else { Some(n.text()?) }));
                        }
                        Ok(Val::Files(Rc::new(out)))
                    }
                    Conv2::Exists => Ok(Val::Bool(true)),
                    Conv2::Dir(f) => f(files),
                    _ => Err(Failure::type_error("n.text is not a function")),
                }
            })?;
            return got.unwrap_or(Ok(Val::Null));
        }
        let got = codec::file_in(snap, silo, |n| self.conv(n))?;
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
    /// Request::Gateway: what takes an answer that comes after the
    /// caller gave up, and when it gave up
    late: RefCell<Option<Late>>,
    gone: Cell<Option<u64>>,
}

/// what takes a late answer (reader.go's peek.late)
type Late = Box<dyn FnOnce(&World, Val)>;

impl PeekSt {
    fn settle(&self, w: &World, i: &Inst, v: Settled) {
        if self.done.get() {
            return;
        }
        if self.gone.get().is_some() {
            // its caller has its answer (none) already: a late one goes
            // to late, as reader.go's finish does
            self.done.set(true);
            i.peeks.borrow_mut().retain(|(k, _)| *k != self.wire);
            drop(self.snap.borrow_mut().take());
            let late = self.late.borrow_mut().take();
            if let (Ok(v), Some(late)) = (v, late)
                && !v.is_null()
            {
                w.log(format!("a late answer came in on {}", self.wire));
                late(w, v);
            }
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
    inbox: RefCell<Option<Listener>>,
    // what we publish, as grub.serve and scry.serve set it
    server: RefCell<Server>,
}

impl Inst {
    fn shutdown(&self) {
        self.eng.close();
        // what waits on it holds it: let go
        self.peeks.borrow_mut().clear();
        self.pokes.borrow_mut().clear();
        self.poking.borrow_mut().clear();
        self.keens.borrow_mut().clear();
        self.server.take();
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
        SENT.with(|s| {
            if let Some(v) = s.borrow_mut().as_mut() {
                v.push((mark.to_string(), n.jam()));
            }
        });
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

// ------------------------------------------------------------ serving

/// grub.serve's file(folder, name): None for a file we don't serve
pub(crate) type Files = Rc<dyn Fn(&World, &str, &str) -> Result<Option<File<'static>>, Failure>>;
/// scry.serve's f(path): [mark, value], or None
pub(crate) type Scries = Rc<dyn Fn(&World, &str) -> Result<Option<Value<'static>>, Failure>>;
/// grub.serve's onKeepers, which hears every change of the keeps
pub(crate) type OnKeepers = Rc<dyn Fn(&World, Vec<Json>)>;

/// pub-cli's folder.replace(DESK_BASE, '<base>'), the first only
fn watched(folder: &str) -> String {
    folder.replacen(crate::url::DESK_BASE, "<base>", 1)
}

/// a JS Map of the keeps we answer, by `<ship> <folder> <name>`, in the
/// order they were first set
#[derive(Default)]
pub(crate) struct Keepers(Vec<(String, Json)>);

impl Keepers {
    fn get(&self, k: &str) -> Option<&Json> {
        self.0.iter().find(|e| e.0 == k).map(|e| &e.1)
    }

    fn set(&mut self, k: String, v: Json) {
        match self.0.iter_mut().find(|e| e.0 == k) {
            Some(e) => e.1 = v,
            None => self.0.push((k, v)),
        }
    }

    fn remove(&mut self, k: &str) -> bool {
        let n = self.0.len();
        self.0.retain(|e| e.0 != k);
        self.0.len() != n
    }

    fn len(&self) -> usize {
        self.0.len()
    }

    /// how many keeps ship holds (k.ship === ship)
    fn kept_by(&self, ship: &str) -> usize {
        self.0
            .iter()
            .filter(
                |e| matches!(&e.1, Json::Obj(o) if o.get("ship") == Some(&Json::Str(ship.into()))),
            )
            .count()
    }

    fn values(&self) -> Vec<Json> {
        self.0.iter().map(|e| e.1.clone()).collect()
    }
}

/// What we publish: our files to other ships' grubbery, the keeps of
/// them, and our scry namespace
#[derive(Default)]
pub(crate) struct Server {
    files: Option<Files>,
    scries: Option<Scries>,
    keepers: Keepers,
    on_keepers: Option<OnKeepers>,
}

impl Inst {
    /// files(folder, name)
    fn file(&self, w: &World, folder: &str, name: &str) -> Result<Option<File<'static>>, Failure> {
        let files = self.server.borrow().files.clone();
        match files {
            Some(f) => {
                let got = f(w, folder, name)?;
                if w.watch.get() {
                    let what = match &got {
                        Some(f) => format!("rev {}", js::string(f.rev.as_ref())),
                        None => "not ours".into(),
                    };
                    w.log(format!("file {}/{name}: {what}", watched(folder)));
                }
                Ok(got)
            }
            None => Ok(None),
        }
    }

    /// keepersChanged([...keepers.values()])
    fn keepers_changed(&self, w: &World) {
        let (f, ks) = {
            let s = self.server.borrow();
            (s.on_keepers.clone(), s.keepers.values())
        };
        if let Some(f) = f {
            if w.watch.get() {
                let each: Vec<String> = ks
                    .iter()
                    .map(|k| {
                        let o = |n| match k {
                            Json::Obj(o) => js::string(o.get(n)),
                            _ => "undefined".into(),
                        };
                        format!("{} {}/{}", o("ship"), watched(&o("folder")), o("name"))
                    })
                    .collect();
                let all = if each.is_empty() {
                    "none".into()
                } else {
                    each.join(", ")
                };
                w.log(format!("keeps: {all}"));
            }
            f(w, ks);
        }
    }

    /// grub.serve: serve file's files to other ships' grubbery, each keep
    /// kept until its ship drops it: saved, the keeps the last run handed
    /// on_keepers, which hears every change
    fn serve(&self, files: Files, saved: &[Json], on_keepers: OnKeepers) {
        let mut keepers = Keepers::default();
        for k in saved {
            let f = |n| {
                js::string(match k {
                    Json::Obj(o) => o.get(n),
                    _ => None,
                })
            };
            keepers.set(
                format!("{} {} {}", f("ship"), f("folder"), f("name")),
                k.clone(),
            );
        }
        let mut s = self.server.borrow_mut();
        s.files = Some(files);
        s.keepers = keepers;
        s.on_keepers = Some(on_keepers);
    }

    /// grub.changed: tell whoever keeps [folder name] its new version
    fn changed(self: &Rc<Self>, w: &World, folder: &str, name: &str) -> Result<(), Failure> {
        let f = self.file(w, folder, name)?;
        let keepers = self.server.borrow().keepers.values();
        self.txn(w, || {
            for k in &keepers {
                let Json::Obj(k) = k else {
                    continue;
                };
                if k.get("folder") != Some(&Json::Str(folder.into()))
                    || k.get("name") != Some(&Json::Str(name.into()))
                {
                    continue;
                }
                let her = ship_of(&js::string(k.get("ship")))?;
                let bone = self.flow_to(her)?;
                let wire = match k.get("wire") {
                    Some(Json::Arr(a)) if !a.is_empty() => {
                        let mut v: Vec<Value<'static>> = a.iter().map(json_value).collect();
                        v.push(Value::Null);
                        Value::Array(v)
                    }
                    _ => Value::Null,
                };
                let dest = Value::Array(vec![0u64.into(), codec::path(folder), name.into()]);
                self.poke(
                    her,
                    bone,
                    "grubbery",
                    "grubbery-intake",
                    msg::intake(wire, dest, msg::wave_of(f.as_ref(), name)?),
                )?;
            }
            Ok(())
        })
    }
}

/// a saved JS value as noun() takes it
fn json_value(j: &Json) -> Value<'static> {
    match j {
        Json::Null => Value::Null,
        Json::Bool(b) => Value::Bool(*b),
        Json::Num(n) if n.fract() == 0.0 && *n >= 0.0 => Value::Int((*n as u64).into()),
        j => Value::Text(js::string(Some(j))),
    }
}

/// what publish() gives: pub.mjs's layout of our pages, served by the
/// engine it was handed
pub(crate) struct Pub {
    i: Rc<Inst>,
    st: Rc<RefCell<PubState>>,
}

impl Pub {
    /// publish(urb, {pages, keepers, save})
    fn publish(i: &Rc<Inst>, st: PubState) -> Rc<Pub> {
        let saved = match &st.keepers {
            Json::Arr(a) => a.clone(),
            _ => Vec::new(),
        };
        let st = Rc::new(RefCell::new(st));
        let (a, b, c) = (st.clone(), st.clone(), st.clone());
        i.serve(
            Rc::new(move |_, folder, name| a.borrow().file(folder, name)),
            &saved,
            Rc::new(move |w, ks| {
                b.borrow_mut().keepers = Json::Arr(ks);
                save(w, &b);
            }),
        );
        i.server.borrow_mut().scries = Some(Rc::new(move |_, path| c.borrow().scry(path)));
        Rc::new(Pub { i: i.clone(), st })
    }

    /// tell whoever follows the page at key, the list and the index
    fn tell(&self, w: &World, key: &str) -> Result<(), Failure> {
        let (dir, vault) = (publish::pub_dir(), publish::vault());
        self.i.changed(w, &format!("{vault}{key}"), "gmi")?;
        self.i.changed(w, &dir, "revs")?;
        self.i.changed(w, &dir, "index")
    }

    /// publish body as the page name in format mark, a new revision with a
    /// new, unguessable id: the record, or null for a name lattice can't
    /// read or a mark it doesn't publish
    fn put(&self, w: &World, name: &str, body: &str, mark: &str) -> Result<Json, Failure> {
        let Some(key) = publish::page_key(name) else {
            return Ok(Json::Null);
        };
        if !publish::page_mark(mark) {
            return Ok(Json::Null);
        }
        let id = publish::page_id(&w.entropy(16));
        let hash = format!("{:x}", noun::sham(&Value::Text(body.into()))?);
        let rec = self
            .st
            .borrow_mut()
            .put(&key, body, mark, id, w.now(), hash);
        save(w, &self.st);
        self.tell(w, &key)?;
        Ok(rec)
    }

    /// take a page down; its rev goes up, so a reader following it hears
    fn remove(&self, w: &World, name: &str) -> Result<bool, Failure> {
        let Some(key) = publish::page_key(name) else {
            return Ok(false);
        };
        if !self.st.borrow_mut().remove(&key, w.now()) {
            return Ok(false);
        }
        save(w, &self.st);
        self.tell(w, &key)?;
        Ok(true)
    }
}

/// save({pages, keepers})
fn save(w: &World, st: &RefCell<PubState>) {
    let value = st.borrow().saved();
    w.emit(Output::Event {
        name: "pub.save".into(),
        value,
    });
}

impl Drop for Inst {
    fn drop(&mut self) {
        self.eng.close();
    }
}

/// a poke's mark and its noun's jam
type Sent = (String, Vec<u8>);

thread_local! {
    static SENT: RefCell<Option<Vec<Sent>>> = const { RefCell::new(None) };
}

/// For the tests that clam what we send against the molds it must fit:
/// keep the mark and the jam of every poke made on this thread from now
/// on (on), or stop keeping them; what was kept.
#[doc(hidden)]
pub fn capture_pokes(on: bool) -> Vec<Sent> {
    SENT.with(|s| s.replace(on.then(Vec::new)).unwrap_or_default())
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
                inbox: RefCell::new(None),
                server: RefCell::new(Server::default()),
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
    peek_late(w, i, her, folder, name, read, wait, None)
}

/// peek, and under Request::Gateway, reader.go's peek: no answer in time
/// is Undef (where an answer of nothing is Null), and an answer that
/// comes after that goes to late, if there is one, for LATE_WAIT
#[allow(clippy::too_many_arguments)]
fn peek_late(
    w: &World,
    i: &Rc<Inst>,
    her: Ship,
    folder: &str,
    name: Option<&str>,
    read: Read,
    wait: u64,
    late: Option<Late>,
) -> Promise {
    if w.gateway.get() {
        let now = w.now();
        i.peeks.borrow_mut().retain(|(_, st)| {
            st.gone
                .get()
                .is_none_or(|g| now.saturating_sub(g) <= LATE_WAIT)
        });
    }
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
        late: RefCell::new(late),
        gone: Cell::new(None),
    });
    let (st2, i2) = (st.clone(), i.clone());
    st.timer.set(w.set_timeout(
        Box::new(move |w| {
            if !w.gateway.get() {
                st2.finish(w, &i2, Val::Null)
            } else if st2.late.borrow().is_some() && !st2.done.get() {
                // it waits on for a late answer
                st2.gone.set(Some(w.now()));
                resolve(w, &st2.p, Ok(Val::Undef));
            } else {
                st2.finish(w, &i2, Val::Undef)
            }
        }),
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
///
/// value is what noun() is handed: one it throws on (furum's BigInt() of
/// a fraction) fails the poke where noun() runs, inside its transaction.
fn grub_poke_with(
    w: &World,
    i: &Rc<Inst>,
    her: Ship,
    folder: &str,
    name: &str,
    mark: (String, String),
    value: Result<Value<'static>, Failure>,
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
        let value = value?;
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
fn peek_revs(w: &World, i: &Rc<Inst>, her: Ship, base: &'static str) -> Promise {
    // Request::Gateway: a list that comes late is kept, as reader.go's
    // keepRevs keeps it
    let late: Option<Late> = w.gateway.get().then(|| {
        let i = Rc::downgrade(i);
        Box::new(move |w: &World, v: Val| {
            if let (Some(i), Val::Obj(o)) = (i.upgrade(), v) {
                let who = ship_text(her);
                i.bases.borrow_mut().insert(who.clone(), base);
                i.revs.borrow_mut().insert(who, (w.now(), go_revs(&o)));
            }
        }) as Late
    });
    let p = peek_late(
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
        late,
    );
    let p = on_err(w, &p, |w, e| match &e {
        Val::Err(f) if f.kind == Kind::UrbError => Ok(Val::Obj(Rc::new(Object::default()))),
        _ => Ok(Val::Promise(rejected(w, e))),
    });
    if !w.gateway.get() {
        return p;
    }
    then(
        w,
        &p,
        Box::new(|_, r| match r {
            Ok(Val::Obj(o)) => Ok(Val::Obj(go_revs(&o))),
            r => r,
        }),
    )
}

/// reader.go's revsOf: a list's entries that name an id (a non-empty
/// string); any other entry is no entry
fn go_revs(o: &Object) -> Rc<Object> {
    let mut out = Object::default();
    for (k, v) in o.entries() {
        if let Json::Obj(row) = v
            && let Some(Json::Str(id)) = row.get("id")
            && !id.is_empty()
        {
            out.set(k.to_string(), v.clone());
        }
    }
    Rc::new(out)
}

/// reader.go's words for a ship that answered no base probe in time
fn go_no_answer(who: &str) -> Failure {
    Failure::urb(format!(
        "{who} did not answer within 30s. It may be offline, or still catching up \
         after this gateway restarted, which can take a minute or two: try again shortly"
    ))
}

/// The HTTP status the Go gateway answers a reader's failure with,
/// under Request::Gateway: reader.go's fail() codes, by its words. Any
/// other failure is the gateway's default, 502.
pub fn gateway_status(f: &Failure) -> u16 {
    let m = &f.message;
    if f.kind != Kind::UrbError {
        502
    } else if m.ends_with(" is not published") {
        404
    } else if m.contains(" did not answer within 30s. It may be offline") {
        504
    } else {
        502
    }
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
        if w.gateway.get() {
            return throw(Failure::urb(format!(
                "{who} shares no lattice pages here: it runs no lattice, one older than \
                 release 35, or shares nothing"
            )));
        }
        return base_done(i, who);
    }
    let b = bases[k];
    let p = peek_revs(w, &i, her, b);
    wait_ok(Val::Promise(p), move |w, got| {
        // reader.go: the other place would only wait behind the same
        // stall
        if w.gateway.get() && matches!(got, Val::Undef) {
            return throw(go_no_answer(&who));
        }
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
                    let b = if b == APP_BASE { APP_BASE } else { DESK_BASE };
                    let p = peek_revs(w, &i, her, b);
                    wait_ok(Val::Promise(p), move |w, got| {
                        // reader.go: no answer yet, so the stale list
                        // beats none
                        if w.gateway.get()
                            && matches!(got, Val::Undef)
                            && let Some((_, old)) = r
                        {
                            return Step::Ret(Val::Obj(old));
                        }
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

/// +read-page-body-rev: keen the listed id, else peek the vault file.
/// [body, how, mark]. The vault file is %gmi whatever the page's format.
/// Under Request::Gateway, given is the list the read already has: as
/// in reader.go, each page of one read is looked up in the same list,
/// asked for once.
fn page(
    w: &World,
    i: &Rc<Inst>,
    her: Ship,
    who: String,
    rel: Vec<String>,
    given: Option<Rc<Object>>,
) -> Promise {
    let i = i.clone();
    spawn(w, move |w| {
        let key = format!("/{}", rel.join("/"));
        let list = match given {
            Some(l) => Val::Obj(l),
            None => Val::Promise(rev_list(w, &i, her, who.clone())),
        };
        wait_ok(list, move |w, list| {
            let Val::Obj(list) = list else {
                return throw(Failure::error("no list"));
            };
            let ent = list.get(&key);
            if !js::truthy(ent) {
                return vault(w, i, her, who, key);
            }
            let id = match ent {
                Some(Json::Obj(o)) => js::string(o.get("id")),
                _ => js::string(None),
            };
            let k = match keen(w, &i, her, format!("/g/x/1/grubbery//1/pub/page{key}/{id}")) {
                Ok(k) => k,
                Err(e) => return throw(e),
            };
            wait_ok(Val::Promise(k), move |w, got| {
                if let Val::Gage(mark, Some(body)) = &got
                    && keen_is_page(mark, true)
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

/// Whether a keen got a page (+read-page-body-rev): an answer, one
/// atom, under one of lattice's view-mode marks. A %del tombstone or a
/// mark lattice doesn't publish is a miss. reader.go's `isPage`.
pub fn keen_is_page(mark: &str, answered: bool) -> bool {
    answered && PAGE_MARKS.contains(&mark)
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
                    let given = w.gateway.get().then(|| list.clone());
                    try_pages(w, i, her, who, url, tries, 0, given)
                })
            }
        }
    })
}

#[allow(clippy::too_many_arguments)]
fn try_pages(
    w: &World,
    i: Rc<Inst>,
    her: Ship,
    who: String,
    url: String,
    tries: Vec<Vec<String>>,
    k: usize,
    given: Option<Rc<Object>>,
) -> Step {
    let Some(p) = tries.get(k).cloned() else {
        if w.gateway.get() {
            return throw(Failure::urb(format!("{url} is not published")));
        }
        return throw(Failure::urb(format!("{url}: not published")));
    };
    let key = format!("/{}", p.join("/"));
    let pg = page(w, &i, her, who.clone(), p, given.clone());
    wait_ok(Val::Promise(pg), move |w, got| match got {
        Val::Page(Some((body, how, mark))) => Step::Ret(Val::Reply(Reply::Read(Page {
            key,
            body,
            how,
            mark: Some(mark),
        }))),
        _ => try_pages(w, i, her, who, url, tries, k + 1, given),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(panic = "unwind")]
    #[test]
    fn a_hook_that_panics_gives_its_default() {
        let c = Core::new(|n| vec![0; n]);
        let got = engine::guard(&c.w, 7, |_| -> Result<i32, Failure> { panic!("boom") });
        assert_eq!(got, 7);
        let got = engine::guard(&c.w, 7, |_| Err(Failure::urb("no")));
        assert_eq!(got, 7);
        let logs: Vec<String> =
            c.w.out
                .borrow()
                .iter()
                .filter_map(|o| match o {
                    Output::Log(l) => Some(l.clone()),
                    _ => None,
                })
                .collect();
        assert_eq!(logs, ["panic: boom", "UrbError: no"]);
    }

    #[test]
    fn saved_rows_are_read_as_glue_c_reads_them() {
        let j = Journal::new(std::ptr::null());
        // a row, a deleted row, and nothing more
        assert!(
            j.load(b"\x01\0\0\0k\x01\0\0\0v\x01\0\0\0d\xff\xff\xff\xff")
                .is_ok()
        );
        assert!(j.load(b"").is_ok());
        for bad in [
            &b"\x01\0\0"[..],
            b"\x01\0\0\0k\x02\0\0\0v",
            b"\x05\0\0\0k\0\0\0\0",
            // a klen that wraps a 32-bit usize past the bounds checks
            b"\xf8\xff\xff\xff\0\0\0\0\0\0\0\0",
            b"\x01\0\0\0k\xfe\xff\xff\xffv",
        ] {
            assert!(j.load(bad).is_err(), "{bad:?}");
        }
    }
}
