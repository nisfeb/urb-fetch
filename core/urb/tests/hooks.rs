//! The core's hooks, handed nouns as another ship's grubbery sends them,
//! on a core with a fake engine open and a peek, a poke and a keen
//! waiting (as the hooks fuzz target sets one up).

use urb::codec::path;
use urb::core::{Hook, Peeked};
use urb::noun::{Noun, Value};
use urb::{Core, Failure, Input, Output, Reply, Request, alive};

const ZOD: &str = "~zod";

fn core() -> (Core, Vec<Output>) {
    let mut c = Core::new(|n| vec![7; n]);
    let us = urb::engine::ship_parse("~nec").unwrap();
    let keyfile = Some(urb::engine::fake_keyfile(us));
    let calls = [
        Request::Open {
            keyfile,
            rows: None,
            fresh: true,
            fake: true,
        },
        Request::Read("urb://~zod/page".into()),
        Request::Poke {
            ship: ZOD.into(),
            folder: "/a".into(),
            name: "b".into(),
            mark: ("/x".into(), "y".into()),
            value: Value::Null,
        },
    ];
    let mut outs = Vec::new();
    for (id, req) in calls.into_iter().enumerate() {
        outs.extend(
            c.step(1 + id as u64, Input::Call { id: id as u64, req })
                .unwrap(),
        );
    }
    outs.extend(c.hook(5, Hook::Keen(ZOD, "/g/x/1/grubbery//1/pub/page/x/0v1")));
    (c, outs)
}

fn done(outs: &[Output], call: u64) -> Option<&Result<Reply, Failure>> {
    outs.iter().find_map(|o| match o {
        Output::Done { call: c, result } if *c == call => Some(result),
        _ => None,
    })
}

#[test]
fn a_core_waits_on_a_peek_a_poke_and_a_keen() {
    let (c, outs) = core();
    assert!(
        matches!(done(&outs, 0), Some(Ok(Reply::Open { .. }))),
        "{outs:?}"
    );
    let w = c.waiting();
    assert_eq!(w.peeks.len(), 1, "{w:?} {:?}", done(&outs, 1));
    assert_eq!(w.pokes.len(), 1, "{w:?}");
    assert_eq!(w.keens.len(), 1, "{w:?}");
    assert_eq!(w.bones.len(), 2, "{w:?}");
    drop(c);
    assert_eq!(alive(), 0);
}

/// a tape's noun
fn tape(s: &str) -> Value<'static> {
    let mut v: Vec<Value<'static>> = s.bytes().map(|b| Value::from(u64::from(b))).collect();
    v.push(Value::Null);
    Value::Array(v)
}

/// a tang of one leaf
fn tang(s: &str) -> Value<'static> {
    Value::Array(vec![
        Value::Array(vec!["leaf".into(), tape(s)]),
        Value::Null,
    ])
}

fn transfer(c: &mut Core, v: Value<'_>) -> Vec<Output> {
    let n = Noun::of(&v).unwrap();
    c.hook(10, Hook::Poke(ZOD, "grubbery-transfer", n.borrow()))
}

#[test]
fn a_pack_answers_a_poke() {
    for (r, want) in [
        (Value::Null, (true, None)),
        (
            Value::Array(vec![Value::Null, tang("bad")]),
            (false, Some("bad".to_string())),
        ),
    ] {
        let (mut c, _) = core();
        let wire = c.waiting().pokes[0].clone();
        let outs = transfer(&mut c, Value::Array(vec![path(&wire), "pack".into(), r]));
        let Some(Ok(Reply::Poke(p))) = done(&outs, 2) else {
            panic!("{outs:?}");
        };
        assert_eq!((p.ok, p.error.clone()), want);
        assert!(c.waiting().pokes.is_empty());
    }
    assert_eq!(alive(), 0);
}

#[test]
fn a_nack_on_a_pokes_flow_is_its_answer() {
    let (mut c, _) = core();
    let bone = c.waiting().bones[0];
    let err = Noun::of(&Value::Array(vec!["tag".into(), tang("refused")])).unwrap();
    let outs = c.hook(10, Hook::Done(ZOD, bone, Some(err.borrow())));
    let Some(Ok(Reply::Poke(p))) = done(&outs, 2) else {
        panic!("{outs:?}");
    };
    assert_eq!(
        (p.ok, p.error.as_deref(), p.later),
        (false, Some("tag\n  refused"), false)
    );
    drop((c, err));
    assert_eq!(alive(), 0);
}

#[test]
fn revs_we_cannot_read_are_none_and_the_vault_is_peeked() {
    let (mut c, _) = core();
    let wire = c.waiting().peeks[0].clone();
    // a $json of no kind we know: an UrbError, which peekRevs reads as {}
    let f = urb::msg::File {
        mark: ("/".into(), "json".into()),
        value: Some(Value::Array(vec!["x".into(), 1u64.into()])),
        rev: 1,
        at: 0,
    };
    let (snap, silo) = urb::msg::snap_of(&f).unwrap();
    let dest = Value::Array(vec![true.into(), path("/a"), "revs".into()]);
    let msg = urb::msg::snap(path(&wire), dest, 0u64.into(), Some(snap), Some(silo));
    let outs = transfer(&mut c, msg);
    assert!(done(&outs, 1).is_none(), "{outs:?}");
    let w = c.waiting();
    assert_eq!(w.peeks.len(), 1, "{w:?}");
    assert!(w.peeks[0].ends_with("/3"), "the vault's peek: {w:?}");
    // the vault's answer: nothing there, then page/index's, nothing
    let outs = transfer(
        &mut c,
        Value::Array(vec![path(&w.peeks[0]), "veto".into(), Value::Null]),
    );
    assert!(done(&outs, 1).is_none(), "{outs:?}");
    let w = c.waiting();
    let outs = transfer(
        &mut c,
        Value::Array(vec![path(&w.peeks[0]), "miss".into(), Value::Null]),
    );
    let Some(Err(e)) = done(&outs, 1) else {
        panic!("{outs:?}");
    };
    assert_eq!(e.message, "urb://~zod/page: not published");
    drop(c);
    assert_eq!(alive(), 0);
}

#[test]
fn a_hostile_answer_fails_what_waits_on_it() {
    let (mut c, _) = core();
    let wire = c.waiting().pokes[0].clone();
    // [wire 7]: no tag, so its poke fails with what was wrong
    let outs = transfer(&mut c, Value::Array(vec![path(&wire), 7u64.into()]));
    let Some(Ok(Reply::Poke(p))) = done(&outs, 2) else {
        panic!("{outs:?}");
    };
    assert_eq!(
        p.error.as_deref(),
        Some("another ship's noun has an atom where a cell goes")
    );
    let _ = Peeked::Null;
    drop(c);
    assert_eq!(alive(), 0);
}
