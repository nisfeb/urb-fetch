//! What crosses between the core and the JS shims: each input and
//! output as JSON, in trace.mjs's encoding (`enc` and `dec`), so that a
//! trace's steps can be fed to the wasm build as they were recorded,
//! and the shims build the JS values the JS engine gave.
//!
//! The outputs carry a little more than a trace: an error's class
//! (`kind`, beside `err`), and an inbox poke's grub, mark and jam. A
//! replay drops those before it compares.

use serde_json::{Map, Value as J, json};
use urb::core::{Conv, ForumCall, FurumCall, Kind, Peeked};
use urb::js::{Js, Json, Object};
use urb::msg::{Id, Target};
use urb::noun::Value;
use urb::{Failure, Input, Int, Output, Reply, Request};

pub fn hex(b: &[u8]) -> String {
    const D: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(b.len() * 2);
    for x in b {
        s.push(D[(x >> 4) as usize] as char);
        s.push(D[(x & 15) as usize] as char);
    }
    s
}

pub fn unhex(h: &str) -> Result<Vec<u8>, String> {
    let b = h.as_bytes();
    if !b.len().is_multiple_of(2) {
        return Err("hex of odd length".into());
    }
    let d = |c: u8| match c {
        b'0'..=b'9' => Ok(c - b'0'),
        b'a'..=b'f' => Ok(c - b'a' + 10),
        b'A'..=b'F' => Ok(c - b'A' + 10),
        _ => Err(format!("not hex: {}", c as char)),
    };
    b.chunks(2).map(|p| Ok(d(p[0])? << 4 | d(p[1])?)).collect()
}

/// a non-negative decimal as an Int
pub fn big(dec: &str) -> Result<Int, String> {
    let mut le: Vec<u8> = Vec::new();
    if dec.is_empty() {
        return Err("an empty number".into());
    }
    for d in dec.bytes() {
        if !d.is_ascii_digit() {
            return Err(format!("not a decimal: {dec}"));
        }
        let mut carry = u32::from(d - b'0');
        for b in le.iter_mut() {
            let v = u32::from(*b) * 10 + carry;
            *b = v as u8;
            carry = v >> 8;
        }
        while carry > 0 {
            le.push(carry as u8);
            carry >>= 8;
        }
    }
    Ok(Int::from_le(&le))
}

// ------------------------------------------------------------ values in

/// String(x) of a value noun() refuses, for its TypeError
fn js_string(j: &J) -> String {
    match j {
        J::Null => "null".into(),
        J::Bool(b) => b.to_string(),
        J::Number(n) => urb::js::number_text(n.as_f64().unwrap_or(f64::NAN)),
        J::String(s) => s.clone(),
        J::Array(v) => v
            .iter()
            .map(|x| match x {
                J::Null => String::new(),
                x => js_string(x),
            })
            .collect::<Vec<_>>()
            .join(","),
        J::Object(o) if o.contains_key("$u") => "undefined".into(),
        J::Object(o) if o.contains_key("$big") => js_string(&o["$big"]),
        J::Object(_) => "[object Object]".into(),
    }
}

