//! The decoders of other ships' nouns, with mesa.mjs's caps, and the
//! path builder. Every noun another ship sends is hostile (rule 4 of the
//! port): each is read under these caps, and anything past one is an
//! [`UrbError`] with the JS's message, word for word.
//!
//! - [`MAX_CELLS`]: 200,000 cells in one noun, counted as a tree: a cell
//!   reached twice counts twice, as lattice's +small-noun counts. So a
//!   small noun that shared structure makes vast as a tree is refused,
//!   and no decoder walks more than this.
//! - [`MAX_DEPTH`]: 64, for $json.
//! - [`TANK_DEPTH`]: 32; a tank nested deeper prints as nothing.
//! - [`MAX_NUMBER`]: 32 bytes, for an atom read as a number.
//! - [`MAX_SEGS`] segments in a path, each at most [`MAX_KNOT`] bytes.
//! - A shape we don't expect (an atom for a cell, a cell for an atom, a
//!   tag we don't know) is refused.
//!
//! Nothing here panics, on any noun. The order every decoder reads a noun
//! in is the JS's, so the same noun gives the same error.

use crate::UrbError;
use crate::js::{self, Json, Object};
use crate::noun::{Int, NounRef, Value, hostile};
use mesa_sys as sys;
use std::collections::{HashMap, HashSet};

pub const MAX_CELLS: usize = 200_000;
pub const MAX_DEPTH: usize = 64;
pub const TANK_DEPTH: usize = 32;
pub const MAX_NUMBER: usize = 32;
pub const MAX_SEGS: usize = 64;
pub const MAX_KNOT: usize = 256;

type R<T> = Result<T, UrbError>;

/// `n`, once it is known to have at most `cap` cells as a tree
pub fn measure(n: NounRef<'_>, cap: usize) -> R<NounRef<'_>> {
    let mut seen = 0usize;
    let mut todo = vec![n];
    while let Some(t) = todo.pop() {
        if !t.is_cell() {
            continue;
        }
        seen += 1;
        if seen > cap {
            return Err(hostile(format!("has over {cap} cells")));
        }
        todo.push(t.tail()?);
        todo.push(t.head()?);
    }
    Ok(n)
}

/// A list's items, lazily, as mesa.mjs's generator yields them: a list of
/// over [`MAX_CELLS`] is refused when the walk gets there.
pub fn list(n: NounRef<'_>) -> List<'_> {
    List {
        n,
        i: 0,
        done: false,
    }
}

pub struct List<'a> {
    n: NounRef<'a>,
    i: usize,
    done: bool,
}

impl<'a> Iterator for List<'a> {
    type Item = R<NounRef<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done || !self.n.is_cell() {
            return None;
        }
        self.i += 1;
        if self.i > MAX_CELLS {
            self.done = true;
            return Some(Err(hostile(format!("has a list over {MAX_CELLS} long"))));
        }
        let item = self.n.head().and_then(|h| Ok((h, self.n.tail()?)));
        match item {
            Ok((h, t)) => {
                self.n = t;
                Some(Ok(h))
            }
            Err(e) => {
                self.done = true;
                Some(Err(e))
            }
        }
    }
}

