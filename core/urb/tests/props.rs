//! Property tests of the nouns and codecs.
//!
//! - Against vere (`urbit eval`, as check.mjs runs it; URBIT names it):
//!   random maps and sets must hash (+sham) as +malt and +silt build
//!   them, and random nouns, numbers and dates must mug, uv and da as
//!   Hoon has them.
//! - Against the JS (node, with wasm/build/mesa.wasm): random JS-shaped
//!   values must make the noun mesa.mjs's noun() makes, or the same
//!   error; random bytes must read as the same text; random text must
//!   be the same Number().
//! - In Rust: $json, paths, tapes and jams must round-trip.
//!
//! The batches draw from a seed, printed, which URB_SEED sets. vere and
//! node are skipped where they are missing, but with REQUIRE_ALL set, as
//! CI sets it, a missing one fails. Under WASI only the Rust ones run.
//! Every test ends with no noun alive.

mod common;

use proptest::prelude::*;
use serde_json::{Value as J, json};
use urb::codec::{self, MAX_CELLS};
use urb::js::{self, Json};
use urb::noun::{self, Noun, Value};
use urb::{Int, alive, text};

// ------------------------------------------------------------ batches

/// splitmix64
struct Rng(u64);

impl Rng {
    fn seeded(what: &str) -> Rng {
        let seed = std::env::var("URB_SEED")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(|| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(1, |d| d.as_nanos() as u64)
            });
        eprintln!("{what}: URB_SEED={seed}");
        Rng(seed)
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }

    fn chance(&mut self, pct: u64) -> bool {
        self.below(100) < pct
    }

    fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next() as u8).collect()
    }

    /// lo to lo + span - 1 random bytes
    fn some_bytes(&mut self, lo: u64, span: u64) -> Vec<u8> {
        let n = lo + self.below(span);
        self.bytes(n as usize)
    }
}

/// a tool the batch needs, or None (and a note) if it is missing, which
/// fails with REQUIRE_ALL set
#[cfg(not(target_os = "wasi"))]
fn tool(var: &str, name: &str, why: &str) -> Option<String> {
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
fn run_tool(cmd: &str, args: &[&str], input: &str, dir: Option<&str>) -> String {
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
fn ux(i: &Int) -> String {
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

/// a number as hoon writes it, in dotted groups of three
fn ud(n: u64) -> String {
    let d = n.to_string();
    let lead = match d.len() % 3 {
        0 => 3,
        l => l,
    };
    let mut out = d[..lead].to_owned();
    for g in d.as_bytes()[lead..].chunks(3) {
        out.push('.');
        out.push_str(std::str::from_utf8(g).unwrap());
    }
    out
}

/// a value as hoon, as noun() makes its noun; maps and sets by malt and
/// silt
fn hoon(v: &Value<'_>) -> String {
    match v {
        Value::Null | Value::Bool(true) => "0x0".into(),
        Value::Bool(false) => "0x1".into(),
        Value::Int(i) => ux(i),
        Value::Text(s) => ux(&Int::from_le(s.as_bytes())),
        Value::Array(a) => format!("[{}]", a.iter().map(hoon).collect::<Vec<_>>().join(" ")),
        Value::Map(m) => {
            let kv: Vec<String> = m
                .iter()
                .map(|(k, v)| format!("[{} {}]", hoon(k), hoon(v)))
                .collect();
            format!("(malt `(list [* *])`{})", hoon_list(&kv))
        }
        Value::Set(s) => {
            let k: Vec<String> = s.iter().map(hoon).collect();
            format!("(silt `(list *)`{})", hoon_list(&k))
        }
        Value::Noun(_) => unreachable!("no nouns in the batches"),
    }
}

fn hoon_list(items: &[String]) -> String {
    if items.is_empty() {
        "~".into()
    } else {
        format!("~[{}]", items.join(" "))
    }
}

/// a random cord: knot-ish, or any unicode
fn cord(r: &mut Rng) -> String {
    let n = r.below(12) as usize;
    if r.chance(70) {
        (0..n)
            .map(|_| b"abcdefghijklmnopqrstuvwxyz0123456789-~."[r.below(39) as usize] as char)
            .collect()
    } else {
        (0..n)
            .filter_map(|_| char::from_u32(r.below(0x11000) as u32))
            .collect()
    }
}

/// a random noun-shaped key or value: atoms small and big, cords, paths
/// and cells, and now and then a map or set inside
fn key(r: &mut Rng, depth: u32) -> Value<'static> {
    match r.below(if depth > 2 { 4 } else { 7 }) {
        0 => Value::Int(r.below(1000).into()),
        1 => Value::Int(Int::from_le(&r.some_bytes(1, 40))),
        2 => Value::Text(cord(r)),
        3 => Value::Int(r.next().into()),
        4 => {
            let mut p: Vec<Value<'static>> =
                (0..1 + r.below(4)).map(|_| Value::Text(cord(r))).collect();
            p.push(Value::Null);
            Value::Array(p)
        }
        5 => Value::Array((0..2 + r.below(2)).map(|_| key(r, depth + 1)).collect()),
        _ if r.chance(50) => {
            let n = r.below(6) as usize;
            Value::Set(distinct(r, n, depth + 1))
        }
        _ => {
            let n = r.below(6) as usize;
            let ks = distinct(r, n, depth + 1);
            Value::Map(ks.into_iter().map(|k| (k, key(r, depth + 1))).collect())
        }
    }
}

/// n keys, no two the same noun, as +malt's and +silt's inputs must be
/// for their maps to be the JS's
fn distinct(r: &mut Rng, n: usize, depth: u32) -> Vec<Value<'static>> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    while out.len() < n {
        let k = key(r, depth);
        if seen.insert(Noun::of(&k).unwrap().borrow().jam()) {
            out.push(k);
        }
    }
    out
}

