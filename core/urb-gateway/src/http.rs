//! The part of Go's `net/http` (and `net/url`, `net`) the Go gateway's
//! answers show, and a small HTTP/1.1 server.
//!
//! What a handler sees is read as Go reads it: the host from the
//! request's absolute URL or its Host header, the path percent-decoded
//! (`url.ParseRequestURI`, so a bad escape or a control byte is a 400
//! before any handler runs), and the query as `url.ParseQuery` takes it
//! (a pair with a `;` or a bad escape is dropped, `+` is a space). A
//! decoded path or query may hold any bytes, so they are bytes here.
//!
//! What it answers is what `http.Error` and `http.Redirect` write, and
//! the requests Go's server refuses before its handler runs get Go's
//! words (a missing or malformed Host header, a bad request line).
//! Every answer carries `Content-Length` and `Date`: framing, which the
//! differential run does not compare (Go chunks a long body instead).

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A request, as the gateway's handler reads it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Request {
    pub method: String,
    /// `req.Host`
    pub host: Vec<u8>,
    /// `req.URL.Path`, decoded
    pub path: Vec<u8>,
    /// `req.URL.RawQuery`
    pub raw_query: Vec<u8>,
    /// HTTP/1.x's x, which the answer's status line repeats
    pub minor: u8,
}

impl Request {
    /// `req.URL.Query()`, in order
    pub fn query(&self) -> Vec<(Vec<u8>, Vec<u8>)> {
        parse_query(&self.raw_query)
    }

    /// `req.URL.Query().Has(key)`
    pub fn has(&self, key: &[u8]) -> bool {
        self.query().iter().any(|(k, _)| k == key)
    }

    /// `req.URL.Query().Get(key)`: the first value, or none
    pub fn get(&self, key: &[u8]) -> Vec<u8> {
        self.query()
            .into_iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
            .unwrap_or_default()
    }
}

/// A response: its status, its headers (by their canonical names, as
/// `Header.Set` keeps them, written in order of name as Go writes
/// them) and its body.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub headers: BTreeMap<String, Vec<u8>>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn new(status: u16) -> Response {
        Response {
            status,
            ..Response::default()
        }
    }

    /// `h.Set(key, value)`
    pub fn set(&mut self, key: &str, value: impl Into<Vec<u8>>) {
        self.headers.insert(key.to_string(), value.into());
    }

    pub fn header(&self, key: &str) -> Option<&[u8]> {
        self.headers.get(key).map(Vec::as_slice)
    }
}

/// `http.StatusText(code)`
pub fn status_text(code: u16) -> &'static str {
    match code {
        200 => "OK",
        301 => "Moved Permanently",
        302 => "Found",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        421 => "Misdirected Request",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        505 => "HTTP Version Not Supported",
        _ => "",
    }
}

/// `http.Error(w, error, code)`
pub fn error(msg: &str, code: u16) -> Response {
    let mut r = Response::new(code);
    r.set("Content-Type", "text/plain; charset=utf-8");
    r.set("X-Content-Type-Options", "nosniff");
    r.body = format!("{msg}\n").into_bytes();
    r
}

/// net/http's `hexEscapeNonASCII`: each byte past ASCII as `%` and its
/// value in lower-case hex (strconv.AppendInt's)
pub fn hex_escape_non_ascii(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    for &b in s {
        if b >= 0x80 {
            out.extend_from_slice(format!("%{b:x}").as_bytes());
        } else {
            out.push(b);
        }
    }
    out
}

/// `http.Redirect(w, r, url, code)` to an absolute url (all the gateway
/// redirects to): its Location, and for a GET or HEAD its short HTML
pub fn redirect(method: &str, url: &[u8], code: u16) -> Response {
    let mut r = Response::new(code);
    r.set("Location", hex_escape_non_ascii(url));
    if method == "GET" || method == "HEAD" {
        r.set("Content-Type", "text/html; charset=utf-8");
    }
    if method == "GET" {
        r.body = [
            b"<a href=\"".as_slice(),
            &urb_render::gateway::html_escape(url),
            b"\">",
            status_text(code).as_bytes(),
            b"</a>.\n\n",
        ]
        .concat();
    }
    r
}