/// a value for noun() (trace.mjs's encoding), with noun()'s errors:
/// BigInt()'s RangeError for a number that is not whole, and a
/// TypeError for what it can't make a noun of
pub fn value(j: &J) -> Result<Value<'static>, Failure> {
    let cant = || Failure::type_error(format!("can't make a noun of {}", js_string(j)));
    Ok(match j {
        J::Null => Value::Null,
        J::Bool(b) => Value::Bool(*b),
        J::Number(n) => {
            if let Some(u) = n.as_u64() {
                Value::Int(u.into())
            } else {
                let f = n.as_f64().unwrap_or(f64::NAN);
                if f.fract() != 0.0 || !f.is_finite() {
                    return Err(Failure::range_error(format!(
                        "The number {} cannot be converted to a BigInt because it is not an integer",
                        urb::js::number_text(f)
                    )));
                }
                // BigInt(x) <= 0 makes no bytes; a whole float past
                // 2^64 is its exact integer
                Value::Int(urb::js::bigint_of_number(f.max(0.0))?)
            }
        }
        J::String(s) => Value::Text(s.clone()),
        J::Array(v) if v.len() >= 2 => Value::Array(v.iter().map(value).collect::<Result<_, _>>()?),
        J::Array(_) => return Err(cant()),
        J::Object(o) if o.contains_key("$big") => {
            let d = o["$big"].as_str().ok_or_else(cant)?;
            match d.strip_prefix('-') {
                Some(_) => Value::Int(Int::default()),
                None => Value::Int(big(d).map_err(Failure::type_error)?),
            }
        }
        J::Object(o) if o.contains_key("$map") => Value::Map(
            o["$map"]
                .as_array()
                .ok_or_else(cant)?
                .iter()
                .map(|kv| Ok((value(&kv[0])?, value(&kv[1])?)))
                .collect::<Result<_, Failure>>()?,
        ),
        J::Object(o) if o.contains_key("$set") => Value::Set(
            o["$set"]
                .as_array()
                .ok_or_else(cant)?
                .iter()
                .map(value)
                .collect::<Result<_, _>>()?,
        ),
        _ => return Err(cant()),
    })
}

/// a plain JS value, as trace.mjs's dec() gives it
pub fn json(j: &J) -> Json {
    match j {
        J::Null => Json::Null,
        J::Bool(b) => Json::Bool(*b),
        J::Number(n) => Json::Num(n.as_f64().unwrap_or(f64::NAN)),
        J::String(s) => Json::Str(s.clone()),
        J::Array(v) => Json::Arr(v.iter().map(json).collect()),
        J::Object(o) if o.contains_key("$num") => {
            Json::Num(urb::js::number(o["$num"].as_str().unwrap_or("NaN")))
        }
        J::Object(o) => {
            let mut out = Object::default();
            for (k, v) in o {
                out.set(k.clone(), json(v));
            }
            Json::Obj(out)
        }
    }
}

fn text(j: Option<&J>) -> Result<String, String> {
    j.and_then(J::as_str)
        .map(String::from)
        .ok_or_else(|| format!("not text: {}", j.unwrap_or(&J::Null)))
}

fn opt(j: Option<&J>) -> Option<String> {
    j.and_then(J::as_str).map(String::from)
}

fn num(j: Option<&J>) -> Result<f64, String> {
    match j {
        Some(J::Number(n)) => Ok(n.as_f64().unwrap_or(f64::NAN)),
        Some(J::Object(o)) if o.contains_key("$num") => {
            Ok(urb::js::number(o["$num"].as_str().unwrap_or("NaN")))
        }
        j => Err(format!("not a number: {}", j.unwrap_or(&J::Null))),
    }
}

/// an id, as forum.mjs's idOf is given it
fn id(j: Option<&J>) -> Result<Id, String> {
    Ok(match j {
        Some(J::Number(n)) => Id::Num(n.as_f64().unwrap_or(f64::NAN)),
        Some(J::String(s)) => Id::Text(s.clone()),
        Some(J::Object(o)) if o.contains_key("$big") => {
            let d = text(o.get("$big"))?;
            match d.strip_prefix('-') {
                Some(m) => Id::Big(true, big(m)?),
                None => Id::Big(false, big(&d)?),
            }
        }
        Some(J::Object(o)) if o.contains_key("$num") => {
            Id::Num(urb::js::number(o["$num"].as_str().unwrap_or("NaN")))
        }
        j => return Err(format!("no id {}", j.unwrap_or(&J::Null))),
    })
}

fn target(j: Option<&J>) -> Result<Target, String> {
    let j = j.ok_or("no target")?;
    Ok(Target {
        post: id(j.get("post"))?,
        comment: match j.get("comment") {
            None | Some(J::Null) => None,
            Some(J::Object(o)) if o.contains_key("$u") => None,
            c => Some(id(c)?),
        },
    })
}

