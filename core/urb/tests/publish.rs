//! meta.pub, saved by the JS and by the core, and what is served from
//! it: each saved state is published from again, by pub.mjs
//! (pub-oracle.mjs, on mesa.wasm) and by the core, and the same ops on
//! each (puts, removes, and other ships' peeks, keeps and drops) must
//! give the same results, the same saves, as JSON.stringify writes them,
//! byte for byte, the same pokes (by sham) and the same log lines. The
//! states are trace 5's saves, an empty one, and older and odder shapes
//! a host may hold: pages from before marks, a removed page, a field the
//! JS never wrote, keeps of several files on several ships, keys pub.mjs
//! would never make, and revs and times that aren't whole numbers.

// under WASI there is no node to ask
#![cfg_attr(target_os = "wasi", allow(unused_imports, dead_code))]

mod common;

use serde_json::{Value as J, json};
use urb::codec::path;
use urb::core::Hook;
use urb::js::{self, Json, Object};
use urb::noun::Value;
use urb::{Core, Input, Output, Reply, Request, alive};

const BASE: &str = "/apps/shell.shell/desks/lattice.desk/desk/data/lattice.lattice_app";

/// a JSON value as the JS's JSON.parse gives it
fn json(j: &J) -> Json {
    match j {
        J::Null => Json::Null,
        J::Bool(b) => Json::Bool(*b),
        J::Number(n) => Json::Num(n.as_f64().unwrap()),
        J::String(s) => Json::Str(s.clone()),
        J::Array(v) => Json::Arr(v.iter().map(json).collect()),
        J::Object(o) => {
            let mut out = Object::default();
            for (k, v) in o {
                out.set(k.clone(), json(v));
            }
            Json::Obj(out)
        }
    }
}

/// the saved states to publish from
fn states() -> Vec<J> {
    let mut out = vec![json!({})];
    // trace 5's saves, each as the JS handed it to save
    for l in include_str!("../../../traces/5-publishing.jsonl")
        .lines()
        .skip(1)
    {
        let step: J = serde_json::from_str(l).unwrap();
        for o in step["out"].as_array().unwrap() {
            if o["event"] == "pub.save" {
                out.push(o["value"].clone());
            }
        }
    }
    let vault = format!("{BASE}/pub/vault");
    out.push(json!({
        "pages": {
            // before release 74: no mark
            "/index": {"body": "# home", "rev": 3, "id": "0v1.2bc3d", "at": 1761600000123u64,
                       "hash": "8637ea05a874dc7f9a5799c3497dd554"},
            "/notes/intro": {"body": null, "rev": 2, "id": null, "at": 1761600001000u64, "hash": null},
            // a field the JS never wrote, kept as it is
            "/notes/more": {"body": "more", "rev": 1, "id": "0v5", "at": 1761600002000u64,
                            "hash": "abc", "mark": "md", "x": [1, "two", null]},
        },
        "keepers": [
            {"ship": "~nec", "wire": ["page"], "folder": format!("{vault}/index"), "name": "gmi"},
            {"ship": "~bus", "wire": [], "folder": format!("{BASE}/pub"), "name": "revs"},
            {"ship": "~nec", "wire": ["page", "two"], "folder": format!("{vault}/notes/more"), "name": "gmi"},
        ],
    }));
    // keys pageKey never makes, which the index still lists
    let page = |b: &str| json!({"body": b, "rev": 1, "id": "0v1", "at": 5000, "hash": "1"});
    out.push(json!({"pages": {"": page("empty"), "éx": page("é"), "😀x": page("emoji"), "/ok": page("ok")}}));
    // revs and times that aren't whole numbers, or are missing
    out.push(
        json!({"pages": {"/a1": {"body": "a", "rev": 1.5, "id": "0v1", "at": 5.5, "hash": "1"}}}),
    );
    out.push(json!({"pages": {"/a1": {"body": "a", "id": "0v1", "at": 5000, "hash": "1"}}}));
    out.push(json!({"pages": {"/a1": {"body": "a", "rev": 2, "id": "0v1", "hash": "1"}}}));
    out.push(
        json!({"pages": {"/a1": {"body": "a", "rev": "2", "id": "0v1", "at": 5000, "hash": "1"}}}),
    );
    // only pages, and only keepers
    out.push(json!({"pages": {"/a1": {"body": "a", "rev": 1, "id": "0v1", "at": 5, "hash": "1"}}}));
    out.push(json!({"keepers": [{"ship": "~nec", "wire": ["page"], "folder": format!("{vault}/a1"), "name": "gmi"}]}));
    out
}

