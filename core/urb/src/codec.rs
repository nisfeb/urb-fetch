//! The decoders of other ships' nouns, with mesa.mjs's caps, and the
//! path builder. Every noun another ship sends is hostile (rule 4 of the
//! port): each is read under these caps, and anything past one is an
//! [`UrbError`] with the JS's message, word for word.
//!
//! - [`MAX_CELLS`]: 200,000 cells in one noun, counted as a tree: a cell
//!   reached twice counts twice, as lattice's +small-noun counts. So a
//!   small noun that shared structure makes vast as a tree is refused,
//!   and no decoder walks more than this.
//! - [`MAX_BYTES`]: 64 MiB of atoms in the same, counted as a tree as
//!   cells are: one big atom a jam backrefs at each of its places counts
//!   at each. 64 MiB is the most libmesa takes in one message, so no
//!   noun a peer sends unshared is over it. A decode that reads a noun
//!   more than once (a tank's separator, printed between each two kids,
//!   or a directory's files, a tree read at each place it is named)
//!   spends what it reads, or prints, at each reading from one
//!   [`Budget`].
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
pub const MAX_BYTES: usize = 64 << 20;

type R<T> = Result<T, UrbError>;

/// What one decode may still read: cells and atom bytes, counted as a
/// tree, at each place they are read, and what it prints past them
/// (mesa.mjs's budget()).
#[derive(Clone, Debug)]
pub struct Budget {
    cells: usize,
    bytes: usize,
}

impl Default for Budget {
    fn default() -> Self {
        Budget {
            cells: MAX_CELLS,
            bytes: MAX_BYTES,
        }
    }
}

impl Budget {
    pub fn new() -> Self {
        Self::default()
    }

    fn take(&mut self, bytes: usize, over: impl FnOnce() -> String) -> R<()> {
        let Some(left) = self.bytes.checked_sub(bytes) else {
            return Err(hostile(over()));
        };
        self.bytes = left;
        Ok(())
    }

    /// atom bytes read
    fn spend(&mut self, bytes: usize) -> R<()> {
        self.take(bytes, || {
            format!("has over {} MiB of atoms", MAX_BYTES >> 20)
        })
    }

    /// output a decode makes beyond the atoms it read: a tank's
    /// separators, a directory's paths
    fn print(&mut self, bytes: usize) -> R<()> {
        self.take(bytes, || format!("reads as over {} MiB", MAX_BYTES >> 20))
    }

    fn cell(&mut self) -> R<()> {
        match self.cells.checked_sub(1) {
            Some(left) => {
                self.cells = left;
                Ok(())
            }
            None => Err(hostile(format!("has over {MAX_CELLS} cells"))),
        }
    }
}

/// A noun known to have at most [`MAX_CELLS`] cells and [`MAX_BYTES`]
/// bytes of atoms as a tree, which only [`measure`] makes. Walking it as
/// a tree is bounded, so it is what jams, hashes and compares a peer's
/// noun: [`NounRef`] can't, since libmesa's jam and eq walk a noun as a
/// tree, and a shared one takes as long as its tree is big.
#[derive(Clone, Copy, Debug)]
pub struct Measured<'a>(NounRef<'a>);

impl<'a> Measured<'a> {
    pub fn noun(self) -> NounRef<'a> {
        self.0
    }

    /// +jam, as bytes
    pub fn jam(self) -> Vec<u8> {
        self.0.jam()
    }

    /// +sham
    pub fn sham(self) -> u128 {
        self.0.sham()
    }

    /// its head: a part of a measured noun is measured
    pub fn head(self) -> R<Measured<'a>> {
        Ok(Measured(self.0.head()?))
    }

    /// its tail
    pub fn tail(self) -> R<Measured<'a>> {
        Ok(Measured(self.0.tail()?))
    }
}

impl PartialEq for Measured<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.0.eq(other.0)
    }
}

impl Eq for Measured<'_> {}

impl<'a> From<Measured<'a>> for Value<'a> {
    fn from(m: Measured<'a>) -> Self {
        Value::Noun(m.0)
    }
}

/// `n`, once it is known to be within the caps, as a fresh [`Budget`]
/// has them
pub fn measure(n: NounRef<'_>) -> R<Measured<'_>> {
    measure_in(n, &mut Budget::new())
}

/// `n`, once it is known to be within what is left of `b`, spent from it
pub fn measure_in<'a>(n: NounRef<'a>, b: &mut Budget) -> R<Measured<'a>> {
    let mut todo = vec![n];
    while let Some(t) = todo.pop() {
        if !t.is_cell() {
            b.spend(t.size()?)?;
            continue;
        }
        b.cell()?;
        todo.push(t.tail()?);
        todo.push(t.head()?);
    }
    Ok(Measured(n))
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
    measure(n)?;
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

