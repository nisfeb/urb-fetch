//! What furum.mjs and forum.mjs read out of another ship's grubs:
//! furum's boards ([`furum`]) and lattice's forum boards ([`board`]),
//! and the notes each pokes back. The JS is the specification, its
//! errors word for word and its order of reading too, so a noun of the
//! wrong shape fails with the same message.
//!
//! The host is another ship, so what it sends is hostile. Each decoder
//! takes a [`Measured`] noun (a peek's answer is measured whole, a
//! directory's files each from the peek's budget), and reads only that.

use crate::codec::{self, MAX_NUMBER, Measured};
use crate::js::{Js, MAX_MS, Object, int_number};
use crate::noun::NounRef;
use crate::url::{APP_BASE, DESK_BASE};
use crate::{Int, UrbError};
use std::collections::HashMap;
use std::hash::Hash;

type R<T> = Result<T, UrbError>;

const DA_UNIX_EPOCH: u128 = 0x8000000cce9e0d80;

/// A @da as ms since 1970, or None for one past where a Date reaches
/// (`new Date(...)` of it is invalid). furum.mjs and forum.mjs agree on
/// every @da: each is ((d >> 64) - epoch) * 1000 plus the ms of its
/// fraction, and furum's floats are exact wherever a Date is valid.
pub fn date(d: &Int) -> Option<i64> {
    let b = d.le();
    let mut lo = [0u8; 8];
    let n = b.len().min(8);
    lo[..n].copy_from_slice(&b[..n]);
    let hi = b.get(8..).unwrap_or_default();
    if hi.len() > 12 {
        return None;
    }
    let mut w = [0u8; 16];
    w[..hi.len()].copy_from_slice(hi);
    let secs = u128::from_le_bytes(w) as i128 - DA_UNIX_EPOCH as i128;
    let frac = (u128::from(u64::from_le_bytes(lo)) * 1000) >> 64;
    let ms = secs.checked_mul(1000)? + frac as i128;
    (ms.abs() <= i128::from(MAX_MS)).then_some(ms as i64)
}

/// an ordered JS Map: a key set again keeps its place
pub(crate) struct JsMap<K, V> {
    items: Vec<(K, V)>,
    at: HashMap<K, usize>,
}

impl<K, V> Default for JsMap<K, V> {
    fn default() -> Self {
        JsMap {
            items: Vec::new(),
            at: HashMap::new(),
        }
    }
}

impl<K: Hash + Eq + Clone, V> JsMap<K, V> {
    fn set(&mut self, k: K, v: V) {
        match self.at.get(&k) {
            Some(&i) => self.items[i].1 = v,
            None => {
                self.at.insert(k.clone(), self.items.len());
                self.items.push((k, v));
            }
        }
    }

    fn get(&self, k: &K) -> Option<&V> {
        self.at.get(k).map(|&i| &self.items[i].1)
    }
}

// ------------------------------------------------------------ furum

/// furum's boards (furum.mjs, furum release 7)
pub mod furum {
    use super::*;

    /// the most posts a board may claim: past it, posts() refuses it
    pub const MAX_POSTS: f64 = 100_000.0;
    pub const BASE: &str = "/apps/shell.shell/desks/furum.desk/desk/data/furum.furum_app";
    /// the mark of what goes to and comes from a board's inbox.sig
    pub const MSG: (&str, &str) = ("/furum", "msg");

    /// a post-core, as furum.mjs gives it
    #[derive(Clone, Debug, PartialEq)]
    pub struct Post {
        pub id: f64,
        pub author: String,
        pub title: String,
        pub url: Option<String>,
        pub body: Option<String>,
        pub created: Option<i64>,
    }

    impl Post {
        pub fn js(&self) -> Js {
            Js::obj([
                ("id", Js::Num(self.id)),
                ("author", Js::Str(self.author.clone())),
                ("title", Js::Str(self.title.clone())),
                ("url", Js::text(self.url.clone())),
                ("body", Js::text(self.body.clone())),
                ("created", Js::Date(self.created)),
            ])
        }
    }

