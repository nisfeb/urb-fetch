//! The reader where it reads as the Go gateway's reader.go does, which
//! mesa.mjs took from it (https://github.com/nisfeb/urb-fetch/pull/13):
//! a base probe nobody answers ends the read, a stale list beats none,
//! one read uses one list, a late list is kept, and an entry needs an
//! id. A core with a fake engine open as ~nec reads urb://~zod pages,
//! and ~zod's answers are handed to its hooks as ~zod's grubbery would
//! send them. The gateway's status codes and words for each failure
//! are urb-gateway's (its `status` tests).
//!
//! reader_test.go's cases run here as vectors: gateway-go.json's
//! `lateAnswer` and `onTimeAnswer` (the rest of the file runs in
//! urb-gateway's tests).

use serde_json::Value as J;
use urb::codec::path;
use urb::core::{Hook, NO_LATTICE};
use urb::js::Json;
use urb::msg::{self, File};
use urb::noun::{Noun, Value};
use urb::{Core, Failure, Input, Output, Reply, Request, alive};

const ZOD: &str = "~zod";

/// a core open as fake ~nec
fn core() -> Core {
    let mut c = Core::new(|n| vec![7; n]);
    let us = urb::engine::ship_parse("~nec").unwrap();
    let req = Request::Open {
        keyfile: Some(urb::engine::fake_keyfile(us)),
        rows: None,
        fresh: true,
        fake: true,
    };
    let outs = c.step(1, Input::Call { id: 0, req }).unwrap();
    assert!(done(&outs, 0).is_some_and(Result::is_ok), "{outs:?}");
    c
}

fn done(outs: &[Output], call: u64) -> Option<&Result<Reply, Failure>> {
    outs.iter().find_map(|o| match o {
        Output::Done { call: c, result } if *c == call => Some(result),
        _ => None,
    })
}

fn logs(outs: &[Output]) -> Vec<String> {
    outs.iter()
        .filter_map(|o| match o {
            Output::Log(l) => Some(l.clone()),
            _ => None,
        })
        .collect()
}

/// tick every timer due up to now, in order: what they caused
fn run_to(c: &mut Core, now: u64) -> Vec<Output> {
    let mut outs = Vec::new();
    while let Some((kind, due)) = c.due(now) {
        outs.extend(c.step(now, Input::Tick { kind, due }).unwrap());
    }
    outs
}

/// ~zod's /pub/revs, {"/<page>": {"rev": 1, "id": id}}, as a $json noun
fn revs(pages: &[(&str, &str)]) -> Value<'static> {
    let s = |x: &str| Value::Text(x.into());
    let o = pages
        .iter()
        .map(|(k, id)| {
            let row = Value::Map(vec![
                (s("rev"), Value::Array(vec![s("n"), s("1")])),
                (s("id"), Value::Array(vec![s("s"), s(id)])),
            ]);
            (s(k), Value::Array(vec![s("o"), row]))
        })
        .collect();
    Value::Array(vec![s("o"), Value::Map(o)])
}

/// ~zod's grubbery answering the peek on wire with a file
fn answer(c: &mut Core, now: u64, wire: &str, value: Value<'static>) -> Vec<Output> {
    let f = File {
        mark: ("/".into(), "json".into()),
        value: Some(value),
        rev: Some(Json::Num(1.0)),
        at: Some(Json::Num(1_793_500_000_000.0)),
    };
    let (snap, data) = msg::snap_of(&f).unwrap();
    let v = msg::snap(path(wire), Value::Null, 1u64.into(), Some(snap), Some(data));
    let n = Noun::of(&v).unwrap();
    c.hook(now, Hook::Poke(ZOD, "grubbery-transfer", n.borrow()))
}

/// ~zod's grubbery answering a peek with a %veto
fn veto(c: &mut Core, now: u64, wire: &str) -> Vec<Output> {
    let n = Noun::of(&Value::Array(vec![path(wire), "veto".into(), Value::Null])).unwrap();
    c.hook(now, Hook::Poke(ZOD, "grubbery-transfer", n.borrow()))
}

