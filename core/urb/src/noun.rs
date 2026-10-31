//! Nouns: libmesa's refcounted nouns, owned or borrowed, and how mesa.mjs
//! builds them from values.
//!
//! - A [`Noun`] owns one reference and gives it up when dropped.
//! - A [`NounRef`] is borrowed: from a [`Noun`], which it cannot outlive,
//!   or from a hook (see [`with_borrowed`]), whose call it cannot outlive.
//!   Its readers check a noun's shape before libmesa sees it, since
//!   libmesa aborts on the head of an atom or the bytes of a cell, and a
//!   noun from another ship can be any shape.
//! - [`Noun::of`] builds a noun from a [`Value`], as mesa.mjs's `noun()`
//!   does, maps and sets as `+put` builds them.
//!
//! Nouns are not thread-safe (noun.h), so neither type is `Send`. Every
//! owned noun is counted per thread, and [`alive`] says how many are: a
//! test ends with none.

use crate::UrbError;
use core::cmp::Ordering;
use core::fmt;
use core::marker::PhantomData;
use mesa_sys as sys;
use std::cell::Cell;

thread_local! {
    static ALIVE: Cell<isize> = const { Cell::new(0) };
}

/// The owned nouns this thread holds now.
pub fn alive() -> isize {
    ALIVE.with(Cell::get)
}

fn born() {
    ALIVE.with(|a| a.set(a.get() + 1));
}

fn died() {
    ALIVE.with(|a| a.set(a.get() - 1));
}

/// mesa.mjs's hostile(): an error in another ship's noun
pub(crate) fn hostile(what: impl fmt::Display) -> UrbError {
    UrbError::new(format!("another ship's noun {what}"))
}

// ------------------------------------------------------------ numbers

/// A number, as the little-endian bytes of an atom with no trailing
/// zeros: a JS BigInt, as mesa.mjs's `int()` gives it.
#[derive(Clone, Default, PartialEq, Eq, Hash)]
pub struct Int(Vec<u8>);

impl Int {
    pub fn from_le(bytes: &[u8]) -> Int {
        let mut v = bytes.to_vec();
        while v.last() == Some(&0) {
            v.pop();
        }
        Int(v)
    }

    /// its bytes, little-endian, with no trailing zeros
    pub fn le(&self) -> &[u8] {
        &self.0
    }

    pub fn is_zero(&self) -> bool {
        self.0.is_empty()
    }

    pub fn to_u128(&self) -> Option<u128> {
        (self.0.len() <= 16).then(|| {
            let mut b = [0u8; 16];
            b[..self.0.len()].copy_from_slice(&self.0);
            u128::from_le_bytes(b)
        })
    }

    pub fn to_u64(&self) -> Option<u64> {
        self.to_u128().and_then(|v| u64::try_from(v).ok())
    }

    /// in decimal, as a BigInt's toString() writes it
    pub fn decimal(&self) -> String {
        let mut n: Vec<u32> = self
            .0
            .chunks(4)
            .map(|c| {
                let mut b = [0u8; 4];
                b[..c.len()].copy_from_slice(c);
                u32::from_le_bytes(b)
            })
            .collect();
        let mut out = Vec::new();
        while n.iter().any(|&d| d != 0) {
            let mut rem = 0u64;
            for d in n.iter_mut().rev() {
                let cur = (rem << 32) | u64::from(*d);
                *d = (cur / 1_000_000_000) as u32;
                rem = cur % 1_000_000_000;
            }
            out.push(rem as u32);
        }
        match out.split_last() {
            None => "0".into(),
            Some((top, rest)) => {
                let mut s = top.to_string();
                for d in rest.iter().rev() {
                    s.push_str(&format!("{d:09}"));
                }
                s
            }
        }
    }
}

impl Ord for Int {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0
            .len()
            .cmp(&other.0.len())
            .then_with(|| self.0.iter().rev().cmp(other.0.iter().rev()))
    }
}

