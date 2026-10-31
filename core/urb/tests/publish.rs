//! meta.pub, saved by the JS and by the core: each saved state is
//! published from again, by pub.mjs (pub-oracle.mjs, on mesa.wasm) and
//! by the core, and the same ops on each must give the same results and
//! the same saves, as JSON.stringify writes them, byte for byte. The
//! states are trace 5's saves, an empty one, and older and odder shapes
//! a host may hold: pages from before marks, a removed page, a field the
//! JS never wrote, and keeps of several files on several ships.

// under WASI there is no node to ask
#![cfg_attr(target_os = "wasi", allow(unused_imports, dead_code))]

mod common;

use serde_json::{Value as J, json};
use urb::js::{self, Json, Object};
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
    // only pages, and only keepers
    out.push(json!({"pages": {"/a1": {"body": "a", "rev": 1, "id": "0v1", "at": 5, "hash": "1"}}}));
    out.push(json!({"keepers": [{"ship": "~nec", "wire": ["page"], "folder": format!("{vault}/a1"), "name": "gmi"}]}));
    out
}

/// what each state is put through
fn ops() -> J {
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
            let req = match s(0).unwrap().as_str() {
                "put" => Request::PubPut {
                    name: s(1).unwrap(),
                    body: s(2).unwrap(),
                    mark: s(3),
                },
                "remove" => Request::PubRemove(s(1).unwrap()),
                _ => Request::PubPages,
            };
            let out = call(&mut c, now, 2 + i as u64, req);
            let mut result = J::Null;
            let mut got = Vec::new();
            for o in &out {
                match o {
                    Output::Event { name, value } if name == "pub.save" => {
                        got.push(J::String(js::stringify(value)))
                    }
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
