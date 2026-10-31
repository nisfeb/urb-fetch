//! The grubbery server and the scry, as other ships reach them: a core
//! publishing pages (one taken down, one kept by ~zod from a saved keep)
//! hears a %grubbery-load, with a fuzzer's noun as the whole load, or as
//! its wire, its dest or its folder (each a %peek, %keep or %drop), or a
//! scry of a fuzzer's path. Then a page is edited, which sends a wave to
//! every keeper, and the core's timers run. No hook panics, every noun
//! is let go, the keeps stay under the cap, and nothing the core gives
//! is past OUT_MAX.
#![no_main]
use libfuzzer_sys::fuzz_target;
use urb::codec::path;
use urb::core::{Hook, MAX_KEEPERS};
use urb::js::{Json, Object};
use urb::noun::{Noun, Value};
use urb::publish::{PAGE, page_id, vault};
use urb::{Core, Input, Output, Request};

const ZOD: &str = "~zod";

fn core() -> Core {
    let mut c = Core::new(|n| vec![7; n]);
    let us = urb::engine::ship_parse("~nec").unwrap();
    let mut keep = Object::default();
    keep.set("ship".into(), Json::Str(ZOD.into()));
    keep.set("wire".into(), Json::Arr(vec![Json::Str("page".into())]));
    keep.set("folder".into(), Json::Str(format!("{}/index", vault())));
    keep.set("name".into(), Json::Str("gmi".into()));
    let put = |name: &str, body: &str| Request::PubPut {
        name: name.into(),
        body: body.into(),
        mark: None,
    };
    let calls = [
        Request::Open {
            keyfile: Some(urb::engine::fake_keyfile(us)),
            rows: None,
            fresh: true,
            fake: true,
        },
        Request::Pub {
            pages: None,
            keepers: Some(Json::Arr(vec![Json::Obj(keep)])),
        },
        put("index", "# home"),
        put("notes/intro", "# Intro"),
        put("gone", "gone"),
        Request::PubRemove("gone".into()),
    ];
    for (id, req) in calls.into_iter().enumerate() {
        c.step(1 + id as u64, Input::Call { id: id as u64, req })
            .unwrap();
    }
    c
}

fn check(outs: &[Output]) {
    for o in outs {
        match o {
            Output::Log(l) => {
                assert!(!l.starts_with("panic"), "{l}");
                urb_fuzz::bounded(l.len());
            }
            Output::Event { value, .. } => {
                let Json::Obj(o) = value else {
                    panic!("a save of no object")
                };
                let Some(Json::Arr(ks)) = o.get("keepers") else {
                    panic!("a save with no keepers")
                };
                assert!(ks.len() <= MAX_KEEPERS, "{} keeps", ks.len());
                urb_fuzz::bounded(urb_fuzz::json_len(value));
            }
            Output::Done { result, .. } => assert!(result.is_ok(), "{result:?}"),
            _ => {}
        }
    }
}

fuzz_target!(|data: &[u8]| {
    let (pick, tag) = match data {
        [.., t, p] => (p % 5, ["peek", "keep", "drop"][usize::from(t % 3)]),
        _ => (0, "peek"),
    };
    if let Some(n) = urb_fuzz::nouns(data, 1) {
        let mut c = core();
        let mut outs = Vec::new();
        let x = || Value::Noun(n[0].borrow());
        let file = |folder: Value<'static>| Value::Array(vec![0u64.into(), folder, "gmi".into()]);
        let load = |wire: Value<'_>, dest: Value<'_>| {
            Noun::of(&Value::Array(vec![
                Value::Array(vec![wire, dest]),
                tag.into(),
                Value::Null,
            ]))
            .unwrap()
        };
        let kept = || file(path(&format!("{}/notes/intro", vault())));
        let l = "grubbery-load";
        match pick {
            0 => outs.extend(c.hook(10, Hook::Poke(ZOD, l, n[0].borrow()))),
            1 => outs.extend(c.hook(10, Hook::Poke(ZOD, l, load(x(), kept()).borrow()))),
            2 => outs.extend(c.hook(10, Hook::Poke(ZOD, l, load(path("/w"), x()).borrow()))),
            3 => {
                let dest = Value::Array(vec![0u64.into(), x(), "gmi".into()]);
                outs.extend(c.hook(10, Hook::Poke(ZOD, l, load(path("/w"), dest).borrow())));
            }
            _ => {
                // a scry of the fuzzer's bytes as a path, and of a page's
                let body = data.get(1..data.len().saturating_sub(2)).unwrap_or(&[]);
                let p = String::from_utf8_lossy(body);
                outs.extend(c.hook(10, Hook::Peek(&p)));
                let page = format!("{PAGE}/index/{}", page_id(&[7; 16]));
                outs.extend(c.hook(10, Hook::Peek(&page)));
                assert!(
                    outs.iter()
                        .any(|o| matches!(o, Output::Log(l) if l.starts_with("scry: "))),
                    "the home page's scry gave nothing: {outs:?}"
                );
            }
        }
        check(&outs);
        // an edit: a wave to every keeper, the one just added too
        let edit = Request::PubPut {
            name: "notes/intro".into(),
            body: "# Intro, again".into(),
            mark: Some("md".into()),
        };
        check(&c.step(20, Input::Call { id: 99, req: edit }).unwrap());
        for _ in 0..200 {
            let Some(due) = c.deadline() else { break };
            if due > 60_000 {
                break;
            }
            let (kind, due) = c.due(due).unwrap();
            check(&c.step(due, Input::Tick { kind, due }).unwrap());
        }
        drop(c);
    }
    urb_fuzz::done();
});