impl PartialOrd for Int {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Debug for Int {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Int(0x")?;
        if self.0.is_empty() {
            write!(f, "0")?;
        }
        for b in self.0.iter().rev() {
            write!(f, "{b:02x}")?;
        }
        write!(f, ")")
    }
}

impl From<u64> for Int {
    fn from(v: u64) -> Int {
        Int::from_le(&v.to_le_bytes())
    }
}

impl From<u128> for Int {
    fn from(v: u128) -> Int {
        Int::from_le(&v.to_le_bytes())
    }
}

// ------------------------------------------------------------ owned

/// An owned noun: one reference, given up on drop.
pub struct Noun {
    raw: sys::mesa_noun,
    _not_send: PhantomData<*const ()>,
}

impl Noun {
    /// Takes over a reference the caller holds.
    ///
    /// # Safety
    /// `raw` must be a live noun (never `MESA_NONE`), and the caller gives
    /// up the reference it held to it.
    pub unsafe fn from_raw(raw: sys::mesa_noun) -> Noun {
        debug_assert_ne!(raw, sys::MESA_NONE);
        born();
        Noun {
            raw,
            _not_send: PhantomData,
        }
    }

    /// Gives the reference up, to libmesa, which takes it (a constructor,
    /// or an engine call that consumes its nouns).
    pub fn into_raw(self) -> sys::mesa_noun {
        let raw = self.raw;
        core::mem::forget(self);
        died();
        raw
    }

    /// The noun, borrowed: valid while this owner is.
    pub fn borrow(&self) -> NounRef<'_> {
        NounRef {
            raw: self.raw,
            _life: PhantomData,
        }
    }

    pub fn atom(bytes: &[u8]) -> Noun {
        // a valid pointer even for no bytes
        let one = [0u8; 1];
        let p = if bytes.is_empty() {
            one.as_ptr()
        } else {
            bytes.as_ptr()
        };
        unsafe { Noun::from_raw(sys::mesa_atom_bytes(bytes.len(), p)) }
    }

    pub fn cell(head: Noun, tail: Noun) -> Noun {
        unsafe { Noun::from_raw(sys::mesa_cell(head.into_raw(), tail.into_raw())) }
    }

    /// +cue: the noun a jam's bytes hold, or None if they hold none.
    pub fn cue(bytes: &[u8]) -> Option<Noun> {
        let one = [0u8; 1];
        let p = if bytes.is_empty() {
            one.as_ptr()
        } else {
            bytes.as_ptr()
        };
        let n = unsafe { sys::mesa_cue_bytes(bytes.len(), p) };
        (n != sys::MESA_NONE).then(|| unsafe { Noun::from_raw(n) })
    }

    /// A noun of a value, as mesa.mjs's noun() makes it.
    pub fn of(x: &Value<'_>) -> Result<Noun, TypeError> {
        build(x)
    }
}

impl Drop for Noun {
    fn drop(&mut self) {
        unsafe { sys::mesa_unref(self.raw) };
        died();
    }
}

impl Clone for Noun {
    fn clone(&self) -> Noun {
        self.borrow().to_noun()
    }
}

impl fmt::Debug for Noun {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.borrow().fmt(f)
    }
}

// ------------------------------------------------------------ borrowed

/// A borrowed noun: valid for `'a`, the life of its owner or its hook.
///
/// ```compile_fail,E0597
/// let r;
/// {
///     let n = urb::Noun::atom(b"x");
///     r = n.borrow();
/// }
/// r.is_cell();
/// ```
#[derive(Clone, Copy)]
pub struct NounRef<'a> {
    raw: sys::mesa_noun,
    _life: PhantomData<&'a Noun>,
}

/// Lends `f` a noun a hook was handed, which libmesa owns for the hook's
/// call only. `f` works for any lifetime, so the noun cannot escape it.
/// None for `MESA_NONE`.
///
/// The noun can be read inside, and copied out as an owned noun:
///
/// ```
/// let n = urb::Noun::atom(b"x");
/// let kept = unsafe { urb::noun::with_borrowed(n.borrow().as_raw(), |r| r.map(|r| r.to_noun())) };
/// assert!(kept == Some(n));
/// ```
///
/// but the borrow itself can't be kept:
///
/// ```compile_fail,E0521
/// let n = urb::Noun::atom(b"x");
/// let kept = unsafe { urb::noun::with_borrowed(n.borrow().as_raw(), |r| r) };
/// ```
///
/// # Safety
/// `raw` must be `MESA_NONE` or a noun that stays live while `f` runs.
pub unsafe fn with_borrowed<R>(
    raw: sys::mesa_noun,
    f: impl for<'a> FnOnce(Option<NounRef<'a>>) -> R,
) -> R {
    f((raw != sys::MESA_NONE).then_some(NounRef {
        raw,
        _life: PhantomData,
    }))
}

