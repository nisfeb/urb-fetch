//! Every trace replayed through the native host (src/host, M7): the
//! trace's inputs come in through the host's [`Io`], as a datagram, a
//! fetch's answer or a timer's wake would, its calls go in through the
//! host's calls, and what the host does with each output (a datagram
//! sent, rows committed to its stores, the transport started, a fetch,
//! a log line, a result handed back) must be the trace's, step by step,
//! in order, byte for byte. Each tick must be the timer the host itself
//! would fire at that moment: the earliest due, of two the first set.
//!
//! The commits go to both of the native host's stores, node.mjs's files
//! and libmesa's SQLite, in a scratch folder. At the end each must hold
//! what the commits leave, and when a trace opens again on rows the JS
//! saved (trace 2), both stores must already hold those rows.

mod common;

use common::trace::{Eny, TRACES, out, request, text};
use common::unhex;
use serde_json::Value as J;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use urb::core::{Lane, Output};
use urb::host::store::{FileStore, Loaded, SqliteStore, Store, read_rows};
use urb::host::{Event, Host, Io, Wake};
use urb::{Failure, alive};

/// what the trace's next input brings, for the host's wait
enum Next {
    Event(Event),
    Timer,
}

struct ReplayIo {
    now: u64,
    next: Option<Next>,
    /// the deadline the host last waited for
    asked: Option<Option<u64>>,
    load: Option<Loaded>,
    got: Vec<J>,
    file: FileStore,
    sqlite: SqliteStore,
    /// every row the commits leave
    rows: BTreeMap<Vec<u8>, Vec<u8>>,
    fail_key: bool,
    fail_start: bool,
}

impl ReplayIo {
    fn new(dir: &std::path::Path) -> ReplayIo {
        std::fs::create_dir_all(dir.join("files")).unwrap();
        ReplayIo {
            now: 0,
            next: None,
            asked: None,
            load: None,
            got: Vec::new(),
            file: FileStore::new(&dir.join("files"), None),
            sqlite: SqliteStore::open(&dir.join("x.db"), &dir.join("comet.key"), None).unwrap(),
            rows: BTreeMap::new(),
            fail_key: false,
            fail_start: false,
        }
    }

    fn push(&mut self, o: Output) {
        self.got.push(out(&o));
    }
}

impl Io for ReplayIo {
    fn now(&mut self) -> u64 {
        self.now
    }

    fn wait(&mut self, deadline: Option<u64>) -> Wake {
        self.asked = Some(deadline);
        match self
            .next
            .take()
            .expect("the host waits only when a step brings something")
        {
            Next::Event(e) => Wake::Event(e),
            Next::Timer => Wake::Timer,
        }
    }

    fn load(&mut self) -> Result<Loaded, String> {
        Ok(self.load.take().expect("a load for each open"))
    }

    fn save_key(&mut self, keyfile: &str) -> Result<(), String> {
        self.push(Output::SaveKey(keyfile.into()));
        if self.fail_key {
            return Err("the disk is full".into());
        }
        self.file.save_key(keyfile)?;
        self.sqlite.save_key(keyfile)?;
        self.rows.clear();
        Ok(())
    }

    fn commit(&mut self, rows: &[u8]) -> Result<(), String> {
        self.push(Output::Commit(rows.to_vec()));
        self.file.commit(rows)?;
        self.sqlite.commit(rows)?;
        for (k, v) in read_rows(rows)? {
            match v {
                Some(v) => self.rows.insert(k, v),
                None => self.rows.remove(&k),
            };
        }
        Ok(())
    }

    fn start(&mut self, ship: &str) -> Result<(), String> {
        self.push(Output::Start(ship.into()));
        if self.fail_start {
            return Err("bind EADDRINUSE 0.0.0.0:31000".into());
        }
        Ok(())
    }

    fn send(&mut self, lane: Lane, bytes: &[u8]) {
        self.push(Output::Send {
            lane,
            bytes: bytes.to_vec(),
        });
    }

    fn close(&mut self) {
        self.push(Output::TClose);
    }

    fn fetch(
        &mut self,
        id: u64,
        url: &str,
        method: &str,
        headers: &[(String, String)],
        body: Option<&str>,
    ) {
        self.push(Output::Fetch {
            id,
            url: url.into(),
            method: method.into(),
            headers: headers.to_vec(),
            body: body.map(String::from),
        });
    }

    fn log(&mut self, line: &str) {
        self.push(Output::Log(line.into()));
    }

    fn tap(&mut self, o: &Output) {
        self.got.push(out(o));
    }
}

/// a store's rows, as a map
fn held(s: &mut dyn Store) -> BTreeMap<Vec<u8>, Vec<u8>> {
    let l = s.load().unwrap();
    rows_map(&l.rows.unwrap_or_default())
}

