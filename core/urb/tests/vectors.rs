//! The conformance vectors of nouns and codecs (conformance/*.json, made
//! by the JS), run from Rust: every case's output, or its error, must be
//! the vector's, and every case ends with no noun alive. A hostile
//! vector's error must contain its text, within its time.
//!
//! M2 runs the codec vectors: nouns.json and hash.json whole, and the
//! cases of decoders.json and hostile-nouns.json that are mesa.mjs's own
//! decoders. The rest (furum's and forum's reads, the messages, deUrb and
//! the renderers) are later milestones'; the counts below fail the test
//! if a case of a function M2 runs is skipped.

mod common;

use common::{cue_all, hex, value};
use serde_json::{Map, Value as J, json};
use std::time::Instant;
use urb::codec;
use urb::js::Json;
use urb::noun::{self, Noun, NounRef, Value};
use urb::{Int, alive, text};

/// each file, and how many of its cases M2 runs
const FILES: &[(&str, &str, usize)] = &[
    ("nouns", include_str!("../../../conformance/nouns.json"), 62),
    ("hash", include_str!("../../../conformance/hash.json"), 140),
    (
        "decoders",
        include_str!("../../../conformance/decoders.json"),
        23,
    ),
    (
        "hostile-nouns",
        include_str!("../../../conformance/hostile-nouns.json"),
        30,
    ),
];

/// the functions M2 runs
const M2: &[&str] = &[
    "noun", "sham", "mug", "uv", "da", "json", "tang", "tangText", "pathOf", "segments", "measure",
    "list", "entries", "fileOf", "ballOf", "ship",
];

/// mesa.mjs's json() result as trace.mjs encodes it
fn json_out(v: &Json) -> J {
    match v {
        Json::Null => J::Null,
        Json::Bool(b) => J::Bool(*b),
        Json::Num(n) => {
            serde_json::Number::from_f64(*n).map_or_else(|| json!({"$num": js_num(*n)}), J::Number)
        }
        Json::Str(s) => J::String(s.clone()),
        Json::Arr(a) => J::Array(a.iter().map(json_out).collect()),
        Json::Obj(o) => {
            let mut m = Map::new();
            for (k, x) in o.entries() {
                m.insert(k.into(), json_out(x));
            }
            J::Object(m)
        }
    }
}

fn js_num(n: f64) -> &'static str {
    if n.is_nan() {
        "NaN"
    } else if n > 0.0 {
        "Infinity"
    } else {
        "-Infinity"
    }
}

/// as JSON.stringify compares them: in order, numbers by value
fn same(a: &J, b: &J) -> bool {
    match (a, b) {
        (J::Number(x), J::Number(y)) => x.as_f64() == y.as_f64(),
        (J::Array(x), J::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| same(p, q))
        }
        (J::Object(x), J::Object(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|((k, p), (l, q))| k == l && same(p, q))
        }
        _ => a == b,
    }
}

fn one<'a>(n: &Value<'a>) -> NounRef<'a> {
    match n {
        Value::Noun(r) => *r,
        _ => panic!("not a noun: {n:?}"),
    }
}

fn cord_of(n: NounRef<'_>) -> J {
    match codec::cord(n) {
        Ok(Some(s)) => J::String(s),
        _ => J::Null,
    }
}