impl<'a> NounRef<'a> {
    pub fn as_raw(self) -> sys::mesa_noun {
        self.raw
    }

    pub fn is_cell(self) -> bool {
        unsafe { sys::mesa_is_cell(self.raw) }
    }

    fn cell(self) -> Result<sys::mesa_noun, UrbError> {
        if !self.is_cell() {
            return Err(hostile("has an atom where a cell goes"));
        }
        Ok(self.raw)
    }

    fn at(self, raw: sys::mesa_noun) -> NounRef<'a> {
        NounRef {
            raw,
            _life: PhantomData,
        }
    }

    pub fn head(self) -> Result<NounRef<'a>, UrbError> {
        let c = self.cell()?;
        Ok(self.at(unsafe { sys::mesa_head(c) }))
    }

    pub fn tail(self) -> Result<NounRef<'a>, UrbError> {
        let c = self.cell()?;
        Ok(self.at(unsafe { sys::mesa_tail(c) }))
    }

    /// an atom's length in bytes
    pub fn size(self) -> Result<usize, UrbError> {
        if self.is_cell() {
            return Err(hostile("has a cell where an atom goes"));
        }
        Ok(unsafe { sys::mesa_met3(self.raw) })
    }

    /// an atom's bytes, little-endian
    pub fn bytes(self) -> Result<Vec<u8>, UrbError> {
        let len = self.size()?;
        let mut b = vec![0u8; len];
        unsafe { sys::mesa_atom_get_bytes(self.raw, len, b.as_mut_ptr()) };
        Ok(b)
    }

    /// an atom as text, as a TextDecoder reads its bytes
    pub fn text(self) -> Result<String, UrbError> {
        Ok(crate::js::utf8(&self.bytes()?))
    }

    /// an atom as a number, of at most `max` bytes
    pub fn int(self, max: usize) -> Result<Int, UrbError> {
        if self.size()? > max {
            return Err(hostile(format!("has a number over {max} bytes")));
        }
        Ok(Int(self.bytes()?))
    }

    /// a new reference to it, which may outlive the borrow
    pub fn to_noun(self) -> Noun {
        unsafe { Noun::from_raw(sys::mesa_ref(self.raw)) }
    }

    /// +mug
    pub fn mug(self) -> u32 {
        unsafe { sys::mesa_mug(self.raw) }
    }

    /// +jam, as bytes
    pub fn jam(self) -> Vec<u8> {
        let mut len = 0usize;
        unsafe {
            let p = sys::mesa_jam_bytes(self.raw, &mut len);
            let v = core::slice::from_raw_parts(p, len).to_vec();
            sys::mesa_free(p.cast());
            v
        }
    }

    /// +sham, the 128-bit hash grubbery names content by: +shaf %mash of
    /// an atom, +shaf %sham of a cell's jam (glue.c's u_sham)
    pub fn sham(self) -> u128 {
        let (salt, msg) = if self.is_cell() {
            (b"sham", self.jam())
        } else {
            (b"mash", self.bytes().unwrap_or_default())
        };
        let one = [0u8; 1];
        let p = if msg.is_empty() {
            one.as_ptr()
        } else {
            msg.as_ptr()
        };
        let mut out = [0u8; 16];
        unsafe { sys::mesa_shaf(4, salt.as_ptr(), msg.len(), p, out.as_mut_ptr()) };
        u128::from_le_bytes(out)
    }
}

impl PartialEq<NounRef<'_>> for NounRef<'_> {
    /// noun equality, by value
    fn eq(&self, other: &NounRef<'_>) -> bool {
        unsafe { sys::mesa_eq(self.raw, other.raw) }
    }
}

impl Eq for NounRef<'_> {}

impl PartialEq for Noun {
    fn eq(&self, other: &Noun) -> bool {
        self.borrow() == other.borrow()
    }
}

impl Eq for Noun {}

