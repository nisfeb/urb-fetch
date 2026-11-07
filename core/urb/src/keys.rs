//! The key cache: each ship's point as the roller last gave it, with
//! when, kept across comets so a fresh one need not wait on the roller.
//!
//! A point is public (it is on azimuth), so one cache serves every
//! identity on the device. The core uses a cached point given in the
//! last [`KEY_MAX_AGE`] at once, for an engine's first lookup of a ship
//! only, and asks the roller again in the background; if the roller's answer differs
//! (a new life, rift, key or sponsor), the cache takes it and the engine
//! is given it, which breaches the ship's sessions on a new rift and
//! rekeys them on a new life (libmesa's `eng_point_apply`). While an
//! engine runs, each ship it looked up is asked again every
//! [`KEY_TTL`].
//!
//! Where the cache lives is the host's: [`KeyStore`] is all the core
//! sees. [`MemKeys`] keeps it in memory (the wasm host's); the native
//! host keeps a file per ship (`host::keys::FileKeys`).

use crate::engine::{Point, Ship, ship_parse, ship_text};
use std::cell::RefCell;
use std::collections::HashMap;

/// how often an engine asks the roller again for a ship it uses
pub const KEY_TTL: u64 = 24 * 60 * 60 * 1000;

/// How old a cached point may be and still be used before the roller
/// answers. An older one waits on the roller, as with no cache.
///
/// The engine can't take back a point once given, so a cached point it
/// was given stays in use if the roller lookup then fails. This window
/// bounds how stale that can be: no older than the keys a running engine
/// already uses between its [`KEY_TTL`] lookups. The cost is speed: a
/// device away for more than a day waits on the roller for its first
/// contact with each ship, as it would with no cache.
pub const KEY_MAX_AGE: u64 = KEY_TTL;

/// How long a store keeps a point against an older one: past it, the
/// entry is long unused (it isn't [`fresh`]), and a lower point may take
/// its place, so one bad entry can't keep a ship out of the cache.
pub const KEY_FORGET: u64 = 30 * 24 * 60 * 60 * 1000;

/// How many ships a [`MemKeys`] keeps: past it, the entry the roller
/// gave longest ago goes.
pub const MEM_MAX: usize = 4096;

/// whether e may be used at now without the roller: given in the last
/// KEY_MAX_AGE, and not from the future (a clock set back, or a bad
/// entry)
pub fn fresh(e: &Entry, now: u64) -> bool {
    e.at <= now.saturating_add(60_000) && now.saturating_sub(e.at) <= KEY_MAX_AGE
}

/// whether a is older than b: a lower rift, or the same rift and a
/// lower life
pub fn older(a: &Point, b: &Point) -> bool {
    (a.rift, a.life) < (b.rift, b.life)
}

/// Whether a store holding old may take new in its place: a newer point
/// always, the same point (rift and life) given no earlier, and an older
/// point only once old is past [`KEY_FORGET`] or dated ahead of new (a
/// clock set back, or a bad entry).
pub fn supersedes(new: &Entry, old: &Entry) -> bool {
    if older(&old.point, &new.point) {
        return true;
    }
    let lapsed =
        new.at > old.at.saturating_add(KEY_FORGET) || old.at > new.at.saturating_add(60_000);
    if older(&new.point, &old.point) {
        return lapsed;
    }
    new.at >= old.at || lapsed
}

/// a ship's point and when the roller gave it (ms since 1970)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub at: u64,
    pub point: Point,
}

/// Where the cache is kept. Errors are the store's to swallow: a cache
/// that can't be read or written is only slower.
pub trait KeyStore {
    fn get(&self, who: Ship) -> Option<Entry>;
    /// keep e for who, unless what is held for who [`supersedes`] it not
    fn put(&self, who: Ship, e: &Entry);
}

/// a cache in memory
#[derive(Default)]
pub struct MemKeys(RefCell<HashMap<Ship, Entry>>);

impl KeyStore for MemKeys {
    fn get(&self, who: Ship) -> Option<Entry> {
        self.0.borrow().get(&who).cloned()
    }

    fn put(&self, who: Ship, e: &Entry) {
        self.keep(who, e);
    }
}

