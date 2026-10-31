//! pub.mjs: pages published from our ship, for lattice's readers on
//! other ships to read at urb://~our-ship/<name>, laid out as lattice
//! publishes its own (nex/lattice/app.hoon):
//!
//! ```text
//! <base>/pub index             grubbery peek: where we run lattice, and
//!                              the page list
//! <base>/pub revs              grubbery peek: each page's rev and id
//! <base>/pub/vault/<name> gmi  grubbery peek, the slow path; a keep
//!                              here follows the page
//! /g/x/1/grubbery//1/pub/page/<name>/<id>   remote scry: the body
//! ```
//!
//! The state is meta.pub's, as the JS saves it: `{pages: {'/<name>':
//! {body, rev, id, at, hash, mark}}, keepers: [{ship, wire, folder,
//! name}]}`, kept as the JS values it is ([`Json`]), so what was saved is
//! saved again as it was, field for field and in its order. The core
//! ([`crate::core`]) serves it and hands each save out.

use crate::core::Failure;
use crate::js::{self, Json, Object};
use crate::msg::File;
use crate::noun::Value;
use crate::text::da;
use crate::url::DESK_BASE;
use crate::{Int, url};

/// +keen-path: where a page's body is bound
pub const PAGE: &str = "/g/x/1/grubbery//1/pub/page";

/// our lattice's /pub folder
pub fn pub_dir() -> String {
    format!("{DESK_BASE}/pub")
}

/// the vault, which holds each page as %gmi
pub fn vault() -> String {
    format!("{DESK_BASE}/pub/vault")
}

/// /^[a-z0-9~._-]+$/
fn knot(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"~._-".contains(&b))
}

/// pageKey: '/<name>' for a page's name ('notes/intro', '/notes/intro/'),
/// or None. lattice reserves one-letter first segments for urb:// mounts.
pub fn page_key(name: &str) -> Option<String> {
    let t = js::trim(name).trim_start_matches('/').trim_end_matches('/');
    let segs: Vec<&str> = t.split('/').collect();
    if !segs.iter().all(|s| knot(s)) || segs[0].len() < 2 {
        return None;
    }
    Some(format!("/{}", segs.join("/")))
}

/// what one put or remove makes of a page's rev, as JS adds: put's
/// `(rev ?? 0) + 1`, and remove's `rev + 1`
fn plus_one(rev: Option<&Json>, put: bool) -> Json {
    match rev {
        None if put => Json::Num(1.0),
        None => Json::Num(f64::NAN),
        Some(Json::Null) => Json::Num(1.0),
        Some(Json::Num(n)) => Json::Num(n + 1.0),
        Some(Json::Bool(b)) => Json::Num(f64::from(u8::from(*b)) + 1.0),
        Some(j) => Json::Str(format!("{}1", js::string(Some(j)))),
    }
}

/// a JS number as a whole number of the JS's BigInt() or da()
fn whole(j: Option<&Json>) -> u64 {
    match j {
        Some(Json::Num(n)) if *n >= 0.0 => *n as u64,
        _ => 0,
    }
}

fn num(j: Option<&Json>) -> f64 {
    match j {
        Some(Json::Num(n)) => *n,
        Some(Json::Bool(b)) => f64::from(u8::from(*b)),
        Some(Json::Null) => 0.0,
        Some(Json::Str(s)) => js::number(s),
        _ => f64::NAN,
    }
}

fn field<'a>(p: &'a Json, k: &str) -> Option<&'a Json> {
    match p {
        Json::Obj(o) => o.get(k),
        _ => None,
    }
}

fn text(j: Option<&Json>) -> Option<&str> {
    match j {
        Some(Json::Str(s)) => Some(s),
        _ => None,
    }
}

/// what one publish() holds: the pages and the keeps it saves
#[derive(Clone, Debug)]
pub struct PubState {
    pub pages: Object,
    pub keepers: Json,
}

impl PubState {
    pub fn new(pages: Option<Object>, keepers: Option<Json>) -> PubState {
        PubState {
            pages: pages.unwrap_or_default(),
            keepers: keepers.unwrap_or(Json::Arr(Vec::new())),
        }
    }