impl fmt::Debug for NounRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // iteratively, as a hoon literal: a deep list is no trouble
        enum Step<'a> {
            Noun(NounRef<'a>),
            Text(&'static str),
        }
        let mut todo = vec![Step::Noun(*self)];
        while let Some(s) = todo.pop() {
            match s {
                Step::Text(t) => f.write_str(t)?,
                Step::Noun(n) if n.is_cell() => {
                    let (h, t) =
                        unsafe { (n.at(sys::mesa_head(n.raw)), n.at(sys::mesa_tail(n.raw))) };
                    f.write_str("[")?;
                    todo.extend([
                        Step::Text("]"),
                        Step::Noun(t),
                        Step::Text(" "),
                        Step::Noun(h),
                    ]);
                }
                Step::Noun(n) => {
                    let b = n.bytes().unwrap_or_default();
                    f.write_str("0x")?;
                    if b.is_empty() {
                        f.write_str("0")?;
                    }
                    for x in b.iter().rev() {
                        write!(f, "{x:02x}")?;
                    }
                }
            }
        }
        Ok(())
    }
}

// ------------------------------------------------------------ values

/// A JS value as mesa.mjs's noun() takes it: null is ~, a boolean a
/// loobean, a number or BigInt an atom, a string a cord, an array of two
/// or more right-associated cells, a Map or Set a hoon map or set, and a
/// noun itself. A Map's entries are as given: a JS Map with two keys that
/// are equal nouns (two arrays, say) has both, and so does this.
#[derive(Clone, Debug)]
pub enum Value<'a> {
    Null,
    Bool(bool),
    Int(Int),
    Text(String),
    Array(Vec<Value<'a>>),
    Map(Vec<(Value<'a>, Value<'a>)>),
    Set(Vec<Value<'a>>),
    Noun(NounRef<'a>),
}

impl From<bool> for Value<'_> {
    fn from(v: bool) -> Self {
        Value::Bool(v)
    }
}
impl From<u64> for Value<'_> {
    fn from(v: u64) -> Self {
        Value::Int(v.into())
    }
}
impl From<u128> for Value<'_> {
    fn from(v: u128) -> Self {
        Value::Int(v.into())
    }
}
impl From<Int> for Value<'_> {
    fn from(v: Int) -> Self {
        Value::Int(v)
    }
}
impl From<&str> for Value<'_> {
    fn from(v: &str) -> Self {
        Value::Text(v.into())
    }
}
impl From<String> for Value<'_> {
    fn from(v: String) -> Self {
        Value::Text(v)
    }
}
impl<'a> From<NounRef<'a>> for Value<'a> {
    fn from(v: NounRef<'a>) -> Self {
        Value::Noun(v)
    }
}
impl<'a> From<Vec<Value<'a>>> for Value<'a> {
    fn from(v: Vec<Value<'a>>) -> Self {
        Value::Array(v)
    }
}

/// noun()'s TypeError, for a value it can't make a noun of (an array of
/// fewer than two): a bug in the caller, never another ship's doing
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeError(pub String);

impl fmt::Display for TypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for TypeError {}

/// a value as JS's String() writes it, for the TypeError's text
fn js_string(x: &Value<'_>, inside: bool) -> String {
    match x {
        Value::Null if inside => String::new(),
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.decimal(),
        Value::Text(s) => s.clone(),
        Value::Array(v) => v
            .iter()
            .map(|e| js_string(e, true))
            .collect::<Vec<_>>()
            .join(","),
        Value::Map(_) => "[object Map]".into(),
        Value::Set(_) => "[object Set]".into(),
        Value::Noun(_) => "[object Object]".into(),
    }
}

fn build(x: &Value<'_>) -> Result<Noun, TypeError> {
    Ok(match x {
        Value::Null => Noun::atom(&[]),
        Value::Bool(b) => Noun::atom(if *b { &[] } else { &[1u8][..] }),
        Value::Int(i) => Noun::atom(&i.0),
        Value::Text(s) => Noun::atom(s.as_bytes()),
        Value::Noun(n) => n.to_noun(),
        Value::Array(v) if v.len() >= 2 => {
            let mut n = build(&v[v.len() - 1])?;
            for e in v[..v.len() - 1].iter().rev() {
                n = Noun::cell(build(e)?, n);
            }
            n
        }
        Value::Array(_) => {
            return Err(TypeError(format!(
                "can't make a noun of {}",
                js_string(x, false)
            )));
        }
        Value::Map(m) => treap(m.iter().map(|(k, v)| (k, Some(v))).collect())?,
        Value::Set(s) => treap(s.iter().map(|k| (k, None)).collect())?,
    })
}