/// what each state is put through
fn ops() -> J {
    let dir = format!("{BASE}/pub");
    let peeks = || {
        json!([
            ["peek", dir, "index", 2],
            ["peek", dir, "revs", 2],
            ["peek", format!("{BASE}/pub/vault/a1"), "gmi", 2],
            ["peek", format!("{BASE}/pub/vault/index"), "gmi", 2],
            ["peek", format!("{BASE}/pub/vault/never"), "gmi", 2],
            ["peek", "/not/ours", "x", 2],
        ])
    };
    let mut all = peeks().as_array().unwrap().clone();
    all.extend(
        json!([
            // keeps: of a page, again, of nothing, from another ship, and
            // drops of another's and of one's own
            ["keep", "a1", 1],
            ["keep", "a1", 1],
            ["keep", "never", 1],
            ["keep", "index", 3],
            ["drop", "a1", 3],
            ["keep", "a1", 3],
            ["put", "a1", "a1, edited"],
            ["drop", "a1", 1],
            ["put", "a1", "a1, edited again"],
            // unicode, in bodies and names, and JS's white space trimmed
            [
                "put",
                "uni",
                "é \u{1F600} \u{2028} \u{7} \" \u{feff} \u{0} done",
                "md"
            ],
            ["put", "\u{3000}uni/two\u{feff}", "two"],
            ["put", "caf\u{e9}", "a name lattice can't read"],
            // remove, republish, remove twice, republish under a new mark
            ["remove", "uni"],
            ["put", "uni", "back"],
            ["remove", "uni"],
            ["remove", "uni"],
            ["put", "uni", "<p>back again</p>", "html"],
        ])
        .as_array()
        .unwrap()
        .iter()
        .cloned(),
    );
    all.extend(peeks().as_array().unwrap().iter().cloned());
    all.extend(
        json!([
            ["pages"],
            ["put", "index", "# home, again"],
            ["put", "/notes/intro/", "# Intro, back", "md"],
            ["put", "notes/more", "<p>more</p>", "html"],
            ["put", "m1c-page", "# Trace 5, again", "text"],
            ["put", "a1", "a, again"],
            ["put", "n/x", "a one-letter mount"],
            ["put", "fresh", "a mark lattice doesn't publish", "del"],
            ["remove", "notes/more"],
            ["remove", "notes/more"],
            ["remove", "never"],
            ["remove", "m1c-page"],
            ["put", "fresh/page", "new"],
            ["pages"],
        ])
        .as_array()
        .unwrap()
        .iter()
        .cloned(),
    );
    J::Array(all)
}

