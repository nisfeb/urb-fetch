//! urb:// addresses, as lattice's +de-urb resolves them (mesa.mjs's
//! deUrb), and which renderer a page gets (renderAs, looksHtml).

use crate::{Int, UrbError};

/// mesa.mjs's DESK_BASE: where lattice runs, as grubbery names it
pub const DESK_BASE: &str = "/apps/shell.shell/desks/lattice.desk/desk/data/lattice.lattice_app";
/// the older place lattice ran, which +peer-base asks second
pub const APP_BASE: &str = "/apps/lattice.lattice_app";

/// What an url's path names, by its mount: 'n' a published page, 'k' a
/// know entry, 'p' one of lattice's live pages, 't' a file anywhere in
/// the publisher's tree, or a forum board (and a post on it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mount {
    N,
    K,
    P,
    T,
    Forum { post: Option<Int> },
}

/// An urb:// url, resolved: the ship, the path under the mount (for a
/// forum, the board), and the mount.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Url {
    pub ship: String,
    pub rel: Vec<String>,
    pub mount: Mount,
}

/// a path segment lattice takes: /^[a-z0-9~._-]+$/
fn knot(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"~._-".contains(&b))
}

/// /^~[a-z]+(-[a-z]+)*$/
fn ship_like(s: &str) -> bool {
    let Some(rest) = s.strip_prefix('~') else {
        return false;
    };
    rest.split('-')
        .all(|w| !w.is_empty() && w.bytes().all(|b| b.is_ascii_lowercase()))
}

/// an @ud as +slaw reads it, /^(0|[1-9][0-9]{0,2}(\.[0-9]{3})*)$/: its
/// value, exactly
fn ud(s: &str) -> Option<Int> {
    if s == "0" {
        return Some(Int::default());
    }
    let mut groups = s.split('.');
    let head = groups.next()?;
    let digits = |g: &str| g.bytes().all(|b| b.is_ascii_digit());
    if head.is_empty() || head.len() > 3 || !digits(head) || head.starts_with('0') {
        return None;
    }
    let mut all = head.to_string();
    for g in groups {
        if g.len() != 3 || !digits(g) {
            return None;
        }
        all.push_str(g);
    }
    Some(decimal(&all))
}

/// decimal digits as a number
pub(crate) fn decimal(dec: &str) -> Int {
    let mut le: Vec<u8> = Vec::new();
    for d in dec.bytes() {
        let mut carry = u32::from(d - b'0');
        for b in le.iter_mut() {
            let v = u32::from(*b) * 10 + carry;
            *b = v as u8;
            carry = v >> 8;
        }
        while carry > 0 {
            le.push(carry as u8);
            carry >>= 8;
        }
    }
    Int::from_le(&le)
}

/// What an urb:// url names, as mesa.mjs's deUrb resolves it, with its
/// errors, word for word.
pub fn de_urb(url: &str) -> Result<Url, UrbError> {
    let Some(rest) = url.strip_prefix("urb://") else {
        return Err(UrbError::new("not an urb:// url"));
    };
    let (who, mut tail) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i + 1..]),
        None => (rest, ""),
    };
    if let Some(t) = tail.strip_suffix('/') {
        tail = t;
    }
    if !ship_like(who) {
        return Err(UrbError::new(format!("no ship in {url}")));
    }
    let ship = who.to_string();
    if tail.is_empty() {
        return Ok(Url {
            ship,
            rel: vec!["index".into()],
            mount: Mount::N,
        });
    }
    let segs: Vec<String> = tail.split('/').map(String::from).collect();
    if !segs.iter().all(|s| knot(s)) {
        return Err(UrbError::new(format!("bad path in {url}")));
    }
    if segs[0] == "forum" && segs.len() > 1 {
        if segs.len() == 2 {
            return Ok(Url {
                ship,
                rel: vec![segs[1].clone()],
                mount: Mount::Forum { post: None },
            });
        }
        if segs.len() == 3
            && let Some(id) = ud(&segs[2])
        {
            return Ok(Url {
                ship,
                rel: vec![segs[1].clone()],
                mount: Mount::Forum { post: Some(id) },
            });
        }
    }
    if segs[0].len() > 1 {
        return Ok(Url {
            ship,
            rel: segs,
            mount: Mount::N,
        });
    }
    let mount = match segs[0].as_str() {
        "n" => Mount::N,
        "k" => Mount::K,
        "p" => Mount::P,
        "t" => Mount::T,
        m => return Err(UrbError::new(format!("/{m}/ is not a mount"))),
    };
    Ok(Url {
        ship,
        rel: segs[1..].to_vec(),
        mount,
    })
}

/// lattice's view modes: the marks a page body is published under
/// (mesa.mjs's PAGE_MARKS)
pub const PAGE_MARKS: &[&str] = &["text", "html", "gmi", "md", "js", "css", "noun"];