/// `strings.TrimSpace(s)`: Unicode's white space off each end; a byte
/// that is not UTF-8 is no space
pub fn trim_space(s: &[u8]) -> &[u8] {
    let space_at = |s: &[u8]| -> Option<usize> {
        let n = match s.first()? {
            b if *b < 0x80 => 1,
            b if *b >= 0xf0 => 4,
            b if *b >= 0xe0 => 3,
            _ => 2,
        };
        let c = std::str::from_utf8(s.get(..n)?).ok()?.chars().next()?;
        c.is_whitespace().then_some(n)
    };
    let mut s = s;
    while let Some(n) = space_at(s) {
        s = &s[n..];
    }
    loop {
        let mut cut = None;
        for n in 1..=4.min(s.len()) {
            if let Ok(t) = std::str::from_utf8(&s[s.len() - n..]) {
                if t.chars().count() == 1 && t.chars().all(char::is_whitespace) {
                    cut = Some(n);
                }
                break;
            }
        }
        match cut {
            Some(n) => s = &s[..s.len() - n],
            None => return s,
        }
    }
}

/// `net.SplitHostPort(hostport)`'s host, or None where Go errs
pub fn split_host_port(hp: &[u8]) -> Option<&[u8]> {
    let i = hp.iter().rposition(|&b| b == b':')?;
    let host = if hp.first() == Some(&b'[') {
        let end = hp.iter().position(|&b| b == b']')?;
        // the port's colon must follow the ] at once
        if end + 1 != i {
            return None;
        }
        &hp[1..end]
    } else {
        let h = &hp[..i];
        if h.contains(&b':') {
            return None;
        }
        h
    };
    let after = if hp.first() == Some(&b'[') {
        hp.iter().position(|&b| b == b']').map_or(0, |e| e + 1)
    } else {
        0
    };
    if hp[after..i].contains(&b'[')
        || hp[after..i].contains(&b']')
        || hp[i + 1..].contains(&b'[')
        || hp[i + 1..].contains(&b']')
    {
        return None;
    }
    if hp.first() != Some(&b'[') && (host.contains(&b'[') || host.contains(&b']')) {
        return None;
    }
    Some(host)
}

fn unhex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// `url.PathUnescape` (`plus` false) or `url.QueryUnescape` (`plus`
/// true, a `+` is a space): None for a bad escape
pub fn unescape(s: &[u8], plus: bool) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        match s[i] {
            b'%' => {
                let hi = unhex(*s.get(i + 1)?)?;
                let lo = unhex(*s.get(i + 2)?)?;
                out.push(hi << 4 | lo);
                i += 3;
            }
            b'+' if plus => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    Some(out)
}

/// `url.ParseQuery(raw)`'s pairs, in order: a pair with a `;` or a bad
/// escape is dropped, as Go drops it (with an error `Query()` ignores)
pub fn parse_query(raw: &[u8]) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut out = Vec::new();
    for part in raw.split(|&b| b == b'&') {
        if part.is_empty() || part.contains(&b';') {
            continue;
        }
        let (k, v) = match part.iter().position(|&b| b == b'=') {
            Some(i) => (&part[..i], &part[i + 1..]),
            None => (part, &b""[..]),
        };
        if let (Some(k), Some(v)) = (unescape(k, true), unescape(v, true)) {
            out.push((k, v));
        }
    }
    out
}

/// why a request was refused before any handler ran: the status and
/// text Go's server writes, as `HTTP/1.1 <code> <text>` and the body
#[derive(Debug, PartialEq, Eq)]
pub struct Refused {
    pub code: u16,
    /// Go's statusError text, if any (else the bare status)
    pub text: Option<&'static str>,
}

impl Refused {
    fn bad() -> Refused {
        Refused {
            code: 400,
            text: None,
        }
    }

    /// what Go writes to the connection, which it then closes
    pub fn bytes(&self) -> Vec<u8> {
        let status = match self.text {
            Some(t) => format!("{} {}: {t}", self.code, status_text(self.code)),
            None => format!("{} {}", self.code, status_text(self.code)),
        };
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nConnection: close\r\n\r\n{status}"
        )
        .into_bytes()
    }
}

