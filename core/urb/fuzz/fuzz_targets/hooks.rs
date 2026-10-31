//! The hooks, as libmesa calls them, on a core with an engine open and
//! a peek, a poke and a keen waiting: another ship's %grubbery-transfer
//! on each of their wires and on /want, its %grubbery-load, a nack, and
//! a keen's answer, each a fuzzer's noun. Whatever they hold, no hook
//! panics, every noun is let go, and nothing the core gives is past
//! OUT_MAX.
#![no_main]
use libfuzzer_sys::fuzz_target;
use urb::codec::path;
use urb::core::{Hook, Lane};
use urb::noun::{Noun, Value};
use urb::{Core, Input, Output, Reply, Request};

const ZOD: &str = "~zod";

/// a core waiting on a peek (of ~zod's /pub/revs), a poke and a keen
fn core() -> Core {
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
        Request::Inbox(true),
    ];
    for (id, req) in calls.into_iter().enumerate() {
        c.step(1 + id as u64, Input::Call { id: id as u64, req })
            .unwrap();
    }
    c.hook(5, Hook::Keen(ZOD, "/g/x/1/grubbery//1/pub/page/x/0v1"));
    c
}

fn check(outs: &[Output]) {
    for o in outs {
        match o {
            Output::Log(l) => {
                assert!(!l.starts_with("panic"), "{l}");
                urb_fuzz::bounded(l.len());
            }
            Output::Done { result, .. } => match result {
                Ok(Reply::Read(p)) => urb_fuzz::bounded(p.body.len()),
                Ok(Reply::Poke(p)) => urb_fuzz::bounded(p.error.as_ref().map_or(0, |e| e.len())),
                Ok(_) => {}
                Err(e) => urb_fuzz::bounded(e.message.len()),
            },
            Output::Send { lane, .. } => assert!(matches!(lane, Lane::Galaxy(_) | Lane::Ip(..))),
            _ => {}
        }
    }
}

/// [wire n], wire a path
fn on(wire: &str, n: &Noun) -> Noun {
    Noun::of(&Value::Array(vec![path(wire), Value::Noun(n.borrow())])).unwrap()
}

fuzz_target!(|data: &[u8]| {
    let pick = data.last().copied().unwrap_or(0) % 7;
    if let Some(n) = urb_fuzz::nouns(data, 2) {
        let mut c = core();
        let w = c.waiting();
        let mut outs = Vec::new();
        let mut hook = |c: &mut Core, h: Hook<'_>| outs.extend(c.hook(10, h));
        let t = "grubbery-transfer";
        match pick {
            0 => hook(&mut c, Hook::Poke(ZOD, t, on(&w.peeks[0], &n[0]).borrow())),
            1 => hook(&mut c, Hook::Poke(ZOD, t, on(&w.pokes[0], &n[0]).borrow())),
            2 => {
                // a %snap with no data, then the %want's answer
                let snap = Noun::of(&Value::Array(vec![
                    path(&w.peeks[0]),
                    "snap".into(),
                    Value::Null,
                    Value::Null,
                    Value::Array(vec![Value::Null, Value::Noun(n[1].borrow())]),
                    Value::Null,
                ]))
                .unwrap();
                hook(&mut c, Hook::Poke(ZOD, t, snap.borrow()));
                hook(&mut c, Hook::Poke(ZOD, t, on("/want", &n[0]).borrow()));
            }
            3 => hook(&mut c, Hook::Poke(ZOD, "grubbery-load", n[0].borrow())),
            4 => hook(&mut c, Hook::Sage(ZOD, &w.keens[0], Some(n[0].borrow()))),
            5 => {
                hook(&mut c, Hook::Done(ZOD, w.bones[0], Some(n[0].borrow())));
                hook(&mut c, Hook::Done(ZOD, w.bones[1], Some(n[1].borrow())));
            }
            _ => hook(&mut c, Hook::Poke(ZOD, t, n[0].borrow())),
        }
        check(&outs);
        // the core still works after: its timers fire, past the peek's
        // and the poke's 30 s
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
