//! The replay harness: each trace's inputs (traces/*.jsonl, recorded
//! from the JS by wasm/trace.mjs) fed into the Rust core, and each
//! step's outputs compared with the trace's, byte for byte, as JSON in
//! trace.mjs's encoding. The trace's `timer` and `untimer` outputs are
//! the JS's own scheduling: the core keeps its own timers, which each
//! tick fires by its name and due. After each step the timers the core
//! has set must be the ones the JS had live (by kind and due), and its
//! deadline their earliest; the rest is compared output by output. Each step
//! must also draw the entropy the trace says it drew, and all of it.
//!
//! Every trace must replay: 1, 2, 3, 4 (furum), 5 (serving), 6, 7 (the
//! %want leg), 8 (a lattice forum), 9 and 10 (the roller).

mod common;

use common::trace::{Eny, TRACES, out, request, text};
use common::{big, unhex};
use serde_json::Value as J;
use std::cell::RefCell;
use std::rc::Rc;
use urb::core::{ForumCall, Lane};
use urb::noun::Value;
use urb::{Core, Input, Output, Request, alive};

const MUST: &[u32] = &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10];

enum Replayed {
    Ok(usize),
    /// a call the core has no port of, at this step
    Unported(usize, String),
    Fail(usize, String),
}

fn replay(lines: &str) -> Replayed {
    let mut lines = lines.lines().filter(|l| !l.is_empty());
    let head: J = serde_json::from_str(lines.next().unwrap()).unwrap();
    assert_eq!(head["format"], 1, "trace format");
    let steps: Vec<J> = lines.map(|l| serde_json::from_str(l).unwrap()).collect();
    let eny = Rc::new(RefCell::new(Eny::default()));
    let e2 = eny.clone();
    let mut core = Core::new(move |n| {
        let mut e = e2.borrow_mut();
        match e.left.pop_front() {
            Some(h) if h.len() == 2 * n => {
                e.drawn.push(h.clone());
                unhex(&h)
            }
            got => {
                e.wrong = Some(format!(
                    "drew {n} bytes, the trace has {}",
                    got.map_or("no more".into(), |h| (h.len() / 2).to_string())
                ));
                vec![0; n]
            }
        }
    });
    // the JS's live timers, as the trace sets and clears them: id ->
    // (kind, due); and which steps are close calls
    let mut live: std::collections::BTreeMap<u64, (String, u64)> = Default::default();
    let mut closes = std::collections::HashSet::new();
    for (k, step) in steps.iter().enumerate() {
        let i = &step["in"];
        if i["k"] == "call" && i["op"] == "close" {
            closes.insert(k as u64);
        }
        if i["k"] == "tick" {
            live.remove(&i["timer"].as_u64().unwrap());
        }
        let now = i["now"].as_u64().unwrap();
        {
            let mut e = eny.borrow_mut();
            e.left = i["eny"].as_array().unwrap().iter().map(text).collect();
            e.drawn.clear();
        }
        let input = match i["k"].as_str().unwrap() {
            "call" => {
                let op = i["op"].as_str().unwrap();
                match request(op, i["args"].as_array().unwrap()) {
                    Some(req) => Input::Call { id: k as u64, req },
                    None => return Replayed::Unported(k, op.into()),
                }
            }
            "hear" => Input::Hear {
                ip: i["ip"].as_u64().unwrap() as u32,
                port: i["port"].as_u64().unwrap() as u16,
                bytes: unhex(i["bytes"].as_str().unwrap()),
            },
            "tick" => Input::Tick {
                kind: text(&i["kind"]),
                due: i["due"].as_u64().unwrap(),
            },
            "fetch" => Input::Fetched {
                id: i["id"].as_u64().unwrap(),
                answer: match i.get("error") {
                    Some(e) => Err(text(e)),
                    None => Ok((
                        i["status"].as_u64().unwrap() as u16,
                        unhex(i["body"].as_str().unwrap()),
                    )),
                },
            },
            other => return Replayed::Fail(k, format!("no input {other}")),
        };
        let got = match core.step(now, input) {
            Ok(o) => o,
            Err(e) => return Replayed::Fail(k, e),
        };
        let got: Vec<J> = got.iter().map(out).collect();
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
                // trace.mjs's close clears every timer before its result
                live.clear();
            }
        }
        // the core's timers are the JS's, and its deadline their earliest
        let mut js: Vec<(String, u64)> = live.values().cloned().collect();
        let mut ours = core.timers();
        js.sort();
        ours.sort();
        if js != ours {
            return Replayed::Fail(k, format!("timers: the JS has {js:?}, the core {ours:?}"));
        }
        if core.deadline() != js.iter().map(|t| t.1).min() {
            return Replayed::Fail(k, format!("deadline {:?}, timers {js:?}", core.deadline()));
        }
        let want: Vec<J> = step["out"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|o| o.get("timer").is_none() && o.get("untimer").is_none())
            .cloned()
            .collect();
        let e = eny.borrow();
        if let Some(w) = &e.wrong {
            return Replayed::Fail(k, w.clone());
        }
        if !e.left.is_empty() {
            return Replayed::Fail(k, format!("{} draws not made", e.left.len()));
        }
        let (a, b) = (
            serde_json::to_string(&want).unwrap(),
            serde_json::to_string(&got).unwrap(),
        );
        if a != b {
            let n = want.len().max(got.len());
            let at = (0..n).find(|&j| want.get(j) != got.get(j)).unwrap_or(0);
            let cut = |o: Option<&J>| {
                o.map_or("nothing".into(), |o| {
                    o.to_string().chars().take(300).collect::<String>()
                })
            };
            return Replayed::Fail(
                k,
                format!(
                    "({}) output {at}:\n    recorded {}\n    replayed {}\n  kinds recorded {:?}\n  kinds replayed {:?}",
                    i["k"],
                    cut(want.get(at)),
                    cut(got.get(at)),
                    want.iter()
                        .map(|o| o.as_object().unwrap().keys().next().unwrap().clone())
                        .collect::<Vec<_>>(),
                    got.iter()
                        .map(|o| o.as_object().unwrap().keys().next().unwrap().clone())
                        .collect::<Vec<_>>(),
                ),
            );
        }
    }
    drop(core);
    Replayed::Ok(steps.len())
}

