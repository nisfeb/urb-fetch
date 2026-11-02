//! What the fuzz targets share: nouns from a fuzzer's bytes, and the
//! checks every target makes.
//!
//! An input's first byte says how its nouns are made:
//!
//! - even: as jam. One noun is the rest; two are a 2-byte little-endian
//!   length, the first jam, then the second.
//! - odd: by a small stack machine, so that shapes a jam rarely hits come
//!   often: shared cells that are vast as trees, the tags the decoders
//!   look for, long lists and atoms past every cap. The nouns are the top
//!   of the stack.
//!
//! Besides never panicking and failing only with mesa.mjs's errors, a
//! decoder's output must stay under [`OUT_MAX`], whatever the input:
//! the caps bound what a decode reads, at each place it reads it, and so
//! what it gives back. The time a run takes is libFuzzer's -timeout.

use urb::codec::{MAX_BYTES, MAX_CELLS};
use urb::js::Json;
use urb::{Noun, UrbError, alive};

/// The most bytes a decode's output may hold: its atoms' bytes, each of
/// which may decode to three (U+FFFD), and a few for each cell.
pub const OUT_MAX: usize = 3 * MAX_BYTES + 16 * MAX_CELLS;

/// that a decoder's output, of `len` bytes, is under [`OUT_MAX`]
pub fn bounded(len: usize) {
    assert!(len <= OUT_MAX, "a decode gave {len} bytes, over {OUT_MAX}");
}

/// the bytes a $json value holds: its text and 8 for each value
pub fn json_len(j: &Json) -> usize {
    let mut n = 0;
    let mut todo = vec![j];
    while let Some(j) = todo.pop() {
        n += 8;
        match j {
            Json::Str(s) => n += s.len(),
            Json::Arr(a) => todo.extend(a),
            Json::Obj(o) => {
                for (k, v) in o.entries() {
                    n += k.len();
                    todo.push(v);
                }
            }
            _ => {}
        }
    }
    n
}

/// the bytes a JS value an app gives holds: its text and 8 for each
/// value
pub fn js_len(j: &urb::js::Js) -> usize {
    use urb::js::Js;
    let mut n = 0;
    let mut todo = vec![j];
    while let Some(j) = todo.pop() {
        n += 8;
        match j {
            Js::Str(s) => n += s.len(),
            Js::Big(i) => n += i.le().len(),
            Js::Arr(a) => todo.extend(a),
            Js::Obj(o) => {
                for (k, v) in o.entries() {
                    n += k.len();
                    todo.push(v);
                }
            }
            _ => {}
        }
    }
    n
}

/// an error from furum's or forum's reading of a board: a hostile-noun
/// error, or one of forum.mjs's own, maybe naming the grub
pub fn board_error<T>(r: &Result<T, UrbError>) {
    let Err(e) = r else { return };
    let mut m = e.message.as_str();
    if let Some(rest) = m.strip_prefix("~zod's board grub is unreadable: /") {
        let at = rest.find(" (").expect("the grub's error");
        m = &rest[at + 2..rest.len() - 1];
    }
    let own = ["a cell where an atom goes", "not a flag", "not a unit", "not version 1"];
    assert!(
        m.starts_with("another ship's noun ")
            || own.contains(&m)
            || m.starts_with("no role ")
            || m.starts_with("no vote target %")
            || m == "~zod's board has no card"
            || (m.starts_with("no post ") && m.ends_with(" on ~zod's board b")),
        "not a board error: {}",
        e.message
    );
}

/// tags and texts the decoders look for
const WORDS: &[&[u8]] = &[
    b"o",
    b"a",
    b"b",
    b"n",
    b"s",
    b"leaf",
    b"rose",
    b"palm",
    b"firm",
    b"temp",
    b"tree",
    b"\xef\xbb\xbfo",
    b"1e3",
    b"0x1f",
    b"-Infinity",
    b"\xff",
    b"post",
    b"comment",
    b"note",
    b"mod",
    b"poster",
    b"reader",
];

