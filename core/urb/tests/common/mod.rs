//! What the Rust tests share: the vectors' value encoding (trace.mjs's),
//! read as Values, as the JS reads it.

#![allow(dead_code)]

use serde_json::Value as J;
use std::collections::HashSet;
use urb::Int;
use urb::noun::{Noun, Value};

pub fn unhex(h: &str) -> Vec<u8> {
    (0..h.len() / 2)
        .map(|i| u8::from_str_radix(&h[i * 2..i * 2 + 2], 16).expect("hex"))
        .collect()
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn big(dec: &str) -> Int {
    let mut le: Vec<u8> = Vec::new();
    for d in dec.bytes() {
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
    Int::from_le(&le)
}

/// every {"$noun"} in a vector's arguments, cued, in the order `value`
/// reads them
pub fn cue_all(j: &J, out: &mut Vec<Noun>) {
    match j {
        J::Object(o) if o.contains_key("$noun") => {
            out.push(Noun::cue(&unhex(o["$noun"].as_str().unwrap())).expect("a vector's jam cues"));
        }
        J::Array(v) => v.iter().for_each(|x| cue_all(x, out)),
        J::Object(o) => o.values().for_each(|x| cue_all(x, out)),
        _ => {}
    }
}

/// a JS Map's or Set's key as SameValueZero sees it: a primitive, or
/// nothing for an array, which is a key of its own
#[derive(PartialEq, Eq, Hash)]
enum Prim {
    Null,
    Bool(bool),
    Num(u64),
    Big(String),
    Str(String),
}

fn prim(j: &J) -> Option<Prim> {
    Some(match j {
        J::Null => Prim::Null,
        J::Bool(b) => Prim::Bool(*b),
        J::Number(n) => Prim::Num(n.as_f64().unwrap().to_bits()),
        J::String(s) => Prim::Str(s.clone()),
        J::Object(o) if o.contains_key("$big") => Prim::Big(o["$big"].as_str().unwrap().into()),
        _ => return None,
    })
}

/// a vector's value (trace.mjs's encoding) as a Value, its nouns from
/// `nouns` in order
pub fn value<'a>(j: &J, nouns: &'a [Noun], at: &mut usize) -> Value<'a> {
    match j {
        J::Null => Value::Null,
        J::Bool(b) => Value::Bool(*b),
        J::Number(n) => Value::Int(n.as_u64().expect("a whole number").into()),
        J::String(s) => Value::Text(s.clone()),
        J::Array(v) => Value::Array(v.iter().map(|x| value(x, nouns, at)).collect()),
        J::Object(o) if o.contains_key("$noun") => {
            *at += 1;
            Value::Noun(nouns[*at - 1].borrow())
        }
        J::Object(o) if o.contains_key("$big") => Value::Int(big(o["$big"].as_str().unwrap())),
        J::Object(o) if o.contains_key("$map") => {
            // new Map(entries): a key seen again keeps its place, takes
            // the later value
            let mut out: Vec<(Value<'a>, Value<'a>)> = Vec::new();
            let mut seen: Vec<Option<Prim>> = Vec::new();
            for kv in o["$map"].as_array().unwrap() {
                let (k, v) = (value(&kv[0], nouns, at), value(&kv[1], nouns, at));
                let p = prim(&kv[0]);
                match p
                    .as_ref()
                    .and_then(|p| seen.iter().position(|s| s.as_ref() == Some(p)))
                {
                    Some(i) => out[i].1 = v,
                    None => {
                        seen.push(p);
                        out.push((k, v));
                    }
                }
            }
            Value::Map(out)
        }
        J::Object(o) if o.contains_key("$set") => {
            let mut out = Vec::new();
            let mut seen = HashSet::new();
            for x in o["$set"].as_array().unwrap() {
                let v = value(x, nouns, at);
                if prim(x).is_none_or(|p| seen.insert(p)) {
                    out.push(v);
                }
            }
            Value::Set(out)
        }
        J::Object(o) => panic!("no such value in M2's vectors: {o:?}"),
    }
}

// ------------------------------------------------------------ vere

/// a tool the batch needs, or None (and a note) if it is missing, which
/// fails with REQUIRE_ALL set
#[cfg(not(target_os = "wasi"))]
pub fn tool(var: &str, name: &str, why: &str) -> Option<String> {
    let p = std::env::var(var).unwrap_or_else(|_| name.into());
    let found = std::process::Command::new(&p)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok();
    if found {
        return Some(p);
    }
    assert!(
        std::env::var_os("REQUIRE_ALL").is_none(),
        "no {name} ({var}), which {why} needs"
    );
    eprintln!("skip: no {name} ({var}), which {why} needs");
    None
}

#[cfg(not(target_os = "wasi"))]
pub fn run_tool(cmd: &str, args: &[&str], input: &str, dir: Option<&str>) -> String {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut c = Command::new(cmd);
    c.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    if let Some(d) = dir {
        c.current_dir(d);
    }
    let mut child = c.spawn().expect("spawn");
    let mut stdin = child.stdin.take().unwrap();
    let input = input.to_owned();
    let w = std::thread::spawn(move || stdin.write_all(input.as_bytes()));
    let out = child.wait_with_output().expect("wait");
    w.join().unwrap().expect("write stdin");
    assert!(out.status.success(), "{cmd} failed");
    String::from_utf8(out.stdout).expect("utf-8")
}

/// an atom as a hoon @ux literal, in dotted groups of four
pub fn ux(i: &Int) -> String {
    let mut h: String = i.le().iter().rev().map(|b| format!("{b:02x}")).collect();
    let h = h.split_off(h.find(|c| c != '0').unwrap_or(h.len()));
    if h.is_empty() {
        return "0x0".into();
    }
    let lead = match h.len() % 4 {
        0 => 4,
        n => n,
    };
    let mut out = format!("0x{}", &h[..lead]);
    for g in h.as_bytes()[lead..].chunks(4) {
        out.push('.');
        out.push_str(std::str::from_utf8(g).unwrap());
    }
    out
}

/// vere's output without its terminal escapes: a CSI (ESC [, its
/// parameters and intermediates, then one final byte in 0x40-0x7E), an
/// OSC (ESC ], to BEL or ESC backslash), a charset choice (ESC ( or ESC ) and
/// one more), or any other ESC and the one character after it
pub fn unescaped(out: &str) -> String {
    let mut clean = String::new();
    let mut it = out.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\x1b' {
            clean.push(c);
            continue;
        }
        match it.next() {
            Some('[') => {
                for c in it.by_ref() {
                    if ('\x40'..='\x7e').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(c) = it.next() {
                    if c == '\x07' || (c == '\x1b' && it.next_if_eq(&'\\').is_some()) {
                        break;
                    }
                }
            }
            Some('(' | ')') => {
                it.next();
            }
            _ => {}
        }
    }
    clean
}

/// vere's answers, as hex digits or @uv text
pub fn answers(out: &str) -> Vec<String> {
    let clean = unescaped(out);
    clean
        .split(|c: char| c.is_whitespace() || c == '[' || c == ']')
        .filter_map(|w| {
            if let Some(h) = w.strip_prefix("0x") {
                Some(h.replace('.', ""))
            } else {
                w.strip_prefix("~.").map(str::to_owned)
            }
        })
        .collect()
}

// ------------------------------------------------------------ apps

/// an app's JS value as trace.mjs's enc() writes it
pub fn js_enc(j: &urb::js::Js) -> J {
    use serde_json::json;
    use urb::js::Js;
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
