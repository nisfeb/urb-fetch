//! The Go gateway's state folder, as main.go's `open` keeps it:
//!
//! - `comet.key`: the comet's keyfile, mined on first use (moons and
//!   fake ships bring their own);
//! - `<name>.db`: libmesa's SQLite store of the engine's rows, `<name>`
//!   the ship's name without its `~`, `fake-<name>` for a fake ship, so
//!   a fake run never touches the real ship's state;
//! - `<name>.db.port`: its UDP port, kept across runs, since publishers
//!   answer the address they last saw.
//!
//! The Rust gateway opens the same files the same way, with the same
//! words when it can't, so a folder moves between the two binaries.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use urb::core::Lane;
use urb::host::Event;
use urb::host::net::{Log, Transport, Udp, random};
use urb::host::store::{Given, Loaded, SqliteStore, Store};

/// main.go's `oneIdentity`: -fake and -moon each name who the gateway
/// runs as
pub fn one_identity(fake: &str, moon: &str) -> Result<(), String> {
    if !fake.is_empty() && !moon.is_empty() {
        return Err(
            "-fake and -moon each name who the gateway runs as: pass one or the other".into(),
        );
    }
    Ok(())
}

/// an OS error as Go's os.PathError prints it: `<op> <path>: <errno>`
pub fn go_path_error(op: &str, path: &Path, e: &io::Error) -> String {
    let mut why = e.to_string();
    if let Some(i) = why.find(" (os error") {
        why.truncate(i);
    }
    let why = match why.chars().next() {
        Some(c) => c.to_lowercase().collect::<String>() + &why[c.len_utf8()..],
        None => why,
    };
    format!("{op} {}: {why}", path.display())
}

/// `mesa.LoadKeyfile(path)`: the keyfile's text (trimmed, as
/// ParseKeyfile trims it) and the ship it names, in the binding's words
/// when it can't
pub fn load_keyfile(path: &Path) -> Result<(String, String), String> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::IsADirectory => {
            return Err(go_path_error("read", path, &e));
        }
        Err(e) => return Err(go_path_error("open", path, &e)),
    };
    let text = String::from_utf8_lossy(&bytes).trim().to_string();
    match urb::engine::keyfile_ship(&text) {
        Some((ship, _, _)) => Ok((text, urb::engine::ship_text(ship))),
        None => Err(format!("{}: mesa: not a keyfile", path.display())),
    }
}

/// `mesa.ParseShip(text)`: the ship's name, or the binding's words
pub fn parse_ship(text: &str) -> Result<String, String> {
    let t = if text.starts_with('~') {
        text.to_string()
    } else {
        format!("~{text}")
    };
    match urb::engine::ship_parse(&t) {
        Some(s) => Ok(urb::engine::ship_text(s)),
        None => Err(format!("mesa: not a ship: {t:?}")),
    }
}

/// `udpPort(file)`: the port kept in file, or one chosen now and kept
pub fn udp_port(file: &Path) -> Result<u16, String> {
    if let Ok(b) = fs::read(file) {
        let t = String::from_utf8_lossy(&b);
        let t = t.trim();
        return t
            .parse::<i64>()
            .map_err(|_| format!("strconv.Atoi: parsing {t:?}: invalid syntax"))
            .and_then(|p| u16::try_from(p).map_err(|_| format!("invalid port {p}")));
    }
    let b = random(2);
    let p = 20000 + u16::from_be_bytes([b[0], b[1]]) % 40000;
    write_new(file, format!("{p}\n").as_bytes(), false)?;
    Ok(p)
}

/// os.WriteFile (or, excl, an O_EXCL create), mode 0600, Go's words
fn write_new(path: &Path, data: &[u8], excl: bool) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut o = fs::OpenOptions::new();
    o.write(true).mode(0o600);
    if excl {
        o.create_new(true);
    } else {
        o.create(true).truncate(true);
    }
    let mut f = o.open(path).map_err(|e| go_path_error("open", path, &e))?;
    f.write_all(data)
        .map_err(|e| go_path_error("write", path, &e))
}

/// `mesa.OpenSQLite("sqlite", db)`, in the binding's words when another
/// process has it
pub fn open_db(db: &Path, key_path: &Path, given: Option<Given>) -> Result<SqliteStore, String> {
    SqliteStore::open(db, key_path, given).map_err(|e| {
        if e.ends_with("state is in use by another process") {
            format!("mesa: {} is in use by another process", db.display())
        } else {
            e
        }
    })
}

/// the base of an identity's files: its name, `fake-` before a fake
/// ship's
pub fn base_name(ship: &str, fake: bool) -> String {
    let name = ship.trim_start_matches('~');
    if fake {
        format!("fake-{name}")
    } else {
        name.to_string()
    }
}