#[cfg(not(target_os = "wasi"))]
#[test]
fn maps_and_sets_hash_as_vere_builds_them() {
    let Some(urbit) = tool("URBIT", "urbit", "the vere checks") else {
        return;
    };
    let mut r = Rng::seeded("maps_and_sets_hash_as_vere_builds_them");
    // [hoon, what Rust makes of it]
    let mut cases: Vec<(String, String)> = Vec::new();
    for i in 0..600 {
        let n = if i % 50 == 0 {
            100 + r.below(300)
        } else {
            r.below(40)
        } as usize;
        let ks = distinct(&mut r, n, 0);
        let v = if i % 2 == 0 {
            Value::Map(ks.into_iter().map(|k| (k, key(&mut r, 1))).collect())
        } else {
            Value::Set(ks)
        };
        cases.push((
            format!("`@ux`(sham {})", hoon(&v)),
            format!("{:x}", noun::sham(&v).unwrap()),
        ));
    }
    // and mug, uv and da
    for _ in 0..200 {
        let k = key(&mut r, 0);
        cases.push((
            format!("`@ux`(mug {})", hoon(&k)),
            format!("{:x}", noun::mug(&k).unwrap()),
        ));
        let i = Int::from_le(&r.some_bytes(0, 70));
        cases.push((format!("(scot %uv {})", ux(&i)), text::uv(&i)));
        let ms = r.below(8_640_000_000_000_000);
        cases.push((
            format!("`@ux`(from-unix-ms:chrono:userlib {})", ud(ms)),
            format!("{:x}", text::da(ms)),
        ));
    }
    let expr = format!(
        "~[{}]",
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
        "vere gave {} answers for {}: {}",
        got.len(),
        cases.len(),
        &out[out.len().saturating_sub(600)..]
    );
    let mut bad = 0;
    for ((h, want), have) in cases.iter().zip(&got) {
        if have != want {
            bad += 1;
            if bad <= 5 {
                eprintln!(
                    "MISMATCH {}\n  vere {have}\n  rust {want}",
                    &h[..h.len().min(200)]
                );
            }
        }
    }
    assert_eq!(bad, 0, "{bad} of {} disagree with vere", cases.len());
    assert_eq!(alive(), 0);
}