fn rows_map(r: &[u8]) -> BTreeMap<Vec<u8>, Vec<u8>> {
    read_rows(r)
        .unwrap()
        .into_iter()
        .filter_map(|(k, v)| Some((k, v?)))
        .collect()
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("urb-host-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// One trace through the host: Err with the step and what differs.
fn replay(name: &str, lines: &str) -> Result<usize, String> {
    let mut lines = lines.lines().filter(|l| !l.is_empty());
    let head: J = serde_json::from_str(lines.next().unwrap()).unwrap();
    assert_eq!(head["format"], 1);
    let steps: Vec<J> = lines.map(|l| serde_json::from_str(l).unwrap()).collect();
    let dir = scratch(name);
    let eny = Rc::new(RefCell::new(Eny::default()));
    let e2 = eny.clone();
    let mut host = Host::new(ReplayIo::new(&dir), move |n| {
        let mut e = e2.borrow_mut();
        match e.left.pop_front() {
            Some(h) if h.len() == 2 * n => unhex(&h),
            got => {
                e.wrong = Some(format!("drew {n} bytes, the trace has {got:?}"));
                vec![0; n]
            }
        }
    });
    let mut live: BTreeMap<u64, (String, u64)> = BTreeMap::new();
    let mut closes = HashSet::new();
    for (k, step) in steps.iter().enumerate() {
        let i = &step["in"];
        let fail = |why: String| Err(format!("step {k} ({}): {why}", i["k"]));
        let now = i["now"].as_u64().unwrap();
        {
            let mut e = eny.borrow_mut();
            e.left = i["eny"].as_array().unwrap().iter().map(text).collect();
            e.wrong = None;
        }
        host.io.now = now;
        match i["k"].as_str().unwrap() {
            "call" => {
                let op = i["op"].as_str().unwrap();
                let a = i["args"].as_array().unwrap();
                if op == "close" {
                    closes.insert(k as u64);
                }
                if op == "open" {
                    let load = &a[0];
                    let rows = load.get("rows").and_then(J::as_str).map(unhex);
                    // what the JS saved is what the host's stores hold
                    if let Some(r) = &rows {
                        let js = rows_map(r);
                        if held(&mut host.io.sqlite) != js || held(&mut host.io.file) != js {
                            return fail("the stores do not hold the rows the JS saved".into());
                        }
                    }
                    host.io.load = Some(Loaded {
                        keyfile: load.get("keyfile").and_then(J::as_str).map(String::from),
                        rows,
                        fresh: load.get("fresh").and_then(J::as_bool).unwrap_or(false),
                    });
                    let fake = a
                        .get(1)
                        .and_then(|o| o.get("fake"))
                        .and_then(J::as_bool)
                        .unwrap_or(false);
                    host.submit_open_as(k as u64, fake);
                } else {
                    let Some(req) = request(op, a) else {
                        return fail(format!("no request {op}"));
                    };
                    host.submit_as(k as u64, req);
                }
            }
            "hear" => {
                host.io.next = Some(Next::Event(Event::Packet {
                    ip: i["ip"].as_u64().unwrap() as u32,
                    port: i["port"].as_u64().unwrap() as u16,
                    bytes: unhex(i["bytes"].as_str().unwrap()),
                }));
                host.turn();
            }
            "fetch" => {
                host.io.next = Some(Next::Event(Event::Fetched {
                    id: i["id"].as_u64().unwrap(),
                    answer: match i.get("error") {
                        Some(e) => Err(text(e)),
                        None => Ok((
                            i["status"].as_u64().unwrap() as u16,
                            unhex(i["body"].as_str().unwrap()),
                        )),
                    },
                }));
                host.turn();
            }
            "tick" => {
                live.remove(&i["timer"].as_u64().unwrap());
                let (kind, due) = (text(&i["kind"]), i["due"].as_u64().unwrap());
                if i.get("early").and_then(J::as_bool) == Some(true) {
                    // the JS's timer fired a millisecond early; a host's
                    // clock may too, and the core sets it again
                    host.tick(kind, due);
                } else {
                    let ours = host.core().due(now);
                    if ours != Some((kind.clone(), due)) {
                        return fail(format!(
                            "the host would tick {ours:?}, the trace {kind} {due}"
                        ));
                    }
                    host.io.next = Some(Next::Timer);
                    host.turn();
                    match host.io.asked {
                        Some(Some(d)) if d <= now => {}
                        a => return fail(format!("the host waited for {a:?}, past {now}")),
                    }
                }
            }
            other => return fail(format!("no input {other}")),
        }
        for o in step["out"].as_array().unwrap() {
            if let Some(id) = o.get("timer").and_then(J::as_u64) {
                live.insert(id, (text(&o["kind"]), o["due"].as_u64().unwrap()));
            } else if let Some(id) = o.get("untimer").and_then(J::as_u64) {
                live.remove(&id);
            } else if o
                .get("result")
                .and_then(J::as_u64)
                .is_some_and(|c| closes.contains(&c))
            {
                live.clear();
            }
        }
        let mut js: Vec<(String, u64)> = live.values().cloned().collect();
        let mut ours = host.core().timers();
        js.sort();
        ours.sort();
        if js != ours {
            return fail(format!("timers: the JS has {js:?}, the host {ours:?}"));
        }
        let want: Vec<J> = step["out"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|o| o.get("timer").is_none() && o.get("untimer").is_none())
            .cloned()
            .collect();
        let got = std::mem::take(&mut host.io.got);
        {
            let e = eny.borrow();
            if let Some(w) = &e.wrong {
                return fail(w.clone());
            }
            if !e.left.is_empty() {
                return fail(format!("{} draws not made", e.left.len()));
            }
        }
        if want != got {
            let at = (0..want.len().max(got.len()))
                .find(|&j| want.get(j) != got.get(j))
                .unwrap_or(0);
            let cut = |o: Option<&J>| {
                o.map_or("nothing".into(), |o| {
                    o.to_string().chars().take(300).collect::<String>()
                })
            };
            return fail(format!(
                "output {at}:\n    recorded {}\n    the host {}",
                cut(want.get(at)),
                cut(got.get(at))
            ));
        }
    }
    let io = &mut host.io;
    let rows = io.rows.clone();
    if held(&mut io.sqlite) != rows || held(&mut io.file) != rows {
        return Err("the stores do not hold what the commits left".into());
    }
    let n = steps.len();
    drop(host);
    let _ = std::fs::remove_dir_all(&dir);
    Ok(n)
}

#[test]
fn every_trace_replays_through_the_native_host() {
    let mut bad = Vec::new();
    for &(n, name, lines) in TRACES {
        let r = replay(name, lines);
        assert_eq!(alive(), 0, "{name} left nouns alive");
        match r {
            Ok(s) => eprintln!("ok    {name}: {s} steps"),
            Err(why) => {
                eprintln!("FAIL  {name}: {why}");
                bad.push(n);
            }
        }
    }
    assert_eq!(TRACES.len(), 10);
    assert!(
        bad.is_empty(),
        "traces {bad:?} do not replay through the host"
    );
}

/// a host's open, run on its own: the trace's first open, its result,
/// and what the host did, as the trace encodes it
fn open_alone(
    trace: usize,
    tag: &str,
    io: impl FnOnce(&mut ReplayIo),
) -> (Result<String, Failure>, Vec<J>) {
    let (_, name, lines) = TRACES[trace - 1];
    let step: J = serde_json::from_str(lines.lines().nth(1).unwrap()).unwrap();
    let a = &step["in"]["args"];
    let dir = scratch(&format!("{name}-{tag}"));
    let mut rio = ReplayIo::new(&dir);
    io(&mut rio);
    rio.load = Some(Loaded {
        keyfile: a[0].get("keyfile").and_then(J::as_str).map(String::from),
        rows: None,
        fresh: true,
    });
    let mut host = Host::new(rio, |n| vec![7; n]);
    host.submit_open_as(1, true);
    // a comet's open waits on the stars: hand it trace 1's answer
    if trace == 1 {
        let fetched: J = serde_json::from_str(
            lines
                .lines()
                .find(|l| l.contains("\"k\":\"fetch\""))
                .unwrap(),
        )
        .unwrap();
        let i = &fetched["in"];
        host.io.next = Some(Next::Event(Event::Fetched {
            id: i["id"].as_u64().unwrap(),
            answer: Ok((200, unhex(i["body"].as_str().unwrap()))),
        }));
        host.turn();
    }
    let r = host
        .take(1)
        .expect("open answered")
        .map(|r| format!("{r:?}"));
    // a later call fails as open did
    let later = host.submit(urb::Request::Read("urb://~zod/x".into()));
    let l = host.take(later).expect("answered at once");
    if let Err(e) = &r {
        assert_eq!(l.err().as_ref(), Some(e));
    }
    let got = std::mem::take(&mut host.io.got);
    drop(host);
    let _ = std::fs::remove_dir_all(&dir);
    (r, got)
}

fn kinds(got: &[J]) -> Vec<String> {
    got.iter()
        .map(|o| o.as_object().unwrap().keys().next().unwrap().clone())
        .collect()
}

#[test]
fn a_failed_start_fails_open_and_nothing_goes_out() {
    let (r, got) = open_alone(2, "start", |io| io.fail_start = true);
    assert_eq!(r.unwrap_err().message, "bind EADDRINUSE 0.0.0.0:31000");
    // the start, and then nothing saved, sent or closed
    assert_eq!(kinds(&got), vec!["start", "result"]);
}

#[test]
fn a_failed_save_key_fails_open_and_nothing_starts() {
    let (r, got) = open_alone(1, "key", |io| io.fail_key = true);
    assert_eq!(r.unwrap_err().message, "the disk is full");
    let k = kinds(&got);
    assert!(k.starts_with(&["fetch".into(), "saveKey".into()]), "{k:?}");
    assert!(
        !k.iter()
            .any(|k| ["start", "send", "commit", "tclose"].contains(&k.as_str())),
        "{k:?}"
    );
}

#[test]
fn a_good_open_starts_saves_and_answers() {
    let (r, got) = open_alone(2, "good", |_| {});
    assert!(r.unwrap().starts_with("Open"));
    let k = kinds(&got);
    assert_eq!(k.first().map(String::as_str), Some("start"));
    assert!(k.contains(&"commit".into()), "{k:?}");
}