#[test]
fn traces_replay_byte_for_byte() {
    // a trace added without a line above fails here
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../traces");
    if let Ok(d) = std::fs::read_dir(dir) {
        let on_disk = d
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter(|n| n.ends_with(".jsonl"))
            .count();
        assert_eq!(on_disk, TRACES.len(), "traces/ has {on_disk} traces");
    }
    // TRACES=1,3 replays only those
    let only: Option<Vec<u32>> = std::env::var("TRACES")
        .ok()
        .map(|t| t.split(',').map(|n| n.parse().unwrap()).collect());
    let mut bad = Vec::new();
    for &(n, name, lines) in TRACES {
        if only.as_ref().is_some_and(|o| !o.contains(&n)) {
            continue;
        }
        let r = replay(lines);
        assert_eq!(alive(), 0, "{name} left nouns alive");
        let must = MUST.contains(&n);
        match r {
            Replayed::Ok(s) => eprintln!("ok    {name}: {s} steps"),
            Replayed::Unported(k, op) => {
                eprintln!("wait  {name}: step {k} calls {op}, which the core does not port");
                if must {
                    bad.push(n);
                }
            }
            Replayed::Fail(k, why) => {
                eprintln!(
                    "{}  {name}: step {k} differs {why}",
                    if must { "FAIL" } else { "fail" }
                );
                if must {
                    bad.push(n);
                }
            }
        }
    }
    assert!(bad.is_empty(), "traces {bad:?} do not replay");
}

#[test]
fn a_tick_must_name_a_timer() {
    let mut core = Core::new(|n| vec![0; n]);
    let got = core.step(
        0,
        Input::Tick {
            kind: "wake".into(),
            due: 0,
        },
    );
    assert!(got.is_err());
    let _ = Lane::Galaxy(0);
}

// ------------------------------------------------------------ the molds

/// a load another ship's grubbery sends ours: [[wire dest] tag ~], dest
/// the file [folder name]
fn load(wire: &str, folder: &str, name: &str, tag: &str) -> Value<'static> {
    use urb::codec::path;
    Value::Array(vec![
        Value::Array(vec![
            path(wire),
            Value::Array(vec![0u64.into(), path(folder), name.into()]),
        ]),
        tag.into(),
        Value::Null,
    ])
}