/// drop the oldest entries past MEM_MAX
fn evict(m: &mut HashMap<Ship, Entry>) {
    if m.len() <= MEM_MAX {
        return;
    }
    let mut v: Vec<(u64, Ship)> = m.iter().map(|(w, e)| (e.at, *w)).collect();
    v.sort_unstable();
    for (_, w) in &v[..m.len() - MEM_MAX] {
        m.remove(w);
    }
}

impl MemKeys {
    /// [`KeyStore::put`], and whether e was kept
    pub fn keep(&self, who: Ship, e: &Entry) -> bool {
        let mut m = self.0.borrow_mut();
        if m.get(&who).is_some_and(|o| !supersedes(e, o)) {
            return false;
        }
        m.insert(who, e.clone());
        evict(&mut m);
        true
    }

    /// every entry, one [`encode`]d line each
    pub fn dump(&self) -> String {
        let mut v: Vec<_> = self.0.borrow().iter().map(|(w, e)| encode(*w, e)).collect();
        v.sort();
        v.join("")
    }

    /// take the lines [`MemKeys::dump`] gave; lines that don't read are
    /// skipped, and an entry replaces one held only if it [`supersedes`]
    /// it
    pub fn load(&self, text: &str) {
        let mut m = self.0.borrow_mut();
        for (w, e) in text.lines().filter_map(decode) {
            if m.get(&w).is_none_or(|o| supersedes(&e, o)) {
                m.insert(w, e);
                // bounded as it goes, however long the text
                if m.len() > 2 * MEM_MAX {
                    evict(&mut m);
                }
            }
        }
        evict(&mut m);
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) || !s.is_ascii() {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

/// an entry as one line: `v1 <ship> <at> <rift> <life> <pass in hex>
/// <sponsor or ->`, with its newline
pub fn encode(who: Ship, e: &Entry) -> String {
    let p = &e.point;
    format!(
        "v1 {} {} {} {} {} {}\n",
        ship_text(who),
        e.at,
        p.rift,
        p.life,
        hex(&p.pass),
        p.sponsor.map_or("-".into(), ship_text)
    )
}

/// a line [`encode`] wrote, or None
pub fn decode(line: &str) -> Option<(Ship, Entry)> {
    let f: Vec<&str> = line.split_ascii_whitespace().collect();
    let ["v1", who, at, rift, life, pass, spo] = f[..] else {
        return None;
    };
    let pass: [u8; 65] = unhex(pass)?.try_into().ok()?;
    let sponsor = match spo {
        "-" => None,
        s => Some(ship_parse(s)?),
    };
    Some((
        ship_parse(who)?,
        Entry {
            at: at.parse().ok()?,
            point: Point {
                rift: rift.parse().ok()?,
                life: life.parse().ok()?,
                pass,
                sponsor,
            },
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_round_trip() {
        let mut pass = [7u8; 65];
        pass[0] = b'b';
        let e = Entry {
            at: 1793534021060,
            point: Point {
                rift: 2,
                life: 6,
                pass,
                sponsor: Some((0x100 | 0xb5, 0)),
            },
        };
        let who = ship_parse("~ricsul-bilwyt").unwrap();
        let l = encode(who, &e);
        assert_eq!(decode(&l), Some((who, e.clone())));
        let none = Entry {
            point: Point {
                sponsor: None,
                ..e.point.clone()
            },
            ..e
        };
        assert_eq!(decode(&encode(who, &none)), Some((who, none)));
        for bad in [
            "",
            "v2 ~zod 1 0 1 00 -",
            "v1 ~zod 1 0 1 0011 -",
            "v1 nope 1 0 1 00 -",
        ] {
            assert_eq!(decode(bad), None, "{bad}");
        }
    }

    #[test]
    fn mem_keeps_at_most_mem_max() {
        let k = MemKeys::default();
        let mut pass = [0u8; 65];
        pass[0] = b'b';
        let p = Point {
            rift: 0,
            life: 1,
            pass,
            sponsor: None,
        };
        let n = MEM_MAX as u64 + 10;
        for i in 0..n {
            k.put(
                (0x1_0000 + i, 0),
                &Entry {
                    at: 100 + i,
                    point: p.clone(),
                },
            );
        }
        assert_eq!(k.0.borrow().len(), MEM_MAX);
        // the oldest went, the newest stayed
        assert!(k.get((0x1_0000, 0)).is_none());
        assert!(k.get((0x1_0000 + n - 1, 0)).is_some());
        let text = k.dump();
        let j = MemKeys::default();
        j.load(&text);
        j.load(&text.repeat(2));
        assert_eq!(j.0.borrow().len(), MEM_MAX);
    }

    #[test]
    fn fresh_is_recent_and_not_ahead() {
        let mut pass = [0u8; 65];
        pass[0] = b'b';
        let e = |at| Entry {
            at,
            point: Point {
                rift: 0,
                life: 1,
                pass,
                sponsor: None,
            },
        };
        let now = 1_793_534_021_060;
        assert!(fresh(&e(now), now));
        assert!(fresh(&e(now - KEY_MAX_AGE), now));
        assert!(!fresh(&e(now - KEY_MAX_AGE - 1), now));
        assert!(!fresh(&e(now + 3_600_000), now));
    }

    #[test]
    fn mem_dump_load_keeps_the_newest() {
        let a = MemKeys::default();
        let mut pass = [0u8; 65];
        pass[0] = b'b';
        let p = Point {
            rift: 0,
            life: 1,
            pass,
            sponsor: None,
        };
        a.put(
            (0, 0),
            &Entry {
                at: 5,
                point: p.clone(),
            },
        );
        let b = MemKeys::default();
        b.put(
            (0, 0),
            &Entry {
                at: 9,
                point: Point {
                    life: 2,
                    ..p.clone()
                },
            },
        );
        b.load(&a.dump());
        assert_eq!(b.get((0, 0)).unwrap().point.life, 2);
        a.load(&b.dump());
        assert_eq!(a.get((0, 0)).unwrap().at, 9);
    }

    fn at(at: u64, rift: u64, life: u64) -> Entry {
        let mut pass = [life as u8; 65];
        pass[0] = b'b';
        Entry {
            at,
            point: Point {
                rift,
                life,
                pass,
                sponsor: None,
            },
        }
    }

    #[test]
    fn supersedes_orders_by_rift_then_life() {
        let old = at(1000, 2, 5);
        assert!(
            supersedes(&at(10, 2, 6), &old),
            "a newer life, given earlier"
        );
        assert!(supersedes(&at(10, 3, 1), &old), "a newer rift, lower life");
        assert!(supersedes(&at(1000, 2, 5), &old), "the same, again");
        assert!(supersedes(&at(2000, 2, 5), &old), "the same, later");
        assert!(!supersedes(&at(999, 2, 5), &old), "the same, earlier");
        assert!(!supersedes(&at(2000, 2, 4), &old), "a lower life");
        assert!(!supersedes(&at(2000, 1, 9), &old), "a lower rift");
        // a long-unused entry, or one dated ahead, gives way
        assert!(supersedes(&at(1001 + KEY_FORGET, 1, 1), &old));
        assert!(supersedes(&at(1, 1, 1), &at(1_000_000, 2, 5)));
    }

    #[test]
    fn mem_put_never_takes_an_older_point() {
        let k = MemKeys::default();
        assert!(k.keep((0, 0), &at(100, 2, 6)));
        k.put((0, 0), &at(200, 2, 5));
        k.put((0, 0), &at(300, 1, 9));
        assert_eq!(k.get((0, 0)), Some(at(100, 2, 6)));
        assert!(!k.keep((0, 0), &at(400, 2, 5)));
        assert!(k.keep((0, 0), &at(50, 2, 7)));
        assert_eq!(k.get((0, 0)).unwrap().point.life, 7);
    }

    #[test]
    fn mem_load_keeps_the_newest_point_not_the_latest_entry() {
        // another tab saw ~zod's new life first; this one's older
        // lookup finished later
        let a = MemKeys::default();
        a.put((0, 0), &at(100, 2, 7));
        let b = MemKeys::default();
        b.put((0, 0), &at(200, 2, 6));
        b.load(&a.dump());
        assert_eq!(b.get((0, 0)), Some(at(100, 2, 7)));
        a.load(&b.dump());
        assert_eq!(a.get((0, 0)), Some(at(100, 2, 7)));
    }
}
