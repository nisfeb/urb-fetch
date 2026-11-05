//! The native key cache: a folder with a file per ship, each one
//! [`crate::keys::encode`]d line, shared by every identity that runs
//! from the same state folder (a point is public). A file is written
//! whole to a temporary name and renamed into place, so two processes
//! that write at once leave one whole entry, never a torn one.

use crate::engine::{Ship, ship_text};
use crate::keys::{Entry, KeyStore, decode, encode};
use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

/// the longest file read: an entry is one line of under 300 bytes
const FILE_MAX: u64 = 1024;

/// each temporary file's own number in this process, so two engines in
/// one process never write the same one
static TMP: AtomicU64 = AtomicU64::new(0);

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
    /// a file that is missing, too long, torn or not this ship's reads
    /// as none, and the roller is asked
    fn get(&self, who: Ship) -> Option<Entry> {
        let mut text = String::new();
        fs::File::open(self.path(who))
            .ok()?
            .take(FILE_MAX + 1)
            .read_to_string(&mut text)
            .ok()?;
        if text.len() as u64 > FILE_MAX || !text.ends_with('\n') || text.lines().count() != 1 {
            return None;
        }
        let (w, e) = decode(text.lines().next()?)?;
        (w == who).then_some(e)
    }

    fn put(&self, who: Ship, e: &Entry) {
        let mut b = fs::DirBuilder::new();
        b.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
        if b.create(&self.dir).is_err() {
            return;
        }
        let path = self.path(who);
        let tmp = self.dir.join(format!(
            ".{}.{}.{}.tmp",
            ship_text(who).trim_start_matches('~'),
            std::process::id(),
            TMP.fetch_add(1, Ordering::Relaxed)
        ));
        let mut o = fs::OpenOptions::new();
        o.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
        let wrote = o.open(&tmp).and_then(|mut f| {
            f.write_all(encode(who, e).as_bytes())
                .and_then(|_| f.sync_all())
        });
        if wrote.is_err() || fs::rename(&tmp, &path).is_err() {
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
        assert_eq!(FileKeys::new(&dir).get(who), Some(e.clone()));
        // a file under another ship's name is not this ship's
        fs::copy(dir.join("nisfeb"), dir.join("zod")).unwrap();
        assert_eq!(k.get((0, 0)), None);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let m = fs::metadata(&dir).unwrap().permissions().mode();
            assert_eq!(m & 0o777, 0o700, "{m:o}");
        }
        // torn, truncated, doubled or long files read as none
        let line = encode(who, &e);
        for bad in [
            line[..line.len() / 2].to_string(),
            line.trim_end().to_string(),
            line.repeat(2),
            format!("{}{}", line.trim_end(), " ".repeat(2000)) + "\n",
            String::from_utf8_lossy(&[0xff; 40]).into_owned(),
        ] {
            fs::write(dir.join("nisfeb"), &bad).unwrap();
            assert_eq!(k.get(who), None, "{bad:?}");
        }
        // no temporary files left behind
        k.put(who, &e);
        let n = fs::read_dir(&dir).unwrap().count();
        assert_eq!(n, 2, "nisfeb and zod only");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn writers_at_once_leave_whole_entries() {
        let dir = std::env::temp_dir().join(format!("urb-keys-race-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let who = ship_parse("~nisfeb").unwrap();
        let entry = |life: u64| {
            let mut pass = [life as u8; 65];
            pass[0] = b'b';
            Entry {
                at: life,
                point: Point {
                    rift: 1,
                    life,
                    pass,
                    sponsor: ship_parse("~feb"),
                },
            }
        };
        let hs: Vec<_> = (1..=8u64)
            .map(|t| {
                let dir = dir.clone();
                std::thread::spawn(move || {
                    let k = FileKeys::new(dir);
                    for i in 0..200 {
                        k.put(who, &entry(t * 1000 + i));
                        if let Some(e) = k.get(who) {
                            assert_eq!(e, entry(e.point.life));
                        }
                    }
                })
            })
            .collect();
        for h in hs {
            h.join().unwrap();
        }
        let e = FileKeys::new(&dir).get(who).expect("a whole entry");
        assert_eq!(e, entry(e.point.life));
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1, "no temporary files");
        let _ = fs::remove_dir_all(&dir);
    }
}
