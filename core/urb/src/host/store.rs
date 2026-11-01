//! Where an identity's keyfile and the engine's rows are kept.
//!
//! The core hands its commits out as glue.c's rows, `[klen:4][key]
//! [vlen:4][val]`, little-endian, vlen `0xffffffff` for a deleted row,
//! and takes the saved rows back the same way when it opens. Two stores
//! keep them:
//!
//! - [`FileStore`] is node.mjs's `fileStore`: `comet.key` and `rows` in
//!   the state folder, the rows file rewritten whole on each commit, in
//!   the order a JS Map keeps them. A folder moves between the Node
//!   command lines and this one unchanged.
//! - [`SqliteStore`] is the Go gateway's: libmesa's own SQLite store
//!   (`mesa_kv`, WAL, `synchronous=FULL`, a lock on `<path>.lock`), the
//!   same C code the Go binding opens, so a `<ship>.db` the gateway
//!   wrote opens here as it is.

use mesa_sys as sys;
use std::collections::HashMap;
use std::ffi::{CStr, CString, c_void};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// what a store gives the core to open on: the keyfile (none for a
/// comet not mined yet), the saved rows, and whether a moon may start
/// from empty
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Loaded {
    pub keyfile: Option<String>,
    pub rows: Option<Vec<u8>>,
    pub fresh: bool,
}

/// a keyfile from outside the folder (a moon's, a fake ship's), and
/// whether it may start from empty
#[derive(Clone, Debug)]
pub struct Given {
    pub keyfile: String,
    pub fresh: bool,
}

pub trait Store {
    fn load(&mut self) -> Result<Loaded, String>;
    /// a mined comet's keyfile; the rows start again
    fn save_key(&mut self, keyfile: &str) -> Result<(), String>;
    /// rows the core committed, before the datagrams they released
    fn commit(&mut self, rows: &[u8]) -> Result<(), String>;
}

/// the length glue.c writes for a deleted row
pub const GONE: u32 = u32::MAX;

/// A row: its key, and its value or None for a deleted one.
pub type Row = (Vec<u8>, Option<Vec<u8>>);

/// A row held: its key and value.
pub type Kept = (Vec<u8>, Vec<u8>);

/// mesa.mjs's readRows, with its leniency: a key or a value cut short is
/// taken as far as it goes, a header cut short ends the rows, and a
/// value's length past the end is DataView's RangeError
pub fn read_rows(buf: &[u8]) -> Result<Vec<Row>, String> {
    let mut out = Vec::new();
    let mut at = 0usize;
    let u32at = |at: usize| -> Result<u32, String> {
        buf.get(at..at + 4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
            .ok_or_else(|| "Offset is outside the bounds of the DataView".to_string())
    };
    let slice = |from: usize, to: usize| buf[from.min(buf.len())..to.min(buf.len())].to_vec();
    while at + 8 <= buf.len() {
        let kz = u32at(at)? as usize;
        let key = slice(at + 4, at + 4 + kz);
        let vz = u32at(at + 4 + kz)?;
        let val = (vz != GONE).then(|| slice(at + 8 + kz, at + 8 + kz + vz as usize));
        at += 8 + kz + if val.is_some() { vz as usize } else { 0 };
        out.push((key, val));
    }
    Ok(out)
}

/// mesa.mjs's writeRows
pub fn write_rows<'a>(rows: impl IntoIterator<Item = (&'a [u8], Option<&'a [u8]>)>) -> Vec<u8> {
    let mut out = Vec::new();
    for (key, val) in rows {
        out.extend_from_slice(&(key.len() as u32).to_le_bytes());
        out.extend_from_slice(key);
        match val {
            Some(v) => {
                out.extend_from_slice(&(v.len() as u32).to_le_bytes());
                out.extend_from_slice(v);
            }
            None => out.extend_from_slice(&GONE.to_le_bytes()),
        }
    }
    out
}

/// an io error as Node words it, for the errors a user meets
pub fn node_error(e: &io::Error, syscall: &str, path: &Path) -> String {
    let p = path.display();
    match e.kind() {
        io::ErrorKind::NotFound => {
            format!("ENOENT: no such file or directory, {syscall} '{p}'")
        }
        io::ErrorKind::PermissionDenied => {
            format!("EACCES: permission denied, {syscall} '{p}'")
        }
        io::ErrorKind::IsADirectory => {
            format!("EISDIR: illegal operation on a directory, {syscall}")
        }
        _ => format!("{e}, {syscall} '{p}'"),
    }
}

