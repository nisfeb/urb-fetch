//! The real world, for the [`Host`](super::Host): UDP or the relay, the
//! system clock, ureq for HTTPS, a store, and the OS's randomness.
//!
//! The transports are node.mjs's: [`Udp`] keeps one port per identity
//! in the state folder's `port` file and looks galaxies up as
//! `<name>.urbit.org` in the background (on a fake network every ship
//! is on this machine, galaxy g at port 31337 + g); [`Relay`] is
//! mesa.mjs's relayTransport over a WebSocket to `urb-fetch -relay`
//! (ws:// only).

use super::store::Store;
use super::{Event, Io, Loaded, Wake};
use crate::core::{Lane, ROLLER};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpStream, ToSocketAddrs, UdpSocket};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// what sees each output the host hands its caller
pub type Tap = Box<dyn FnMut(&crate::core::Output)>;

/// a log line's sink, from any thread
pub type Log = Arc<dyn Fn(&str) + Send + Sync>;

/// where libmesa's own log lines go, once [`libmesa_log`] is called
static LIBMESA_LOG: Mutex<Option<Log>> = Mutex::new(None);

unsafe extern "C" fn libmesa_line(_: *mut std::ffi::c_void, line: *const std::ffi::c_char) {
    if line.is_null() {
        return;
    }
    let l = unsafe { std::ffi::CStr::from_ptr(line) }.to_string_lossy();
    if let Some(f) = LIBMESA_LOG.lock().ok().and_then(|g| g.clone()) {
        f(&l);
    }
}

/// Route libmesa's own log lines (its notices, and MESA_TRACE's packet
/// log) to log, as the Node hosts hand the wasm module's stderr to
/// theirs. Process-global, as libmesa's mesa_set_log is.
pub fn libmesa_log(log: Log) {
    *LIBMESA_LOG.lock().unwrap() = Some(log);
    unsafe { mesa_sys::mesa_set_log(Some(libmesa_line), std::ptr::null_mut()) };
}

/// ms since 1970, by the system clock
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// n bytes from the OS
pub fn random(n: usize) -> Vec<u8> {
    let mut b = vec![0u8; n];
    fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut b))
        .expect("the OS's randomness");
    b
}

/// a key lookup is given up on after this long, as the shim does
const ROLLER_WAIT: Duration = Duration::from_secs(30);
/// how long a wait may last with nothing to wait for
const IDLE: Duration = Duration::from_millis(250);

/// How datagrams go out and come in.
pub trait Transport {
    /// start as ship, handing each datagram heard to tx
    fn start(&mut self, ship: &str, tx: Sender<Event>) -> Result<(), String>;
    fn send(&mut self, lane: Lane, bytes: &[u8]);
    fn close(&mut self);
}

/// The real [`Io`].
pub struct NetIo {
    tx: Sender<Event>,
    rx: Receiver<Event>,
    pub transport: Box<dyn Transport>,
    pub store: Box<dyn Store>,
    log: Log,
    /// sees each output the host hands its caller, as it comes
    pub tap: Option<Tap>,
}

impl NetIo {
    pub fn new(transport: Box<dyn Transport>, store: Box<dyn Store>, log: Log) -> NetIo {
        let (tx, rx) = channel();
        NetIo {
            tx,
            rx,
            transport,
            store,
            log,
            tap: None,
        }
    }
}

impl Io for NetIo {
    fn now(&mut self) -> u64 {
        now_ms()
    }

    fn wait(&mut self, deadline: Option<u64>) -> Wake {
        let now = now_ms();
        let left = match deadline {
            Some(d) if d <= now => return Wake::Timer,
            Some(d) => Duration::from_millis(d - now),
            None => IDLE,
        };
        match self.rx.recv_timeout(left.min(IDLE)) {
            Ok(e) => Wake::Event(e),
            Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => {
                if deadline.is_some_and(|d| d <= now_ms()) {
                    Wake::Timer
                } else {
                    Wake::Idle
                }
            }
        }
    }

    fn load(&mut self) -> Result<Loaded, String> {
        self.store.load()
    }

    fn save_key(&mut self, keyfile: &str) -> Result<(), String> {
        self.store.save_key(keyfile)
    }

    fn commit(&mut self, rows: &[u8]) -> Result<(), String> {
        self.store.commit(rows)
    }