fn absent(j: Option<&J>) -> bool {
    match j {
        None | Some(J::Null) => true,
        Some(J::Object(o)) => o.contains_key("$u"),
        _ => false,
    }
}

/// furum(urb, host)[method](...a)
fn furum_call(method: &str, a: &[J]) -> Result<FurumCall, String> {
    Ok(match method {
        "nextPost" => FurumCall::NextPost(text(a.first())?),
        "posts" => FurumCall::Posts(text(a.first())?),
        "comments" => FurumCall::Comments(text(a.first())?, num(a.get(1))?),
        "votes" => FurumCall::Votes(text(a.first())?, num(a.get(1))?),
        "newPost" => {
            let p = a.get(1).ok_or("newPost: no post")?;
            FurumCall::NewPost {
                name: text(a.first())?,
                title: text(p.get("title"))?,
                url: opt(p.get("url")),
                body: opt(p.get("body")),
            }
        }
        "newComment" => FurumCall::NewComment {
            name: text(a.first())?,
            post: num(a.get(1))?,
            body: text(a.get(2))?,
            parent: if absent(a.get(3)) {
                None
            } else {
                Some(num(a.get(3))?)
            },
        },
        "upvote" => FurumCall::Upvote(text(a.first())?, num(a.get(1))?),
        "onNote" => FurumCall::OnNote,
        _ => return Err(format!("no furum call {method}")),
    })
}

/// forum(urb, host)[method](...a)
fn forum_call(method: &str, a: &[J]) -> Result<ForumCall, String> {
    Ok(match method {
        "board" => ForumCall::Board(text(a.first())?),
        "thread" => ForumCall::Thread(text(a.first())?, id(a.get(1))?),
        "newPost" => {
            let p = a.get(1).ok_or("newPost: no post")?;
            ForumCall::NewPost {
                name: text(a.first())?,
                title: opt(p.get("title")),
                url: opt(p.get("url")),
                body: opt(p.get("body")),
            }
        }
        "newComment" => ForumCall::NewComment {
            name: text(a.first())?,
            post: id(a.get(1))?,
            body: opt(a.get(2)),
            parent: if absent(a.get(3)) {
                None
            } else {
                Some(id(a.get(3))?)
            },
        },
        "upvote" => ForumCall::Upvote(text(a.first())?, target(a.get(1))?),
        "downvote" => ForumCall::Downvote(text(a.first())?, target(a.get(1))?),
        "unvote" => ForumCall::Unvote(text(a.first())?, target(a.get(1))?),
        "onNote" => ForumCall::OnNote,
        _ => return Err(format!("no forum call {method}")),
    })
}

