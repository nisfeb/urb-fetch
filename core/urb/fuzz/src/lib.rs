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

use urb::{Noun, UrbError, alive};

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
        match op % 10 {
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
                // a big atom, past the size caps
                let n = 1 + byte().unwrap_or(0) as usize;
                stack.push(Noun::atom(&vec![0xa5; n * 4]));
            }
            8 => {
                // a list of the top, k long, sharing it: now and then
                // one past the list cap, but mostly short, so a run is
                // quick
                if let Some(top) = stack.pop() {
                    let k = match byte().unwrap_or(0) {
                        255 => urb::codec::MAX_CELLS + 1,
                        b => b as usize * 16,
                    };
                    let mut l = Noun::atom(&[]);
                    for _ in 0..k {
                        l = Noun::cell(top.clone(), l);
                    }
                    stack.push(l);
                }
            }
            _ => {
                let n = stack.len();
                if n >= 2 {
                    stack.swap(n - 1, n - 2);
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