    /// a comment-core
    #[derive(Clone, Debug, PartialEq)]
    pub struct Comment {
        pub id: f64,
        pub parent: Option<f64>,
        pub author: String,
        pub body: String,
        pub created: Option<i64>,
    }

    impl Comment {
        pub fn js(&self) -> Js {
            Js::obj([
                ("id", Js::Num(self.id)),
                ("parent", self.parent.map_or(Js::Null, Js::Num)),
                ("author", Js::Str(self.author.clone())),
                ("body", Js::Str(self.body.clone())),
                ("created", Js::Date(self.created)),
            ])
        }
    }

    /// a post's votes: the ships for, and against
    pub fn votes_js(up: &[String], down: &[String]) -> Js {
        let ships = |s: &[String]| Js::Arr(s.iter().cloned().map(Js::Str).collect());
        Js::obj([("up", ships(up)), ("down", ships(down))])
    }

    fn number(n: NounRef<'_>) -> R<f64> {
        Ok(int_number(&n.int(MAX_NUMBER)?))
    }

    fn text(n: Option<NounRef<'_>>) -> R<Option<String>> {
        n.map(|n| n.text()).transpose()
    }

    fn at_date(n: NounRef<'_>) -> R<Option<i64>> {
        Ok(date(&n.int(MAX_NUMBER)?))
    }

    fn post(p: NounRef<'_>) -> R<Post> {
        let id = p.head()?;
        let t = p.tail()?;
        let author = t.head()?;
        let t = t.tail()?;
        let title = t.head()?;
        let t = t.tail()?;
        let url = t.head()?;
        let t = t.tail()?;
        let body = t.head()?;
        let created = t.tail()?;
        Ok(Post {
            id: number(id)?,
            author: codec::ship(author)?,
            title: title.text()?,
            url: text(codec::unit(url))?,
            body: text(codec::unit(body))?,
            created: at_date(created)?,
        })
    }

    fn comment(c: NounRef<'_>) -> R<Comment> {
        let id = c.head()?;
        let t = c.tail()?;
        let parent = t.head()?;
        let t = t.tail()?;
        let author = t.head()?;
        let t = t.tail()?;
        let body = t.head()?;
        let created = t.tail()?;
        let up = codec::unit(parent);
        Ok(Comment {
            id: number(id)?,
            parent: up.map(number).transpose()?,
            author: codec::ship(author)?,
            body: body.text()?,
            created: at_date(created)?,
        })
    }

    /// pub/conf's next-post: the id the next post gets, or None
    pub fn next_post(n: Measured<'_>) -> R<Option<f64>> {
        let n = n.noun();
        if !n.is_cell() {
            return Ok(None);
        }
        number(n.tail()?.head()?).map(Some)
    }

    /// a posts bucket's posts, in its map's order
    pub fn posts(n: Measured<'_>) -> R<Vec<Post>> {
        codec::entries(n.noun().tail()?)?
            .into_iter()
            .map(|e| post(e.tail()?))
            .collect()
    }

    /// the entry for id in a bucket of [%1 (map @ud value)]
    fn in_bucket(n: NounRef<'_>, id: f64) -> R<Option<NounRef<'_>>> {
        for e in codec::entries(n.tail()?)? {
            if number(e.head()?)? == id {
                return Ok(Some(e.tail()?));
            }
        }
        Ok(None)
    }

    /// a threads bucket's comments on post id, None if it has none
    pub fn comments(n: Measured<'_>, id: f64) -> R<Option<Vec<Comment>>> {
        let Some(t) = in_bucket(n.noun(), id)? else {
            return Ok(None);
        };
        let got: R<Vec<Comment>> = codec::entries(t.tail()?)?
            .into_iter()
            .map(|e| comment(e.tail()?))
            .collect();
        got.map(Some)
    }

    /// a votes bucket's tally of post id: (up, down), None if it has none
    pub fn votes(n: Measured<'_>, id: f64) -> R<Option<(Vec<String>, Vec<String>)>> {
        for e in codec::entries(n.noun().tail()?)? {
            let k = e.head()?;
            if k.head()?.text()? == "post" && number(k.tail()?)? == id {
                let ships = |n: NounRef<'_>| -> R<Vec<String>> {
                    codec::entries(n)?.into_iter().map(codec::ship).collect()
                };
                let t = e.tail()?;
                return Ok(Some((ships(t.head()?)?, ships(t.tail()?)?)));
            }
        }
        Ok(None)
    }