/// the whole list, as `[...list(n)]`
pub fn items(n: NounRef<'_>) -> R<Vec<NounRef<'_>>> {
    list(n).collect()
}

/// The nodes of a hoon map or set (a treap of [n l r]), in mesa.mjs's
/// order: a node, then its right side, then its left.
pub fn entries(n: NounRef<'_>) -> R<Vec<NounRef<'_>>> {
    let mut out = Vec::new();
    let mut stack = vec![n];
    while let Some(t) = stack.pop() {
        if t.is_cell() {
            if out.len() >= MAX_CELLS {
                return Err(hostile(format!("has a map over {MAX_CELLS} entries")));
            }
            out.push(t.head()?);
            let lr = t.tail()?;
            stack.push(lr.head()?);
            stack.push(lr.tail()?);
        }
    }
    Ok(out)
}

/// a unit's value: None for ~
pub fn unit(n: NounRef<'_>) -> Option<NounRef<'_>> {
    n.tail().ok()
}

/// an atom's text, or None for a cell
pub fn cord(n: NounRef<'_>) -> R<Option<String>> {
    if n.is_cell() {
        return Ok(None);
    }
    n.text().map(Some)
}

/// a path's segments
pub fn segments(p: NounRef<'_>) -> R<Vec<String>> {
    let mut out = Vec::new();
    for s in list(p) {
        let s = s?;
        if out.len() >= MAX_SEGS {
            return Err(hostile(format!("has a path over {MAX_SEGS} segments")));
        }
        if s.size()? > MAX_KNOT {
            return Err(hostile(format!("has a path segment over {MAX_KNOT} bytes")));
        }
        out.push(s.text()?);
    }
    Ok(out)
}

/// a path as text: /a/b/c
pub fn path_of(p: NounRef<'_>) -> R<String> {
    Ok(format!("/{}", segments(p)?.join("/")))
}

/// one segment of a path
pub fn knot(s: NounRef<'_>) -> R<String> {
    if s.size()? > MAX_KNOT {
        return Err(hostile(format!("has a path segment over {MAX_KNOT} bytes")));
    }
    s.text()
}

/// A path noun of "/a/b/c", keeping empty segments as stab does
/// (mesa.mjs's path()).
pub fn path(text: &str) -> Value<'static> {
    let t = text.strip_prefix('/').unwrap_or(text);
    if t.is_empty() {
        return Value::Null;
    }
    let mut v: Vec<Value<'static>> = t.split('/').map(Value::from).collect();
    v.push(Value::Null);
    Value::Array(v)
}

/// a $json noun as JS
pub fn json(n: NounRef<'_>) -> R<Json> {
    measure(n, MAX_CELLS)?;
    json_at(n, 0)
}

fn json_at(n: NounRef<'_>, depth: usize) -> R<Json> {
    if !n.is_cell() {
        return Ok(Json::Null);
    }
    if depth >= MAX_DEPTH {
        return Err(hostile(format!("has $json nested over {MAX_DEPTH} deep")));
    }
    let tag = n.head()?.text()?;
    let p = n.tail()?;
    Ok(match tag.as_str() {
        "o" => {
            let mut o = Object::default();
            for kv in entries(p)? {
                let k = kv.head()?.text()?;
                let v = json_at(kv.tail()?, depth + 1)?;
                o.set(k, v);
            }
            Json::Obj(o)
        }
        "a" => Json::Arr(
            items(p)?
                .into_iter()
                .map(|v| json_at(v, depth + 1))
                .collect::<R<_>>()?,
        ),
        "b" => Json::Bool(p.int(MAX_NUMBER)?.is_zero()),
        "n" => Json::Num(js::number(&p.text()?)),
        "s" => Json::Str(p.text()?),
        _ => return Err(hostile("has $json of no kind we know")),
    })
}

/// the text of a tape (a list of bytes), as +ram has it; a char that is
/// a cell or over 32 bytes is a ?
fn tape(t: NounRef<'_>) -> R<String> {
    let mut b = Vec::new();
    for c in items(t)? {
        b.push(if c.is_cell() || c.size()? > MAX_NUMBER {
            b'?'
        } else {
            c.int(MAX_NUMBER)?.le().first().copied().unwrap_or(0)
        });
    }
    Ok(js::utf8(&b))
}

/// the text of a tank, as +ram flattens it; '' past [`TANK_DEPTH`]
pub fn tank_text(t: NounRef<'_>, depth: usize) -> R<String> {
    if depth > TANK_DEPTH || !t.is_cell() || t.head()?.is_cell() {
        return Ok(String::new());
    }
    let tag = t.head()?.text()?;
    if tag == "leaf" {
        return tape(t.tail()?);
    }
    if tag != "rose" && tag != "palm" {
        return Ok(String::new());
    }
    let pre = t.tail()?.head()?;
    let (sep, opn, cls) = if tag == "rose" {
        let sep = tape(pre.head()?)?;
        let opn = tape(pre.tail()?.head()?)?;
        (sep, opn, tape(pre.tail()?.tail()?)?)
    } else {
        let sep = tape(pre.head()?)?;
        let mut opn = tape(pre.tail()?.head()?)?;
        opn.push_str(&tape(pre.tail()?.tail()?.head()?)?);
        (sep, opn, tape(pre.tail()?.tail()?.tail()?)?)
    };
    let kids = items(t.tail()?.tail()?)?
        .into_iter()
        .map(|k| tank_text(k, depth + 1))
        .collect::<R<Vec<_>>>()?;
    Ok(format!("{opn}{}{cls}", kids.join(&sep)))
}

/// the lines of a tang. It measures nothing, so its callers measure the
/// noun it is in first, as mesa.mjs's on_poke does.
pub fn tang_text(t: NounRef<'_>) -> R<String> {
    Ok(items(t)?
        .into_iter()
        .map(|k| tank_text(k, 0))
        .collect::<R<Vec<_>>>()?
        .join("\n  "))
}

/// the lines of an error noun [tag tang] (a nack's naxplanation), or
/// what was wrong with it
pub fn tang(err: NounRef<'_>) -> String {
    let lines = || -> R<String> {
        measure(err, MAX_CELLS)?;
        if !err.is_cell() {
            return Ok(String::new());
        }
        let head = err.head()?;
        let mut out = vec![if head.is_cell() {
            String::new()
        } else {
            head.text()?
        }];
        for t in items(err.tail()?)? {
            out.push(tank_text(t, 0)?);
        }
        Ok(out.join("\n  "))
    };
    lines().unwrap_or_else(|e| e.message)
}

/// [%firm ~ jobe] or [%temp ~ jobe]: the jobe a %snap's pace names
fn jobe_of<'a>(snap: NounRef<'a>) -> R<Option<NounRef<'a>>> {
    let pace = snap.tail()?.head()?;
    let tag = pace.head()?.text()?;
    Ok(if tag == "firm" || tag == "temp" {
        unit(pace.tail()?)
    } else {
        None
    })
}