/// vere's answers, as hex digits or @uv text
fn answers(out: &str) -> Vec<String> {
    let mut clean = String::new();
    let mut esc = false;
    for c in out.chars() {
        match c {
            '\x1b' => esc = true,
            'm' if esc => esc = false,
            _ if esc => {}
            _ => clean.push(c),
        }
    }
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

/// a random JS value, in trace.mjs's encoding: what a caller of noun()
/// hands it, with arrays too short to be cells now and then, and Maps
/// and Sets whose keys repeat
fn js_value(r: &mut Rng, depth: u32) -> J {
    let leaf = depth > 3;
    match r.below(if leaf { 6 } else { 10 }) {
        0 => J::Null,
        1 => J::Bool(r.chance(50)),
        2 => json!(r.below(1 << 53)),
        3 => json!({"$big": Int::from_le(&r.some_bytes(0, 70)).decimal()}),
        4 | 5 => J::String(cord(r)),
        6 | 7 => {
            let n = if r.chance(5) {
                r.below(2)
            } else {
                2 + r.below(4)
            };
            J::Array((0..n).map(|_| js_value(r, depth + 1)).collect())
        }
        8 => {
            let n = r.below(8);
            let mut keys: Vec<J> = Vec::new();
            for _ in 0..n {
                keys.push(if !keys.is_empty() && r.chance(15) {
                    keys[r.below(keys.len() as u64) as usize].clone()
                } else {
                    js_value(r, depth + 1)
                });
            }
            json!({"$map": keys.into_iter().map(|k| json!([k, js_value(r, depth + 1)])).collect::<Vec<_>>()})
        }
        _ => {
            let mut keys: Vec<J> = Vec::new();
            for _ in 0..r.below(8) {
                keys.push(if !keys.is_empty() && r.chance(15) {
                    keys[r.below(keys.len() as u64) as usize].clone()
                } else {
                    js_value(r, depth + 1)
                });
            }
            json!({"$set": keys})
        }
    }
}

/// random bytes, mostly text: UTF-8, with byte-order marks, cut
/// sequences, overlong forms and surrogates now and then
fn js_bytes(r: &mut Rng) -> Vec<u8> {
    let mut b = Vec::new();
    for _ in 0..r.below(12) {
        match r.below(8) {
            0 => b.extend_from_slice(b"\xef\xbb\xbf"),
            1 => b.extend(r.some_bytes(1, 3)),
            2 => b.extend_from_slice(&[0xed, 0xa0 + r.below(32) as u8, 0x80]),
            3 => b.extend_from_slice(&[0xc0 + r.below(2) as u8, 0x80 + r.below(64) as u8]),
            4 => {
                let c = char::from_u32(r.below(0x110000) as u32).unwrap_or('x');
                let mut s = [0u8; 4];
                let e = c.encode_utf8(&mut s).as_bytes().to_vec();
                b.extend_from_slice(&e[..e.len() - r.below(e.len() as u64) as usize]);
            }
            _ => b.extend(cord(r).bytes()),
        }
    }
    b
}

/// random text for Number(): whitespace, signs, digits, points,
/// exponents, prefixes, Infinity, and junk
fn js_number_text(r: &mut Rng) -> String {
    const PARTS: &[&str] = &[
        " ",
        "\t",
        "\n",
        "\u{a0}",
        "\u{feff}",
        "\u{85}",
        "\u{2028}",
        "\u{3000}",
        "\u{180e}",
        "+",
        "-",
        "0",
        "1",
        "7",
        "9",
        "00",
        "123456789",
        ".",
        "e",
        "E",
        "e+",
        "e-",
        "x",
        "0x",
        "0X",
        "0o",
        "0b",
        "0B",
        "ff",
        "Infinity",
        "infinity",
        "NaN",
        "_",
        "a",
        "f",
        "1e400",
        "1e-400",
        "99999999999999999999999",
    ];
    if r.chance(10) {
        return format!(
            "0x{}",
            (0..1 + r.below(80))
                .map(|_| b"0123456789abcdef"[r.below(16) as usize] as char)
                .collect::<String>()
        );
    }
    (0..r.below(6))
        .map(|_| PARTS[r.below(PARTS.len() as u64) as usize])
        .collect()
}

fn num_out(n: f64) -> J {
    if n.is_nan() {
        json!({"$num": "NaN"})
    } else if n.is_infinite() {
        json!({"$num": if n > 0.0 { "Infinity" } else { "-Infinity" }})
    } else if n == 0.0 && n.is_sign_negative() {
        json!({"$num": "-0"})
    } else {
        json!(n)
    }
}

#[cfg(not(target_os = "wasi"))]
#[test]
fn js_values_make_the_js_nouns() {
    let repo = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let wasm = format!("{repo}/wasm/build/mesa.wasm");
    let Some(node) = tool("NODE", "node", "the JS checks") else {
        return;
    };
    if !std::path::Path::new(&wasm).exists() {
        assert!(
            std::env::var_os("REQUIRE_ALL").is_none(),
            "no {wasm}: run wasm/build.sh"
        );
        eprintln!("skip: no {wasm}, which the JS checks need");
        return;
    }
    let mut r = Rng::seeded("js_values_make_the_js_nouns");
    let values: Vec<J> = (0..2000).map(|_| js_value(&mut r, 0)).collect();
    let texts: Vec<Vec<u8>> = (0..2000).map(|_| js_bytes(&mut r)).collect();
    let numbers: Vec<String> = (0..4000).map(|_| js_number_text(&mut r)).collect();
    let input = json!({
        "values": values,
        "texts": texts.iter().map(|b| common::hex(b)).collect::<Vec<_>>(),
        "numbers": numbers,
    });
    let out: J = serde_json::from_str(&run_tool(
        &node,
        &["core/urb/tests/oracle.mjs"],
        &input.to_string(),
        Some(repo),
    ))
    .unwrap();
    let mut bad = 0;
    let mut miss = |what: &str, i: usize, input: String, js: &J, rust: &J| {
        bad += 1;
        if bad <= 8 {
            eprintln!(
                "MISMATCH {what} {i}: {}\n  js   {js}\n  rust {rust}",
                &input[..input.len().min(300)]
            );
        }
    };
    for (i, v) in values.iter().enumerate() {
        let mut nouns = Vec::new();
        common::cue_all(v, &mut nouns);
        let mut at = 0;
        let got = match Noun::of(&common::value(v, &nouns, &mut at)) {
            Ok(n) => J::String(common::hex(&n.borrow().jam())),
            Err(e) => json!({"error": e.0}),
        };
        if got != out["values"][i] {
            miss("value", i, v.to_string(), &out["values"][i], &got);
        }
    }
    for (i, b) in texts.iter().enumerate() {
        let got = J::String(js::utf8(b));
        if got != out["texts"][i] {
            miss("text", i, common::hex(b), &out["texts"][i], &got);
        }
    }
    for (i, s) in numbers.iter().enumerate() {
        let got = num_out(js::number(s));
        let js = &out["numbers"][i];
        let same = match (js.as_f64(), got.as_f64()) {
            (Some(a), Some(b)) => a.to_bits() == b.to_bits(),
            _ => *js == got,
        };
        if !same {
            miss("number", i, format!("{s:?}"), js, &got);
        }
    }
    assert_eq!(bad, 0, "{bad} differ from the JS");
    assert_eq!(alive(), 0);
}

// ------------------------------------------------------------ round trips

/// a $json value whose numbers are whole or simple, as JS-shaped as
/// mesa.mjs's json() gives back
fn json_strategy() -> impl Strategy<Value = Json> {
    let leaf = prop_oneof![
        Just(Json::Null),
        any::<bool>().prop_map(Json::Bool),
        any::<f64>()
            .prop_filter("finite", |f| f.is_finite())
            .prop_map(Json::Num),
        (-1_000_000i64..1_000_000).prop_map(|n| Json::Num(n as f64)),
        cord_text(".*").prop_map(Json::Str),
    ];
    leaf.prop_recursive(6, 64, 8, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..6).prop_map(Json::Arr),
            prop::collection::vec((cord_text(".{0,8}"), inner), 0..6).prop_map(|kv| {
                let mut o = js::Object::default();
                for (k, v) in kv {
                    o.set(k, v);
                }
                Json::Obj(o)
            }),
        ]
    })
}