    /// as posts() orders them: newest first
    pub fn sort_posts(p: &mut [Post]) {
        p.sort_by(|a, b| b.id.partial_cmp(&a.id).unwrap_or(std::cmp::Ordering::Equal));
    }

    /// as comments() orders them: oldest first
    pub fn sort_comments(c: &mut [Comment]) {
        c.sort_by(|a, b| a.id.partial_cmp(&b.id).unwrap_or(std::cmp::Ordering::Equal));
    }

    /// a note furum pokes into our inbox.sig, such as a refusal: (title,
    /// body), or None for any other poke
    pub fn note(mark: (&str, &str), noun: Measured<'_>) -> R<Option<(String, String)>> {
        let n = noun.noun();
        if mark.0 != MSG.0 || mark.1 != MSG.1 || n.head()?.text()? != "note" {
            return Ok(None);
        }
        let t = n.tail()?;
        Ok(Some((t.head()?.text()?, t.tail()?.head()?.text()?)))
    }
}

// ------------------------------------------------------------ lattice

/// lattice's forum boards, as its reader reads them (forum.mjs, lattice
/// release 81, load:fb): a board's pub/ and content/ grubs into an
/// [`Acc`], and then the board.
pub mod board {
    use super::*;

    /// +peer-bases: where a ship may run lattice
    pub const BASES: [&str; 2] = [DESK_BASE, APP_BASE];
    /// how long each base is asked for
    pub const BASE_WAIT: u64 = 8_000;
    pub const ACTION: (&str, &str) = ("/lattice", "forum-action");
    pub const NOTE: (&str, &str) = ("/lattice", "forum-note");
    const ROLES: [&str; 3] = ["mod", "poster", "reader"];

    /// (sane %tas), which a board name must be
    pub fn is_name(s: &str) -> bool {
        let b = s.as_bytes();
        b.first().is_some_and(u8::is_ascii_lowercase)
            && b.iter()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
    }

    fn bad(m: impl Into<String>) -> UrbError {
        UrbError::new(m)
    }