/// What only another ship's grubbery brings about, by the hooks: a core
/// publishing pages, peeked and kept from ~zod, edited, and dropped, as
/// check.mjs's loop does. What it gives is not checked here, only that
/// every message it sends fits its mold.
fn serving() {
    use urb::core::Hook;
    use urb::publish::{pub_dir, vault};
    let mut c = Core::new(|n| vec![7; n]);
    let us = urb::engine::ship_parse("~nec").unwrap();
    let call = |c: &mut Core, id: u64, req: Request| {
        let o = c.step(id, Input::Call { id, req }).unwrap();
        assert!(
            !o.iter()
                .any(|o| matches!(o, Output::Done { result: Err(_), .. })),
            "{o:?}"
        );
    };
    call(
        &mut c,
        1,
        Request::Open {
            keyfile: Some(urb::engine::fake_keyfile(us)),
            rows: None,
            fresh: true,
            fake: true,
        },
    );
    call(
        &mut c,
        2,
        Request::Pub {
            pages: None,
            keepers: None,
        },
    );
    let put = |name: &str, body: &str, mark: Option<&str>| Request::PubPut {
        name: name.into(),
        body: body.into(),
        mark: mark.map(String::from),
    };
    call(&mut c, 3, put("index", "# home", None));
    call(&mut c, 4, put("notes/intro", "# Intro", Some("md")));
    call(&mut c, 5, put("gone", "soon gone", Some("text")));
    call(&mut c, 6, Request::PubRemove("gone".into()));
    let (dir, vault) = (pub_dir(), vault());
    let hear = |c: &mut Core, v: Value<'static>| {
        let n = urb::Noun::of(&v).unwrap();
        let o = c.hook(10, Hook::Poke("~zod", "grubbery-load", n.borrow()));
        assert!(!o.iter().any(|o| matches!(o, Output::Log(_))), "{o:?}");
    };
    for page in ["notes/intro", "gone", "never"] {
        hear(
            &mut c,
            load("/keep/page", &format!("{vault}/{page}"), "gmi", "keep"),
        );
    }
    for (folder, name) in [
        (dir.clone(), "index"),
        (dir.clone(), "revs"),
        (format!("{vault}/notes/intro"), "gmi"),
        (format!("{vault}/gone"), "gmi"),
        (format!("{vault}/never"), "gmi"),
        ("/not/ours".into(), "x"),
    ] {
        hear(&mut c, load("/peek/1", &folder, name, "peek"));
    }
    // a deep peek, of a directory
    hear(&mut c, msg_peek("/peek/2", &dir));
    call(&mut c, 7, put("notes/intro", "# Intro, again", Some("md")));
    call(&mut c, 8, put("gone", "back", None));
    hear(
        &mut c,
        load("/keep/page", &format!("{vault}/notes/intro"), "gmi", "drop"),
    );
    // a grub there writing to ours: taken in with a %pack
    hear(
        &mut c,
        urb::msg::poke(
            "/grub-poke/1/1",
            &dir,
            "x",
            ("/lattice", "msg"),
            "hi".into(),
        ),
    );
    drop(c);
}

fn msg_peek(wire: &str, folder: &str) -> Value<'static> {
    let mut v = urb::msg::peek(wire, folder, None);
    // another ship's peek: its act is [%peek ~ deep]
    if let Value::Array(a) = &mut v {
        a[1] = Value::Array(vec!["peek".into(), Value::Null, true.into()]);
    }
    v
}

