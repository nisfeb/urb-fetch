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
//! Every trace up to M4 must replay: 1, 2, 3, 5 (serving), 6, 7 (the
//! %want leg), 9 and 10 (the roller). The others (furum and lattice
//! forums) are reported, and wait for M5.

mod common;

use common::{big, hex, unhex};
use serde_json::{Map, Value as J, json};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use urb::core::{Conv, Lane, Peeked};
use urb::js::{Json, Object};
use urb::noun::Value;
use urb::{Core, Failure, Input, Output, Reply, Request, alive};

const MUST: &[u32] = &[1, 2, 3, 5, 6, 7, 9, 10];

/// the entropy a step may draw: the trace's, in order
#[derive(Default)]
struct Eny {
    left: VecDeque<String>,
    drawn: Vec<String>,
    wrong: Option<String>,
}

/// a call's argument (trace.mjs's encoding) as a value for noun()
fn value(j: &J) -> Value<'static> {
    match j {
        J::Null => Value::Null,
        J::Bool(b) => Value::Bool(*b),
        J::Number(n) => Value::Int(n.as_u64().expect("a whole number").into()),
        J::String(s) => Value::Text(s.clone()),
        J::Array(v) => Value::Array(v.iter().map(value).collect()),
        J::Object(o) if o.contains_key("$big") => Value::Int(big(o["$big"].as_str().unwrap())),
        J::Object(o) if o.contains_key("$map") => Value::Map(
            o["$map"]
                .as_array()
                .unwrap()
                .iter()
                .map(|kv| (value(&kv[0]), value(&kv[1])))
                .collect(),
        ),
        J::Object(o) if o.contains_key("$set") => {
            Value::Set(o["$set"].as_array().unwrap().iter().map(value).collect())
        }
        _ => panic!("no value {j}"),
    }
}

fn text(j: &J) -> String {
    j.as_str().expect("text").to_string()
}

/// a JS value, as trace.mjs's dec() gives it
fn json(j: &J) -> Json {
    match j {
        J::Null => Json::Null,
        J::Bool(b) => Json::Bool(*b),
        J::Number(n) => Json::Num(n.as_f64().unwrap()),
        J::String(s) => Json::Str(s.clone()),
        J::Array(v) => Json::Arr(v.iter().map(json).collect()),
        J::Object(o) => {
            assert!(!o.keys().any(|k| k.starts_with('$')), "no plain value {j}");
            let mut out = Object::default();
            for (k, v) in o {
                out.set(k.clone(), json(v));
            }
            Json::Obj(out)
        }
    }
}

/// a JS value as trace.mjs's enc() writes it
fn unjson(j: &Json) -> J {
    match j {
        Json::Null => J::Null,
        Json::Bool(b) => J::Bool(*b),
        Json::Num(n) if !n.is_finite() => json!({ "$num": urb::js::number_text(*n) }),
        Json::Num(n) if n.fract() == 0.0 && n.abs() < 9007199254740992.0 => J::from(*n as i64),
        Json::Num(n) => J::from(*n),
        Json::Str(s) => J::String(s.clone()),
        Json::Arr(v) => J::Array(v.iter().map(unjson).collect()),
        Json::Obj(o) => J::Object(
            o.entries()
                .into_iter()
                .map(|(k, v)| (k.to_string(), unjson(v)))
                .collect(),
        ),
    }
}

/// a trace's call as a request, or None for one M4 has no port of
fn request(op: &str, a: &[J]) -> Option<Request> {
    Some(match op {
        "open" => {
            let load = &a[0];
            let opts = a.get(1).cloned().unwrap_or(json!({}));
            Request::Open {
                keyfile: load.get("keyfile").and_then(J::as_str).map(String::from),
                rows: load.get("rows").and_then(J::as_str).map(unhex),
                fresh: load.get("fresh").and_then(J::as_bool).unwrap_or(false),
                fake: opts.get("fake").and_then(J::as_bool).unwrap_or(false),
            }
        }
        "close" => Request::Close,
        "read" => Request::Read(text(&a[0])),
        "poke" => Request::Poke {
            ship: text(&a[0]),
            folder: text(&a[1]),
            name: text(&a[2]),
            mark: (text(&a[3][0]), text(&a[3][1])),
            value: value(&a[4]),
        },
        "peek" => Request::Peek {
            ship: text(&a[0]),
            folder: text(&a[1]),
            name: a[2].as_str().map(String::from),
            conv: match a[3].as_str()? {
                "cord" => Conv::Cord,
                "text" => Conv::Text,
                "exists" => Conv::Exists,
                "files" => Conv::Files,
                _ => return None,
            },
            wait: a.get(4).and_then(J::as_u64),
        },
        "pub" => {
            let st = a.first().map_or(Json::Obj(Object::default()), json);
            let Json::Obj(st) = st else { return None };
            Request::Pub {
                pages: match st.get("pages") {
                    Some(Json::Obj(o)) => Some(o.clone()),
                    _ => None,
                },
                keepers: st.get("keepers").cloned(),
            }
        }
        "pub.put" => Request::PubPut {
            name: text(&a[0]),
            body: text(&a[1]),
            mark: a.get(2).and_then(J::as_str).map(String::from),
        },
        "pub.remove" => Request::PubRemove(text(&a[0])),
        "pub.pages" => Request::PubPages,
        _ => return None,
    })
}