    /// {pages, keepers}, what save is handed
    pub fn saved(&self) -> Json {
        let mut o = Object::default();
        o.set("pages".into(), Json::Obj(self.pages.clone()));
        o.set("keepers".into(), self.keepers.clone());
        Json::Obj(o)
    }

    /// the pages not taken down
    fn live(&self) -> Vec<(&str, &Json)> {
        self.pages
            .entries()
            .into_iter()
            .filter(|(_, p)| field(p, "body") != Some(&Json::Null))
            .collect()
    }

    /// the list and the index change with every page: their version is
    /// the count of every page's revisions
    fn listed(&self) -> (u64, u64) {
        let all = self.pages.entries();
        let rev = all.iter().fold(0.0, |n, (_, p)| n + num(field(p, "rev")));
        let at = all
            .iter()
            .fold(0.0f64, |m, (_, p)| m.max(num(field(p, "at"))));
        (rev.max(0.0) as u64, at as u64)
    }

    /// +revs-json: {"/<name>": {"rev": n, "id": "0v…"}} as a $json noun
    fn revs(&self) -> Value<'static> {
        let s = |x: &str| Value::Text(x.into());
        let pages = self
            .live()
            .into_iter()
            .map(|(k, p)| {
                let rev = js::string(field(p, "rev"));
                let id = js::string(field(p, "id"));
                let o = Value::Map(vec![
                    (s("rev"), Value::Array(vec![s("n"), s(&rev)])),
                    (s("id"), Value::Array(vec![s("s"), s(&id)])),
                ]);
                (s(k), Value::Array(vec![s("o"), o]))
            })
            .collect();
        Value::Array(vec![s("o"), Value::Map(pages)])
    }

    /// +pub-index: {/pub/<name>/gmi: [updated bytes hash]}, hash (sham
    /// body)
    fn index(&self) -> Result<Value<'static>, Failure> {
        let mut out = Vec::new();
        for (k, p) in self.live() {
            let mut key: Vec<Value<'static>> = vec!["pub".into()];
            key.extend(k[1..].split('/').map(Value::from));
            key.push("gmi".into());
            key.push(Value::Null);
            let body = js::string(field(p, "body"));
            let hash = js::string(field(p, "hash"));
            let h = hex(&hash)
                .ok_or_else(|| Failure::error(format!("Cannot convert 0x{hash} to a BigInt")))?;
            out.push((
                Value::Array(key),
                Value::Array(vec![
                    da(whole(field(p, "at"))).into(),
                    (body.len() as u64).into(),
                    h.into(),
                ]),
            ));
        }
        Ok(Value::Map(out))
    }

    /// The file [folder name] we serve, as grub.serve's file() gives it:
    /// None for one we don't serve, and no value while there is nothing
    /// there.
    pub fn file(&self, folder: &str, name: &str) -> Result<Option<File<'static>>, Failure> {
        let (pub_dir, vault) = (pub_dir(), vault());
        let (rev, at) = self.listed();
        let mark = |a: &str, b: &str| (a.to_string(), b.to_string());
        if folder == pub_dir && name == "index" {
            return Ok(Some(File {
                mark: mark("/lattice", "pub-index"),
                value: Some(self.index()?),
                rev,
                at,
            }));
        }
        if folder == pub_dir && name == "revs" {
            return Ok(Some(File {
                mark: mark("/", "json"),
                value: Some(self.revs()),
                rev,
                at,
            }));
        }
        let Some(rest) = folder.strip_prefix(&vault).filter(|r| r.starts_with('/')) else {
            return Ok(None);
        };
        if name != "gmi" {
            return Ok(None);
        }
        let Some(key) = page_key(rest) else {
            return Ok(None);
        };
        let Some(p) = self.pages.get(&key) else {
            // not yet: a keep waits for it
            return Ok(Some(File {
                mark: mark("", ""),
                value: None,
                rev: 0,
                at: 0,
            }));
        };
        let (rev, at) = (whole(field(p, "rev")), whole(field(p, "at")));
        Ok(Some(match field(p, "body") {
            Some(Json::Null) => File {
                mark: mark("", ""),
                value: None,
                rev,
                at,
            },
            b => File {
                mark: mark("/lattice", "page"),
                value: Some(Value::Text(js::string(b))),
                rev,
                at,
            },
        }))
    }

    /// The body at the id the list gives, as [mark body]. Only the newest
    /// revision answers, so a reader holding an older id waits out its
    /// keen and reads the vault file instead.
    pub fn scry(&self, path: &str) -> Option<Value<'static>> {
        let rest = path.strip_prefix(PAGE)?;
        if !rest.starts_with('/') {
            return None;
        }
        let cut = rest.rfind('/')?;
        let p = self.pages.get(&rest[..cut])?;
        let body = field(p, "body");
        if body == Some(&Json::Null) || text(field(p, "id")) != Some(&rest[cut + 1..]) {
            return None;
        }
        let mark = match field(p, "mark") {
            None | Some(Json::Null) => "gmi".to_string(),
            m => js::string(m),
        };
        Some(Value::Array(vec![mark.into(), js::string(body).into()]))
    }

    /// put's record: a new revision of key, with its id, time and hash
    pub fn put(
        &mut self,
        key: &str,
        body: &str,
        mark: &str,
        id: String,
        at: u64,
        hash: String,
    ) -> Json {
        let rev = plus_one(self.pages.get(key).and_then(|p| field(p, "rev")), true);
        let mut o = Object::default();
        o.set("body".into(), Json::Str(body.into()));
        o.set("rev".into(), rev);
        o.set("id".into(), Json::Str(id));
        o.set("at".into(), Json::Num(at as f64));
        o.set("hash".into(), Json::Str(hash));
        o.set("mark".into(), Json::Str(mark.into()));
        self.pages.set(key.into(), Json::Obj(o.clone()));
        Json::Obj(o)
    }

    /// remove: false if there is nothing up at key
    pub fn remove(&mut self, key: &str, at: u64) -> bool {
        let Some(p) = self.pages.get(key) else {
            return false;
        };
        if !js::truthy(Some(p)) || field(p, "body") == Some(&Json::Null) {
            return false;
        }
        let mut o = Object::default();
        o.set("body".into(), Json::Null);
        o.set("rev".into(), plus_one(field(p, "rev"), false));
        o.set("id".into(), Json::Null);
        o.set("at".into(), Json::Num(at as f64));
        o.set("hash".into(), Json::Null);
        self.pages.set(key.into(), Json::Obj(o));
        true
    }
}