#[cfg(not(target_os = "wasi"))]
#[test]
fn every_message_sent_clams_against_grubberys_molds() {
    use common::{answers, run_tool, tool, ux};
    use urb::Int;
    let Some(urbit) = tool("URBIT", "urbit", "the mold checks") else {
        return;
    };
    urb::core::capture_pokes(true);
    for &(n, name, lines) in TRACES {
        if MUST.contains(&n) {
            assert!(matches!(replay(lines), Replayed::Ok(_)), "{name}");
        }
    }
    serving();
    let sent = urb::core::capture_pokes(false);
    assert_eq!(alive(), 0);
    let mold = |m: &str| match m {
        "grubbery-load" => Some("load:remo:nexus"),
        "grubbery-transfer" => Some("transfer:remo:nexus"),
        "grubbery-intake" => Some("intake:remo:nexus"),
        _ => None,
    };
    // each message once: [hoon, rust's sham, what it is]
    let mut seen = std::collections::HashSet::new();
    let mut cases: Vec<(String, String, String)> = Vec::new();
    let mut kinds: std::collections::BTreeMap<String, usize> = Default::default();
    for (mark, jam) in &sent {
        let m = mold(mark).unwrap_or_else(|| panic!("we sent a {mark}, which has no mold here"));
        if !seen.insert((mark.clone(), jam.clone())) {
            continue;
        }
        let n = urb::Noun::cue(jam).expect("our own jam");
        let tag = if mark == "grubbery-intake" {
            "wave".into()
        } else {
            n.borrow().tail().unwrap().head().unwrap().text().unwrap()
        };
        let what = format!("{} %{tag}", &mark[9..]);
        *kinds.entry(what.clone()).or_default() += 1;
        cases.push((
            format!(
                "`@ux`(fall (mole |.((sham ;;({m} (cue {}))))) 0)",
                ux(&Int::from_le(jam))
            ),
            format!("{:x}", n.sham()),
            what,
        ));
    }
    for want in [
        "load %peek",
        "load %poke",
        "transfer %pack",
        "transfer %snap",
        "transfer %want",
        "intake %wave",
    ] {
        assert!(kinds.contains_key(want), "no {want} was sent: {kinds:?}");
    }
    // and the clam bites: a load whose deep flag is 2, and a %snap whose
    // pace is %fir, are refused; the same %snap with %firm fits
    let snap = |pace: &str| {
        let s = Value::Array(vec![
            Value::Array(vec![1u64.into(), urb::text::da(0).into()]),
            Value::Array(vec![pace.into(), Value::Null, 0x1234u64.into()]),
            Value::Array(vec![Value::Set(vec![0x1234u64.into()]), Value::Set(vec![])]),
        ]);
        let v = urb::msg::snap(
            urb::codec::path("/w"),
            Value::Array(vec![0u64.into(), urb::codec::path("/a"), "b".into()]),
            7u64.into(),
            Some(s),
            None,
        );
        urb::Noun::of(&v).unwrap()
    };
    let mut deep = urb::msg::peek("/w", "/a", None);
    if let Value::Array(a) = &mut deep
        && let Value::Array(wd) = &mut a[0]
        && let Value::Array(d) = &mut wd[1]
    {
        d[0] = 2u64.into();
    }
    let deep = urb::Noun::of(&deep).unwrap();
    let (firm, fir) = (snap("firm"), snap("fir"));
    for (m, n, want) in [
        ("load:remo:nexus", &deep, "0".to_string()),
        ("transfer:remo:nexus", &fir, "0".to_string()),
        ("transfer:remo:nexus", &firm, format!("{:x}", firm.sham())),
    ] {
        cases.push((
            format!(
                "`@ux`(fall (mole |.((sham ;;({m} (cue {}))))) 0)",
                ux(&Int::from_le(&n.jam()))
            ),
            want,
            format!("refused or not: {m}"),
        ));
    }
    drop((deep, firm, fir));
    let molds = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../wasm/test/grubbery-molds.hoon"
    ))
    .unwrap();
    let expr = format!(
        "{molds}\n~[{}]",
        cases
            .iter()
            .map(|c| c.0.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    );
    let out = run_tool(&urbit, &["eval"], &expr, None);
    let got = answers(&out);
    assert_eq!(
        got.len(),
        cases.len(),
        "vere gave {} answers for {}: ...{}",
        got.len(),
        cases.len(),
        &out[out.floor_char_boundary(out.len().saturating_sub(600))..]
    );
    let mut bad = 0;
    for ((h, want, what), have) in cases.iter().zip(&got) {
        if have != want {
            bad += 1;
            eprintln!(
                "MISMATCH {what}: {}\n  vere {have}\n  rust {want}",
                &h[..h.len().min(200)]
            );
        }
    }
    eprintln!(
        "{} of {} messages fit grubbery's molds ({kinds:?}), and the clam bites",
        cases.len() - 3 - bad.min(cases.len() - 3),
        cases.len() - 3
    );
    assert_eq!(bad, 0, "{bad} of {} disagree with vere", cases.len());
    assert_eq!(alive(), 0);
}