/// a $json value as a noun, as enjs:format would make it (numbers as
/// their shortest text)
fn enjs(j: &Json) -> Value<'static> {
    match j {
        Json::Null => Value::Null,
        Json::Bool(b) => Value::Array(vec!["b".into(), Value::Bool(*b)]),
        Json::Num(n) => Value::Array(vec!["n".into(), format!("{n}").into()]),
        Json::Str(s) => Value::Array(vec!["s".into(), s.as_str().into()]),
        Json::Arr(a) => {
            let mut l: Vec<Value<'static>> = a.iter().map(enjs).collect();
            l.push(Value::Null);
            let list = if l.len() == 1 {
                Value::Null
            } else {
                Value::Array(l)
            };
            Value::Array(vec!["a".into(), list])
        }
        Json::Obj(o) => Value::Array(vec![
            "o".into(),
            Value::Map(
                o.entries()
                    .into_iter()
                    .map(|(k, v)| (k.into(), enjs(v)))
                    .collect(),
            ),
        ]),
    }
}

/// the same $json, objects compared as sets of keys
fn same_json(a: &Json, b: &Json) -> bool {
    match (a, b) {
        (Json::Num(x), Json::Num(y)) => x == y,
        (Json::Arr(x), Json::Arr(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| same_json(p, q))
        }
        (Json::Obj(x), Json::Obj(y)) => {
            x.len() == y.len()
                && x.entries()
                    .iter()
                    .all(|(k, v)| y.get(k).is_some_and(|w| same_json(v, w)))
        }
        _ => a == b,
    }
}