/// write a file through a `.new` beside it and a rename, as node.mjs's
/// save does, mode 0600
fn save(path: &Path, data: &[u8]) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let new = PathBuf::from(format!("{}.new", path.display()));
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&new)
        .map_err(|e| node_error(&e, "open", &new))?;
    f.write_all(data).map_err(|e| e.to_string())?;
    drop(f);
    fs::rename(&new, path).map_err(|e| node_error(&e, "rename", &new))
}

/// the keyfile a folder keeps, or none
fn read_key(path: &Path) -> Option<String> {
    fs::read(path)
        .ok()
        .map(|b| String::from_utf8_lossy(&b).trim().to_string())
}

/// A JS Map of rows by key, in insertion order: setting a key it has
/// keeps its place, deleting one and setting it again moves it last.
#[derive(Default)]
struct Rows {
    order: Vec<Option<Row>>,
    at: HashMap<Vec<u8>, usize>,
}

impl Rows {
    fn set(&mut self, key: Vec<u8>, val: Option<Vec<u8>>) {
        match self.at.get(&key) {
            Some(&i) => self.order[i].as_mut().unwrap().1 = val,
            None => {
                self.at.insert(key.clone(), self.order.len());
                self.order.push(Some((key, val)));
            }
        }
    }

    fn delete(&mut self, key: &[u8]) {
        if let Some(i) = self.at.remove(key) {
            self.order[i] = None;
        }
    }

    fn clear(&mut self) {
        self.order.clear();
        self.at.clear();
    }

    fn bytes(&self) -> Vec<u8> {
        write_rows(
            self.order
                .iter()
                .flatten()
                .map(|(k, v)| (k.as_slice(), v.as_deref())),
        )
    }
}

/// node.mjs's fileStore: the identity's keyfile and the engine's rows,
/// in a folder. The comet's keyfile is kept there; a moon's or a fake
/// ship's comes from outside. Every commit rewrites the rows file whole.
pub struct FileStore {
    key_path: PathBuf,
    rows_path: PathBuf,
    given: Option<Given>,
    rows: Rows,
}

impl FileStore {
    pub fn new(dir: &Path, given: Option<Given>) -> FileStore {
        FileStore {
            key_path: dir.join("comet.key"),
            rows_path: dir.join("rows"),
            given,
            rows: Rows::default(),
        }
    }
}

impl Store for FileStore {
    fn load(&mut self) -> Result<Loaded, String> {
        let keyfile = match &self.given {
            Some(g) => Some(g.keyfile.clone()),
            None => read_key(&self.key_path),
        };
        let saved = fs::read(&self.rows_path).ok();
        if let Some(s) = &saved {
            // the Map keeps a deleted row too, as {key, val: null}
            for (k, v) in read_rows(s)? {
                self.rows.set(k, v);
            }
        }
        Ok(Loaded {
            keyfile,
            rows: saved,
            fresh: self.given.as_ref().is_some_and(|g| g.fresh),
        })
    }

    fn save_key(&mut self, keyfile: &str) -> Result<(), String> {
        save(&self.key_path, format!("{keyfile}\n").as_bytes())?;
        self.rows.clear();
        save(&self.rows_path, &[])
    }

    fn commit(&mut self, batch: &[u8]) -> Result<(), String> {
        for (k, v) in read_rows(batch)? {
            match v {
                Some(v) => self.rows.set(k, Some(v)),
                None => self.rows.delete(&k),
            }
        }
        save(&self.rows_path, &self.rows.bytes())
    }
}

/// The Go gateway's store: libmesa's SQLite store at `<ship>.db`, with
/// the comet's keyfile in `comet.key` beside it. Opening it takes the
/// lock on `<path>.lock`, so two processes never run one identity.
pub struct SqliteStore {
    sto: *mut sys::mesa_store,
    key_path: PathBuf,
    given: Option<Given>,
}

