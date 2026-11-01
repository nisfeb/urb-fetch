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
use crate::msg::da_of;
use crate::noun::Value;
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

/// `k.slice(1)`: a key without its first UTF-16 unit. Without the low
/// half of a character outside the BMP, the high half is left alone,
/// which UTF-8 (the noun's TextEncoder) writes as U+FFFD.
fn slice1(k: &str) -> String {
    let mut c = k.chars();
    match c.next() {
        None => String::new(),
        Some(first) if first.len_utf16() == 2 => format!("\u{fffd}{}", c.as_str()),
        Some(_) => c.as_str().to_string(),
    }
}

/// a saved JS value as noun() takes it: undefined throws, a number must
/// be whole, and an array or object (which pub.mjs never saves) is its
/// text
fn noun_of(j: Option<&Json>) -> Result<Value<'static>, Failure> {
    Ok(match j {
        None => return Err(Failure::type_error("can't make a noun of undefined")),
        Some(Json::Null) => Value::Null,
        Some(Json::Bool(b)) => Value::Bool(*b),
        Some(Json::Num(n)) => Value::Int(js::bigint_of_number(*n)?),
        Some(Json::Str(s)) => Value::Text(s.clone()),
        Some(j) => Value::Text(js::string(Some(j))),
    })
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
    /// the count of every page's revisions, as JS adds them, and their
    /// time the latest, as Math.max(0, ...) finds it
    fn listed(&self) -> (Option<Json>, Option<Json>) {
        let all = self.pages.entries();
        let rev = all.iter().fold(Json::Num(0.0), |n, (_, p)| {
            js::add(Some(&n), field(p, "rev"))
        });
        let at = all.iter().fold(0.0f64, |m, (_, p)| {
            let x = js::to_number(field(p, "at"));
            if m.is_nan() || x.is_nan() {
                f64::NAN
            } else {
                m.max(x)
            }
        });
        (Some(rev), Some(Json::Num(at)))
    }

    /// +revs-json: {"/<name>": {"rev": n, "id": "0v…"}} as a $json noun
    fn revs(&self) -> Result<Value<'static>, Failure> {
        let s = |x: &str| Value::Text(x.into());
        let mut pages = Vec::new();
        for (k, p) in self.live() {
            let rev = js::string(field(p, "rev"));
            let id = noun_of(field(p, "id"))?;
            let o = Value::Map(vec![
                (s("rev"), Value::Array(vec![s("n"), s(&rev)])),
                (s("id"), Value::Array(vec![s("s"), id])),
            ]);
            pages.push((s(k), Value::Array(vec![s("o"), o])));
        }
        Ok(Value::Array(vec![s("o"), Value::Map(pages)]))
    }

    /// +pub-index: {/pub/<name>/gmi: [updated bytes hash]}, hash (sham
    /// body)
    fn index(&self) -> Result<Value<'static>, Failure> {
        let mut out = Vec::new();
        for (k, p) in self.live() {
            let mut key: Vec<Value<'static>> = vec!["pub".into()];
            key.extend(slice1(k).split('/').map(Value::from));
            key.push("gmi".into());
            key.push(Value::Null);
            let at = da_of(field(p, "at"))?;
            // TextEncoder's encode(undefined) is of ''
            let body = field(p, "body").map_or(0, |b| js::string(Some(b)).len());
            let hash = format!("0x{}", js::string(field(p, "hash")));
            let h = js::bigint(Some(&Json::Str(hash)))?;
            out.push((
                Value::Array(key),
                Value::Array(vec![at.into(), (body as u64).into(), h.into()]),
            ));
        }
        Ok(Value::Map(out))
    }

    /// The file [folder name] we serve, as grub.serve's file() gives it:
    /// None for one we don't serve, and no value while there is nothing
    /// there.
    pub fn file(&self, folder: &str, name: &str) -> Result<Option<File<'static>>, Failure> {
        let (pub_dir, vault) = (pub_dir(), vault());
        let mark = |a: &str, b: &str| (a.to_string(), b.to_string());
        if folder == pub_dir && name == "index" {
            let value = Some(self.index()?);
            let (rev, at) = self.listed();
            return Ok(Some(File {
                mark: mark("/lattice", "pub-index"),
                value,
                rev,
                at,
            }));
        }
        if folder == pub_dir && name == "revs" {
            let value = Some(self.revs()?);
            let (rev, at) = self.listed();
            return Ok(Some(File {
                mark: mark("/", "json"),
                value,
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
        let p = match self.pages.get(&key) {
            Some(p) if js::truthy(Some(p)) => p,
            // not yet: a keep gets an empty wave, and isn't kept
            _ => {
                return Ok(Some(File {
                    mark: mark("", ""),
                    value: None,
                    rev: File::num(0),
                    at: File::num(0),
                }));
            }
        };
        let (rev, at) = (field(p, "rev").cloned(), field(p, "at").cloned());
        Ok(Some(match field(p, "body") {
            Some(Json::Null) => File {
                mark: mark("", ""),
                value: None,
                rev,
                at,
            },
            // a body of undefined is no value, as f.value !== undefined
            b => File {
                mark: mark("/lattice", "page"),
                value: b.map(|b| noun_of(Some(b))).transpose()?,
                rev,
                at,
            },
        }))
    }

    /// The body at the id the list gives, as [mark body]. Only the newest
    /// revision answers, so a reader holding an older id waits out its
    /// keen and reads the vault file instead.
    pub fn scry(&self, path: &str) -> Result<Option<Value<'static>>, Failure> {
        let Some(rest) = path.strip_prefix(PAGE).filter(|r| r.starts_with('/')) else {
            return Ok(None);
        };
        let cut = rest.rfind('/').unwrap_or(0);
        let Some(p) = self.pages.get(&rest[..cut]) else {
            return Ok(None);
        };
        let body = field(p, "body");
        if body == Some(&Json::Null) || text(field(p, "id")) != Some(&rest[cut + 1..]) {
            return Ok(None);
        }
        let mark = match field(p, "mark") {
            None | Some(Json::Null) => Value::Text("gmi".into()),
            m => noun_of(m)?,
        };
        Ok(Some(Value::Array(vec![mark, noun_of(body)?])))
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
        // (pages[key]?.rev ?? 0) + 1
        let rev = match self.pages.get(key).and_then(|p| field(p, "rev")) {
            None | Some(Json::Null) => Json::Num(1.0),
            r => js::add(r, Some(&Json::Num(1.0))),
        };
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
        o.set(
            "rev".into(),
            js::add(field(p, "rev"), Some(&Json::Num(1.0))),
        );
        o.set("id".into(), Json::Null);
        o.set("at".into(), Json::Num(at as f64));
        o.set("hash".into(), Json::Null);
        self.pages.set(key.into(), Json::Obj(o));
        true
    }
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
}
