//! furum's and forum's calls on the core, answered by a host whose
//! grubs are the fuzzer's: a furum board's conf, posts, threads and
//! votes buckets, and a lattice board's pub/ and content/ directories,
//! each a file or a directory of files whose nouns are the fuzzer's in
//! a well-formed %snap, so the apps' own convs read them during the
//! hook; a whole lattice board, a and b split into its grubs; and
//! notes poked into our inbox for each app's onNote. Then the
//! timers run out. No hook panics, every noun is let go, and nothing
//! the core gives is past OUT_MAX.
#![no_main]
use libfuzzer_sys::fuzz_target;
use urb::codec::path;
use urb::core::{ForumCall, FurumCall, Hook};
use urb::forum::{board, furum};
use urb::noun::{Noun, NounRef, Value};
use urb::url::DESK_BASE;
use urb::{Core, Input, Output, Reply, Request};
use urb_fuzz::{bounded, js_len};

const ZOD: &str = "~zod";

/// a cell's head and tail, or an atom twice
fn half(x: NounRef<'_>) -> (NounRef<'_>, NounRef<'_>) {
    (x.head().unwrap_or(x), x.tail().unwrap_or(x))
}

fn v(x: Vec<Value<'_>>) -> Value<'_> {
    Value::Array(x)
}

/// a %snap's silo and the jobe of its top, as grubbery's are made:
/// files at their paths, under trees, each a leaf of its noun
#[derive(Default)]
struct Silo<'a> {
    nouns: Vec<(Value<'a>, Value<'a>)>,
    jects: Vec<(Value<'a>, Value<'a>)>,
    next: u64,
}

impl<'a> Silo<'a> {
    fn id(&mut self) -> u64 {
        self.next += 1;
        self.next
    }

    fn leaf(&mut self, n: Value<'a>) -> u64 {
        let (nobe, jobe) = (self.id(), self.id());
        self.nouns.push((nobe.into(), v(vec![0u64.into(), n])));
        let ject = v(vec!["leaf".into(), nobe.into(), 0u64.into()]);
        self.jects.push((jobe.into(), v(vec![0u64.into(), ject])));
        jobe
    }

