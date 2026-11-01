//! The messages we send other ships' grubbery, as values for
//! [`crate::Noun::of`] (mesa.mjs's messages, cass, waveOf and snapOf).
//! The molds are grubbery's load:remo, transfer:remo and intake:remo,
//! which check.mjs clams them against. A wire given as text is a path;
//! one that came in a message we answer goes back as it came, a noun.

use crate::codec::path;
use crate::core::Failure;
use crate::js::{self, Json};
use crate::noun::{self, TypeError, Value};
use crate::{Int, UrbError};

fn v<'a>(items: Vec<Value<'a>>) -> Value<'a> {
    Value::Array(items)
}

/// %grubbery-load: a peek of [folder name], or with name None of the
/// directory folder and all under it
pub fn peek(wire: &str, folder: &str, name: Option<&str>) -> Value<'static> {
    let dest = match name {
        None => v(vec![false.into(), path(folder)]),
        Some(n) => v(vec![true.into(), path(folder), n.into()]),
    };
    v(vec![
        v(vec![path(wire), dest]),
        v(vec!["peek".into(), Value::Null, true.into()]),
    ])
}

/// %grubbery-load: [mark value] to the grub [folder name], mark a blot
/// (its path and its name)
pub fn poke<'a>(
    wire: &str,
    folder: &str,
    name: &str,
    mark: (&str, &str),
    value: Value<'a>,
) -> Value<'a> {
    v(vec![
        v(vec![
            path(wire),
            v(vec![true.into(), path(folder), name.into()]),
        ]),
        v(vec![
            "poke".into(),
            v(vec![path(mark.0), mark.1.into()]),
            value,
        ]),
    ])
}

/// %grubbery-transfer: a poke taken in
pub fn pack(wire: Value<'_>) -> Value<'_> {
    v(vec![wire, "pack".into(), Value::Null])
}

/// %grubbery-transfer: the content of a %snap that came without it
pub fn want<'a>(wire: Value<'a>, dest: Value<'a>, id: Value<'a>) -> Value<'a> {
    v(vec![wire, "want".into(), dest, id])
}

/// %grubbery-transfer: our answer to a peek, snap and data None for none
pub fn snap<'a>(
    wire: Value<'a>,
    dest: Value<'a>,
    id: Value<'a>,
    snap: Option<Value<'a>>,
    data: Option<Value<'a>>,
) -> Value<'a> {
    let some = |x: Option<Value<'a>>| x.map_or(Value::Null, |x| v(vec![Value::Null, x]));
    v(vec![wire, "snap".into(), dest, id, some(snap), some(data)])
}

/// %grubbery-intake: a keep's wave
pub fn intake<'a>(wire: Value<'a>, dest: Value<'a>, wave: Value<'a>) -> Value<'a> {
    v(vec![wire, dest, wave])
}

/// A file we serve: its mark (a blot), its value (None while there is
/// nothing there), its revision and when it was made, in ms. The rev
/// and the time are the JS values the host gave (None is undefined), so
/// a file whose rev or time isn't a whole number fails as the JS's
/// BigInt() fails.
#[derive(Clone, Debug)]
pub struct File<'a> {
    pub mark: (String, String),
    pub value: Option<Value<'a>>,
    pub rev: Option<Json>,
    pub at: Option<Json>,
}

impl File<'_> {
    /// a whole rev and time
    pub fn num(n: u64) -> Option<Json> {
        Some(Json::Num(n as f64))
    }
}

/// mesa.mjs's da() of a JS value: ms since the epoch as a @da, or what
/// BigInt() throws for one that isn't a whole number
pub fn da_of(ms: Option<&Json>) -> Result<Int, Failure> {
    let ms = js::to_number(ms);
    let secs = js::bigint_of_number((ms / 1000.0).floor())?;
    let frac = js::bigint_of_number(ms % 1000.0)?;
    let (Some(secs), Some(frac)) = (secs.to_u128(), frac.to_u128()) else {
        return Err(Failure::range_error("a date past 2^128 seconds"));
    };
    let hi = Int::from(secs + 0x8000000cce9e0d80);
    // (hi << 64) | ((frac << 64) / 1000)
    let lo = ((frac << 64) / 1000) as u64;
    let mut le = lo.to_le_bytes().to_vec();
    le.extend_from_slice(hi.le());
    Ok(Int::from_le(&le))
}