/// The text of a tank, as +ram flattens it; '' past [`TANK_DEPTH`]. A
/// rose's separator is printed between each two kids, so a long one over
/// many kids prints far more than the noun holds: what it prints past
/// the first is spent from `b`, a byte a character.
pub fn tank_text(t: NounRef<'_>, b: &mut Budget, depth: usize) -> R<String> {
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
    let kids = items(t.tail()?.tail()?)?;
    if kids.len() > 1 {
        let chars = items(pre.head()?)?.len();
        b.print((kids.len() - 1).saturating_mul(chars))?;
    }
    let kids = kids
        .into_iter()
        .map(|k| tank_text(k, b, depth + 1))
        .collect::<R<Vec<_>>>()?;
    Ok(format!("{opn}{}{cls}", kids.join(&sep)))
}

/// the lines of a tang, out of the noun it is in, which its caller
/// measured from `b`, as mesa.mjs's on_poke measures the %pack: what it
/// prints is spent from the same budget
pub fn tang_text(t: Measured<'_>, b: &mut Budget) -> R<String> {
    Ok(items(t.noun())?
        .into_iter()
        .map(|k| tank_text(k, b, 0))
        .collect::<R<Vec<_>>>()?
        .join("\n  "))
}

/// the lines of an error noun [tag tang] (a nack's naxplanation), or
/// what was wrong with it
pub fn tang(err: NounRef<'_>) -> String {
    let lines = || -> R<String> {
        let mut b = Budget::new();
        measure_in(err, &mut b)?;
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
            out.push(tank_text(t, &mut b, 0)?);
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

/// [`file_of`] of a silo measured whole: the file is a part of it, so it
/// is measured too
pub fn file_in<'a, T>(
    snap: NounRef<'a>,
    silo: Measured<'a>,
    conv: impl FnOnce(Measured<'a>) -> T,
) -> R<Option<T>> {
    file_of(snap, silo.0, |n| conv(Measured(n)))
}

/// A directory's files, as [(path, noun)], each path relative to it,
/// out of its %snap: mesa.mjs's ballOf. A tree that holds itself, one
/// deeper than [`MAX_SEGS`], or a walk past [`MAX_CELLS`] trees and
/// their entries, files and directories, is refused. Each file's noun
/// and its path, as /-separated text, are spent from one [`Budget`] at
/// each place the file is: one noun named by many files, or one tree at
/// many places, counts at each, and so does each segment of a path, even
/// an empty one. So a ball over 64 MiB as files is refused, even if its
/// message is smaller. None if the silo lacks the tree.
pub fn ball_of<'a, T>(
    snap: NounRef<'a>,
    silo: NounRef<'a>,
    conv: impl FnOnce(Vec<(Vec<String>, Measured<'a>)>) -> T,
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
        budget: Budget::new(),
    };
    for e in entries(silo.tail()?)? {
        let k = e.head()?.int(MAX_NUMBER)?;
        ball.jects.insert(k, e.tail()?.tail()?);
    }
    for e in entries(silo.head()?)? {
        let k = e.head()?.int(MAX_NUMBER)?;
        ball.nouns.insert(k, e.tail()?.tail()?);
    }
    if !ball.walk(top.int(MAX_NUMBER)?, &mut Vec::new(), 0)? {
        return Ok(None);
    }
    Ok(Some(conv(ball.out)))
}

struct Ball<'a> {
    jects: HashMap<Int, NounRef<'a>>,
    nouns: HashMap<Int, NounRef<'a>>,
    out: Vec<(Vec<String>, Measured<'a>)>,
    above: HashSet<Int>,
    work: usize,
    budget: Budget,
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

    /// at: the path's segments, and len its bytes as text, a / for each.
    /// Each directory's
    /// name is pushed on at for its walk and popped after, so a name that
    /// is no tree costs no copy of the path.
    fn walk(&mut self, jobe: Int, at: &mut Vec<String>, len: usize) -> R<bool> {
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
            let Some(&val) = self.nouns.get(&k) else {
                continue;
            };
            let name = e.head()?;
            let seg = knot(name)?;
            self.budget.print(len + 1 + name.size()?)?;
            let val = measure_in(val, &mut self.budget)?;
            let mut p = at.to_vec();
            p.push(seg);
            self.out.push((p, val));
        }
        for e in entries(dir)? {
            self.count()?;
            let sub = e.tail()?.head()?.int(MAX_NUMBER)?;
            let name = e.head()?;
            at.push(knot(name)?);
            self.walk(sub, at, len + 1 + name.size()?)?;
            at.pop();
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