/// ~zod answering a keen of a page under mark
fn sage(c: &mut Core, now: u64, path: &str, mark: &str, body: &str) -> Vec<Output> {
    let n = Noun::of(&Value::Array(vec![mark.into(), Value::Text(body.into())])).unwrap();
    c.hook(now, Hook::Sage(ZOD, path, Some(n.borrow())))
}

/// ~zod answering a keen with nothing: a miss
fn miss(c: &mut Core, now: u64, path: &str) -> Vec<Output> {
    c.hook(now, Hook::Sage(ZOD, path, None))
}

fn read(c: &mut Core, now: u64, id: u64, url: &str) -> Vec<Output> {
    c.step(
        now,
        Input::Call {
            id,
            req: Request::Read(url.into()),
        },
    )
    .unwrap()
}

fn only_peek(c: &Core) -> String {
    let w = c.waiting();
    assert_eq!(w.peeks.len(), 1, "{w:?}");
    w.peeks[0].clone()
}

/// lateAnswer: the base probe nobody answers in time fails the read,
/// later (the gateway's 504); the list that comes after is kept, and
/// its peek waits no more. What it gives is reader_test.go's {kept,
/// waiting}.
fn late_answer(list: &[(&str, &str)]) -> J {
    let mut c = core();
    read(&mut c, 10, 9, "urb://~zod/n/a");
    let wire = only_peek(&c);
    let outs = run_to(&mut c, 10 + 30_000);
    let Some(Err(f)) = done(&outs, 9) else {
        panic!("{outs:?}");
    };
    assert_eq!(
        (f.message.as_str(), f.later),
        (
            "~zod answered no lattice peek (offline, or no lattice)",
            true
        )
    );
    // it waits on for the answer, and no second base is asked
    assert_eq!(c.waiting().peeks, vec![wire.clone()]);
    let outs = answer(&mut c, 40_000, &wire, revs(list));
    assert_eq!(
        logs(&outs),
        vec![format!("a late answer came in on {wire}")]
    );
    let kept: serde_json::Map<String, J> = c
        .cached_revs(ZOD)
        .unwrap_or_default()
        .into_iter()
        .map(|(k, id)| (k, J::String(id)))
        .collect();
    let waiting = c.waiting().peeks.contains(&wire);
    // and the next read keens at once, with no peek of the list
    read(&mut c, 41_000, 10, "urb://~zod/n/a");
    assert!(c.waiting().peeks.is_empty(), "{:?}", c.waiting());
    let id = list.iter().find(|(k, _)| *k == "/a").map_or("", |x| x.1);
    assert_eq!(
        c.waiting().keens,
        vec![format!("/g/x/1/grubbery//1/pub/page/a/{id}")]
    );
    drop(c);
    serde_json::json!({"kept": kept, "waiting": waiting})
}

/// onTimeAnswer: an answer in time goes to the read, and nothing comes
/// in late. What it gives is reader_test.go's {caller, late}.
fn on_time_answer(body: &str) -> J {
    let mut c = core();
    read(&mut c, 10, 9, "urb://~zod/n/a");
    let wire = only_peek(&c);
    let outs = answer(&mut c, 20, &wire, revs(&[("/a", "0v1")]));
    assert!(logs(&outs).is_empty(), "{outs:?}");
    let path = "/g/x/1/grubbery//1/pub/page/a/0v1";
    let outs = sage(&mut c, 30, path, "md", body);
    let Some(Ok(Reply::Read(p))) = done(&outs, 9) else {
        panic!("{outs:?}");
    };
    // the peek's timer is gone with it: nothing later is late
    let outs = run_to(&mut c, 100_000);
    let late = logs(&outs).into_iter().find(|l| l.contains("late answer"));
    serde_json::json!({"caller": p.body, "late": late})
}

#[test]
fn reader_tests_cases_pass_as_vectors() {
    let v: J = serde_json::from_str(include_str!("../../../conformance/gateway-go.json")).unwrap();
    let mut ran = 0;
    for c in v["cases"].as_array().unwrap() {
        let got = match c["fn"].as_str().unwrap() {
            "lateAnswer" => {
                let list: Vec<(String, String)> = c["in"][0]
                    .as_object()
                    .unwrap()
                    .iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string()))
                    .collect();
                let l: Vec<(&str, &str)> =
                    list.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
                late_answer(&l)
            }
            "onTimeAnswer" => on_time_answer(c["in"][0].as_str().unwrap()),
            _ => continue,
        };
        assert_eq!(got, c["out"], "{c}");
        ran += 1;
    }
    assert_eq!(ran, 2);
    assert_eq!(alive(), 0);
}