    fn atom(n: NounRef<'_>) -> R<NounRef<'_>> {
        if n.is_cell() {
            return Err(bad("a cell where an atom goes"));
        }
        Ok(n)
    }

    fn big(n: NounRef<'_>) -> R<Int> {
        atom(n)?.int(MAX_NUMBER)
    }

    /// an id or count, exactly: a number up to 2^53 - 1, a BigInt past it
    pub fn exact(v: &Int) -> Js {
        match v.to_u64().filter(|&n| n <= 9_007_199_254_740_991) {
            Some(n) => Js::Num(n as f64),
            None => Js::Big(v.clone()),
        }
    }

    fn text(n: NounRef<'_>) -> R<String> {
        atom(n)?.text()
    }

    fn flag(n: NounRef<'_>) -> R<bool> {
        let v = big(n)?;
        if v > Int::from(1u64) {
            return Err(bad("not a flag"));
        }
        Ok(v.is_zero())
    }

    fn unit<'a, T>(n: NounRef<'a>, f: impl FnOnce(NounRef<'a>) -> R<T>) -> R<Option<T>> {
        if !n.is_cell() {
            if !big(n)?.is_zero() {
                return Err(bad("not a unit"));
            }
            return Ok(None);
        }
        if !big(n.head()?)?.is_zero() {
            return Err(bad("not a unit"));
        }
        f(n.tail()?).map(Some)
    }

    fn role(n: NounRef<'_>) -> R<String> {
        let r = text(n)?;
        if !ROLES.contains(&r.as_str()) {
            return Err(bad(format!("no role {r}")));
        }
        Ok(r)
    }

    fn ships(n: NounRef<'_>) -> R<Vec<String>> {
        codec::entries(n)?
            .into_iter()
            .map(|s| codec::ship(atom(s)?))
            .collect()
    }

    /// the k items of an n-tuple
    fn tuple(mut n: NounRef<'_>, k: usize) -> R<Vec<NounRef<'_>>> {
        let mut out = Vec::with_capacity(k);
        for _ in 0..k - 1 {
            out.push(n.head()?);
            n = n.tail()?;
        }
        out.push(n);
        Ok(out)
    }

    /// [%1 x]
    fn versioned(n: NounRef<'_>) -> R<NounRef<'_>> {
        if big(n.head()?)? != Int::from(1u64) {
            return Err(bad("not version 1"));
        }
        n.tail()
    }

    fn at_date(n: NounRef<'_>) -> R<Option<i64>> {
        Ok(date(&big(n)?))
    }

    /// (v * 1000) >> 64: a @dr's ms
    fn ms_of(v: &Int) -> Int {
        let mut out = Vec::with_capacity(v.le().len() + 2);
        let mut carry = 0u32;
        for &b in v.le() {
            let x = u32::from(b) * 1000 + carry;
            out.push(x as u8);
            carry = x >> 8;
        }
        while carry > 0 {
            out.push(carry as u8);
            carry >>= 8;
        }
        Int::from_le(out.get(8..).unwrap_or_default())
    }

    fn map<'a, K, V>(
        n: NounRef<'a>,
        k: impl Fn(NounRef<'a>) -> R<K>,
        v: impl Fn(NounRef<'a>) -> R<V>,
    ) -> R<Vec<(K, V)>> {
        codec::entries(n)?
            .into_iter()
            .map(|e| Ok((k(e.head()?)?, v(e.tail()?)?)))
            .collect()
    }

    #[derive(Clone, Debug)]
    struct Core {
        id: Int,
        author: String,
        title: String,
        url: Option<String>,
        body: Option<String>,
        created: Option<i64>,
    }

    #[derive(Clone, Debug)]
    struct CommentCore {
        id: Int,
        parent: Option<Int>,
        author: String,
        body: String,
        created: Option<i64>,
    }

    #[derive(Clone, Debug)]
    struct Thread {
        comments: Vec<(Int, CommentCore)>,
    }

    type Tally = (Vec<String>, Vec<String>);

    fn info(n: NounRef<'_>) -> R<Js> {
        let t = tuple(n, 7)?;
        Ok(Js::obj([
            ("name", Js::Str(text(t[0])?)),
            ("title", Js::Str(text(t[1])?)),
            ("description", Js::Str(text(t[2])?)),
            ("host", Js::Str(codec::ship(atom(t[3])?)?)),
            ("created", Js::Date(at_date(t[4])?)),
            ("defaultRole", Js::Str(role(t[5])?)),
            ("public", Js::Bool(flag(t[6])?)),
        ]))
    }

    fn post_core(n: NounRef<'_>) -> R<Core> {
        let t = tuple(n, 6)?;
        Ok(Core {
            id: big(t[0])?,
            author: codec::ship(atom(t[1])?)?,
            title: text(t[2])?,
            url: unit(t[3], text)?,
            body: unit(t[4], text)?,
            created: at_date(t[5])?,
        })
    }

    fn comment_core(n: NounRef<'_>) -> R<CommentCore> {
        let t = tuple(n, 5)?;
        Ok(CommentCore {
            id: big(t[0])?,
            parent: unit(t[1], big)?,
            author: codec::ship(atom(t[2])?)?,
            body: text(t[3])?,
            created: at_date(t[4])?,
        })
    }

    fn target(n: NounRef<'_>) -> R<String> {
        let tag = text(n.head()?)?;
        match tag.as_str() {
            "post" => Ok(format!("post {}", big(n.tail()?)?.decimal())),
            "comment" => {
                let t = n.tail()?;
                let p = big(t.head()?)?.decimal();
                Ok(format!("comment {p} {}", big(t.tail()?)?.decimal()))
            }
            _ => Err(bad(format!("no vote target %{tag}"))),
        }
    }

    /// load:fb's reading of a board, a grub at a time (a grub's noun is
    /// borrowed only during its peek), then [`Acc::assemble`]
    pub struct Acc {
        host: String,
        card: Option<Js>,
        roles: Vec<(String, String)>,
        next_post: Int,
        prune: Option<(Int, Int)>,
        pins: Vec<Int>,
        side: String,
        cores: JsMap<Int, Core>,
        threads: JsMap<Int, Thread>,
        votes: JsMap<String, Tally>,
    }

    impl Acc {
        /// a board of host's, with nothing read yet
        pub fn new(host: &str) -> Acc {
            Acc {
                host: host.into(),
                card: None,
                roles: Vec::new(),
                next_post: Int::default(),
                prune: None,
                pins: Vec::new(),
                side: String::new(),
                cores: JsMap::default(),
                threads: JsMap::default(),
                votes: JsMap::default(),
            }
        }

        /// one grub, at pax under the board ([sub, ...path]); a grub it
        /// doesn't know is skipped, and one it can't read fails, named
        pub fn read(&mut self, pax: &[String], val: Measured<'_>) -> R<()> {
            let at = pax.join("/");
            self.read_at(pax, &at, val.noun()).map_err(|e| {
                bad(format!(
                    "{}'s board grub is unreadable: /{at} ({})",
                    self.host, e.message
                ))
            })
        }

        fn read_at(&mut self, pax: &[String], at: &str, val: NounRef<'_>) -> R<()> {
            match at {
                "pub/card" => self.card = Some(info(versioned(val)?)?),
                "pub/roles" => {
                    self.roles = map(versioned(val)?, |n| codec::ship(atom(n)?), role)?;
                }
                "pub/conf" => {
                    let c = versioned(val)?;
                    let next = big(c.head()?)?;
                    let prune = unit(c.tail()?, |p| {
                        Ok((big(p.head()?)?, ms_of(&big(p.tail()?)?)))
                    })?;
                    self.next_post = next;
                    self.prune = prune;
                }
                "content/pins" => {
                    self.pins = codec::entries(versioned(val)?)?
                        .into_iter()
                        .map(big)
                        .collect::<R<_>>()?;
                }
                "content/sidebar" => self.side = text(versioned(val)?)?,
                _ if pax.len() == 3 && pax[0] == "content" => {
                    let v = versioned(val)?;
                    match pax[1].as_str() {
                        "posts" => {
                            for (id, p) in map(v, big, post_core)? {
                                self.cores.set(id, p);
                            }
                        }
                        "threads" => {
                            let got = map(v, big, |t| {
                                big(t.head()?)?;
                                Ok(Thread {
                                    comments: map(t.tail()?, big, comment_core)?,
                                })
                            })?;
                            for (id, t) in got {
                                self.threads.set(id, t);
                            }
                        }
                        "votes" => {
                            for (k, t) in
                                map(v, target, |t| Ok((ships(t.head()?)?, ships(t.tail()?)?)))?
                            {
                                self.votes.set(k, t);
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
            Ok(())
        }

        /// the board, from what was read
        pub fn assemble(&self) -> R<Board> {
            let Some(card) = &self.card else {
                return Err(bad(format!("{}'s board has no card", self.host)));
            };
            let none: Tally = (Vec::new(), Vec::new());
            let tally = |k: &str| self.votes.get(&k.to_string()).unwrap_or(&none);
            let ships = |s: &[String]| Js::Arr(s.iter().cloned().map(Js::Str).collect());
            let mut posts: Vec<(Int, Js)> = self
                .cores
                .items
                .iter()
                .map(|(_, c)| {
                    let (up, down) = tally(&format!("post {}", c.id.decimal()));
                    let n = self.threads.get(&c.id).map_or(0, |t| t.comments.len());
                    let js = Js::obj([
                        ("id", exact(&c.id)),
                        ("author", Js::Str(c.author.clone())),
                        ("title", Js::Str(c.title.clone())),
                        ("url", Js::text(c.url.clone())),
                        ("body", Js::text(c.body.clone())),
                        ("created", Js::Date(c.created)),
                        ("up", ships(up)),
                        ("down", ships(down)),
                        ("comments", Js::Num(n as f64)),
                    ]);
                    (c.id.clone(), js)
                })
                .collect();
            let mut comments = Object::default();
            for (pid, t) in &self.threads.items {
                let mut cs: Vec<(Int, Js)> = t
                    .comments
                    .iter()
                    .map(|(cid, c)| {
                        let (up, down) =
                            tally(&format!("comment {} {}", pid.decimal(), cid.decimal()));
                        let js = Js::obj([
                            ("id", exact(&c.id)),
                            ("parent", c.parent.as_ref().map_or(Js::Null, exact)),
                            ("author", Js::Str(c.author.clone())),
                            ("body", Js::Str(c.body.clone())),
                            ("created", Js::Date(c.created)),
                            ("up", ships(up)),
                            ("down", ships(down)),
                        ]);
                        (c.id.clone(), js)
                    })
                    .collect();
                cs.sort_by(|a, b| a.0.cmp(&b.0));
                comments.set(
                    pid.decimal(),
                    Js::Arr(cs.into_iter().map(|(_, j)| j).collect()),
                );
            }
            let mut roles = Object::default();
            for (s, r) in &self.roles {
                roles.set(s.clone(), Js::Str(r.clone()));
            }
            let mut pins = self.pins.clone();
            pins.sort();
            posts.sort_by(|a, b| b.0.cmp(&a.0));
            Ok(Board {
                info: card.clone(),
                roles: Js::Obj(roles),
                next_post: exact(&self.next_post),
                prune: match &self.prune {
                    None => Js::Null,
                    Some((min, after)) => {
                        Js::obj([("minScore", exact(min)), ("after", exact(after))])
                    }
                },
                pinned: Js::Arr(pins.iter().map(exact).collect()),
                sidebar: self.side.clone(),
                posts,
                comments,
            })
        }
    }

    /// a board, as forum.mjs's board() gives it
    #[derive(Clone, Debug)]
    pub struct Board {
        pub info: Js,
        roles: Js,
        next_post: Js,
        prune: Js,
        pinned: Js,
        sidebar: String,
        /// newest first, each with its id
        posts: Vec<(Int, Js)>,
        comments: Object<Js>,
    }

    impl Board {
        /// {info, roles, nextPost, prune, pinned, sidebar, posts, comments}
        pub fn js(&self) -> Js {
            Js::obj([
                ("info", self.info.clone()),
                ("roles", self.roles.clone()),
                ("nextPost", self.next_post.clone()),
                ("prune", self.prune.clone()),
                ("pinned", self.pinned.clone()),
                ("sidebar", Js::Str(self.sidebar.clone())),
                (
                    "posts",
                    Js::Arr(self.posts.iter().map(|(_, p)| p.clone()).collect()),
                ),
                ("comments", Js::Obj(self.comments.clone())),
            ])
        }

        /// thread(): one post and its comments, oldest first
        pub fn thread(&self, host: &str, name: &str, post: &Int) -> R<Js> {
            let Some((_, p)) = self.posts.iter().find(|(id, _)| id == post) else {
                return Err(bad(format!(
                    "no post {} on {host}'s board {name}",
                    post.decimal()
                )));
            };
            let cs = self
                .comments
                .get(&post.decimal())
                .cloned()
                .unwrap_or(Js::Arr(Vec::new()));
            Ok(Js::obj([
                ("info", self.info.clone()),
                ("post", p.clone()),
                ("comments", cs),
            ]))
        }
    }

    /// a refusal note host pokes to our /forum.sig: (board, reason), or
    /// None for a poke that isn't one, or isn't host's
    pub fn note(
        from: &str,
        host: &str,
        to: (&str, &str),
        mark: (&str, &str),
        noun: Measured<'_>,
    ) -> R<Option<(String, String)>> {
        if from != host || to.1 != "forum.sig" || !BASES.contains(&to.0) {
            return Ok(None);
        }
        let n = noun.noun();
        if mark.0 != NOTE.0 || mark.1 != NOTE.1 || !n.is_cell() {
            return Ok(None);
        }
        let (b, r) = (n.head()?, n.tail()?);
        if b.is_cell() || r.is_cell() {
            return Ok(None);
        }
        Ok(Some((b.text()?, r.text()?)))
    }
}

#[cfg(test)]
mod tests {
    use super::board::{Acc, exact};
    use super::*;
    use crate::Noun;
    use crate::noun::Value;

    fn big(e: u32, plus: u64) -> Int {
        let mut le = vec![0u8; e as usize / 8 + 1];
        le[e as usize / 8] = 1 << (e % 8);
        let mut carry = u128::from(plus);
        for b in le.iter_mut() {
            let v = u128::from(*b) + (carry & 0xff);
            *b = v as u8;
            carry = (carry >> 8) + (v >> 8);
        }
        Int::from_le(&le)
    }

    // forum.test.mjs's 'dates past a Date are null, and ids past 2^53 are
    // exact'
    #[test]
    fn dates_past_a_date_are_null_and_ids_past_2_53_exact() {
        let t0 = 1_793_448_000_000; // Date.UTC(2026, 9, 31, 12)
        let da = Value::from(crate::text::da(t0));
        let v = |x: Vec<Value<'static>>| Value::Array(x);
        let core = |id: &Int, title: &str, at: Value<'static>| {
            v(vec![
                id.clone().into(),
                0u64.into(),
                title.into(),
                Value::Null,
                Value::Null,
                at,
            ])
        };
        let (a, b) = (big(60, 1), big(60, 0));
        let posts = v(vec![
            1u64.into(),
            Value::Map(vec![
                (a.clone().into(), core(&a, "big", big(255, 0).into())),
                (b.clone().into(), core(&b, "big too", da.clone())),
            ]),
        ]);
        let conf = v(vec![1u64.into(), big(60, 2).into(), Value::Null]);
        let card = v(vec![
            1u64.into(),
            "test".into(),
            "T".into(),
            "d".into(),
            0u64.into(),
            da,
            "poster".into(),
            0u64.into(),
        ]);
        let mut acc = Acc::new("~zod");
        for (p, val) in [
            ("pub/card", card),
            ("pub/conf", conf),
            ("content/posts/b0", posts),
        ] {
            let n = Noun::of(&val).unwrap();
            let pax: Vec<String> = p.split('/').map(String::from).collect();
            acc.read(&pax, codec::measure(n.borrow()).unwrap()).unwrap();
        }
        let board = acc.assemble().unwrap();
        let Js::Obj(o) = board.js() else { panic!() };
        assert_eq!(o.get("nextPost"), Some(&Js::Big(big(60, 2))));
        let Some(Js::Arr(ps)) = o.get("posts") else {
            panic!()
        };
        let got: Vec<(Js, Js)> = ps
            .iter()
            .map(|p| match p {
                Js::Obj(p) => (
                    p.get("id").unwrap().clone(),
                    p.get("created").unwrap().clone(),
                ),
                _ => panic!(),
            })
            .collect();
        assert_eq!(
            got,
            [
                (exact(&a), Js::Date(None)),
                (exact(&b), Js::Date(Some(t0 as i64)))
            ]
        );
        assert!(matches!(exact(&a), Js::Big(_)));
        let t = board.thread("~zod", "test", &b).unwrap();
        let Js::Obj(t) = t else { panic!() };
        let Some(Js::Obj(p)) = t.get("post") else {
            panic!()
        };
        assert_eq!(p.get("title"), Some(&Js::Str("big too".into())));
        assert_eq!(crate::alive(), 0);
        assert_eq!(
            exact(&Int::from(9_007_199_254_740_991u64)),
            Js::Num(9007199254740991.0)
        );
        assert!(matches!(
            exact(&Int::from(9_007_199_254_740_992u64)),
            Js::Big(_)
        ));
    }
}