/// mesa.mjs's mug of a JS number: the mug of its atom
fn mug_of_u32(m: u32) -> u32 {
    Noun::atom(&m.to_le_bytes()).borrow().mug()
}

/// +dor, depth-first order: the tiebreak when two mugs are equal
fn dor(mut a: NounRef<'_>, mut b: NounRef<'_>) -> bool {
    loop {
        if a == b {
            return true;
        }
        if a.is_cell() {
            if !b.is_cell() {
                return false;
            }
            let (ah, bh) = unsafe { (a.at(sys::mesa_head(a.raw)), b.at(sys::mesa_head(b.raw))) };
            if ah == bh {
                unsafe {
                    a = a.at(sys::mesa_tail(a.raw));
                    b = b.at(sys::mesa_tail(b.raw));
                }
            } else {
                a = ah;
                b = bh;
            }
            continue;
        }
        if b.is_cell() {
            return true;
        }
        let (x, y) = (
            Int(a.bytes().unwrap_or_default()),
            Int(b.bytes().unwrap_or_default()),
        );
        return x < y;
    }
}

struct Item<'v, 'a> {
    key: Option<Noun>,
    val: Option<&'v Value<'a>>,
    m: u32,
    mm: u32,
}

/// A hoon map (with values) or set as +put:by and +put:in build it: a
/// treap, keys in +gor order (by mug) and the +mor-first key (by the mug
/// of the mug) on top (mesa.mjs's treap()). O(n^2) in the keys, as the
/// JS is.
fn treap(entries: Vec<(&Value<'_>, Option<&Value<'_>>)>) -> Result<Noun, TypeError> {
    let mut items = Vec::with_capacity(entries.len());
    for (k, val) in entries {
        let key = build(k)?;
        let m = key.borrow().mug();
        items.push(Item {
            key: Some(key),
            val,
            m,
            mm: mug_of_u32(m),
        });
    }
    let all: Vec<usize> = (0..items.len()).collect();
    tree(&mut items, &all)
}

fn before(items: &[Item<'_, '_>], a: usize, b: usize, mm: bool) -> bool {
    let (x, y) = (&items[a], &items[b]);
    let (p, q) = if mm { (x.mm, y.mm) } else { (x.m, y.m) };
    if p != q {
        return p < q;
    }
    match (&x.key, &y.key) {
        (Some(a), Some(b)) => dor(a.borrow(), b.borrow()),
        _ => unreachable!("a key in the list is not yet placed"),
    }
}

fn tree(items: &mut [Item<'_, '_>], list: &[usize]) -> Result<Noun, TypeError> {
    let Some(&first) = list.first() else {
        return Ok(Noun::atom(&[]));
    };
    let mut top = first;
    for &e in &list[1..] {
        if before(items, e, top, true) {
            top = e;
        }
    }
    let rest: Vec<usize> = list.iter().copied().filter(|&e| e != top).collect();
    let (left, right): (Vec<usize>, Vec<usize>) = rest
        .into_iter()
        .partition(|&e| before(items, e, top, false));
    let l = tree(items, &left)?;
    let r = tree(items, &right)?;
    let key = items[top].key.take().expect("each key is placed once");
    let node = match items[top].val {
        Some(v) => Noun::cell(key, build(v)?),
        None => key,
    };
    Ok(Noun::cell(node, Noun::cell(l, r)))
}

/// +sham of a value, as mesa.mjs's nouns.sham
pub fn sham(x: &Value<'_>) -> Result<u128, TypeError> {
    Ok(Noun::of(x)?.borrow().sham())
}

/// +mug of a value, as mesa.mjs's nouns.mug
pub fn mug(x: &Value<'_>) -> Result<u32, TypeError> {
    Ok(Noun::of(x)?.borrow().mug())
}