#[test]
fn a_ship_that_answers_no_base_with_anything_is_not_later() {
    let mut c = core();
    read(&mut c, 10, 9, "urb://~zod/n/a");
    let wire = only_peek(&c);
    veto(&mut c, 20, &wire);
    let second = only_peek(&c);
    assert_ne!(second, wire);
    let outs = veto(&mut c, 30, &second);
    let Some(Err(f)) = done(&outs, 9) else {
        panic!("{outs:?}");
    };
    assert_eq!(
        (f.message.as_str(), f.later),
        (format!("~zod {NO_LATTICE}").as_str(), false)
    );
    drop(c);
    assert_eq!(alive(), 0);
}

#[test]
fn a_page_not_listed_is_not_published() {
    let mut c = core();
    read(&mut c, 10, 9, "urb://~zod/n/b");
    let wire = only_peek(&c);
    answer(&mut c, 20, &wire, revs(&[("/a", "0v1")]));
    // the list names pages, not /b: only /b is tried, by the vault
    let vault = only_peek(&c);
    let outs = veto(&mut c, 30, &vault);
    let Some(Err(f)) = done(&outs, 9) else {
        panic!("{outs:?}");
    };
    assert_eq!(f.message, "urb://~zod/n/b: not published");
    drop(c);
    assert_eq!(alive(), 0);
}

#[test]
fn a_stale_list_beats_none_and_an_entry_needs_an_id() {
    let mut c = core();
    read(&mut c, 10, 9, "urb://~zod/n/a");
    let wire = only_peek(&c);
    // /c has no id: no entry, as reader.go's revsOf reads it
    let mut list = revs(&[("/a", "0v1")]);
    if let Value::Array(o) = &mut list
        && let Value::Map(m) = &mut o[1]
    {
        m.push((
            Value::Text("/c".into()),
            Value::Array(vec![Value::Text("o".into()), Value::Map(vec![])]),
        ));
    }
    answer(&mut c, 20, &wire, list);
    let path = "/g/x/1/grubbery//1/pub/page/a/0v1";
    let outs = sage(&mut c, 30, path, "md", "body");
    assert!(matches!(done(&outs, 9), Some(Ok(_))), "{outs:?}");
    assert_eq!(
        c.cached_revs(ZOD),
        Some(vec![("/a".to_string(), "0v1".to_string())])
    );
    // past the list's 60 s, a refresh nobody answers keeps the old list
    read(&mut c, 70_000, 10, "urb://~zod/n/a");
    assert_ne!(only_peek(&c), wire);
    run_to(&mut c, 70_000 + 30_000);
    assert_eq!(c.waiting().keens, vec![path.to_string()]);
    drop(c);
    assert_eq!(alive(), 0);
}

#[test]
fn one_read_looks_in_the_one_list_it_got() {
    let mut c = core();
    read(&mut c, 10, 9, "urb://~zod/n/a");
    let wire = only_peek(&c);
    answer(&mut c, 20, &wire, revs(&[("/a", "0v1")]));
    // the listed id misses, which drops the cached list, and the vault
    // has no /a
    miss(&mut c, 30, "/g/x/1/grubbery//1/pub/page/a/0v1");
    let vault = only_peek(&c);
    veto(&mut c, 40, &vault);
    // so /a/index, from the vault: the peek out is the vault's, not a
    // second ask for the list
    let index = only_peek(&c);
    let outs = answer(&mut c, 50, &index, Value::Text("the index".into()));
    let Some(Ok(Reply::Read(p))) = done(&outs, 9) else {
        panic!("{outs:?}");
    };
    assert_eq!((p.key.as_str(), p.how), ("/a/index", "peek"));
    assert_eq!(p.body, "the index");
    drop(c);
    assert_eq!(alive(), 0);
}