/// BigInt('0x' + h)
fn hex(h: &str) -> Option<Int> {
    if h.is_empty() || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let digits: Vec<u8> = h
        .bytes()
        .rev()
        .map(|b| (b as char).to_digit(16).unwrap_or(0) as u8)
        .collect();
    let le: Vec<u8> = digits
        .chunks(2)
        .map(|c| c[0] | c.get(1).map_or(0, |d| d << 4))
        .collect();
    Some(Int::from_le(&le))
}

/// a page's id: two 64-bit words of entropy, each little-endian, as a
/// BigUint64Array of crypto.getRandomValues would be, hi first
pub fn page_id(e: &[u8]) -> String {
    let mut le = e[8..16].to_vec();
    le.extend_from_slice(&e[..8]);
    crate::text::uv(&Int::from_le(&le))
}

/// whether lattice publishes pages under this mark
pub fn page_mark(m: &str) -> bool {
    url::PAGE_MARKS.contains(&m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_keys_as_pub_mjs_makes_them() {
        for (n, k) in [
            ("notes/intro", Some("/notes/intro")),
            ("/notes/intro/", Some("/notes/intro")),
            ("  //a.b~c_d-e// ", Some("/a.b~c_d-e")),
            ("\u{feff}ab\u{3000}", Some("/ab")),
            ("\u{85}ab", None),
            ("n/x", None),
            ("ab//c", None),
            ("Ab", None),
            ("", None),
            ("///", None),
        ] {
            assert_eq!(page_key(n).as_deref(), k, "{n:?}");
        }
    }

    #[test]
    fn hex_reads_as_bigint() {
        assert_eq!(
            hex("8637ea05a874dc7f9a5799c3497dd554").unwrap().to_u128(),
            Some(0x8637ea05a874dc7f9a5799c3497dd554)
        );
        assert_eq!(hex("abc").unwrap().to_u128(), Some(0xabc));
        assert!(hex("").is_none() && hex("null").is_none());
    }
}