/// The store: `<name>.db`, opened at once for an identity known now, or
/// once a new comet is mined and its keyfile saved (as LoadComet saves
/// it: created, never overwritten, mode 0600).
pub struct GatewayStore {
    pub state: PathBuf,
    pub db: Option<SqliteStore>,
}

impl Store for GatewayStore {
    fn load(&mut self) -> Result<Loaded, String> {
        match &mut self.db {
            Some(db) => db.load(),
            None => Ok(Loaded::default()),
        }
    }

    fn save_key(&mut self, keyfile: &str) -> Result<(), String> {
        let key_path = self.state.join("comet.key");
        write_new(&key_path, format!("{keyfile}\n").as_bytes(), true)?;
        let Some((ship, _, _)) = urb::engine::keyfile_ship(keyfile) else {
            return Err("mesa: not a keyfile".into());
        };
        let name = base_name(&urb::engine::ship_text(ship), false);
        let db = open_db(&self.state.join(format!("{name}.db")), &key_path, None)?;
        self.db = Some(db);
        Ok(())
    }

    fn commit(&mut self, rows: &[u8]) -> Result<(), String> {
        match &mut self.db {
            Some(db) => db.commit(rows),
            None => Err("no store open".into()),
        }
    }
}

/// UDP from the identity's kept port: known now, or read (or chosen)
/// from `<name>.db.port` once a new comet's name is known.
pub struct GatewayUdp {
    pub state: PathBuf,
    pub fake: bool,
    pub port: Option<u16>,
    pub log: Log,
    pub udp: Option<Udp>,
}

impl Transport for GatewayUdp {
    fn start(&mut self, ship: &str, tx: Sender<Event>) -> Result<(), String> {
        if self.udp.is_none() {
            let port = match self.port {
                Some(p) => p,
                None => udp_port(
                    &self
                        .state
                        .join(format!("{}.db.port", base_name(ship, self.fake))),
                )?,
            };
            self.udp = Some(Udp::with_port(port, self.fake, self.log.clone()));
        }
        self.udp.as_mut().expect("a socket").start(ship, tx)
    }

    fn send(&mut self, lane: Lane, bytes: &[u8]) {
        if let Some(u) = &mut self.udp {
            u.send(lane, bytes)
        }
    }

    fn close(&mut self) {
        if let Some(u) = &mut self.udp {
            u.close()
        }
    }
}

/// What open found: the identity's keyfile (none for a comet not yet
/// mined), and its store and port where they are known now.
pub struct Found {
    pub keyfile: Option<String>,
    pub fresh: bool,
    pub store: GatewayStore,
    pub port: Option<u16>,
}

/// main.go's `open`, up to `mesa.Open`: the identity, its store and its
/// port, with the Go gateway's words for each failure
pub fn find(state: &Path, moon: &str, new_moon: bool, fake: &str) -> Result<Found, String> {
    let (keyfile, ship) = if !fake.is_empty() {
        let ship = parse_ship(fake)?;
        let who = urb::engine::ship_parse(&ship).expect("parsed");
        (Some(urb::engine::fake_keyfile(who)), Some(ship))
    } else if !moon.is_empty() {
        let (k, s) = load_keyfile(Path::new(moon))?;
        (Some(k), Some(s))
    } else {
        let p = state.join("comet.key");
        if p.exists() {
            let (k, s) = load_keyfile(&p)?;
            (Some(k), Some(s))
        } else {
            // mined by the core, then saved by GatewayStore
            (None, None)
        }
    };
    let Some(ship) = ship else {
        return Ok(Found {
            keyfile: None,
            fresh: false,
            store: GatewayStore {
                state: state.to_path_buf(),
                db: None,
            },
            port: None,
        });
    };
    let name = base_name(&ship, !fake.is_empty());
    let db = state.join(format!("{name}.db"));
    let fresh = !db.exists();
    if !moon.is_empty() && fresh && !new_moon {
        return Err(format!(
            "no state for {ship} in {}. Pass -new-moon only if this moon has never run anywhere; \
             a moon that has run must be breached first (|moon-breach on its parent)",
            state.display()
        ));
    }
    let given = if fake.is_empty() && moon.is_empty() {
        None
    } else {
        Some(Given {
            keyfile: keyfile.clone().expect("a keyfile"),
            // Config.Breached: a moon whose state is new here
            fresh: !moon.is_empty() && fresh,
        })
    };
    let store = open_db(&db, &state.join("comet.key"), given)?;
    let port = udp_port(&state.join(format!("{name}.db.port")))?;
    Ok(Found {
        keyfile,
        fresh: !moon.is_empty() && fresh,
        store: GatewayStore {
            state: state.to_path_buf(),
            db: Some(store),
        },
        port: Some(port),
    })
}