impl SqliteStore {
    /// open (creating) the database at db; the comet's keyfile is kept
    /// in key_path unless one is given
    pub fn open(db: &Path, key_path: &Path, given: Option<Given>) -> Result<SqliteStore, String> {
        let c = CString::new(db.as_os_str().as_encoded_bytes()).map_err(|e| e.to_string())?;
        let mut err: *const std::ffi::c_char = std::ptr::null();
        let sto = unsafe { sys::mesa_store_sqlite(c.as_ptr(), &mut err) };
        if sto.is_null() {
            let why = if err.is_null() {
                "can't open the store".to_string()
            } else {
                unsafe { CStr::from_ptr(err) }
                    .to_string_lossy()
                    .into_owned()
            };
            return Err(format!("{}: {why}", db.display()));
        }
        Ok(SqliteStore {
            sto,
            key_path: key_path.to_path_buf(),
            given,
        })
    }

    fn s(&self) -> &sys::mesa_store {
        unsafe { &*self.sto }
    }

    /// every row, in the keys' byte order
    pub fn rows(&self) -> Result<Vec<Kept>, String> {
        unsafe extern "C" fn one(
            arg: *mut c_void,
            k: *const u8,
            kz: usize,
            v: *const u8,
            vz: usize,
        ) -> i32 {
            let out = unsafe { &mut *(arg as *mut Vec<Kept>) };
            let b = |p: *const u8, n: usize| {
                if n == 0 {
                    Vec::new()
                } else {
                    unsafe { std::slice::from_raw_parts(p, n) }.to_vec()
                }
            };
            out.push((b(k, kz), b(v, vz)));
            0
        }
        let mut out: Vec<Kept> = Vec::new();
        let s = self.s();
        let scan = s.scan.ok_or("the store has no scan")?;
        // an empty prefix, but not a null pointer: SQLite binds a null
        // blob as NULL, and `k >= NULL` holds for no row
        let empty = [0u8; 1];
        let r = unsafe {
            scan(
                s.ctx,
                empty.as_ptr(),
                0,
                Some(one),
                (&mut out as *mut Vec<Kept>).cast(),
            )
        };
        if r < 0 {
            return Err(format!("reading the store failed: {r}"));
        }
        Ok(out)
    }

    /// f inside one transaction, rolled back if it fails
    fn txn(
        &mut self,
        f: impl FnOnce(&sys::mesa_store) -> Result<(), String>,
    ) -> Result<(), String> {
        let s = self.s();
        if unsafe { (s.begin.unwrap())(s.ctx) } != 0 {
            return Err("the store would not begin".into());
        }
        match f(s) {
            Ok(()) => {
                if unsafe { (s.commit.unwrap())(s.ctx) } != 0 {
                    unsafe { (s.rollback.unwrap())(s.ctx) };
                    return Err("the store would not commit".into());
                }
                Ok(())
            }
            Err(e) => {
                unsafe { (s.rollback.unwrap())(s.ctx) };
                Err(e)
            }
        }
    }
}

impl Drop for SqliteStore {
    fn drop(&mut self) {
        unsafe { sys::mesa_store_free(self.sto) };
    }
}

fn put(s: &sys::mesa_store, k: &[u8], v: &[u8]) -> Result<(), String> {
    match unsafe { (s.put.unwrap())(s.ctx, k.as_ptr(), k.len(), v.as_ptr(), v.len()) } {
        0 => Ok(()),
        r => Err(format!("the store's put failed: {r}")),
    }
}

fn del(s: &sys::mesa_store, k: &[u8]) -> Result<(), String> {
    match unsafe { (s.del.unwrap())(s.ctx, k.as_ptr(), k.len()) } {
        0 => Ok(()),
        r => Err(format!("the store's del failed: {r}")),
    }
}

impl Store for SqliteStore {
    fn load(&mut self) -> Result<Loaded, String> {
        let keyfile = match &self.given {
            Some(g) => Some(g.keyfile.clone()),
            None => read_key(&self.key_path),
        };
        let rows = self.rows()?;
        let rows = (!rows.is_empty())
            .then(|| write_rows(rows.iter().map(|(k, v)| (k.as_slice(), Some(v.as_slice())))));
        Ok(Loaded {
            keyfile,
            rows,
            fresh: self.given.as_ref().is_some_and(|g| g.fresh),
        })
    }

    fn save_key(&mut self, keyfile: &str) -> Result<(), String> {
        save(&self.key_path, format!("{keyfile}\n").as_bytes())?;
        let keys: Vec<Vec<u8>> = self.rows()?.into_iter().map(|r| r.0).collect();
        self.txn(|s| keys.iter().try_for_each(|k| del(s, k)))
    }

