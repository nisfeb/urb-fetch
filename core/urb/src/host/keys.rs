//! The native key cache: a folder with a file per ship, each one
//! [`crate::keys::encode`]d line, shared by every identity that runs
//! from the same state folder (a point is public). A file is written
//! whole to a temporary name and renamed into place, so two processes
//! that write at once leave one whole entry, never a torn one.

use crate::engine::{Ship, ship_text};
use crate::keys::{Entry, KeyStore, decode, encode};
use std::fs;
use std::path::PathBuf;

pub struct FileKeys {
    dir: PathBuf,
}

impl FileKeys {
    /// the cache in dir, made once there is something to keep
    pub fn new(dir: impl Into<PathBuf>) -> FileKeys {
        FileKeys { dir: dir.into() }
    }

    fn path(&self, who: Ship) -> PathBuf {
        self.dir.join(ship_text(who).trim_start_matches('~'))
    }
}

impl KeyStore for FileKeys {
    fn get(&self, who: Ship) -> Option<Entry> {
        let text = fs::read_to_string(self.path(who)).ok()?;
        let (w, e) = decode(text.lines().next()?)?;
        (w == who).then_some(e)
    }

    fn put(&self, who: Ship, e: &Entry) {
        let _ = fs::create_dir_all(&self.dir);
        let path = self.path(who);
        let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
        if fs::write(&tmp, encode(who, e)).is_ok() && fs::rename(&tmp, &path).is_err() {
            let _ = fs::remove_file(&tmp);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Point, ship_parse};

    #[test]
    fn files_keep_entries() {
        let dir = std::env::temp_dir().join(format!("urb-keys-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let k = FileKeys::new(&dir);
        let who = ship_parse("~nisfeb").unwrap();
        assert_eq!(k.get(who), None);
        let mut pass = [3u8; 65];
        pass[0] = b'b';
        let e = Entry {
            at: 42,
            point: Point {
                rift: 1,
                life: 4,
                pass,
                sponsor: ship_parse("~feb"),
            },
        };
        k.put(who, &e);
        assert_eq!(FileKeys::new(&dir).get(who), Some(e));
        // a file under another ship's name is not this ship's
        fs::copy(dir.join("nisfeb"), dir.join("zod")).unwrap();
        assert_eq!(k.get((0, 0)), None);
        let _ = fs::remove_dir_all(&dir);
    }
}
