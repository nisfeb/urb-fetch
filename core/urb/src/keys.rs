//! The key cache: each ship's point as the roller last gave it, with
//! when, kept across comets so a fresh one need not wait on the roller.
//!
//! A point is public (it is on azimuth), so one cache serves every
//! identity on the device. The core uses a cached point at once and asks
//! the roller again in the background; if the roller's answer differs
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
        self.0.borrow_mut().insert(who, e.clone());
    }
}

impl MemKeys {
    /// every entry, one [`encode`]d line each
    pub fn dump(&self) -> String {
        let mut v: Vec<_> = self.0.borrow().iter().map(|(w, e)| encode(*w, e)).collect();
        v.sort();
        v.join("")
    }

    /// take the lines [`MemKeys::dump`] gave; lines that don't read are
    /// skipped, and an entry newer than one held replaces it
    pub fn load(&self, text: &str) {
        for (w, e) in text.lines().filter_map(decode) {
            let mut m = self.0.borrow_mut();
            if m.get(&w).is_none_or(|o| o.at < e.at) {
                m.insert(w, e);
            }
        }
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
}
