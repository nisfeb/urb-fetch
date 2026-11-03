//! The public gateway's lists of ships and pages: the blocklist
//! (`-blocklist`), and the publishers who opted in (`-optin`). A list
//! is a file of lines, each `~ship` (every page of the ship) or
//! `~ship/path/prefix` (that page and every page under it), with `#`
//! starting a comment. It is read again whenever its modification time
//! changes, so the operator edits it without a restart. A line that is
//! not one of these is logged and skipped; a file that can't be read
//! keeps the list it had.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::SystemTime;
use urb_render::ship;

/// one entry: a ship, and a path prefix in segments (none: the whole ship)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub ship: String,
    pub prefix: Vec<String>,
}

/// a page segment as lattice takes it: `^[a-z0-9~._-]+$`
fn knot(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"~._-".contains(&b))
}

/// a list's text: its entries, and the lines skipped with why
pub fn parse(text: &str) -> (Vec<Entry>, Vec<String>) {
    let (mut out, mut bad) = (Vec::new(), Vec::new());
    for (i, line) in text.lines().enumerate() {
        let l = line.split('#').next().unwrap_or_default().trim();
        if l.is_empty() {
            continue;
        }
        let l = l.strip_prefix("urb://").unwrap_or(l);
        let l = l.trim_end_matches('/');
        let (who, rest) = match l.find('/') {
            Some(j) => (&l[..j], &l[j + 1..]),
            None => (l, ""),
        };
        let prefix: Vec<String> = if rest.is_empty() {
            Vec::new()
        } else {
            rest.split('/').map(String::from).collect()
        };
        if !who.starts_with('~') || !ship::parses(who.as_bytes()) {
            bad.push(format!("line {}: no ship", i + 1));
            continue;
        }
        if !prefix.iter().all(|s| knot(s)) {
            bad.push(format!("line {}: bad path", i + 1));
            continue;
        }
        out.push(Entry {
            ship: who.to_string(),
            prefix,
        });
    }
    (out, bad)
}

/// whether the page at path (its segments) on ship is on the list: a
/// prefix matches on whole segments only, so `~zod/a` holds `a` and
/// `a/b` but not `ab`
pub fn holds(entries: &[Entry], ship: &str, path: &[&str]) -> bool {
    entries.iter().any(|e| {
        e.ship == ship
            && e.prefix.len() <= path.len()
            && e.prefix.iter().zip(path).all(|(a, b)| a == b)
    })
}

struct Loaded {
    mtime: Option<SystemTime>,
    entries: Vec<Entry>,
    /// the file could not be read, and that was logged
    missing: bool,
}

/// A list kept from its file.
pub struct List {
    path: Option<PathBuf>,
    loaded: Mutex<Loaded>,
    log: Box<dyn Fn(&str) + Send + Sync>,
}

impl List {
    /// the list in path (none: an empty list), its problems told to log
    pub fn new(path: Option<PathBuf>, log: Box<dyn Fn(&str) + Send + Sync>) -> List {
        let l = List {
            path,
            loaded: Mutex::new(Loaded {
                mtime: None,
                entries: Vec::new(),
                missing: false,
            }),
            log,
        };
        l.refresh();
        l
    }

    /// whether the list has a file
    pub fn configured(&self) -> bool {
        self.path.is_some()
    }

    /// read the file again if it changed
    pub fn refresh(&self) {
        let Some(p) = &self.path else {
            return;
        };
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned());
        let name = name.unwrap_or_default();
        let mtime = match std::fs::metadata(p).and_then(|m| m.modified()) {
            Ok(t) => t,
            Err(e) => {
                let mut l = self.loaded.lock().unwrap_or_else(|e| e.into_inner());
                if !l.missing {
                    (self.log)(&format!("list {name}: {e}; keeping what it held"));
                    l.missing = true;
                    l.mtime = None;
                }
                return;
            }
        };
        let mut l = self.loaded.lock().unwrap_or_else(|e| e.into_inner());
        if l.mtime == Some(mtime) {
            return;
        }
        match std::fs::read_to_string(p) {
            Ok(text) => {
                let (entries, bad) = parse(&text);
                for b in bad {
                    (self.log)(&format!("list {name}: {b}, skipped"));
                }
                (self.log)(&format!("list {name}: {} entries", entries.len()));
                l.entries = entries;
                l.mtime = Some(mtime);
                l.missing = false;
            }
            Err(e) => (self.log)(&format!("list {name}: {e}; keeping what it held")),
        }
    }

    /// whether the list holds the page at path on ship (re-read first
    /// if the file changed)
    pub fn holds(&self, ship: &str, path: &[&str]) -> bool {
        self.refresh();
        let l = self.loaded.lock().unwrap_or_else(|e| e.into_inner());
        holds(&l.entries, ship, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn lines_are_ships_or_prefixes_with_comments() {
        let (e, bad) = parse(
            "# blocked\n~zod\n  ~nec/spam/  # a folder\nurb://~bud/x/y\n\nzod\n~zod/Bad\n~notaship\n",
        );
        assert_eq!(
            e,
            vec![
                Entry {
                    ship: "~zod".into(),
                    prefix: vec![]
                },
                Entry {
                    ship: "~nec".into(),
                    prefix: vec!["spam".into()]
                },
                Entry {
                    ship: "~bud".into(),
                    prefix: vec!["x".into(), "y".into()]
                },
            ]
        );
        assert_eq!(
            bad,
            vec!["line 6: no ship", "line 7: bad path", "line 8: no ship"]
        );
    }

    #[test]
    fn a_prefix_matches_on_segment_boundaries() {
        let (e, _) = parse("~nec/spam\n~zod\n");
        assert!(holds(&e, "~nec", &["spam"]));
        assert!(holds(&e, "~nec", &["spam", "a"]));
        assert!(!holds(&e, "~nec", &["spammer"]));
        assert!(!holds(&e, "~nec", &["index"]));
        assert!(holds(&e, "~zod", &["index"]));
        assert!(!holds(&e, "~bud", &["spam"]));
    }

    #[test]
    fn the_file_is_read_again_when_it_changes() {
        let dir = std::env::temp_dir().join(format!("urb-lists-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("blocklist");
        std::fs::write(&p, "~zod\n").unwrap();
        let lines = Arc::new(Mutex::new(Vec::<String>::new()));
        let l2 = lines.clone();
        let l = List::new(
            Some(p.clone()),
            Box::new(move |s| l2.lock().unwrap().push(s.into())),
        );
        assert!(l.holds("~zod", &["index"]) && !l.holds("~nec", &["index"]));
        // a new mtime, as an editor's save gives
        std::fs::write(&p, "~nec\nbad line\n").unwrap();
        let later = SystemTime::now() + std::time::Duration::from_secs(5);
        std::fs::File::options()
            .write(true)
            .open(&p)
            .unwrap()
            .set_modified(later)
            .unwrap();
        assert!(!l.holds("~zod", &["index"]) && l.holds("~nec", &["index"]));
        assert!(
            lines
                .lock()
                .unwrap()
                .iter()
                .any(|s| s == "list blocklist: line 2: no ship, skipped"),
            "{:?}",
            lines.lock().unwrap()
        );
        // gone: what it held stays
        std::fs::remove_file(&p).unwrap();
        assert!(l.holds("~nec", &["index"]));
        std::fs::remove_dir_all(&dir).unwrap();
        // no file at all: an empty list
        let none = List::new(None, Box::new(|_| {}));
        assert!(!none.configured() && !none.holds("~zod", &[]));
    }
}
