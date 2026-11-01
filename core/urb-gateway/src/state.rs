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
use std::path::Path;
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

/// `mesa.LoadComet(path)` for a comet not yet mined: the comet-stars
/// list read (as the binding reads it, with a 30 s timeout and whatever
/// status), a comet mined under one of them, and its keyfile saved at
/// path (created, never overwritten, 0600), in the binding's words when
/// any of it fails
pub fn mine_comet(path: &Path) -> Result<(String, String), String> {
    use std::io::Read;
    let url = urb::core::STARS;
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(30))
        .build();
    let res = match agent.get(url).set("user-agent", "urb-fetch").call() {
        Ok(r) | Err(ureq::Error::Status(_, r)) => r,
        // url.Error's words, then the transport's own
        Err(e) => return Err(format!("Get {url:?}: {e}")),
    };
    let mut jam = Vec::new();
    match res.into_reader().read_to_end(&mut jam) {
        Err(e) => return Err(format!("mesa: comet-stars: {e}")),
        Ok(_) if jam.is_empty() => return Err("mesa: comet-stars: <nil>".into()),
        Ok(_) => {}
    }
    let stars = urb::engine::comet_stars(&jam).ok_or("mesa: bad comet-stars.jam")?;
    if stars.is_empty() {
        return Err("mesa: no stars to mine a comet under".into());
    }
    let eny: [u8; 64] = random(64).try_into().expect("64 bytes");
    let keyfile = urb::engine::mine_comet_under(&stars, &eny).ok_or("mesa: comet mining failed")?;
    write_new(path, format!("{keyfile}\n").as_bytes(), true)?;
    let ship = urb::engine::keyfile_ship(&keyfile)
        .ok_or("mesa: not a keyfile")?
        .0;
    Ok((keyfile, urb::engine::ship_text(ship)))
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

/// The store: `<name>.db`, through libmesa's SQLite store.
pub struct GatewayStore {
    pub db: SqliteStore,
}

impl Store for GatewayStore {
    fn load(&mut self) -> Result<Loaded, String> {
        self.db.load()
    }

    /// never asked: the gateway opens the core with a keyfile always
    /// (a new comet is mined by [`mine_comet`] first, as LoadComet does)
    fn save_key(&mut self, _: &str) -> Result<(), String> {
        Err("the gateway saves its own comet.key".into())
    }

    fn commit(&mut self, rows: &[u8]) -> Result<(), String> {
        self.db.commit(rows)
    }
}

/// UDP from the identity's kept port, in Go's words when it can't bind.
pub struct GatewayUdp {
    pub fake: bool,
    pub port: u16,
    pub log: Log,
    pub udp: Option<Udp>,
}

impl Transport for GatewayUdp {
    fn start(&mut self, ship: &str, tx: Sender<Event>) -> Result<(), String> {
        let (port, fake, log) = (self.port, self.fake, self.log.clone());
        self.udp
            .get_or_insert_with(|| Udp::with_port(port, fake, log))
            .start(ship, tx)
            .map_err(|e| go_udp_error(&e))
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

/// a UDP socket the host couldn't bind (`bind <code> <ip>:<port>`, in
/// node's words, as the core's transport says it), as Go's
/// net.ListenUDP says it
pub fn go_udp_error(e: &str) -> String {
    let Some((code, addr)) = e.strip_prefix("bind ").and_then(|r| r.split_once(' ')) else {
        return e.to_string();
    };
    let why = match code {
        "EADDRINUSE" => "address already in use",
        "EACCES" => "permission denied",
        _ => "invalid argument",
    };
    format!("listen udp4 {addr}: bind: {why}")
}

/// What open found: the identity's keyfile, its store and its port.
pub struct Found {
    pub keyfile: String,
    pub fresh: bool,
    pub store: GatewayStore,
    pub port: u16,
}

/// main.go's `open`, up to `mesa.Open`: the identity, its store and its
/// port, with the Go gateway's words for each failure
pub fn find(state: &Path, moon: &str, new_moon: bool, fake: &str) -> Result<Found, String> {
    let (keyfile, ship) = if !fake.is_empty() {
        let ship = parse_ship(fake)?;
        let who = urb::engine::ship_parse(&ship).expect("parsed");
        (urb::engine::fake_keyfile(who), ship)
    } else if !moon.is_empty() {
        load_keyfile(Path::new(moon))?
    } else {
        // os.Stat: anything there is loaded, as a keyfile
        let p = state.join("comet.key");
        if fs::metadata(&p).is_ok() {
            load_keyfile(&p)?
        } else {
            mine_comet(&p)?
        }
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
            keyfile: keyfile.clone(),
            // Config.Breached: a moon whose state is new here
            fresh: !moon.is_empty() && fresh,
        })
    };
    let store = open_db(&db, &state.join("comet.key"), given)?;
    let port = udp_port(&state.join(format!("{name}.db.port")))?;
    Ok(Found {
        keyfile,
        fresh: !moon.is_empty() && fresh,
        store: GatewayStore { db: store },
        port,
    })
}
