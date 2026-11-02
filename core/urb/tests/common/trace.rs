//! A trace's encoding (wasm/trace.mjs's), shared by the replays: its
//! calls as requests, and outputs and results as the trace writes them.

#![allow(dead_code)]

use super::{big, hex, unhex};
use serde_json::{Map, Value as J, json};
use std::collections::VecDeque;
use urb::core::{Conv, ForumCall, FurumCall, Peeked};
use urb::js::{Json, Object};
use urb::msg::{Id, Target};
use urb::noun::Value;
use urb::{Failure, Output, Reply, Request};

/// the entropy a step may draw: the trace's, in order
#[derive(Default)]
pub struct Eny {
    pub left: VecDeque<String>,
    pub drawn: Vec<String>,
    pub wrong: Option<String>,
}

/// a call's argument (trace.mjs's encoding) as a value for noun()
pub fn value(j: &J) -> Value<'static> {
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

pub fn text(j: &J) -> String {
    j.as_str().expect("text").to_string()
}

/// a JS value, as trace.mjs's dec() gives it
pub fn json(j: &J) -> Json {
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
pub fn unjson(j: &Json) -> J {
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

/// a trace's call as a request, or None for one the core has no port of
pub fn request(op: &str, a: &[J]) -> Option<Request> {
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
        "furum" => Request::Furum {
            host: text(&a[0]),
            call: furum_call(&text(&a[1]), &a[2..])?,
        },
        "forum" => Request::Forum {
            host: text(&a[0]),
            call: forum_call(&text(&a[1]), &a[2..])?,
        },
        _ => return None,
    })
}

pub fn opt(j: Option<&J>) -> Option<String> {
    j.and_then(J::as_str).map(String::from)
}

/// furum(urb, host)[method](...a)
pub fn furum_call(method: &str, a: &[J]) -> Option<FurumCall> {
    let num = |j: &J| j.as_f64().expect("a number");
    Some(match method {
        "nextPost" => FurumCall::NextPost(text(&a[0])),
        "posts" => FurumCall::Posts(text(&a[0])),
        "comments" => FurumCall::Comments(text(&a[0]), num(&a[1])),
        "votes" => FurumCall::Votes(text(&a[0]), num(&a[1])),
        "newPost" => FurumCall::NewPost {
            name: text(&a[0]),
            title: text(&a[1]["title"]),
            url: opt(a[1].get("url")),
            body: opt(a[1].get("body")),
        },
        "newComment" => FurumCall::NewComment {
            name: text(&a[0]),
            post: num(&a[1]),
            body: text(&a[2]),
            parent: a.get(3).and_then(J::as_f64),
        },
        "upvote" => FurumCall::Upvote(text(&a[0]), num(&a[1])),
        "onNote" => FurumCall::OnNote,
        _ => return None,
    })
}

/// an id, as forum.mjs's idOf is given it
pub fn id(j: &J) -> Id {
    match j {
        J::Number(n) => Id::Num(n.as_f64().unwrap()),
        J::String(s) => Id::Text(s.clone()),
        J::Object(o) if o.contains_key("$big") => {
            let d = o["$big"].as_str().unwrap();
            match d.strip_prefix('-') {
                Some(m) => Id::Big(true, big(m)),
                None => Id::Big(false, big(d)),
            }
        }
        _ => panic!("no id {j}"),
    }
}

pub fn target(j: &J) -> Target {
    Target {
        post: id(&j["post"]),
        comment: j.get("comment").filter(|c| !c.is_null()).map(id),
    }
}

/// forum(urb, host)[method](...a)
pub fn forum_call(method: &str, a: &[J]) -> Option<ForumCall> {
    Some(match method {
        "board" => ForumCall::Board(text(&a[0])),
        "thread" => ForumCall::Thread(text(&a[0]), id(&a[1])),
        "newPost" => ForumCall::NewPost {
            name: text(&a[0]),
            title: opt(a[1].get("title")),
            url: opt(a[1].get("url")),
            body: opt(a[1].get("body")),
        },
        "newComment" => ForumCall::NewComment {
            name: text(&a[0]),
            post: id(&a[1]),
            body: opt(a.get(2)),
            parent: a.get(3).filter(|p| !p.is_null()).map(id),
        },
        "upvote" => ForumCall::Upvote(text(&a[0]), target(&a[1])),
        "downvote" => ForumCall::Downvote(text(&a[0]), target(&a[1])),
        "unvote" => ForumCall::Unvote(text(&a[0]), target(&a[1])),
        "onNote" => ForumCall::OnNote,
        _ => return None,
    })
}

/// an error as trace.mjs encodes it
pub fn err(f: &Failure) -> J {
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
pub fn reply(r: &Reply) -> J {
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
        Reply::Js(j) => super::js_enc(j),
    }
}

/// an output as trace.mjs encodes it
pub fn out(o: &Output) -> J {
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
        Output::Note { name, value } => {
            json!({ "event": name, "value": super::js_enc(value) })
        }
    }
}

/// every trace, by number: in the binary, so the WASI run has them too
pub const TRACES: &[(u32, &str, &str)] = &[
    (
        1,
        "1-cold-comet",
        include_str!("../../../../traces/1-cold-comet.jsonl"),
    ),
    (
        2,
        "2-moon-runs",
        include_str!("../../../../traces/2-moon-runs.jsonl"),
    ),
    (
        3,
        "3-private-reads",
        include_str!("../../../../traces/3-private-reads.jsonl"),
    ),
    (
        4,
        "4-furum",
        include_str!("../../../../traces/4-furum.jsonl"),
    ),
    (
        5,
        "5-publishing",
        include_str!("../../../../traces/5-publishing.jsonl"),
    ),
    (
        6,
        "6-timeouts",
        include_str!("../../../../traces/6-timeouts.jsonl"),
    ),
    (
        7,
        "7-want-leg",
        include_str!("../../../../traces/7-want-leg.jsonl"),
    ),
    (
        8,
        "8-lattice-forum",
        include_str!("../../../../traces/8-lattice-forum.jsonl"),
    ),
    (
        9,
        "9-marks-and-salt",
        include_str!("../../../../traces/9-marks-and-salt.jsonl"),
    ),
    (
        10,
        "10-roller",
        include_str!("../../../../traces/10-roller.jsonl"),
    ),
];