    fn start(&mut self, ship: &str) -> Result<(), String> {
        self.transport.start(ship, self.tx.clone())
    }

    fn send(&mut self, lane: Lane, bytes: &[u8]) {
        self.transport.send(lane, bytes)
    }

    fn close(&mut self) {
        self.transport.close()
    }

    fn fetch(
        &mut self,
        id: u64,
        url: &str,
        method: &str,
        headers: &[(String, String)],
        body: Option<&str>,
    ) {
        let (tx, url, method) = (self.tx.clone(), url.to_string(), method.to_string());
        let headers = headers.to_vec();
        let body = body.map(String::from);
        thread::spawn(move || {
            let answer = fetch(&url, &method, &headers, body.as_deref());
            let _ = tx.send(Event::Fetched { id, answer });
        });
    }

    fn log(&mut self, line: &str) {
        (self.log)(line)
    }

    fn tap(&mut self, o: &crate::core::Output) {
        if let Some(t) = &mut self.tap {
            t(o)
        }
    }
}

/// An HTTPS request as fetch() answers it: the status and body of any
/// response, or the error's message as Node's fetch gives it (`fetch
/// failed`, and the roller's 30 s limit as AbortSignal.timeout's).
pub fn fetch(
    url: &str,
    method: &str,
    headers: &[(String, String)],
    body: Option<&str>,
) -> Result<(u16, Vec<u8>), String> {
    let mut agent = ureq::AgentBuilder::new();
    if url == ROLLER {
        agent = agent.timeout(ROLLER_WAIT);
    }
    let mut req = agent.build().request(method, url);
    for (k, v) in headers {
        req = req.set(k, v);
    }
    let t = std::time::Instant::now();
    let r = match body {
        Some(b) => req.send_string(b),
        None => req.call(),
    };
    let resp = match r {
        Ok(r) => r,
        Err(ureq::Error::Status(_, r)) => r,
        Err(ureq::Error::Transport(_)) => {
            return Err(if url == ROLLER && t.elapsed() >= ROLLER_WAIT {
                "The operation was aborted due to timeout".into()
            } else {
                "fetch failed".into()
            });
        }
    };
    let status = resp.status();
    let mut out = Vec::new();
    resp.into_reader()
        .take(64 << 20)
        .read_to_end(&mut out)
        .map_err(|_| "fetch failed".to_string())?;
    Ok((status, out))
}

fn ip_text(ip: u32) -> String {
    Ipv4Addr::from(ip).to_string()
}

fn galaxy_name(g: u8) -> String {
    crate::engine::ship_text((u64::from(g), 0))
        .trim_start_matches('~')
        .to_string()
}

/// node.mjs's udpTransport: UDP from one port per identity, kept in
/// `<dir>/port` across runs, since publishers answer the address they
/// last saw.
pub struct Udp {
    port: u16,
    fake: bool,
    sock: Option<UdpSocket>,
    stop: Arc<AtomicBool>,
    /// galaxy -> (ip, when it was looked up)
    galaxies: Arc<Mutex<HashMap<u8, (Ipv4Addr, u64)>>>,
    asking: Arc<Mutex<HashSet<u8>>>,
    log: Log,
}

impl Udp {
    /// the port kept in dir, chosen now if there is none
    pub fn new(dir: &Path, fake: bool, log: Log) -> Result<Udp, String> {
        let path = dir.join("port");
        // Number() of the file: blank or junk is no port
        let kept = fs::read_to_string(&path)
            .ok()
            .map(|t| crate::js::number(&t))
            .filter(|n| *n >= 1.0 && *n <= 65535.0 && n.fract() == 0.0)
            .map(|n| n as u16);
        let port = match kept {
            Some(p) => p,
            None => {
                let b = random(4);
                let x = u32::from_le_bytes(b.try_into().unwrap());
                let p = 20000 + (u64::from(x) * 40000 / (1u64 << 32)) as u16;
                fs::write(&path, p.to_string())
                    .map_err(|e| super::store::node_error(&e, "open", &path))?;
                p
            }
        };
        Ok(Udp {
            port,
            fake,
            sock: None,
            stop: Arc::new(AtomicBool::new(false)),
            galaxies: Arc::default(),
            asking: Arc::default(),
            log,
        })
    }