/// a token (a method, a header's name), as httpguts reads one
fn token(s: &[u8]) -> bool {
    !s.is_empty()
        && s.iter()
            .all(|&b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

/// httpguts.ValidHostHeader
fn valid_host(h: &[u8]) -> bool {
    h.iter()
        .all(|&b| b.is_ascii_alphanumeric() || b"!$%&()*+,-.:;=[]'_~".contains(&b))
}

/// a request target as `url.ParseRequestURI` reads it: (host, path,
/// raw query), or None where Go errs
fn request_uri(t: &[u8]) -> Option<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    if t.is_empty() || t.iter().any(|&b| b < 0x20 || b == 0x7f) {
        return None;
    }
    if t == b"*" {
        return Some((Vec::new(), b"*".to_vec(), Vec::new()));
    }
    let (rest, query) = match t.iter().position(|&b| b == b'?') {
        Some(i) => (&t[..i], t[i + 1..].to_vec()),
        None => (t, Vec::new()),
    };
    if rest.starts_with(b"/") {
        return Some((Vec::new(), unescape(rest, false)?, query));
    }
    // the absolute form: scheme://authority/path
    let colon = rest.iter().position(|&b| b == b':')?;
    let scheme = &rest[..colon];
    if scheme.is_empty()
        || !scheme[0].is_ascii_alphabetic()
        || !scheme
            .iter()
            .all(|&b| b.is_ascii_alphanumeric() || b"+-.".contains(&b))
    {
        return None;
    }
    let after = rest[colon + 1..].strip_prefix(b"//")?;
    let (auth, path) = match after.iter().position(|&b| b == b'/') {
        Some(i) => (&after[..i], &after[i..]),
        None => (after, &b""[..]),
    };
    let host = match auth.iter().rposition(|&b| b == b'@') {
        Some(i) => &auth[i + 1..],
        None => auth,
    };
    Some((host.to_vec(), unescape(path, false)?, query))
}

/// What one read of a request gave.
pub enum Got {
    /// a request, and whether the connection stays open after it
    Req(Request, bool),
    /// refused: write this and close
    Refused(Refused),
    /// the connection closed or timed out
    Gone,
}

/// the most header bytes Go reads (DefaultMaxHeaderBytes, and its 4096
/// of slack)
const MAX_HEADER: usize = (1 << 20) + 4096;

/// one line, without its \r\n or \n; Err(true) past the limit
fn line(r: &mut impl BufRead, used: &mut usize) -> Result<Vec<u8>, bool> {
    let mut l = Vec::new();
    let n = r
        .take((MAX_HEADER - (*used).min(MAX_HEADER)) as u64 + 1)
        .read_until(b'\n', &mut l)
        .map_err(|_| false)?;
    if n == 0 {
        return Err(false);
    }
    *used += n;
    if *used > MAX_HEADER {
        return Err(true);
    }
    if l.last() != Some(&b'\n') {
        return Err(false);
    }
    l.pop();
    if l.last() == Some(&b'\r') {
        l.pop();
    }
    Ok(l)
}

/// Read one request's head (and drop its body), as Go's server reads
/// it.
pub fn read_request(conn: &TcpStream, r: &mut BufReader<TcpStream>) -> Got {
    // wait as long as it takes for a request to start, then 10 s for
    // its head (ReadHeaderTimeout)
    let _ = conn.set_read_timeout(None);
    match r.fill_buf() {
        Ok(b) if !b.is_empty() => {}
        _ => return Got::Gone,
    }
    let _ = conn.set_read_timeout(Some(Duration::from_secs(10)));
    let mut used = 0;
    let too_big = Got::Refused(Refused {
        code: 431,
        text: None,
    });
    let first = match line(r, &mut used) {
        Ok(l) => l,
        Err(true) => return too_big,
        Err(false) => return Got::Gone,
    };
    let mut parts = first.splitn(3, |&b| b == b' ');
    let (Some(method), Some(target), Some(proto)) = (parts.next(), parts.next(), parts.next())
    else {
        return Got::Refused(Refused::bad());
    };
    if !token(method) {
        return Got::Refused(Refused::bad());
    }
    let (major, minor) = match proto {
        b"HTTP/1.1" => (1, 1),
        b"HTTP/1.0" => (1, 0),
        p => {
            let v = p.strip_prefix(b"HTTP/").and_then(|v| {
                let s = std::str::from_utf8(v).ok()?;
                let (a, b) = s.split_once('.')?;
                Some((a.parse::<u32>().ok()?, b.parse::<u32>().ok()?))
            });
            match v {
                Some(v) => v,
                None => return Got::Refused(Refused::bad()),
            }
        }
    };
    let Some((url_host, path, raw_query)) = request_uri(target) else {
        return Got::Refused(Refused::bad());
    };
    let mut headers: Vec<(String, Vec<u8>)> = Vec::new();
    loop {
        let l = match line(r, &mut used) {
            Ok(l) => l,
            Err(true) => return too_big,
            Err(false) => return Got::Gone,
        };
        if l.is_empty() {
            break;
        }
        let Some(colon) = l.iter().position(|&b| b == b':') else {
            return Got::Refused(Refused::bad());
        };
        let name = &l[..colon];
        if !token(name) {
            return Got::Refused(Refused::bad());
        }
        let v = &l[colon + 1..];
        let a = v
            .iter()
            .position(|&b| b != b' ' && b != b'\t')
            .unwrap_or(v.len());
        let z = v
            .iter()
            .rposition(|&b| b != b' ' && b != b'\t')
            .map_or(a, |i| i + 1);
        headers.push((
            String::from_utf8_lossy(name).to_ascii_lowercase(),
            v[a..z.max(a)].to_vec(),
        ));
    }
    if major != 1 {
        return Got::Refused(Refused {
            code: 505,
            text: Some("unsupported protocol version"),
        });
    }
    let hosts: Vec<&Vec<u8>> = headers
        .iter()
        .filter(|(k, _)| k == "host")
        .map(|(_, v)| v)
        .collect();
    if minor >= 1 && hosts.is_empty() && method != b"CONNECT" {
        return Got::Refused(Refused {
            code: 400,
            text: Some("missing required Host header"),
        });
    }
    if hosts.len() > 1 {
        // http.ReadRequest's error, not a statusError: the bare 400
        return Got::Refused(Refused::bad());
    }
    if hosts.len() == 1 && !valid_host(hosts[0]) {
        return Got::Refused(Refused {
            code: 400,
            text: Some("malformed Host header"),
        });
    }
    if headers
        .iter()
        .any(|(_, v)| v.iter().any(|&b| (b < 0x20 && b != b'\t') || b == 0x7f))
    {
        return Got::Refused(Refused {
            code: 400,
            text: Some("invalid header value"),
        });
    }
    let get = |k: &str| headers.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
    // drop the body; a chunked one closes the connection after
    let mut keep = match get("connection").map(|v| v.to_ascii_lowercase()) {
        Some(c) if c.split(|&b| b == b',').any(|t| trim_space(t) == b"close") => false,
        Some(c) if minor == 0 => c
            .split(|&b| b == b',')
            .any(|t| trim_space(t) == b"keep-alive"),
        _ => minor >= 1,
    };
    if get("transfer-encoding").is_some() {
        keep = false;
    } else if let Some(n) = get("content-length") {
        let Some(n) = std::str::from_utf8(&n)
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
        else {
            return Got::Refused(Refused {
                code: 400,
                text: Some("bad Content-Length"),
            });
        };
        if n > 256 << 10 {
            keep = false;
        } else if std::io::copy(&mut r.take(n), &mut std::io::sink()).ok() != Some(n) {
            return Got::Gone;
        }
    }
    let host = if url_host.is_empty() {
        hosts.first().map(|h| h.to_vec()).unwrap_or_default()
    } else {
        url_host
    };
    Got::Req(
        Request {
            method: String::from_utf8_lossy(method).into_owned(),
            host,
            path,
            raw_query,
            minor: minor.min(1) as u8,
        },
        keep,
    )
}

/// `http.TimeFormat` of now
pub fn date() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    // Howard Hinnant's civil_from_days
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    const WD: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MO: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    format!(
        "{}, {:02} {} {} {:02}:{:02}:{:02} GMT",
        WD[days.rem_euclid(7) as usize],
        d,
        MO[(m - 1) as usize],
        y,
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// a header's value as Go writes it: a CR or LF a space, then trimmed
fn header_value(v: &[u8]) -> Vec<u8> {
    let v: Vec<u8> = v
        .iter()
        .map(|&b| if b == b'\r' || b == b'\n' { b' ' } else { b })
        .collect();
    let a = v
        .iter()
        .position(|b| !b" \t".contains(b))
        .unwrap_or(v.len());
    let z = v
        .iter()
        .rposition(|b| !b" \t".contains(b))
        .map_or(a, |i| i + 1);
    v[a..z.max(a)].to_vec()
}

/// a response on the wire, to an HTTP/1.minor request: the status
/// line, the handler's headers in order of name, then Connection as Go
/// says it (close, when an HTTP/1.1 connection closes after it;
/// keep-alive, when an HTTP/1.0 one stays open), Date and
/// Content-Length; no body for a HEAD
pub fn wire(resp: &Response, minor: u8, head: bool, close: bool) -> Vec<u8> {
    let mut out = format!(
        "HTTP/1.{minor} {} {}\r\n",
        resp.status,
        status_text(resp.status)
    )
    .into_bytes();
    let mut put = |k: &str, v: &[u8]| {
        out.extend_from_slice(k.as_bytes());
        out.extend_from_slice(b": ");
        out.extend_from_slice(&header_value(v));
        out.extend_from_slice(b"\r\n");
    };
    for (k, v) in &resp.headers {
        put(k, v);
    }
    match (minor, close) {
        (1, true) => put("Connection", b"close"),
        (0, false) => put("Connection", b"keep-alive"),
        _ => {}
    }
    put("Date", date().as_bytes());
    put("Content-Length", resp.body.len().to_string().as_bytes());
    out.extend_from_slice(b"\r\n");
    if !head {
        out.extend_from_slice(&resp.body);
    }
    out
}

/// Serve one connection: each request handed to handle, until it closes
pub fn serve_conn(conn: TcpStream, handle: &(dyn Fn(&Request) -> Response + Sync)) {
    let Ok(rd) = conn.try_clone() else {
        return;
    };
    let mut r = BufReader::new(rd);
    let mut w = &conn;
    loop {
        match read_request(&conn, &mut r) {
            Got::Gone => return,
            Got::Refused(e) => {
                let _ = w.write_all(&e.bytes());
                return;
            }
            Got::Req(req, keep) => {
                let resp = handle(&req);
                let head = req.method == "HEAD";
                if w.write_all(&wire(&resp, req.minor, head, !keep)).is_err() || !keep {
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queries_read_as_go_reads_them() {
        assert_eq!(
            parse_query(b"url=urb%3A%2F%2F~zod&raw&x=a+b&bad=%zz&semi=a;b&&url=2"),
            vec![
                (b"url".to_vec(), b"urb://~zod".to_vec()),
                (b"raw".to_vec(), b"".to_vec()),
                (b"x".to_vec(), b"a b".to_vec()),
                (b"url".to_vec(), b"2".to_vec()),
            ]
        );
        assert_eq!(unescape(b"/a%2Fb+c", false).unwrap(), b"/a/b+c");
        assert_eq!(unescape(b"/a%2", false), None);
    }

    #[test]
    fn hosts_split_as_go_splits_them() {
        assert_eq!(
            split_host_port(b"zod.urb.localhost:8475"),
            Some(&b"zod.urb.localhost"[..])
        );
        assert_eq!(split_host_port(b"[::1]:8475"), Some(&b"::1"[..]));
        assert_eq!(
            split_host_port(b"zod.urb.localhost:"),
            Some(&b"zod.urb.localhost"[..])
        );
        assert_eq!(split_host_port(b"zod.urb.localhost"), None);
        assert_eq!(split_host_port(b"a:b:c"), None);
    }

    #[test]
    fn trims_unicode_space() {
        assert_eq!(trim_space(" \u{a0}x y\t\u{3000}".as_bytes()), b"x y");
        assert_eq!(trim_space(b" \xffx "), b"\xffx");
        assert_eq!(trim_space(b"   "), b"");
    }

    #[test]
    fn redirects_as_go_redirects() {
        let r = redirect("GET", "http://zod.urb.localhost:8475/é\"".as_bytes(), 302);
        assert_eq!(
            r.header("Location"),
            Some(&b"http://zod.urb.localhost:8475/%c3%a9\""[..])
        );
        assert_eq!(
            r.body,
            "<a href=\"http://zod.urb.localhost:8475/é&#34;\">Found</a>.\n\n".as_bytes()
        );
        assert!(redirect("HEAD", b"http://x/", 302).body.is_empty());
    }
}