/// A call (trace.mjs's op and args) as a request. Err(Failure) is the
/// call's own error, which it settles with, as the JS throws it; Err of
/// the outer Result is the shim's mistake.
pub fn request(op: &str, a: &[J]) -> Result<Result<Request, Failure>, String> {
    let empty = json!({});
    Ok(Ok(match op {
        "open" => {
            let load = a.first().unwrap_or(&empty);
            let opts = a.get(1).unwrap_or(&empty);
            Request::Open {
                keyfile: opt(load.get("keyfile")),
                rows: match load.get("rows").and_then(J::as_str) {
                    Some(h) => Some(unhex(h)?),
                    None => None,
                },
                fresh: load.get("fresh").and_then(J::as_bool).unwrap_or(false),
                fake: opts.get("fake").and_then(J::as_bool).unwrap_or(false),
            }
        }
        "close" => Request::Close,
        "read" => Request::Read(text(a.first())?),
        "poke" => {
            let mark = a.get(3).ok_or("poke: no mark")?;
            let value = match value(a.get(4).unwrap_or(&J::Null)) {
                Ok(v) => v,
                Err(f) => return Ok(Err(f)),
            };
            Request::Poke {
                ship: text(a.first())?,
                folder: text(a.get(1))?,
                name: text(a.get(2))?,
                mark: (text(mark.get(0))?, text(mark.get(1))?),
                value,
            }
        }
        "peek" => Request::Peek {
            ship: text(a.first())?,
            folder: text(a.get(1))?,
            name: opt(a.get(2)),
            conv: match a.get(3).and_then(J::as_str) {
                Some("cord") => Conv::Cord,
                Some("text") => Conv::Text,
                Some("exists") => Conv::Exists,
                Some("files") => Conv::Files,
                c => return Err(format!("no converter {}", c.unwrap_or("?"))),
            },
            wait: a.get(4).and_then(J::as_u64),
        },
        "inbox" => Request::Inbox(a.first().and_then(J::as_bool).unwrap_or(true)),
        "pub" => {
            let st = a.first().map_or(Json::Obj(Object::default()), json);
            let Json::Obj(st) = st else {
                return Err("pub: not an object".into());
            };
            Request::Pub {
                pages: match st.get("pages") {
                    Some(Json::Obj(o)) => Some(o.clone()),
                    _ => None,
                },
                keepers: st.get("keepers").cloned(),
            }
        }
        "pub.put" => Request::PubPut {
            name: text(a.first())?,
            body: text(a.get(1))?,
            mark: opt(a.get(2)),
        },
        "pub.remove" => Request::PubRemove(text(a.first())?),
        "pub.pages" => Request::PubPages,
        // furum(urb, null).onNote: the host is not used
        "furum" => Request::Furum {
            host: opt(a.first()).unwrap_or_default(),
            call: furum_call(&text(a.get(1))?, a.get(2..).unwrap_or(&[]))?,
        },
        "forum" => Request::Forum {
            host: opt(a.first()).unwrap_or_default(),
            call: forum_call(&text(a.get(1))?, a.get(2..).unwrap_or(&[]))?,
        },
        _ => return Err(format!("no call {op}")),
    }))
}

/// An input, as a trace's step has it (its `in`), or as a shim makes
/// one. A call's id is `id`, or `k` when the trace gives none. Ok(Err)
/// is a call that fails before the core sees it: its id and error.
pub fn input(i: &J, k: u64) -> Result<Result<Input, (u64, Failure)>, String> {
    let u = |f: &str| {
        i.get(f)
            .and_then(J::as_u64)
            .ok_or_else(|| format!("no {f} in {i}"))
    };
    Ok(Ok(match i.get("k").and_then(J::as_str) {
        Some("call") => {
            let id = i.get("id").and_then(J::as_u64).unwrap_or(k);
            let op = text(i.get("op"))?;
            let args = i
                .get("args")
                .and_then(J::as_array)
                .cloned()
                .unwrap_or_default();
            match request(&op, &args)? {
                Ok(req) => Input::Call { id, req },
                Err(f) => return Ok(Err((id, f))),
            }
        }
        Some("hear") => Input::Hear {
            ip: u("ip")? as u32,
            port: u("port")? as u16,
            bytes: unhex(&text(i.get("bytes"))?)?,
        },
        Some("tick") => Input::Tick {
            kind: text(i.get("kind"))?,
            due: u("due")?,
        },
        Some("fetch") => Input::Fetched {
            id: u("id")?,
            answer: match i.get("error") {
                Some(e) => Err(text(Some(e))?),
                None => Ok((u("status")? as u16, unhex(&text(i.get("body"))?)?)),
            },
        },
        other => return Err(format!("no input {}", other.unwrap_or("?"))),
    }))
}

// ------------------------------------------------------------ values out

/// a plain JS value as trace.mjs's enc() writes it
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