#[test]
fn serving_logs_nothing_and_lets_every_noun_go() {
    serving();
    assert_eq!(alive(), 0);
}

// ------------------------------------------------------------ forum actions

/// A core posting to ~zod's lattice forum: each forum action, as the
/// core's calls build them. The first waits on ~zod's /pub/index, which
/// is answered, so the base is known for the rest.
fn forum_session() {
    use urb::core::Hook;
    use urb::msg::{File, Id::*, Target};
    let mut c = Core::new(|n| vec![7; n]);
    let us = urb::engine::ship_parse("~nec").unwrap();
    let mut id = 0;
    let mut call = |c: &mut Core, req: Request| {
        id += 1;
        c.step(id, Input::Call { id, req }).unwrap()
    };
    call(
        &mut c,
        Request::Open {
            keyfile: Some(urb::engine::fake_keyfile(us)),
            rows: None,
            fresh: true,
            fake: true,
        },
    );
    let t = |post: u64, comment: Option<u64>| Target {
        post: Num(post as f64),
        comment: comment.map(|c| Num(c as f64)),
    };
    let post =
        |name: &str, title: &str, url: Option<&str>, body: Option<String>| ForumCall::NewPost {
            name: name.into(),
            title: Some(title.into()),
            url: url.map(String::from),
            body,
        };
    let comment = |post: u64, body: &str, parent: Option<u64>| ForumCall::NewComment {
        name: "test".into(),
        post: Num(post as f64),
        body: Some(body.into()),
        parent: parent.map(|p| Num(p as f64)),
    };
    let calls = vec![
        post("test", "Hello", None, None),
        post(
            "test",
            "Hello",
            Some("https://example.com"),
            Some("A body\nof two lines".into()),
        ),
        post("my-board-2", "ünïcode ✓", None, Some("x".repeat(2000))),
        comment(1, "a comment", None),
        comment(1_000_000, "a reply", Some(12)),
        ForumCall::NewComment {
            name: "test".into(),
            post: Big(false, big("1152921504606846976")),
            body: Some("big".into()),
            parent: Some(Text("3".into())),
        },
        ForumCall::Upvote("test".into(), t(3, None)),
        ForumCall::Upvote("test".into(), t(3, Some(4))),
        ForumCall::Downvote("test".into(), t(3, None)),
        ForumCall::Unvote("test".into(), t(3, Some(0))),
    ];
    for (k, f) in calls.into_iter().enumerate() {
        let o = call(
            &mut c,
            Request::Forum {
                host: "~zod".into(),
                call: f,
            },
        );
        assert!(
            !o.iter()
                .any(|o| matches!(o, Output::Done { result: Err(_), .. })),
            "{o:?}"
        );
        if k == 0 {
            // ~zod's lattice is at the desk: its /pub/index answers
            let wire = c.waiting().peeks[0].clone();
            let f = File {
                mark: ("/lattice".into(), "index".into()),
                value: Some("index".into()),
                rev: File::num(1),
                at: File::num(0),
            };
            let (snap, silo) = urb::msg::snap_of(&f).unwrap();
            let dest = Value::Array(vec![
                0u64.into(),
                urb::codec::path(&format!("{}/pub", urb::url::DESK_BASE)),
                "index".into(),
            ]);
            let v = urb::msg::snap(
                urb::codec::path(&wire),
                dest,
                9u64.into(),
                Some(snap),
                Some(silo),
            );
            let n = urb::Noun::of(&v).unwrap();
            c.hook(2, Hook::Poke("~zod", "grubbery-transfer", n.borrow()));
        }
    }
    drop(c);
}

/// the forum actions among the pokes sent: each %grubbery-load %poke of
/// [/lattice %forum-action], its action's jam
fn forum_actions(sent: &[(String, Vec<u8>)]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for (mark, jam) in sent {
        if mark != "grubbery-load" {
            continue;
        }
        let n = urb::Noun::cue(jam).unwrap();
        // [[wire dest] %poke [blot noun]]
        let act = n.borrow().tail().unwrap();
        if act.head().unwrap().text().unwrap() != "poke" {
            continue;
        }
        let blot = act.tail().unwrap().head().unwrap();
        let mark = (
            urb::codec::path_of(blot.head().unwrap()).unwrap(),
            blot.tail().unwrap().text().unwrap(),
        );
        if mark == ("/lattice".into(), "forum-action".into()) {
            let a = act.tail().unwrap().tail().unwrap().to_noun();
            if !out.contains(&a.jam()) {
                out.push(a.jam());
            }
        }
    }
    out
}

