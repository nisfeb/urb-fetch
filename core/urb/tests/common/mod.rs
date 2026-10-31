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