    /// where galaxy g is, if known; looked up in the background when not,
    /// or when its answer is five minutes old
    fn galaxy(&self, g: u8) -> Option<SocketAddrV4> {
        if self.fake {
            return Some(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 31337 + u16::from(g)));
        }
        let hit = self.galaxies.lock().unwrap().get(&g).copied();
        let stale = hit.is_none_or(|(_, at)| now_ms() - at > 300_000);
        if stale && self.asking.lock().unwrap().insert(g) {
            let (galaxies, asking, log) =
                (self.galaxies.clone(), self.asking.clone(), self.log.clone());
            thread::spawn(move || {
                let name = galaxy_name(g);
                let found = (format!("{name}.urbit.org"), 0u16)
                    .to_socket_addrs()
                    .map_err(|e| e.to_string())
                    .and_then(|mut a| {
                        a.find_map(|a| match a {
                            std::net::SocketAddr::V4(v) => Some(*v.ip()),
                            _ => None,
                        })
                        .ok_or_else(|| "no IPv4 address".to_string())
                    });
                match found {
                    Ok(ip) => {
                        galaxies.lock().unwrap().insert(g, (ip, now_ms()));
                    }
                    Err(e) => log(&format!("can't find galaxy {name}: {e}")),
                }
                asking.lock().unwrap().remove(&g);
            });
        }
        hit.map(|(ip, _)| SocketAddrV4::new(ip, 13337 + u16::from(g)))
    }
}

impl Transport for Udp {
    fn start(&mut self, _ship: &str, tx: Sender<Event>) -> Result<(), String> {
        let at = if self.fake {
            Ipv4Addr::LOCALHOST
        } else {
            Ipv4Addr::UNSPECIFIED
        };
        let sock = UdpSocket::bind(SocketAddrV4::new(at, self.port)).map_err(|e| {
            let code = match e.kind() {
                std::io::ErrorKind::AddrInUse => "EADDRINUSE",
                std::io::ErrorKind::PermissionDenied => "EACCES",
                _ => "EINVAL",
            };
            format!("bind {code} {at}:{}", self.port)
        })?;
        let rx = sock.try_clone().map_err(|e| e.to_string())?;
        rx.set_read_timeout(Some(Duration::from_millis(200)))
            .map_err(|e| e.to_string())?;
        let stop = self.stop.clone();
        thread::spawn(move || {
            let mut buf = vec![0u8; 65536];
            while !stop.load(Ordering::Relaxed) {
                if let Ok((n, std::net::SocketAddr::V4(from))) = rx.recv_from(&mut buf) {
                    let ev = Event::Packet {
                        ip: u32::from(*from.ip()),
                        port: from.port(),
                        bytes: buf[..n].to_vec(),
                    };
                    if tx.send(ev).is_err() {
                        break;
                    }
                }
            }
        });
        self.sock = Some(sock);
        Ok(())
    }

    fn send(&mut self, lane: Lane, bytes: &[u8]) {
        let to = match lane {
            Lane::Galaxy(g) => self.galaxy(g),
            Lane::Ip(ip, port) => Some(SocketAddrV4::new(
                if self.fake {
                    Ipv4Addr::LOCALHOST
                } else {
                    Ipv4Addr::from(ip)
                },
                port,
            )),
        };
        if let (Some(s), Some(to)) = (&self.sock, to) {
            let _ = s.send_to(bytes, to);
        }
    }

    fn close(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.sock = None;
    }
}

/// node.mjs's traced(): every datagram, logged
pub struct Traced {
    pub inner: Box<dyn Transport>,
    pub log: Log,
}

fn lane_text(l: Lane) -> String {
    match l {
        Lane::Galaxy(g) => format!("~{}", galaxy_name(g)),
        Lane::Ip(ip, port) => format!("{}:{port}", ip_text(ip)),
    }
}

impl Transport for Traced {
    fn start(&mut self, ship: &str, tx: Sender<Event>) -> Result<(), String> {
        // heard datagrams pass through a thread that logs them
        let (mid, from) = channel::<Event>();
        let log = self.log.clone();
        thread::spawn(move || {
            for e in from {
                if let Event::Packet { ip, port, bytes } = &e {
                    log(&format!(
                        "hear {} from {}:{port}",
                        bytes.len(),
                        ip_text(*ip)
                    ));
                }
                if tx.send(e).is_err() {
                    break;
                }
            }
        });
        self.inner.start(ship, mid)
    }