/// `want` nouns from the input, or None if it holds none
pub fn nouns(data: &[u8], want: usize) -> Option<Vec<Noun>> {
    let (&mode, rest) = data.split_first()?;
    if mode & 1 == 0 {
        return jams(rest, want);
    }
    let mut stack: Vec<Noun> = Vec::new();
    let mut i = 0;
    let mut byte = || {
        let b = rest.get(i).copied();
        i += 1;
        b
    };
    while let Some(op) = byte() {
        match op % 12 {
            0 => {
                let n = byte().unwrap_or(0) as usize % 48;
                let b: Vec<u8> = (0..n).map(|k| byte().unwrap_or(k as u8)).collect();
                stack.push(Noun::atom(&b));
            }
            1 => stack.push(Noun::atom(&[byte().unwrap_or(0)])),
            2 | 3 => {
                if stack.len() >= 2 {
                    let t = stack.pop()?;
                    let h = stack.pop()?;
                    stack.push(Noun::cell(h, t));
                } else {
                    stack.push(Noun::atom(&[]));
                }
            }
            4 => {
                if let Some(top) = stack.last() {
                    stack.push(top.clone());
                }
            }
            5 => {
                let w = WORDS[byte().unwrap_or(0) as usize % WORDS.len()];
                stack.push(Noun::atom(w));
            }
            6 => {
                // [x x]: shared, so its tree doubles
                if let Some(top) = stack.pop() {
                    let again = top.clone();
                    stack.push(Noun::cell(top, again));
                }
            }
            7 => {
                // a big atom, past the size caps; now and then 2 MiB, which
                // shared at many places is past the byte cap
                let n = match byte().unwrap_or(0) {
                    255 => 2 << 20,
                    b => 4 + b as usize * 4,
                };
                stack.push(Noun::atom(&vec![0xa5; n]));
            }
            8 => {
                // a list of the top, k long, sharing it: now and then
                // one past the list cap, but mostly short, so a run is
                // quick
                if let Some(top) = stack.pop() {
                    let k = match byte().unwrap_or(0) {
                        255 => MAX_CELLS + 1,
                        // a tape long enough that, as a separator between
                        // as many kids, it prints past the byte cap
                        254 => 10_000,
                        b => b as usize * 16,
                    };
                    let mut l = Noun::atom(&[]);
                    for _ in 0..k {
                        l = Noun::cell(top.clone(), l);
                    }
                    stack.push(l);
                }
            }
            9 => {
                let n = stack.len();
                if n >= 2 {
                    stack.swap(n - 1, n - 2);
                }
            }
            10 => {
                // a map (a treap [n l r], leaning right) of k entries, each
                // [key top] with keys 1 to k: many names for one ject
                if let Some(top) = stack.pop() {
                    let k = match byte().unwrap_or(0) {
                        255 => 1000,
                        b => b as usize,
                    };
                    let mut m = Noun::atom(&[]);
                    for i in (1..=k as u32).rev() {
                        let e = Noun::cell(Noun::atom(&i.to_le_bytes()), top.clone());
                        m = Noun::cell(e, Noun::cell(Noun::atom(&[]), m));
                    }
                    stack.push(m);
                }
            }
            _ => {
                // a tree ject [%tree ~ 0 ~ fil dir], dir the top, fil
                // under it
                if stack.len() >= 2 {
                    let dir = stack.pop()?;
                    let fil = stack.pop()?;
                    let z = || Noun::atom(&[]);
                    let t = Noun::cell(fil, dir);
                    let t = Noun::cell(z(), Noun::cell(z(), Noun::cell(z(), t)));
                    stack.push(Noun::cell(Noun::atom(b"tree"), t));
                }
            }
        }
        if stack.len() > 4096 {
            return None;
        }
    }
    while stack.len() < want {
        stack.push(Noun::atom(&[]));
    }
    Some(stack.split_off(stack.len() - want))
}

fn jams(rest: &[u8], want: usize) -> Option<Vec<Noun>> {
    if want == 1 {
        return Some(vec![Noun::cue(rest)?]);
    }
    let len = u16::from_le_bytes([*rest.first()?, *rest.get(1)?]) as usize;
    let rest = &rest[2..];
    let at = len.min(rest.len());
    Some(vec![Noun::cue(&rest[..at])?, Noun::cue(&rest[at..])?])
}

/// an error from a decoder: one of mesa.mjs's own, about the noun
pub fn hostile<T>(r: &Result<T, UrbError>) {
    if let Err(e) = r {
        assert!(
            e.message.starts_with("another ship's noun "),
            "not a hostile-noun error: {}",
            e.message
        );
    }
}

/// the end of every run: no noun alive
pub fn done() {
    assert_eq!(alive(), 0, "nouns left alive");
}