/// a key that is a JS array index
fn index(k: &str) -> Option<u64> {
    let canonical =
        !k.is_empty() && k.bytes().all(|b| b.is_ascii_digit()) && (k == "0" || !k.starts_with('0'));
    k.parse::<u64>()
        .ok()
        .filter(|&n| canonical && n < u64::from(u32::MAX))
}

/// text a cord keeps whole: a TextDecoder drops a leading byte-order
/// mark, and an atom has no trailing zero bytes
fn cord_text(re: &'static str) -> impl Strategy<Value = String> {
    re.prop_filter("a cord's text", |s: &String| {
        !s.starts_with('\u{feff}') && !s.ends_with('\0')
    })
}

fn knot() -> impl Strategy<Value = String> {
    "[a-z0-9~._-]{0,12}"
}

proptest! {
    #[test]
    fn json_round_trips(j in json_strategy()) {
        let n = Noun::of(&enjs(&j)).unwrap();
        let back = codec::json(n.borrow()).unwrap();
        prop_assert!(same_json(&j, &back), "{j:?} came back as {back:?}");
        // and its keys in JS order, whatever order the map holds them
        // in: array indices first, in numeric order
        if let Json::Obj(o) = &back {
            let idx: Vec<Option<u64>> = o.entries().iter().map(|e| index(e.0)).collect();
            let n = idx.iter().take_while(|i| i.is_some()).count();
            prop_assert!(idx[n..].iter().all(Option::is_none));
            prop_assert!(idx[..n].windows(2).all(|w| w[0] < w[1]));
        }
        drop(n);
        prop_assert_eq!(alive(), 0);
    }

    #[test]
    fn paths_round_trip(segs in prop::collection::vec(knot(), 1..=64)) {
        let t = format!("/{}", segs.join("/"));
        let n = Noun::of(&codec::path(&t)).unwrap();
        // "/" is ~, no segments, as stab has it
        let segs = if t == "/" { vec![] } else { segs };
        prop_assert_eq!(codec::path_of(n.borrow()).unwrap(), t);
        prop_assert_eq!(codec::segments(n.borrow()).unwrap(), segs);
        drop(n);
        prop_assert_eq!(alive(), 0);
    }

    #[test]
    fn leaves_round_trip(s in ".*") {
        // [%leaf tape]: the tape is the text's bytes
        let mut tape: Vec<Value<'static>> = s.bytes().map(|b| Value::Int(u64::from(b).into())).collect();
        tape.push(Value::Null);
        let tape = if tape.len() == 1 { Value::Null } else { Value::Array(tape) };
        let n = Noun::of(&Value::Array(vec!["leaf".into(), tape])).unwrap();
        let want = s.strip_prefix('\u{feff}').unwrap_or(&s).to_owned();
        prop_assert_eq!(codec::tank_text(n.borrow(), 0).unwrap(), want);
        drop(n);
        prop_assert_eq!(alive(), 0);
    }

    #[test]
    fn jams_round_trip(seed in any::<u64>()) {
        let mut r = Rng(seed);
        let v = key(&mut r, 0);
        let n = Noun::of(&v).unwrap();
        let back = Noun::cue(&n.borrow().jam()).unwrap();
        prop_assert!(back == n);
        prop_assert_eq!(back.borrow().sham(), n.borrow().sham());
        prop_assert_eq!(back.borrow().mug(), n.borrow().mug());
        drop((n, back));
        prop_assert_eq!(alive(), 0);
    }
}

#[test]
fn the_counter_counts() {
    let a = Noun::atom(b"a");
    let b = a.clone();
    let c = Noun::cell(a, b.borrow().to_noun());
    assert_eq!(alive(), 2);
    let raw = c.into_raw();
    assert_eq!(alive(), 1);
    drop(unsafe { Noun::from_raw(raw) });
    drop(b);
    assert_eq!(alive(), 0);
    // a hostile list of over MAX_CELLS, refused without a noun kept
    let mut l = Noun::atom(&[]);
    for _ in 0..=MAX_CELLS {
        l = Noun::cell(Noun::atom(&[1]), l);
    }
    assert!(codec::items(l.borrow()).is_err());
    drop(l);
    assert_eq!(alive(), 0);
}