/// a case's function, on its arguments
fn run<'a>(f: &str, a: &[Value<'a>]) -> Result<J, String> {
    let e = |r: urb::UrbError| r.message;
    let t = |r: noun::TypeError| r.0;
    match f {
        "noun" => Noun::of(&a[0]).map(|n| J::String(hex(&n.jam()))).map_err(t),
        "sham" => noun::sham(&a[0])
            .map(|h| J::String(format!("{h:x}")))
            .map_err(t),
        "mug" => noun::mug(&a[0]).map(|m| json!(m)).map_err(t),
        "uv" => match &a[0] {
            Value::Int(i) => Ok(J::String(text::uv(i))),
            _ => panic!("uv of {:?}", a[0]),
        },
        "da" => match &a[0] {
            Value::Int(i) => {
                Ok(json!({"$big": Int::from(text::da(i.to_u64().unwrap())).decimal()}))
            }
            _ => panic!("da of {:?}", a[0]),
        },
        "json" => codec::json(one(&a[0])).map(|v| json_out(&v)).map_err(e),
        "tang" => Ok(J::String(codec::tang(one(&a[0])))),
        "tangText" => {
            let mut b = codec::Budget::new();
            codec::measure_in(one(&a[0]), &mut b)
                .and_then(|m| codec::tang_text(m, &mut b))
                .map(J::String)
                .map_err(e)
        }
        "pathOf" => codec::path_of(one(&a[0])).map(J::String).map_err(e),
        "segments" => codec::segments(one(&a[0])).map(|s| json!(s)).map_err(e),
        "measure" => codec::measure(one(&a[0])).map(|_| J::Bool(true)).map_err(e),
        "list" => codec::items(one(&a[0])).map(|v| json!(v.len())).map_err(e),
        "entries" => codec::entries(one(&a[0]))
            .map(|v| json!(v.len()))
            .map_err(e),
        "fileOf" => codec::file_of(one(&a[0]), one(&a[1]), cord_of)
            .map(|v| v.unwrap_or(J::Null))
            .map_err(e),
        "ballOf" => codec::ball_of(one(&a[0]), one(&a[1]), |files| {
            J::Array(
                files
                    .into_iter()
                    .map(|(p, n)| json!([p, cord_of(n)]))
                    .collect(),
            )
        })
        .map(|v| v.unwrap_or(J::Null))
        .map_err(e),
        "ship" => codec::ship(one(&a[0])).map(J::String).map_err(e),
        _ => unreachable!("{f} is not in M2"),
    }
}

#[test]
fn every_codec_vector() {
    let mut bad = 0;
    for (name, src, want) in FILES {
        let v: J = serde_json::from_str(src).unwrap();
        let quick = v.get("quick").and_then(J::as_u64);
        let (mut ran, mut fails) = (0, 0);
        for (i, c) in v["cases"].as_array().unwrap().iter().enumerate() {
            let f = c["fn"].as_str().unwrap();
            if !M2.contains(&f) {
                continue;
            }
            let mut nouns = Vec::new();
            cue_all(&c["in"], &mut nouns);
            let start = Instant::now();
            let got = {
                let mut at = 0;
                let args: Vec<Value<'_>> = c["in"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|x| value(x, &nouns, &mut at))
                    .collect();
                run(f, &args)
            };
            let ms = start.elapsed().as_millis() as u64;
            drop(nouns);
            ran += 1;
            let ok = match (&got, c.get("error")) {
                (Err(m), Some(J::String(w))) => m == w,
                (Ok(o), None) => same(o, &c["out"]),
                _ => false,
            };
            let slow = quick.is_some_and(|q| ms > q);
            assert_eq!(alive(), 0, "{name} {i} {f} left nouns alive");
            if !ok || slow {
                fails += 1;
                if fails <= 5 {
                    let input = c["in"].to_string();
                    eprintln!(
                        "  {name} {i} {f}({}){}\n    want {}\n    got  {:?}",
                        input.chars().take(100).collect::<String>(),
                        if slow {
                            format!(" took {ms} ms")
                        } else {
                            String::new()
                        },
                        c.get("error").unwrap_or(&c["out"]),
                        got
                    );
                }
            }
        }
        eprintln!(
            "{}  {name}: {}/{ran}",
            if fails > 0 { "FAIL" } else { "ok  " },
            ran - fails
        );
        assert_eq!(ran, *want, "{name}: M2 ran {ran} cases, not {want}");
        bad += fails;
    }
    assert_eq!(bad, 0, "{bad} codec vectors fail");
}