    fn tree(&mut self, files: &[(&[&str], Value<'a>)]) -> u64 {
        let (mut fil, mut dir) = (Vec::new(), Vec::new());
        let mut subs: Vec<&str> = Vec::new();
        for (p, n) in files {
            match p {
                [name] => {
                    let l = self.leaf(n.clone());
                    fil.push(((*name).into(), l.into()));
                }
                [d, ..] if !subs.contains(d) => subs.push(d),
                _ => {}
            }
        }
        for d in subs {
            let under: Vec<(&[&str], Value<'a>)> = files
                .iter()
                .filter(|(p, _)| p.len() > 1 && p[0] == d)
                .map(|(p, n)| (&p[1..], n.clone()))
                .collect();
            let t = self.tree(&under);
            dir.push((d.into(), v(vec![t.into(), 0u64.into()])));
        }
        let jobe = self.id();
        let ject = v(vec![
            "tree".into(),
            0u64.into(),
            0u64.into(),
            0u64.into(),
            Value::Map(fil),
            Value::Map(dir),
        ]);
        self.jects.push((jobe.into(), v(vec![0u64.into(), ject])));
        jobe
    }

    /// the %snap of top, with this silo inline
    fn snap(self, wire: &str, top: u64) -> Noun {
        let snap = v(vec![
            v(vec![1u64.into(), 0u64.into()]),
            v(vec!["firm".into(), Value::Null, top.into()]),
            0u64.into(),
        ]);
        let silo = v(vec![Value::Map(self.nouns), Value::Map(self.jects)]);
        let dest = v(vec![0u64.into(), path("/x"), "y".into()]);
        let m = urb::msg::snap(path(wire), dest, 9u64.into(), Some(snap), Some(silo));
        Noun::of(&m).unwrap()
    }
}

fn check(outs: &[Output]) {
    for o in outs {
        match o {
            Output::Log(l) => {
                assert!(!l.starts_with("panic"), "{l}");
                bounded(l.len());
            }
            Output::Done { result, .. } => match result {
                Ok(Reply::Js(j)) => bounded(js_len(j)),
                Ok(_) => {}
                Err(e) => bounded(e.message.len()),
            },
            Output::Note { value, .. } => bounded(js_len(value)),
            _ => {}
        }
    }
}

struct Run {
    c: Core,
    id: u64,
    outs: Vec<Output>,
}

impl Run {
    fn call(&mut self, req: Request) {
        self.id += 1;
        let o = self.c.step(self.id, Input::Call { id: self.id, req }).unwrap();
        self.outs.extend(o);
    }

    /// answer the newest peek with a file, or a directory, of these
    fn answer(&mut self, files: &[(&[&str], Value<'_>)], dir: bool) {
        let Some(wire) = self.c.waiting().peeks.last().cloned() else {
            return;
        };
        let mut s = Silo::default();
        let top = if dir {
            s.tree(files)
        } else {
            s.leaf(files[0].1.clone())
        };
        let n = s.snap(&wire, top);
        let o = self.c.hook(self.id, Hook::Poke(ZOD, "grubbery-transfer", n.borrow()));
        self.outs.extend(o);
    }

    /// a grubbery-load %poke from ~zod to our [folder name], of mark
    fn poke(&mut self, folder: &str, name: &str, mark: (&str, &str), n: Value<'_>) {
        let m = urb::msg::poke("/grub-poke/1/1", folder, name, mark, n);
        let n = Noun::of(&m).unwrap();
        let o = self.c.hook(self.id, Hook::Poke(ZOD, "grubbery-load", n.borrow()));
        self.outs.extend(o);
    }
}

fuzz_target!(|data: &[u8]| {
    let [.., k, pick] = data else { return };
    let Some(n) = urb_fuzz::nouns(data, 2) else {
        urb_fuzz::done();
        return;
    };
    let us = urb::engine::ship_parse("~nec").unwrap();
    let mut r = Run {
        c: Core::new(|n| vec![7; n]),
        id: 0,
        outs: Vec::new(),
    };
    r.call(Request::Open {
        keyfile: Some(urb::engine::fake_keyfile(us)),
        rows: None,
        fresh: true,
        fake: true,
    });
    let (a, b) = (Value::Noun(n[0].borrow()), Value::Noun(n[1].borrow()));
    let fu = |call| Request::Furum {
        host: ZOD.into(),
        call,
    };
    let fo = |call| Request::Forum {
        host: ZOD.into(),
        call,
    };
    let id = f64::from(*k % 4);
    match pick % 8 {
        0 => {
            r.call(fu(FurumCall::NextPost("b".into())));
            r.answer(&[(&["conf"], a)], false);
        }
        1 => {
            r.call(fu(FurumCall::Posts("b".into())));
            let conf = v(vec![1u64.into(), 150u64.into(), Value::Null]);
            r.answer(&[(&["conf"], conf)], false);
            r.answer(&[(&["b0"], a)], false);
            r.answer(&[(&["b1"], b)], false);
        }
        2 => {
            r.call(fu(FurumCall::Comments("b".into(), id)));
            r.answer(&[(&["b0"], a)], false);
        }
        3 => {
            r.call(fu(FurumCall::Votes("b".into(), id)));
            r.answer(&[(&["b0"], a)], false);
        }
        4 => {
            r.call(fo(ForumCall::Thread("b".into(), urb::msg::Id::Num(id))));
            r.answer(&[(&["index"], Value::Null)], false);
            let pub_ = ["card", "roles", "conf"];
            let content = ["pins", "sidebar", "posts", "threads", "votes"];
            let at = |k: u8, names: &[&'static str]| names[k as usize % names.len()];
            r.answer(
                &[(&[at(*k, &pub_)], a.clone()), (&[at(*k / 3, &pub_)], b.clone())],
                true,
            );
            let (x, y) = (at(*k, &content), at(*k / 5, &content));
            let px: &[&str] = if matches!(x, "pins" | "sidebar") { &[x] } else { &[x, "b0"][..] };
            let py: &[&str] = if matches!(y, "pins" | "sidebar") { &[y] } else { &[y, "b0"][..] };
            r.answer(&[(px, b), (py, a)], true);
        }
        5 => {
            r.call(fu(FurumCall::OnNote));
            r.poke(furum::BASE, "inbox.sig", furum::MSG, a);
        }
        6 => {
            r.call(fo(ForumCall::OnNote));
            let to = board::BASES[*k as usize % 2];
            r.poke(to, "forum.sig", board::NOTE, a);
            r.poke(DESK_BASE, "forum.sig", board::NOTE, b);
        }
        _ => {
            // a whole board, each grub where load:fb reads it: a is
            // [card conf] and b is [posts threads votes], buckets b0,
            // so a well-formed pair (seed.mjs's) is assembled and its
            // thread found
            r.call(fo(ForumCall::Thread("b".into(), urb::msg::Id::Num(id))));
            r.answer(&[(&["index"], Value::Null)], false);
            let (x, y) = (n[0].borrow(), n[1].borrow());
            let (card, conf) = half(x);
            let (posts, rest) = half(y);
            let (threads, votes) = half(rest);
            let nv = Value::Noun;
            r.answer(&[(&["card"], nv(card)), (&["conf"], nv(conf))], true);
            r.answer(
                &[
                    (&["posts", "b0"], nv(posts)),
                    (&["threads", "b0"], nv(threads)),
                    (&["votes", "b0"], nv(votes)),
                ],
                true,
            );
        }
    }
    let mut outs = std::mem::take(&mut r.outs);
    // the timers run out, past every peek's and poke's 30 s
    for _ in 0..200 {
        let Some(due) = r.c.deadline() else { break };
        if due > 120_000 {
            break;
        }
        let (kind, due) = r.c.due(due).unwrap();
        outs.extend(r.c.step(due, Input::Tick { kind, due }).unwrap());
    }
    check(&outs);
    drop(r);
    drop(n);
    urb_fuzz::done();
});