/// A file's version, as grubbery's cass:clay: [ud=rev da=at]
pub fn cass(f: &File<'_>) -> Result<Value<'static>, Failure> {
    let rev = js::bigint(f.rev.as_ref())?;
    Ok(v(vec![rev.into(), da_of(f.at.as_ref())?.into()]))
}

/// A file's wave: its folder's [fold-cass {name: cass}] with the one file
/// in it. Empty for a file that never was (no rev, as JS reads one).
pub fn wave_of(f: Option<&File<'_>>, name: &str) -> Result<Value<'static>, Failure> {
    Ok(match f {
        Some(f) if js::truthy(f.rev.as_ref()) => v(vec![
            v(vec![
                Value::Null,
                cass(f)?,
                Value::Map(vec![(name.into(), cass(f)?)]),
            ]),
            Value::Null,
        ]),
        _ => v(vec![Value::Null, Value::Null]),
    })
}

/// A file as grubbery's %snap, [snap silo]: its version, and the
/// content-addressed silo that holds it, inline.
pub fn snap_of<'a>(f: &File<'a>) -> Result<(Value<'a>, Value<'a>), Failure> {
    let value = f
        .value
        .clone()
        .ok_or_else(|| TypeError("can't make a noun of undefined".into()))?;
    let nobe = noun::sham(&value)?;
    let ject = v(vec![
        "leaf".into(),
        nobe.into(),
        v(vec![
            v(vec![path(&f.mark.0), f.mark.1.as_str().into()]),
            0u64.into(),
            Value::Null,
        ]),
        true.into(),
        Value::Null,
    ]);
    let jobe = noun::sham(&ject)?;
    let snap = v(vec![
        cass(f)?,
        v(vec!["firm".into(), Value::Null, jobe.into()]),
        v(vec![
            Value::Set(vec![jobe.into()]),
            Value::Set(vec![nobe.into()]),
        ]),
    ]);
    let silo = v(vec![
        Value::Map(vec![(nobe.into(), v(vec![0u64.into(), value]))]),
        Value::Map(vec![(jobe.into(), v(vec![0u64.into(), ject]))]),
    ]);
    Ok((snap, silo))
}

/// An id as forum.mjs's idOf is given one: a number, a BigInt (its sign
/// and magnitude) or text.
#[derive(Clone, Debug)]
pub enum Id {
    Num(f64),
    Big(bool, Int),
    Text(String),
}

/// forum.mjs's idOf: a non-negative integer under 2^256, or `not an id`
pub fn id_of(x: &Id) -> Result<Int, UrbError> {
    let v = match x {
        Id::Num(n) if n.fract() == 0.0 && n.abs() <= 9_007_199_254_740_991.0 => {
            (*n >= 0.0).then(|| Int::from(n.abs() as u64))
        }
        Id::Big(neg, i) => (!*neg || i.is_zero()).then(|| i.clone()),
        Id::Text(t)
            if t.len() <= 77
                && !t.is_empty()
                && t.bytes().all(|b| b.is_ascii_digit())
                && (t == "0" || !t.starts_with('0')) =>
        {
            Some(crate::url::decimal(t))
        }
        _ => None,
    };
    match v {
        Some(v) if v.le().len() <= 32 => Ok(v),
        _ => Err(UrbError::new(format!(
            "not an id: {}",
            match x {
                Id::Num(n) => crate::js::number_text(*n),
                Id::Big(neg, i) => format!("{}{}", if *neg { "-" } else { "" }, i.decimal()),
                Id::Text(t) => t.clone(),
            }
        ))),
    }
}

/// a forum vote's target: a post, or a comment on one
#[derive(Clone, Debug)]
pub struct Target {
    pub post: Id,
    pub comment: Option<Id>,
}

fn target_of(t: &Target) -> Result<Value<'static>, UrbError> {
    Ok(match &t.comment {
        None => v(vec!["post".into(), id_of(&t.post)?.into()]),
        Some(c) => v(vec![
            "comment".into(),
            id_of(&t.post)?.into(),
            id_of(c)?.into(),
        ]),
    })
}

/// an optional text as lattice's form sends it: empty is none
fn some(x: Option<&str>) -> Value<'static> {
    match x {
        None | Some("") => Value::Null,
        Some(s) => v(vec![Value::Null, s.into()]),
    }
}

/// The forum actions lattice's /forum-post-remote sends (forum.mjs's
/// actions, action:ft in lattice-forum-types.hoon).
pub mod forum {
    use super::*;

    pub fn new_post(
        name: &str,
        title: Option<&str>,
        url: Option<&str>,
        body: Option<&str>,
    ) -> Result<Value<'static>, UrbError> {
        let title = match title {
            None | Some("") => return Err(UrbError::new("a post needs a title")),
            Some(t) => t,
        };
        Ok(v(vec![
            "new-post".into(),
            name.into(),
            title.into(),
            some(url),
            some(body),
        ]))
    }

    pub fn new_comment(
        name: &str,
        post: &Id,
        body: Option<&str>,
        parent: Option<&Id>,
    ) -> Result<Value<'static>, UrbError> {
        let body = match body {
            None | Some("") => return Err(UrbError::new("a comment needs a body")),
            Some(b) => b,
        };
        let post = id_of(post)?;
        let parent = match parent {
            None => Value::Null,
            Some(p) => v(vec![Value::Null, id_of(p)?.into()]),
        };
        Ok(v(vec![
            "new-comment".into(),
            name.into(),
            post.into(),
            parent,
            body.into(),
        ]))
    }

    pub fn upvote(name: &str, t: &Target) -> Result<Value<'static>, UrbError> {
        Ok(v(vec!["upvote".into(), name.into(), target_of(t)?]))
    }

    pub fn downvote(name: &str, t: &Target) -> Result<Value<'static>, UrbError> {
        Ok(v(vec!["downvote".into(), name.into(), target_of(t)?]))
    }

    pub fn unvote(name: &str, t: &Target) -> Result<Value<'static>, UrbError> {
        Ok(v(vec!["remove-vote".into(), name.into(), target_of(t)?]))
    }
}
