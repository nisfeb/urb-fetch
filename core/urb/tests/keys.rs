//! The key cache (crate::keys) and the sponsor lookups made ahead, on a
//! core off the fake network whose roller answers are made up here.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use urb::engine::{Point, Ship, ship_parse};
use urb::keys::{Entry, KeyStore, MemKeys};
use urb::{Core, Input, Output, Request};

/// trace 10's comet, under ~hus
const KEYFILE: &str = "0wai4.GAOgV.EPyf8.MQ8dU.2KkF6.VfQjW.ugbTu.JHyNp.6vCM7.J8yac.FdEhf.jGJQG.RNSFz.iP~KC.9vr-S.XaoA3.Y2UwB.9G5gw.h01yP.dckca.0000Q.vM3i5";

fn hex32(b: u8) -> String {
    format!("0x{}", format!("{b:02x}").repeat(32))
}

/// a roller getPoint answer
fn roller(who: &str, life: u64, rift: u64, key: u8, sponsor: Option<&str>) -> String {
    let spo = match sponsor {
        Some(s) => format!(r#"{{"has":true,"who":{}}}"#, ship_parse(s).unwrap().0),
        None => r#"{"has":false,"who":0}"#.into(),
    };
    format!(
        r#"{{"id":"{who}","jsonrpc":"2.0","result":{{"dominion":"l1","network":{{"keys":{{"life":"{life}","suite":"1","auth":"{}","crypt":"{}"}},"sponsor":{spo},"rift":"{rift}"}}}}}}"#,
        hex32(key),
        hex32(key.wrapping_add(1))
    )
}

fn point(who: &str, life: u64, rift: u64, key: u8, sponsor: Option<&str>) -> Point {
    let w = ship_parse(who).unwrap();
    urb::engine::point_parse(w, roller(who, life, rift, key, sponsor).as_bytes())
        .expect("a getPoint answer")
}

struct T {
    c: Core,
    now: u64,
    keys: Rc<dyn KeyStore>,
    out: Vec<Output>,
}

impl T {
    fn new(keys: Rc<dyn KeyStore>) -> T {
        let mut c = Core::new(|n| vec![7; n]);
        c.set_keys(keys.clone());
        let mut t = T {
            c,
            now: 1_793_534_021_060,
            keys,
            out: Vec::new(),
        };
        t.call(
            1,
            Request::Open {
                keyfile: Some(KEYFILE.into()),
                rows: None,
                fresh: true,
                fake: false,
            },
        );
        t
    }

    fn step(&mut self, i: Input) {
        self.now += 1;
        let o = self.c.step(self.now, i).unwrap();
        self.out.extend(o);
    }

    fn call(&mut self, id: u64, req: Request) {
        self.step(Input::Call { id, req });
    }

    /// fire every timer due within ms
    fn run(&mut self, ms: u64) {
        let end = self.now + ms;
        while let Some((kind, due)) = self.c.due(end) {
            self.now = self.now.max(due);
            self.step(Input::Tick { kind, due });
        }
    }

    /// the roller lookups asked for and not yet answered, by ship
    fn asked(&self) -> Vec<(u64, String)> {
        self.out
            .iter()
            .filter_map(|o| match o {
                Output::Fetch {
                    id, body: Some(b), ..
                } if b.contains("getPoint") => {
                    let s = b.split("\"ship\":\"").nth(1)?.split('"').next()?;
                    Some((*id, s.to_string()))
                }
                _ => None,
            })
            .collect()
    }

    fn fetch_of(&self, who: &str) -> Option<u64> {
        self.asked()
            .into_iter()
            .find(|(_, s)| s == who)
            .map(|(id, _)| id)
    }

    fn answer(&mut self, who: &str, body: String) {
        let id = self
            .fetch_of(who)
            .unwrap_or_else(|| panic!("no lookup of {who}"));
        self.out
            .retain(|o| !matches!(o, Output::Fetch { id: i, .. } if *i == id));
        self.step(Input::Fetched {
            id,
            answer: Ok((200, body.into_bytes())),
        });
    }

    fn logged(&self, s: &str) -> bool {
        self.out
            .iter()
            .any(|o| matches!(o, Output::Log(l) if l.contains(s)))
    }
}

fn cached(who: &str, p: Point) -> Rc<MemKeys> {
    let k = Rc::new(MemKeys::default());
    k.put(ship_parse(who).unwrap(), &Entry { at: AT, point: p });
    k
}

/// when cached() entries were given: just before each T starts
const AT: u64 = 1_793_534_021_000;

#[test]
fn an_old_entry_waits_on_the_roller() {
    let k = Rc::new(MemKeys::default());
    k.put(
        (0, 0),
        &Entry {
            at: AT - urb::keys::KEY_MAX_AGE - 1,
            point: point("~zod", 6, 2, 0x11, None),
        },
    );
    let mut t = T::new(k);
    t.call(2, Request::Read("urb://~zod/page".into()));
    t.run(0);
    assert_eq!(t.c.peer("~zod"), None);
    t.answer("~zod", roller("~zod", 7, 2, 0x22, None));
    assert_eq!(t.c.peer("~zod").unwrap().life, 7);
}

#[test]
fn a_roller_answer_before_the_cached_one_stands() {
    let mut t = T::new(cached("~zod", point("~zod", 5, 2, 0x11, None)));
    t.call(2, Request::Read("urb://~zod/page".into()));
    // the roller answers before the cached keys' timer fires
    t.answer("~zod", roller("~zod", 6, 2, 0x22, None));
    t.run(0);
    assert_eq!(t.c.peer("~zod"), Some(point("~zod", 6, 2, 0x22, None)));
}

#[test]
fn a_rotation_mid_session_takes_the_new_keys_and_never_the_old() {
    // two engines on the device share the cache
    let keys = cached("~zod", point("~zod", 6, 2, 0x11, None));
    let mut a = T::new(keys.clone());
    a.call(2, Request::Read("urb://~zod/page".into()));
    a.run(0);
    a.answer("~zod", roller("~zod", 6, 2, 0x11, None));
    assert_eq!(a.c.peer("~zod").unwrap().life, 6);
    // ~zod rotates its keys; a's next lookup sees it
    a.run(urb::keys::KEY_TTL);
    assert_eq!(
        a.c.peer("~zod").unwrap().life,
        6,
        "the old keys until the roller says"
    );
    a.answer("~zod", roller("~zod", 7, 2, 0x22, None));
    assert_eq!(a.c.peer("~zod"), Some(point("~zod", 7, 2, 0x22, None)));
    assert_eq!(keys.get((0, 0)).unwrap().point.life, 7);
    // a later lookup by a's engine is answered by the roller alone: an
    // older entry another process left can't take the keys back
    keys.put(
        (0, 0),
        &Entry {
            at: a.now,
            point: point("~zod", 6, 2, 0x11, None),
        },
    );
    a.run(urb::keys::KEY_TTL);
    assert_eq!(a.c.peer("~zod").unwrap().life, 7);
    a.answer("~zod", roller("~zod", 8, 3, 0x33, None));
    assert_eq!(a.c.peer("~zod"), Some(point("~zod", 8, 3, 0x33, None)));
    // a fresh engine on the device starts from what a learned
    let mut b = T::new(keys.clone());
    b.now = a.now;
    b.call(2, Request::Read("urb://~zod/page".into()));
    b.run(0);
    assert_eq!(b.c.peer("~zod").unwrap().rift, 3);
}

#[test]
fn cached_keys_are_used_before_the_roller_answers() {
    let mut t = T::new(cached("~zod", point("~zod", 6, 2, 0x11, None)));
    t.call(2, Request::Read("urb://~zod/page".into()));
    // still asked, in the background
    assert!(t.fetch_of("~zod").is_some(), "{:?}", t.asked());
    assert_eq!(t.c.peer("~zod"), None);
    t.run(0);
    assert_eq!(t.c.peer("~zod").unwrap().life, 6);
    // the same answer: nothing changes but when it was checked
    t.answer("~zod", roller("~zod", 6, 2, 0x11, None));
    assert!(!t.logged("changed"));
    assert_eq!(t.c.peer("~zod"), Some(point("~zod", 6, 2, 0x11, None)));
    assert!(t.keys.get((0, 0)).unwrap().at > 1);
}

#[test]
fn a_new_life_replaces_cached_keys() {
    let mut t = T::new(cached("~zod", point("~zod", 5, 2, 0x11, None)));
    t.call(2, Request::Read("urb://~zod/page".into()));
    t.run(0);
    assert_eq!(t.c.peer("~zod").unwrap().life, 5);
    t.answer("~zod", roller("~zod", 6, 2, 0x22, None));
    assert!(t.logged("the keys of ~zod changed: life 5 to 6, rift 2 to 2"));
    assert_eq!(t.c.peer("~zod"), Some(point("~zod", 6, 2, 0x22, None)));
    assert_eq!(t.keys.get((0, 0)).unwrap().point.life, 6);
}

#[test]
fn a_new_rift_replaces_cached_keys() {
    let mut t = T::new(cached("~zod", point("~zod", 6, 1, 0x11, None)));
    t.call(2, Request::Read("urb://~zod/page".into()));
    t.run(0);
    assert_eq!(t.c.peer("~zod").unwrap().rift, 1);
    t.answer("~zod", roller("~zod", 1, 2, 0x33, None));
    assert!(t.logged("the keys of ~zod changed: life 6 to 1, rift 1 to 2"));
    assert_eq!(t.c.peer("~zod"), Some(point("~zod", 1, 2, 0x33, None)));
    assert_eq!(t.keys.get((0, 0)).unwrap().point.rift, 2);
}

#[test]
fn a_failed_lookup_keeps_cached_keys() {
    let mut t = T::new(cached("~zod", point("~zod", 6, 2, 0x11, None)));
    t.call(2, Request::Read("urb://~zod/page".into()));
    t.run(0);
    let id = t.fetch_of("~zod").unwrap();
    t.step(Input::Fetched {
        id,
        answer: Err("fetch failed".into()),
    });
    assert!(t.logged("azimuth lookup of ~zod failed"));
    assert_eq!(t.c.peer("~zod").unwrap().life, 6);
}

#[test]
fn keys_are_asked_again_after_the_ttl() {
    let mut t = T::new(cached("~zod", point("~zod", 6, 2, 0x11, None)));
    t.call(2, Request::Read("urb://~zod/page".into()));
    t.run(0);
    t.answer("~zod", roller("~zod", 6, 2, 0x11, None));
    assert!(t.c.timers().iter().any(|(k, _)| k == "rekey ~zod"));
    t.run(urb::keys::KEY_TTL);
    t.answer("~zod", roller("~zod", 7, 2, 0x44, None));
    assert_eq!(t.c.peer("~zod").unwrap().life, 7);
}

#[test]
fn a_planets_default_sponsor_is_asked_for_at_once() {
    let mut t = T::new(Rc::new(MemKeys::default()));
    t.call(2, Request::Read("urb://~sampel-palnet/page".into()));
    let star = urb::engine::ship_text((ship_parse("~sampel-palnet").unwrap().0 & 0xffff, 0));
    let gal = urb::engine::ship_text((ship_parse("~sampel-palnet").unwrap().0 & 0xff, 0));
    let asked: Vec<String> = t.asked().into_iter().map(|(_, s)| s).collect();
    for s in ["~sampel-palnet", &star, &gal] {
        assert!(asked.iter().any(|a| a == s), "{s} in {asked:?}");
    }
    // its sponsor is the star by its name: the engine's lookup of it is
    // the one made ahead
    t.answer(
        "~sampel-palnet",
        roller("~sampel-palnet", 1, 0, 0x55, Some(&star)),
    );
    t.run(10_000);
    let again = t.asked().into_iter().filter(|(_, s)| *s == star).count();
    assert_eq!(again, 1, "{:?}", t.asked());
    t.answer(&star, roller(&star, 1, 0, 0x66, Some(&gal)));
    t.run(0);
    assert!(t.c.peer(&star).is_some());
    assert!(t.keys.get(ship_parse(&star).unwrap()).is_some());
}

#[test]
fn a_sponsor_unlike_the_name_is_looked_up_after() {
    let mut t = T::new(Rc::new(MemKeys::default()));
    t.call(2, Request::Read("urb://~ricsul-bilwyt/page".into()));
    // ~ricsul-bilwyt's star by its number is ~batbus, but ~nisfeb
    // sponsors it
    let star = urb::engine::ship_text((ship_parse("~ricsul-bilwyt").unwrap().0 & 0xffff, 0));
    assert_eq!(star, "~batbus");
    assert!(t.fetch_of(&star).is_some(), "{:?}", t.asked());
    assert!(t.fetch_of("~nisfeb").is_none());
    t.answer(
        "~ricsul-bilwyt",
        roller("~ricsul-bilwyt", 1, 0, 0x55, Some("~nisfeb")),
    );
    t.run(10_000);
    // the real sponsor, asked for once its planet named it
    assert!(t.fetch_of("~nisfeb").is_some(), "{:?}", t.asked());
    t.answer("~nisfeb", roller("~nisfeb", 2, 0, 0x77, Some("~feb")));
    t.run(0);
    assert_eq!(t.c.peer("~nisfeb").unwrap().life, 2);
    // the guess is kept, for whoever needs it, but the engine isn't
    // handed it
    t.answer(&star, roller(&star, 1, 0, 0x78, Some("~bus")));
    t.run(0);
    assert!(t.keys.get(ship_parse(&star).unwrap()).is_some());
    assert_eq!(t.c.peer(&star), None);
}

/// a store that keeps whatever it is given, as a host's own might: the
/// core must not hand it an older point
#[derive(Default)]
struct Plain(RefCell<HashMap<Ship, Entry>>);

impl KeyStore for Plain {
    fn get(&self, who: Ship) -> Option<Entry> {
        self.0.borrow().get(&who).cloned()
    }

    fn put(&self, who: Ship, e: &Entry) {
        self.0.borrow_mut().insert(who, e.clone());
    }
}

#[test]
fn a_stale_roller_answer_never_replaces_newer_cached_keys() {
    let k = Rc::new(Plain::default());
    k.put(
        (0, 0),
        &Entry {
            at: AT,
            point: point("~zod", 7, 2, 0x22, None),
        },
    );
    let mut t = T::new(k.clone());
    t.call(2, Request::Read("urb://~zod/page".into()));
    t.run(0);
    assert_eq!(t.c.peer("~zod").unwrap().life, 7);
    // a roller replica behind the chain answers
    t.answer("~zod", roller("~zod", 6, 2, 0x11, None));
    assert!(t.logged("an older point for ~zod was refused"));
    assert_eq!(t.c.peer("~zod"), Some(point("~zod", 7, 2, 0x22, None)));
    assert_eq!(k.get((0, 0)).unwrap().point.life, 7);
}

#[test]
fn a_rekey_never_takes_an_older_life() {
    let mut t = T::new(Rc::new(MemKeys::default()));
    t.call(2, Request::Read("urb://~zod/page".into()));
    t.answer("~zod", roller("~zod", 7, 2, 0x22, None));
    t.run(0);
    assert_eq!(t.c.peer("~zod").unwrap().life, 7);
    t.run(urb::keys::KEY_TTL);
    t.answer("~zod", roller("~zod", 6, 2, 0x11, None));
    assert_eq!(t.c.peer("~zod"), Some(point("~zod", 7, 2, 0x22, None)));
    assert_eq!(t.keys.get((0, 0)).unwrap().point.life, 7);
    // a later rotation still lands
    t.run(urb::keys::KEY_TTL);
    t.answer("~zod", roller("~zod", 8, 2, 0x33, None));
    assert_eq!(t.c.peer("~zod"), Some(point("~zod", 8, 2, 0x33, None)));
}

#[test]
fn a_cached_point_past_a_day_is_not_used_when_the_roller_fails() {
    let k = Rc::new(MemKeys::default());
    k.put(
        (0, 0),
        &Entry {
            at: AT - 2 * 24 * 60 * 60 * 1000,
            point: point("~zod", 6, 2, 0x11, None),
        },
    );
    let mut t = T::new(k);
    t.call(2, Request::Read("urb://~zod/page".into()));
    t.run(0);
    assert_eq!(t.c.peer("~zod"), None, "not used before the roller answers");
    let id = t.fetch_of("~zod").unwrap();
    t.step(Input::Fetched {
        id,
        answer: Err("fetch failed".into()),
    });
    assert_eq!(t.c.peer("~zod"), None, "nor once it fails");
}

#[test]
fn keys_the_engine_kept_beat_an_older_cached_point() {
    let k = Rc::new(Plain::default());
    let mut t = T::new(k.clone());
    t.call(2, Request::Read("urb://~zod/page".into()));
    t.answer("~zod", roller("~zod", 7, 2, 0x22, None));
    t.run(0);
    assert_eq!(t.c.peer("~zod").unwrap().life, 7);
    let rows: Vec<u8> = t
        .out
        .iter()
        .filter_map(|o| match o {
            Output::Commit(r) => Some(r.clone()),
            _ => None,
        })
        .flatten()
        .collect();
    t.call(3, Request::Close);
    t.run(0);
    // a process from before the compare left an older point
    k.put(
        (0, 0),
        &Entry {
            at: t.now,
            point: point("~zod", 6, 2, 0x11, None),
        },
    );
    t.out.clear();
    t.call(
        4,
        Request::Open {
            keyfile: Some(KEYFILE.into()),
            rows: Some(rows),
            fresh: false,
            fake: false,
        },
    );
    t.run(0);
    t.call(5, Request::Read("urb://~zod/page".into()));
    // the engine asks the roller again; the cache's older point, though
    // fresh, must not be given it
    assert!(t.fetch_of("~zod").is_some(), "{:?}", t.asked());
    t.run(0);
    assert_eq!(t.c.peer("~zod"), Some(point("~zod", 7, 2, 0x22, None)));
    // the roller confirms, and the cache takes it back
    t.answer("~zod", roller("~zod", 7, 2, 0x22, None));
    assert_eq!(k.get((0, 0)).unwrap().point.life, 7);
}