/// an app's value as trace.mjs's enc() writes it
pub fn js_enc(j: &Js) -> J {
    match j {
        Js::Null => J::Null,
        Js::Bool(b) => J::Bool(*b),
        Js::Num(n) if !n.is_finite() => json!({ "$num": urb::js::number_text(*n) }),
        Js::Num(n) if n.fract() == 0.0 && n.abs() < 9007199254740992.0 => J::from(*n as i64),
        Js::Num(n) => J::from(*n),
        Js::Big(i) => json!({ "$big": i.decimal() }),
        Js::Str(s) => J::String(s.clone()),
        Js::Date(d) => json!({ "$date": d.map(urb::js::iso) }),
        Js::Arr(v) => J::Array(v.iter().map(js_enc).collect()),
        Js::Obj(o) => J::Object(
            o.entries()
                .into_iter()
                .map(|(k, v)| (k.to_string(), js_enc(v)))
                .collect(),
        ),
    }
}

/// an error as trace.mjs encodes it
pub fn err(f: &Failure) -> J {
    let mut m = Map::new();
    m.insert("$err".into(), J::String(f.message.clone()));
    if f.later {
        m.insert("later".into(), J::Bool(true));
    }
    if f.kind == Kind::UrbError {
        m.insert("urb".into(), J::Bool(true));
    }
    J::Object(m)
}

fn kind(k: Kind) -> &'static str {
    match k {
        Kind::UrbError => "UrbError",
        Kind::Error => "Error",
        Kind::TypeError => "TypeError",
        Kind::RangeError => "RangeError",
        Kind::SyntaxError => "SyntaxError",
    }
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
        Reply::Js(j) => js_enc(j),
    }
}

/// a call that failed before the core saw it, as its result
pub fn failed(id: u64, f: &Failure) -> J {
    json!({ "result": id, "err": err(f), "kind": kind(f.kind) })
}

/// an output as trace.mjs encodes it, with what a shim needs beside
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
            Err(f) => failed(*call, f),
        },
        Output::Inbox(m) => json!({
            "inbox": m.from,
            "to": [m.to.0, m.to.1],
            "mark": [m.mark.0, m.mark.1],
            "jam": hex(&m.noun.jam()),
        }),
        Output::Event { name, value } => json!({ "event": name, "value": unjson(value) }),
        Output::Note { name, value } => json!({ "event": name, "value": js_enc(value) }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_noun_refuses_fail_as_the_js_fails() {
        let e = value(&json!(1.5)).unwrap_err();
        assert_eq!(e.kind, Kind::RangeError);
        assert_eq!(
            e.message,
            "The number 1.5 cannot be converted to a BigInt because it is not an integer"
        );
        let e = value(&json!([1])).unwrap_err();
        assert_eq!(
            (e.kind, e.message.as_str()),
            (Kind::TypeError, "can't make a noun of 1")
        );
        let e = value(&json!({"a": 1})).unwrap_err();
        assert_eq!(e.message, "can't make a noun of [object Object]");
        // BigInt(-3) makes no bytes: the atom 0
        assert!(matches!(value(&json!(-3)), Ok(Value::Int(i)) if i.is_zero()));
        assert!(matches!(value(&json!({"$big": "-7"})), Ok(Value::Int(i)) if i.is_zero()));
    }

    #[test]
    fn a_call_the_shim_got_wrong_is_its_mistake() {
        assert!(input(&json!({"k": "call", "op": "nope", "args": []}), 0).is_err());
        assert!(input(&json!({"k": "hear", "ip": 1, "port": 2, "bytes": "abc"}), 0).is_err());
        assert!(input(&json!({"k": "later"}), 0).is_err());
        let r = input(
            &json!({"k": "call", "id": 7, "op": "poke", "args": ["~zod", "/a", "b", ["m", "n"], 0.5]}),
            0,
        );
        assert!(matches!(r, Ok(Err((7, f))) if f.kind == Kind::RangeError));
    }

    #[test]
    fn big_and_hex_round_trip() {
        assert_eq!(
            big("18446744073709551616").unwrap().decimal(),
            "18446744073709551616"
        );
        assert!(big("").is_err() && big("1a").is_err());
        assert_eq!(
            unhex(&hex(&[0, 1, 0xab, 0xff])).unwrap(),
            vec![0, 1, 0xab, 0xff]
        );
    }
}