    fn send(&mut self, lane: Lane, bytes: &[u8]) {
        (self.log)(&format!("send {} to {}", bytes.len(), lane_text(lane)));
        self.inner.send(lane, bytes)
    }

    fn close(&mut self) {
        self.inner.close()
    }
}

// ------------------------------------------------------------ the relay

const LANE_GALAXY: u8 = 1;
const LANE_IP: u8 = 2;

/// mesa.mjs's relayTransport: datagrams over a WebSocket to `urb-fetch
/// -relay`. The first message names the ship; each frame out is
/// [kind:1][galaxy:1][ip:4][port:2][datagram] and each frame in
/// [ip:4][port:2][datagram], big-endian. While the socket is down
/// datagrams are dropped, and it connects again every 2 s.
pub struct Relay {
    url: String,
    ws: Arc<Mutex<Option<TcpStream>>>,
    closed: Arc<AtomicBool>,
}

impl Relay {
    pub fn new(url: &str) -> Relay {
        Relay {
            url: url.into(),
            ws: Arc::default(),
            closed: Arc::new(AtomicBool::new(false)),
        }
    }
}

/// ws://host[:port][/path]: its address and path
fn ws_parts(url: &str) -> Option<(String, String, String)> {
    let rest = url.strip_prefix("ws://")?;
    let (hostport, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let addr = if hostport
        .rsplit_once(':')
        .is_some_and(|(_, p)| p.parse::<u16>().is_ok())
    {
        hostport.to_string()
    } else {
        format!("{hostport}:80")
    };
    Some((addr, hostport.to_string(), path.to_string()))
}

fn base64(b: &[u8]) -> String {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for c in b.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= c.len() {
                s.push(A[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

/// a WebSocket frame from us: masked, as a client's must be
fn frame(op: u8, data: &[u8]) -> Vec<u8> {
    let mut f = vec![0x80 | op];
    match data.len() {
        n if n < 126 => f.push(0x80 | n as u8),
        n if n < 65536 => {
            f.push(0x80 | 126);
            f.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
            f.push(0x80 | 127);
            f.extend_from_slice(&(n as u64).to_be_bytes());
        }
    }
    let mask: [u8; 4] = random(4).try_into().unwrap();
    f.extend_from_slice(&mask);
    f.extend(data.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    f
}

/// open the WebSocket and name the ship
fn ws_connect(url: &str, ship: &str) -> Result<TcpStream, String> {
    let fail = || format!("can't reach the relay at {url}");
    let (addr, host, path) = ws_parts(url).ok_or_else(fail)?;
    let mut s = TcpStream::connect(&addr).map_err(|_| fail())?;
    let key = base64(&random(16));
    let hello = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
         Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
    );
    s.write_all(hello.as_bytes()).map_err(|_| fail())?;
    let mut head = Vec::new();
    let mut b = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if s.read(&mut b).map_err(|_| fail())? == 0 || head.len() > 16384 {
            return Err(fail());
        }
        head.push(b[0]);
    }
    let first = String::from_utf8_lossy(&head);
    if !first.starts_with("HTTP/1.1 101") {
        return Err(fail());
    }
    s.write_all(&frame(1, ship.as_bytes()))
        .map_err(|_| fail())?;
    Ok(s)
}

/// read one frame: its opcode and payload
fn ws_read(s: &mut TcpStream) -> std::io::Result<(u8, Vec<u8>)> {
    let mut h = [0u8; 2];
    s.read_exact(&mut h)?;
    let mut n = u64::from(h[1] & 0x7f);
    if n == 126 {
        let mut b = [0u8; 2];
        s.read_exact(&mut b)?;
        n = u64::from(u16::from_be_bytes(b));
    } else if n == 127 {
        let mut b = [0u8; 8];
        s.read_exact(&mut b)?;
        n = u64::from_be_bytes(b);
    }
    if n > 1 << 24 {
        return Err(std::io::Error::other("frame too big"));
    }
    let mask = if h[1] & 0x80 != 0 {
        let mut m = [0u8; 4];
        s.read_exact(&mut m)?;
        Some(m)
    } else {
        None
    };
    let mut data = vec![0u8; n as usize];
    s.read_exact(&mut data)?;
    if let Some(m) = mask {
        for (i, b) in data.iter_mut().enumerate() {
            *b ^= m[i % 4];
        }
    }
    Ok((h[0] & 0x0f, data))
}

/// hear the relay until the socket goes; then, unless closed, connect
/// again every 2 s
fn ws_listen(
    url: String,
    ship: String,
    mut s: TcpStream,
    ws: Arc<Mutex<Option<TcpStream>>>,
    closed: Arc<AtomicBool>,
    tx: Sender<Event>,
) {
    loop {
        let mut msg: Vec<u8> = Vec::new();
        let mut op0 = 0u8;
        while let Ok((op, data)) = ws_read(&mut s) {
            match op {
                0 => msg.extend_from_slice(&data),
                1 | 2 => {
                    op0 = op;
                    msg = data;
                }
                8 => break,
                9 => {
                    if let Some(w) = ws.lock().unwrap().as_mut() {
                        let _ = w.write_all(&frame(10, &data));
                    }
                    continue;
                }
                _ => continue,
            }
            // a whole message: binary, at least an address
            if op0 == 2 && msg.len() >= 6 {
                let ip = u32::from_be_bytes(msg[0..4].try_into().unwrap());
                let port = u16::from_be_bytes(msg[4..6].try_into().unwrap());
                if tx
                    .send(Event::Packet {
                        ip,
                        port,
                        bytes: msg[6..].to_vec(),
                    })
                    .is_err()
                {
                    return;
                }
            }
        }
        *ws.lock().unwrap() = None;
        loop {
            if closed.load(Ordering::Relaxed) {
                return;
            }
            thread::sleep(Duration::from_secs(2));
            if closed.load(Ordering::Relaxed) {
                return;
            }
            if let Ok(n) = ws_connect(&url, &ship)
                && let Ok(w) = n.try_clone()
            {
                *ws.lock().unwrap() = Some(w);
                s = n;
                break;
            }
        }
    }
}

impl Transport for Relay {
    fn start(&mut self, ship: &str, tx: Sender<Event>) -> Result<(), String> {
        let s = match ws_connect(&self.url, ship) {
            Ok(s) => s,
            Err(e) => {
                self.closed.store(true, Ordering::Relaxed);
                return Err(e);
            }
        };
        let w = s.try_clone().map_err(|e| e.to_string())?;
        *self.ws.lock().unwrap() = Some(w);
        let (url, ship, ws, closed) = (
            self.url.clone(),
            ship.to_string(),
            self.ws.clone(),
            self.closed.clone(),
        );
        thread::spawn(move || ws_listen(url, ship, s, ws, closed, tx));
        Ok(())
    }

    fn send(&mut self, lane: Lane, bytes: &[u8]) {
        let mut f = vec![0u8; 8];
        match lane {
            Lane::Galaxy(g) => {
                f[0] = LANE_GALAXY;
                f[1] = g;
            }
            Lane::Ip(ip, port) => {
                f[0] = LANE_IP;
                f[2..6].copy_from_slice(&ip.to_be_bytes());
                f[6..8].copy_from_slice(&port.to_be_bytes());
            }
        }
        f.extend_from_slice(bytes);
        if let Some(w) = self.ws.lock().unwrap().as_mut()
            && w.write_all(&frame(2, &f)).is_err()
        {
            // the listener sees it go, and connects again
        }
    }

    fn close(&mut self) {
        self.closed.store(true, Ordering::Relaxed);
        if let Some(mut w) = self.ws.lock().unwrap().take() {
            let _ = w.write_all(&frame(8, &[]));
            let _ = w.shutdown(std::net::Shutdown::Both);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_as_btoa() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
    }

    #[test]
    fn ws_urls() {
        assert_eq!(
            ws_parts("ws://localhost:8476"),
            Some(("localhost:8476".into(), "localhost:8476".into(), "/".into()))
        );
        assert_eq!(
            ws_parts("ws://h/x"),
            Some(("h:80".into(), "h".into(), "/x".into()))
        );
        assert_eq!(ws_parts("wss://h"), None);
    }
}