/// the first entry of a map whose key is `want`, as mesa.mjs's
/// entries(m).find(e => e.head.int() === want)
fn find<'a>(m: NounRef<'a>, want: &Int) -> R<Option<NounRef<'a>>> {
    for e in entries(m)? {
        if e.head()?.int(MAX_NUMBER)? == *want {
            return Ok(Some(e));
        }
    }
    Ok(None)
}

/// The file a %snap's pace names, out of its inline or %data silo, as
/// `conv` reads it; None if the silo lacks it.
pub fn file_of<'a, T>(
    snap: NounRef<'a>,
    silo: NounRef<'a>,
    conv: impl FnOnce(NounRef<'a>) -> T,
) -> R<Option<T>> {
    let Some(jobe) = jobe_of(snap)? else {
        return Ok(None);
    };
    let want = jobe.int(MAX_NUMBER)?;
    let Some(ject) = find(silo.tail()?, &want)? else {
        return Ok(None);
    };
    let ject = ject.tail()?.tail()?;
    if ject.head()?.text()? != "leaf" {
        return Ok(None);
    }
    let key = ject.tail()?.head()?.int(MAX_NUMBER)?;
    let Some(val) = find(silo.head()?, &key)? else {
        return Ok(None);
    };
    Ok(Some(conv(val.tail()?.tail()?)))
}