#[test]
fn the_forum_session_lets_every_noun_go() {
    urb::core::capture_pokes(true);
    forum_session();
    let sent = urb::core::capture_pokes(false);
    assert_eq!(forum_actions(&sent).len(), 10);
    assert_eq!(alive(), 0);
}

#[cfg(not(target_os = "wasi"))]
#[test]
fn every_forum_action_clams_against_lattices_action_ft() {
    use common::{answers, run_tool, tool, ux};
    use urb::Int;
    let Some(urbit) = tool("URBIT", "urbit", "the forum action clams") else {
        return;
    };
    urb::core::capture_pokes(true);
    for &(n, name, lines) in TRACES {
        assert!(matches!(replay(lines), Replayed::Ok(_)), "{name}");
        let _ = n;
    }
    let from_traces = forum_actions(&urb::core::capture_pokes(false));
    urb::core::capture_pokes(true);
    forum_session();
    let from_calls = forum_actions(&urb::core::capture_pokes(false));
    assert_eq!(alive(), 0);
    // trace 8's two posts and trace 6's upvote, and the session's ten
    assert_eq!(from_traces.len(), 3, "the traces' forum actions");
    assert_eq!(from_calls.len(), 10);
    let mut cases: Vec<(String, String)> = Vec::new();
    for jam in from_traces.iter().chain(&from_calls) {
        let n = urb::Noun::cue(jam).unwrap();
        cases.push((
            format!(
                "`@ux`(fall (mole |.((sham ;;(action:ft (cue {}))))) 0)",
                ux(&Int::from_le(jam))
            ),
            format!("{:x}", n.sham()),
        ));
    }
    // and the clam bites: shapes no action has are refused
    let v = |x: Vec<Value<'static>>| Value::Array(x);
    for bad in [
        v(vec![
            "new-post".into(),
            "test".into(),
            v(vec!["a".into(), "b".into()]),
            Value::Null,
            Value::Null,
        ]),
        v(vec![
            "new-post".into(),
            "test".into(),
            "x".into(),
            v(vec![1u64.into(), "u".into()]),
            Value::Null,
        ]),
        v(vec![
            "upvote".into(),
            "test".into(),
            v(vec!["reply".into(), 1u64.into()]),
        ]),
        v(vec!["create".into(), "test".into()]),
        v(vec![
            "remove-vote".into(),
            "test".into(),
            v(vec!["comment".into(), 1u64.into()]),
        ]),
    ] {
        let n = urb::Noun::of(&bad).unwrap();
        cases.push((
            format!(
                "`@ux`?~((mole |.(;;(action:ft (cue {})))) 1 0)",
                ux(&Int::from_le(&n.jam()))
            ),
            "1".into(),
        ));
    }
    let types = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../wasm/test/lattice-forum-types.hoon"
    ))
    .unwrap();
    let expr = format!(
        "=/  ft\n{types}\n~[{}]",
        cases
            .iter()
            .map(|c| c.0.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    );
    let out = run_tool(&urbit, &["eval"], &expr, None);
    let got = answers(&out);
    assert_eq!(
        got.len(),
        cases.len(),
        "vere gave {} answers for {}: ...{}",
        got.len(),
        cases.len(),
        &out[out.floor_char_boundary(out.len().saturating_sub(600))..]
    );
    let bad = cases
        .iter()
        .zip(&got)
        .filter(|((h, want), have)| {
            let differ = have != &want;
            if differ {
                eprintln!(
                    "MISMATCH {}\n  vere {have}\n  rust {want}",
                    &h[..h.len().min(200)]
                );
            }
            differ
        })
        .count();
    eprintln!(
        "{} of {} forum actions fit lattice's action:ft, and the clam bites",
        cases.len() - 5 - bad.min(cases.len() - 5),
        cases.len() - 5
    );
    assert_eq!(bad, 0, "{bad} of {} disagree with vere", cases.len());
}