    fn commit(&mut self, batch: &[u8]) -> Result<(), String> {
        let rows = read_rows(batch)?;
        self.txn(|s| {
            for (k, v) in &rows {
                match v {
                    Some(v) => put(s, k, v)?,
                    None => del(s, k)?,
                }
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("urb-store-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn rows_read_as_the_js_reads_them() {
        let rows = write_rows([
            (&b"a"[..], Some(&b"1"[..])),
            (&b""[..], Some(&b""[..])),
            (&b"gone"[..], None),
        ]);
        assert_eq!(
            read_rows(&rows).unwrap(),
            vec![
                (b"a".to_vec(), Some(b"1".to_vec())),
                (vec![], Some(vec![])),
                (b"gone".to_vec(), None),
            ]
        );
        // a value cut short is taken as far as it goes; a short header
        // ends the rows; a key past the end is DataView's RangeError
        let mut cut = write_rows([(&b"k"[..], Some(&b"value"[..]))]);
        cut.truncate(cut.len() - 2);
        assert_eq!(
            read_rows(&cut).unwrap(),
            vec![(b"k".to_vec(), Some(b"val".to_vec()))]
        );
        assert_eq!(read_rows(&[1, 0, 0]).unwrap(), vec![]);
        assert!(read_rows(&[200, 0, 0, 0, 0, 0, 0, 0, 0]).is_err());
    }

    #[test]
    fn the_file_store_keeps_a_maps_order() {
        let d = tmp("file");
        let mut s = FileStore::new(&d, None);
        assert_eq!(s.load().unwrap(), Loaded::default());
        s.save_key("0wkey").unwrap();
        assert_eq!(fs::read_to_string(d.join("comet.key")).unwrap(), "0wkey\n");
        assert_eq!(fs::read(d.join("rows")).unwrap(), Vec::<u8>::new());
        s.commit(&write_rows([
            (&b"b"[..], Some(&b"1"[..])),
            (&b"a"[..], Some(&b"2"[..])),
        ]))
        .unwrap();
        s.commit(&write_rows([
            (&b"b"[..], Some(&b"3"[..])),
            (&b"c"[..], Some(&b""[..])),
        ]))
        .unwrap();
        s.commit(&write_rows([(&b"a"[..], None)])).unwrap();
        s.commit(&write_rows([(&b"a"[..], Some(&b"4"[..]))]))
            .unwrap();
        let want = write_rows([
            (&b"b"[..], Some(&b"3"[..])),
            (&b"c"[..], Some(&b""[..])),
            (&b"a"[..], Some(&b"4"[..])),
        ]);
        assert_eq!(fs::read(d.join("rows")).unwrap(), want);
        let mut again = FileStore::new(&d, None);
        let l = again.load().unwrap();
        assert_eq!(l.keyfile.as_deref(), Some("0wkey"));
        assert_eq!(l.rows, Some(want));
        assert!(!l.fresh);
        let g = FileStore::new(
            &d,
            Some(Given {
                keyfile: "0wmoon".into(),
                fresh: true,
            }),
        )
        .load()
        .unwrap();
        assert_eq!((g.keyfile.as_deref(), g.fresh), (Some("0wmoon"), true));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_sqlite_store_keeps_rows_in_key_order_and_locks() {
        let d = tmp("sqlite");
        let db = d.join("x.db");
        let mut s = SqliteStore::open(&db, &d.join("comet.key"), None).unwrap();
        assert_eq!(s.load().unwrap(), Loaded::default());
        s.commit(&write_rows([
            (&b"b"[..], Some(&b"1"[..])),
            (&b"a"[..], Some(&b"2"[..])),
        ]))
        .unwrap();
        s.commit(&write_rows([
            (&b"b"[..], None),
            (&b"c"[..], Some(&b""[..])),
        ]))
        .unwrap();
        // a second process (or store) can't open it while this one has it
        assert!(SqliteStore::open(&db, &d.join("comet.key"), None).is_err());
        drop(s);
        let mut s = SqliteStore::open(&db, &d.join("comet.key"), None).unwrap();
        let l = s.load().unwrap();
        assert_eq!(
            l.rows,
            Some(write_rows([
                (&b"a"[..], Some(&b"2"[..])),
                (&b"c"[..], Some(&b""[..]))
            ]))
        );
        s.save_key("0wkey").unwrap();
        let l = s.load().unwrap();
        assert_eq!((l.keyfile.as_deref(), l.rows), (Some("0wkey"), None));
        drop(s);
        fs::remove_dir_all(&d).unwrap();
    }
}