/// an error as trace.mjs encodes it
fn err(f: &Failure) -> J {
    let mut m = Map::new();
    m.insert("$err".into(), J::String(f.message.clone()));
    if f.later {
        m.insert("later".into(), J::Bool(true));
    }
    if f.kind == urb::core::Kind::UrbError {
        m.insert("urb".into(), J::Bool(true));
    }
    J::Object(m)
}

/// a result as trace.mjs encodes it
fn reply(r: &Reply) -> J {
    match r {
        Reply::Open { our } => json!({ "our": our }),
        Reply::True => J::Bool(true),
        Reply::Read(p) => {
            let mut m = Map::new();
            m.insert("key".into(), J::String(p.key.clone()));
            m.insert("body".into(), J::String(p.body.clone()));
            m.insert("how".into(), J::String(p.how.into()));
            if let Some(mark) = &p.mark {
                m.insert("mark".into(), J::String(mark.clone()));
            }
            J::Object(m)
        }
        Reply::Poke(p) => {
            let mut m = Map::new();
            m.insert("ok".into(), J::Bool(p.ok));
            if let Some(e) = &p.error {
                m.insert("error".into(), J::String(e.clone()));
            }
            if p.later {
                m.insert("later".into(), J::Bool(true));
            }
            J::Object(m)
        }
        Reply::Peek(Peeked::Null) => J::Null,
        Reply::Peek(Peeked::Text(s)) => J::String(s.clone()),
        Reply::Peek(Peeked::True) => J::Bool(true),
        Reply::Peek(Peeked::Files(f)) => J::Array(f.iter().map(|(p, t)| json!([p, t])).collect()),
        Reply::Json(j) => unjson(j),
    }
}

/// an output as trace.mjs encodes it
fn out(o: &Output) -> J {
    match o {
        Output::Send { lane, bytes } => json!({ "send": lane.text(), "bytes": hex(bytes) }),
        Output::Commit(rows) => json!({ "commit": hex(rows) }),
        Output::SaveKey(k) => json!({ "saveKey": k }),
        Output::Start(s) => json!({ "start": s }),
        Output::TClose => json!({ "tclose": true }),
        Output::Log(l) => json!({ "log": l }),
        Output::Fetch {
            id,
            url,
            method,
            headers,
            body,
        } => {
            let mut h = Map::new();
            for (k, v) in headers {
                h.insert(k.clone(), J::String(v.clone()));
            }
            json!({ "fetch": id, "url": url, "method": method, "headers": h, "body": body })
        }
        Output::Done { call, result } => match result {
            Ok(r) => json!({ "result": call, "ok": reply(r) }),
            Err(f) => json!({ "result": call, "err": err(f) }),
        },
        Output::Inbox(m) => json!({ "inbox": m.from }),
        Output::Event { name, value } => json!({ "event": name, "value": unjson(value) }),
    }
}

enum Replayed {
    Ok(usize),
    /// a call M3 has no port of, at this step
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

/// every trace, by number: in the binary, so the WASI run has them too
const TRACES: &[(u32, &str, &str)] = &[
    (
        1,
        "1-cold-comet",
        include_str!("../../../traces/1-cold-comet.jsonl"),
    ),
    (
        2,
        "2-moon-runs",
        include_str!("../../../traces/2-moon-runs.jsonl"),
    ),
    (
        3,
        "3-private-reads",
        include_str!("../../../traces/3-private-reads.jsonl"),
    ),
    (4, "4-furum", include_str!("../../../traces/4-furum.jsonl")),
    (
        5,
        "5-publishing",
        include_str!("../../../traces/5-publishing.jsonl"),
    ),
    (
        6,
        "6-timeouts",
        include_str!("../../../traces/6-timeouts.jsonl"),
    ),
    (
        7,
        "7-want-leg",
        include_str!("../../../traces/7-want-leg.jsonl"),
    ),
    (
        8,
        "8-lattice-forum",
        include_str!("../../../traces/8-lattice-forum.jsonl"),
    ),
    (
        9,
        "9-marks-and-salt",
        include_str!("../../../traces/9-marks-and-salt.jsonl"),
    ),
    (
        10,
        "10-roller",
        include_str!("../../../traces/10-roller.jsonl"),
    ),
];

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
                eprintln!("wait  {name}: step {k} calls {op}, which M4 does not port");
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