#[cfg(not(target_os = "wasi"))]
#[test]
fn saved_state_round_trips_as_the_js_saves_it() {
    use common::{run_tool, tool};
    let repo = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let wasm = format!("{repo}/wasm/build/mesa.wasm");
    let Some(node) = tool("NODE", "node", "the meta.pub checks") else {
        return;
    };
    if !std::path::Path::new(&wasm).exists() {
        assert!(
            std::env::var_os("REQUIRE_ALL").is_none(),
            "no {wasm}: run wasm/build.sh"
        );
        eprintln!("skip: no {wasm}, which the meta.pub checks need");
        return;
    }
    const NOW: u64 = 1793440000000;
    let (states, ops) = (states(), ops());
    let cases: Vec<J> = states
        .iter()
        .map(|s| json!({"state": s, "now": NOW, "ops": ops}))
        .collect();
    let js: J = serde_json::from_str(&run_tool(
        &node,
        &["core/urb/tests/pub-oracle.mjs"],
        &json!({ "cases": cases }).to_string(),
        Some(repo),
    ))
    .unwrap();
    let mut saves = 0;
    for (k, state) in states.iter().enumerate() {
        let n = std::cell::Cell::new(0u8);
        let mut c = Core::new(move |len| {
            (0..len)
                .map(|_| {
                    let b = n.get();
                    n.set(b.wrapping_add(1));
                    b
                })
                .collect()
        });
        let zod = urb::engine::ship_parse("~zod").unwrap();
        let mut now = NOW;
        let call = |c: &mut Core, now: u64, id: u64, req: Request| {
            c.step(now, Input::Call { id, req }).unwrap()
        };
        call(
            &mut c,
            now,
            0,
            Request::Open {
                keyfile: Some(urb::engine::fake_keyfile(zod)),
                rows: None,
                fresh: true,
                fake: true,
            },
        );
        let st = json(state);
        let field = |f: &str| match &st {
            Json::Obj(o) => o.get(f).cloned(),
            _ => None,
        };
        call(
            &mut c,
            now,
            1,
            Request::Pub {
                pages: match field("pages") {
                    Some(Json::Obj(o)) => Some(o),
                    _ => None,
                },
                keepers: field("keepers"),
            },
        );
        for (i, op) in ops.as_array().unwrap().iter().enumerate() {
            now += 1000;
            let a = op.as_array().unwrap();
            let s = |i: usize| a.get(i).and_then(J::as_str).map(String::from);
            let ship = |i: usize| ["~zod", "~nec", "~bud", "~wes"][a[i].as_u64().unwrap() as usize];
            urb::core::capture_pokes(true);
            let (out, result) = match s(0).unwrap().as_str() {
                op @ ("keep" | "drop" | "peek") => {
                    let (wire, folder, name, from) = if op == "peek" {
                        ("/peek/1", s(1).unwrap(), s(2).unwrap(), ship(3))
                    } else {
                        (
                            "/keep/page",
                            format!("{BASE}/pub/vault/{}", s(1).unwrap()),
                            "gmi".into(),
                            ship(2),
                        )
                    };
                    let v = Value::Array(vec![
                        Value::Array(vec![
                            path(wire),
                            Value::Array(vec![0u64.into(), path(&folder), name.into()]),
                        ]),
                        op.into(),
                        Value::Null,
                    ]);
                    let n = urb::Noun::of(&v).unwrap();
                    (
                        c.hook(now, Hook::Poke(from, "grubbery-load", n.borrow())),
                        Some(J::Null),
                    )
                }
                op => {
                    let req = match op {
                        "put" => Request::PubPut {
                            name: s(1).unwrap(),
                            body: s(2).unwrap(),
                            mark: s(3),
                        },
                        "remove" => Request::PubRemove(s(1).unwrap()),
                        _ => Request::PubPages,
                    };
                    (call(&mut c, now, 2 + i as u64, req), None)
                }
            };
            let pokes: Vec<J> = urb::core::capture_pokes(false)
                .iter()
                .map(|(m, jam)| json!([m, format!("{:x}", urb::Noun::cue(jam).unwrap().sham())]))
                .collect();
            let mut result = result.unwrap_or(J::Null);
            let mut got = Vec::new();
            let mut logs = Vec::new();
            for o in &out {
                match o {
                    Output::Event { name, value } if name == "pub.save" => {
                        got.push(J::String(js::stringify(value)))
                    }
                    Output::Log(l) => logs.push(J::String(l.clone())),
                    Output::Done { result: r, .. } => {
                        result = match r {
                            Ok(Reply::Json(j)) => J::String(js::stringify(j)),
                            Ok(r) => panic!("{r:?}"),
                            Err(f) => json!({ "error": f.message }),
                        }
                    }
                    _ => {}
                }
            }
            let want = &js[k][i];
            assert_eq!(result, want["result"], "state {k}, op {i} {op}: its result");
            assert_eq!(
                J::Array(got.clone()),
                want["saves"],
                "state {k}, op {i} {op}: its saves"
            );
            assert_eq!(
                J::Array(pokes),
                want["pokes"],
                "state {k}, op {i} {op}: its pokes"
            );
            assert_eq!(
                J::Array(logs),
                want["logs"],
                "state {k}, op {i} {op}: its logs"
            );
            saves += got.len();
        }
        drop(c);
    }
    eprintln!(
        "{} saved states, {} ops each, {saves} saves: as the JS's",
        states.len(),
        ops.as_array().unwrap().len()
    );
    assert!(saves > 50, "only {saves} saves");
    assert_eq!(alive(), 0);
}