/// A directory's files, as [(path, noun)], each path relative to it,
/// out of its %snap: mesa.mjs's ballOf. A tree that holds itself, one
/// deeper than [`MAX_SEGS`], or a walk past [`MAX_CELLS`] trees and
/// files, is refused. None if the silo lacks the tree.
pub fn ball_of<'a, T>(
    snap: NounRef<'a>,
    silo: NounRef<'a>,
    conv: impl FnOnce(Vec<(Vec<String>, NounRef<'a>)>) -> T,
) -> R<Option<T>> {
    let Some(top) = jobe_of(snap)? else {
        return Ok(None);
    };
    let mut ball = Ball {
        jects: HashMap::new(),
        nouns: HashMap::new(),
        out: Vec::new(),
        above: HashSet::new(),
        work: 0,
    };
    for e in entries(silo.tail()?)? {
        let k = e.head()?.int(MAX_NUMBER)?;
        ball.jects.insert(k, e.tail()?.tail()?);
    }
    for e in entries(silo.head()?)? {
        let k = e.head()?.int(MAX_NUMBER)?;
        ball.nouns.insert(k, e.tail()?.tail()?);
    }
    if !ball.walk(top.int(MAX_NUMBER)?, &[])? {
        return Ok(None);
    }
    Ok(Some(conv(ball.out)))
}

struct Ball<'a> {
    jects: HashMap<Int, NounRef<'a>>,
    nouns: HashMap<Int, NounRef<'a>>,
    out: Vec<(Vec<String>, NounRef<'a>)>,
    above: HashSet<Int>,
    work: usize,
}

impl<'a> Ball<'a> {
    fn count(&mut self) -> R<()> {
        self.work += 1;
        if self.work > MAX_CELLS {
            return Err(hostile(format!(
                "has a directory of over {MAX_CELLS} trees and files"
            )));
        }
        Ok(())
    }

    fn walk(&mut self, jobe: Int, at: &[String]) -> R<bool> {
        let Some(&ject) = self.jects.get(&jobe) else {
            return Ok(false);
        };
        if ject.head()?.text()? != "tree" {
            return Ok(false);
        }
        if self.above.contains(&jobe) {
            return Err(hostile("has a directory that holds itself"));
        }
        if at.len() > MAX_SEGS {
            return Err(hostile(format!("has a directory over {MAX_SEGS} deep")));
        }
        self.count()?;
        self.above.insert(jobe.clone());
        let t4 = ject.tail()?.tail()?.tail()?.tail()?;
        let (fil, dir) = (t4.head()?, t4.tail()?);
        for e in entries(fil)? {
            self.count()?;
            let Some(&leaf) = self.jects.get(&e.tail()?.int(MAX_NUMBER)?) else {
                continue;
            };
            if leaf.head()?.text()? != "leaf" {
                continue;
            }
            let k = leaf.tail()?.head()?.int(MAX_NUMBER)?;
            if let Some(&val) = self.nouns.get(&k) {
                let mut p = at.to_vec();
                p.push(knot(e.head()?)?);
                self.out.push((p, val));
            }
        }
        for e in entries(dir)? {
            let sub = e.tail()?.head()?.int(MAX_NUMBER)?;
            let mut p = at.to_vec();
            p.push(knot(e.head()?)?);
            self.walk(sub, &p)?;
        }
        self.above.remove(&jobe);
        Ok(true)
    }
}

/// a ship's name, from its atom
pub fn ship(n: NounRef<'_>) -> R<String> {
    if n.size()? > 16 {
        return Err(hostile("has a ship over 128 bits"));
    }
    let mut b = [0u8; 16];
    let v = n.bytes()?;
    b[..v.len()].copy_from_slice(&v);
    let w = u128::from_le_bytes(b);
    Ok(ship_name(w as u64, (w >> 64) as u64))
}

/// a ship's @p, from its two halves
pub fn ship_name(lo: u64, hi: u64) -> String {
    unsafe {
        let p = sys::mesa_ship_render(sys::mesa_ship { lo, hi });
        let s = std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned();
        sys::mesa_free(p.cast());
        s
    }
}
