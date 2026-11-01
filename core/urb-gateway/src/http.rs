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
use std::io::{BufRead, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
        417 => "Expectation Failed",
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

    fn code(code: u16) -> Refused {
        Refused { code, text: None }
    }

    fn with(code: u16, text: &'static str) -> Refused {
        Refused {
            code,
            text: Some(text),
        }
    }

    /// what Go writes to the connection, which it then closes
    pub fn bytes(&self) -> Vec<u8> {
        const HEADERS: &str =
            "\r\nContent-Type: text/plain; charset=utf-8\r\nConnection: close\r\n\r\n";
        if self.code == 501 {
            // a transfer coding Go doesn't take, not echoed back
            return format!("HTTP/1.1 501 Not Implemented{HEADERS}Unsupported transfer encoding")
                .into_bytes();
        }
        let status = match self.text {
            Some(t) => format!("{} {}: {t}", self.code, status_text(self.code)),
            None => format!("{} {}", self.code, status_text(self.code)),
        };
        format!("HTTP/1.1 {status}{HEADERS}{status}").into_bytes()
    }
}

/// a token (a method, a header's name), as httpguts reads one
fn token(s: &[u8]) -> bool {
    !s.is_empty() && s.iter().all(|&b| token_byte(b))
}

fn token_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)
}

/// httpguts.ValidHostHeader
fn valid_host(h: &[u8]) -> bool {
    h.iter()
        .all(|&b| b.is_ascii_alphanumeric() || b"!$%&()*+,-.:;=[]'_~".contains(&b))
}

/// textproto's validHeaderValueByte: a tab, a space, a visible ASCII
/// character or any byte past ASCII
fn value_byte(b: u8) -> bool {
    b == b'\t' || (0x20..0x7f).contains(&b) || b >= 0x80
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

/// `http.ParseHTTPVersion`: HTTP/1.1, HTTP/1.0, or HTTP/ and a digit,
/// a dot and a digit
fn http_version(p: &[u8]) -> Option<(u8, u8)> {
    match p {
        b"HTTP/1.1" => return Some((1, 1)),
        b"HTTP/1.0" => return Some((1, 0)),
        _ => {}
    }
    match p {
        [b'H', b'T', b'T', b'P', b'/', a, b'.', b] if a.is_ascii_digit() && b.is_ascii_digit() => {
            Some((a - b'0', b - b'0'))
        }
        _ => None,
    }
}

/// a header's key as textproto keeps it: canonical (`Content-Length`),
/// or as it came if it holds a space (which Go reads, and its server
/// then refuses); None if it is no key
fn header_key(k: &[u8]) -> Option<Vec<u8>> {
    if k.is_empty() || !k.iter().all(|&b| token_byte(b) || b == b' ') {
        return None;
    }
    if k.contains(&b' ') {
        return Some(k.to_vec());
    }
    let mut upper = true;
    Some(
        k.iter()
            .map(|&c| {
                let c = if upper {
                    c.to_ascii_uppercase()
                } else {
                    c.to_ascii_lowercase()
                };
                upper = c == b'-';
                c
            })
            .collect(),
    )
}

/// textproto's trim: spaces and tabs off each end
fn trim_ows(s: &[u8]) -> &[u8] {
    let a = s
        .iter()
        .position(|&b| b != b' ' && b != b'\t')
        .unwrap_or(s.len());
    let z = s
        .iter()
        .rposition(|&b| b != b' ' && b != b'\t')
        .map_or(a, |i| i + 1);
    &s[a..z.max(a)]
}

/// `textproto.TrimString`: ASCII space off each end
fn trim_ascii(s: &[u8]) -> &[u8] {
    s.trim_ascii()
}

/// httpguts.HeaderValuesContainsToken over the values given
fn has_token(values: &[&Vec<u8>], token: &[u8]) -> bool {
    values.iter().any(|v| {
        v.split(|&b| b == b',')
            .any(|t| trim_ows(t).eq_ignore_ascii_case(token))
    })
}

/// A request's body, as Go's readTransfer frames it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Body {
    /// none, or a Content-Length of 0
    None,
    /// a Content-Length
    Len(u64),
    /// Transfer-Encoding: chunked
    Chunked,
}

/// A request read, with what its answer needs from the head.
#[derive(Debug)]
pub struct Head {
    pub req: Request,
    /// the connection closes after the answer (Go's shouldClose)
    pub close: bool,
    pub body: Body,
    /// Expect: 100-continue, or another expectation Go fails with 417
    pub expect: Option<bool>,
    /// `OPTIONS *`, which Go's server answers itself
    pub options_star: bool,
}

/// What one read of a request gave.
#[derive(Debug)]
pub enum Got {
    Req(Box<Head>),
    /// refused: write this and close
    Refused(Refused),
    /// the connection closed or timed out: close, saying nothing
    Gone,
}

/// the most header bytes Go reads (DefaultMaxHeaderBytes, and its 4096
/// of slack)
const MAX_HEADER: usize = (1 << 20) + 4096;
/// the most header lines Go reads (DefaultMaxHeaderValueCount)
const MAX_HEADERS: usize = 500;

/// how reading one line ended, if it gave none
enum Cut {
    /// the connection closed (EOF) before the line began
    Eof,
    /// past the limit: Go's 431
    TooBig,
    /// a timeout or a read error: close, saying nothing
    Gone,
}

/// one line, without its \r\n or \n, as bufio.ReadLine reads it: a last
/// line with no \n before EOF is a line
fn line(r: &mut impl BufRead, used: &mut usize) -> Result<Vec<u8>, Cut> {
    let mut l = Vec::new();
    let n = r
        .take((MAX_HEADER - (*used).min(MAX_HEADER)) as u64 + 1)
        .read_until(b'\n', &mut l)
        .map_err(|_| Cut::Gone)?;
    if n == 0 {
        return Err(if *used >= MAX_HEADER {
            Cut::TooBig
        } else {
            Cut::Eof
        });
    }
    *used += n;
    if *used > MAX_HEADER {
        return Err(Cut::TooBig);
    }
    if l.last() == Some(&b'\n') {
        l.pop();
        if l.last() == Some(&b'\r') {
            l.pop();
        }
    }
    Ok(l)
}

/// the next byte, if one comes
fn peek_byte(r: &mut impl BufRead) -> Option<u8> {
    r.fill_buf().ok()?.first().copied()
}

/// Read one request's head, as Go's server reads it (http.readRequest,
/// textproto's readMIMEHeader, readTransfer, then conn.readRequest's
/// checks, in that order), leaving its body unread.
pub fn read_request(r: &mut impl BufRead) -> Got {
    let mut used = 0;
    let too_big = Got::Refused(Refused::code(431));
    let first = match line(r, &mut used) {
        Ok(l) => l,
        Err(Cut::TooBig) => return too_big,
        Err(_) => return Got::Gone,
    };
    // past the first line, an EOF is io.ErrUnexpectedEOF: a bare 400
    let cut = |c: Cut| match c {
        Cut::TooBig => Got::Refused(Refused::code(431)),
        Cut::Eof => Got::Refused(Refused::bad()),
        Cut::Gone => Got::Gone,
    };
    let mut parts = first.splitn(2, |&b| b == b' ');
    let (Some(method), Some(rest)) = (parts.next(), parts.next()) else {
        return Got::Refused(Refused::bad());
    };
    let Some(sp) = rest.iter().position(|&b| b == b' ') else {
        return Got::Refused(Refused::bad());
    };
    let (target, proto) = (&rest[..sp], &rest[sp + 1..]);
    if !token(method) {
        return Got::Refused(Refused::bad());
    }
    let Some((major, minor)) = http_version(proto) else {
        return Got::Refused(Refused::bad());
    };
    // CONNECT host:port names only an authority
    let authority = method == b"CONNECT" && !target.starts_with(b"/");
    let uri = if authority {
        request_uri(&[b"http://".as_slice(), target].concat())
    } else {
        request_uri(target)
    };
    let Some((url_host, path, raw_query)) = uri else {
        return Got::Refused(Refused::bad());
    };

    // readMIMEHeader: the first line can't start with a space
    if matches!(peek_byte(r), Some(b' ' | b'\t')) {
        return match line(r, &mut used) {
            Ok(l) if l.len() > 80 => too_big,
            Ok(_) => Got::Refused(Refused::bad()),
            Err(c) => cut(c),
        };
    }
    let mut headers: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    loop {
        let l = match line(r, &mut used) {
            Ok(l) => l,
            Err(c) => return cut(c),
        };
        if l.is_empty() {
            break;
        }
        if !l.contains(&b':') {
            return Got::Refused(Refused::bad());
        }
        // an obs-fold: each line after that starts with a space or tab
        // joins it, after one space
        let mut kv = trim_ows(&l).to_vec();
        while matches!(peek_byte(r), Some(b' ' | b'\t')) {
            while matches!(peek_byte(r), Some(b' ' | b'\t')) {
                r.consume(1);
                used += 1;
            }
            kv.push(b' ');
            match line(r, &mut used) {
                Ok(more) => kv.extend_from_slice(trim_ows(&more)),
                Err(Cut::TooBig) => return too_big,
                Err(_) => break,
            }
        }
        let colon = kv.iter().position(|&b| b == b':').expect("a colon");
        let (k, v) = (&kv[..colon], &kv[colon + 1..]);
        let Some(key) = header_key(k) else {
            return Got::Refused(Refused::bad());
        };
        if !v.iter().all(|&b| value_byte(b)) {
            return Got::Refused(Refused::bad());
        }
        if headers.len() >= MAX_HEADERS {
            return too_big;
        }
        let a = v
            .iter()
            .position(|&b| b != b' ' && b != b'\t')
            .unwrap_or(v.len());
        headers.push((key, v[a..].to_vec()));
    }
    let all = |k: &str| -> Vec<&Vec<u8>> {
        headers
            .iter()
            .filter(|(n, _)| n == k.as_bytes())
            .map(|(_, v)| v)
            .collect()
    };
    let hosts = all("Host");
    if hosts.len() > 1 {
        // "too many Host headers", not a statusError: the bare 400
        return Got::Refused(Refused::bad());
    }
    // shouldClose
    let conn = all("Connection");
    let close = if major < 1 {
        true
    } else if (major, minor) == (1, 0) {
        has_token(&conn, b"close") || !has_token(&conn, b"keep-alive")
    } else {
        has_token(&conn, b"close")
    };

    // readTransfer: a Transfer-Encoding (ignored before HTTP/1.1) must
    // be one chunked
    let te = all("Transfer-Encoding");
    let chunked = if te.is_empty() || (major, minor) < (1, 1) {
        false
    } else if te.len() != 1 || !te[0].eq_ignore_ascii_case(b"chunked") {
        return Got::Refused(Refused::code(501));
    } else {
        true
    };
    // fixLength: Content-Lengths that differ, an empty one, or one
    // that is no number (strconv.ParseUint's, so no sign) are a bare 400
    let cls = all("Content-Length");
    let mut len = None;
    if let Some(first) = cls.first() {
        let first = trim_ascii(first);
        if cls[1..].iter().any(|c| trim_ascii(c) != first) {
            return Got::Refused(Refused::bad());
        }
        if first.is_empty() || !first.iter().all(u8::is_ascii_digit) {
            return Got::Refused(Refused::bad());
        }
        match std::str::from_utf8(first)
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
        {
            Some(n) if n < 1 << 63 => len = Some(n),
            _ => return Got::Refused(Refused::bad()),
        }
    }
    // fixTrailer: a chunked body's trailer can't name these
    if chunked {
        for v in all("Trailer") {
            for k in v.split(|&b| b == b',') {
                let k = header_key(trim_ows(k)).unwrap_or_default();
                if [&b"Transfer-Encoding"[..], b"Trailer", b"Content-Length"].contains(&&k[..]) {
                    return Got::Refused(Refused::bad());
                }
            }
        }
    }
    let body = match (chunked, len) {
        (true, _) => Body::Chunked,
        (false, Some(n)) if n > 0 => Body::Len(n),
        _ => Body::None,
    };

    // conn.readRequest's checks
    if major != 1 {
        return Got::Refused(Refused::with(505, "unsupported protocol version"));
    }
    if minor >= 1 && hosts.is_empty() && method != b"CONNECT" {
        return Got::Refused(Refused::with(400, "missing required Host header"));
    }
    if hosts.len() == 1 && !valid_host(hosts[0]) {
        return Got::Refused(Refused::with(400, "malformed Host header"));
    }
    if headers.iter().any(|(k, _)| !token(k)) {
        return Got::Refused(Refused::with(400, "invalid header name"));
    }
    if headers
        .iter()
        .any(|(_, v)| v.iter().any(|&b| (b < 0x20 && b != b'\t') || b == 0x7f))
    {
        return Got::Refused(Refused::with(400, "invalid header value"));
    }
    let expect = all("Expect")
        .first()
        .map(|e| has_token(&[e], b"100-continue"));
    let host = if url_host.is_empty() {
        hosts.first().map(|h| h.to_vec()).unwrap_or_default()
    } else {
        url_host
    };
    Got::Req(Box::new(Head {
        options_star: method == b"OPTIONS" && target == b"*",
        req: Request {
            method: String::from_utf8_lossy(method).into_owned(),
            host,
            path,
            raw_query,
            minor: minor.min(1),
        },
        close,
        body,
        expect,
    }))
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

/// The server's timeouts. [`Limits::GO`] are the Go gateway's (main.go's
/// newServer): 10 s for a request's head, 10 s from its start for its
/// head and body (the body Go discards for a kept-alive connection),
/// and 10 s for the next request to start on a kept-alive connection.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub header: Duration,
    pub read: Duration,
    pub idle: Duration,
}

impl Limits {
    pub const GO: Limits = Limits {
        header: Duration::from_secs(10),
        read: Duration::from_secs(10),
        idle: Duration::from_secs(10),
    };
}

/// the most connections served at once, each on a thread of its own;
/// more wait to be accepted. A browser opens at most 256 sockets in
/// all, so no web page fills it, and the timeouts close every
/// connection that isn't being answered.
pub const MAX_CONNS: usize = 512;
/// the most of an unread body Go discards to keep a connection
/// (maxPostHandlerReadBytes)
const MAX_DISCARD: u64 = 256 << 10;

/// A connection's reads, buffered as bufio buffers them, each bounded by
/// a deadline as Go's SetReadDeadline bounds it: before each read from
/// the socket its timeout is what is left, and none left is a timeout.
/// What is buffered already is read whatever the time.
pub struct Conn {
    s: TcpStream,
    buf: Vec<u8>,
    at: usize,
    pub until: Option<Instant>,
}

impl Conn {
    pub fn new(s: TcpStream) -> Conn {
        Conn {
            s,
            buf: Vec::new(),
            at: 0,
            until: None,
        }
    }

    /// one read from the socket onto the buffer: how many bytes came.
    /// The deadline is kept by poll(2), to the millisecond, as Go's
    /// poller keeps it (SO_RCVTIMEO runs a few percent late).
    fn more(&mut self) -> std::io::Result<usize> {
        if let Some(t) = self.until {
            loop {
                let left = t.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return Err(std::io::ErrorKind::TimedOut.into());
                }
                let ms = left.as_micros().div_ceil(1000).min(i32::MAX as u128) as i32;
                if readable(&self.s, ms)? {
                    break;
                }
            }
        }
        if self.at > 0 {
            self.buf.drain(..self.at);
            self.at = 0;
        }
        let mut chunk = [0u8; 4096];
        let n = self.s.read(&mut chunk)?;
        self.buf.extend_from_slice(&chunk[..n]);
        Ok(n)
    }

    /// bufio's Peek(n): whether n bytes came before the deadline or EOF
    fn peek(&mut self, n: usize) -> bool {
        while self.buf.len() - self.at < n {
            if !matches!(self.more(), Ok(1..)) {
                return false;
            }
        }
        true
    }
}

impl Read for Conn {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let b = self.fill_buf()?;
        let n = b.len().min(out.len());
        out[..n].copy_from_slice(&b[..n]);
        self.consume(n);
        Ok(n)
    }
}

impl BufRead for Conn {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        if self.at == self.buf.len() {
            self.more()?;
        }
        Ok(&self.buf[self.at..])
    }

    fn consume(&mut self, n: usize) {
        self.at = (self.at + n).min(self.buf.len());
    }
}

#[repr(C)]
struct PollFd {
    fd: i32,
    events: i16,
    revents: i16,
}

unsafe extern "C" {
    fn poll(fds: *mut PollFd, n: std::ffi::c_ulong, timeout: i32) -> i32;
    fn listen(fd: i32, backlog: i32) -> i32;
}

/// Give ln the listen backlog Go's net.Listen gives (maxListenerBacklog:
/// net.core.somaxconn, at most 65535), not std's 128: past the backlog a
/// connection the client takes as open is never accepted.
pub fn go_backlog(ln: &TcpListener) {
    use std::os::fd::AsRawFd;
    let n = std::fs::read_to_string("/proc/sys/net/core/somaxconn")
        .ok()
        .and_then(|t| t.trim().parse::<i32>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(4096)
        .min(65535);
    unsafe { listen(ln.as_raw_fd(), n) };
}

/// whether s has something to read (bytes, an EOF or an error) within
/// ms milliseconds
fn readable(s: &TcpStream, ms: i32) -> std::io::Result<bool> {
    use std::os::fd::AsRawFd;
    const POLLIN: i16 = 1;
    let mut p = PollFd {
        fd: s.as_raw_fd(),
        events: POLLIN,
        revents: 0,
    };
    match unsafe { poll(&mut p, 1, ms) } {
        n if n > 0 => Ok(true),
        0 => Ok(false),
        _ => {
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::Interrupted {
                Ok(false)
            } else {
                Err(e)
            }
        }
    }
}

/// Serve one connection, as Go's server does, with Go's timeouts: each
/// request handed to handle, until it closes.
pub fn serve_conn(conn: TcpStream, handle: &(dyn Fn(&Request) -> Response + Sync)) {
    serve_conn_with(conn, handle, Limits::GO)
}

/// serve_conn, with these timeouts
pub fn serve_conn_with(
    conn: TcpStream,
    handle: &(dyn Fn(&Request) -> Response + Sync),
    lim: Limits,
) {
    let Ok(rd) = conn.try_clone() else {
        return;
    };
    let mut r = Conn::new(rd);
    let mut w = &conn;
    // a new connection's head is due from when it was accepted
    r.until = Some(Instant::now() + lim.header);
    let mut first = true;
    let mut after_post = false;
    loop {
        if !first {
            // kept alive: the next request starts within the idle
            // timeout, and its head is due from then
            r.until = Some(Instant::now() + lim.idle);
            if !r.peek(4) {
                return;
            }
            r.until = Some(Instant::now() + lim.header);
        }
        first = false;
        let t0 = Instant::now();
        if after_post {
            // after a POST, Go skips a CR or LF its client sent after
            // the body, in the next four bytes
            r.peek(4);
            let n = r.buf[r.at..]
                .iter()
                .take(4)
                .take_while(|&&b| b == b'\r' || b == b'\n')
                .count();
            r.consume(n);
        }
        let head = match read_request(&mut r) {
            Got::Gone => return,
            Got::Refused(e) => {
                let _ = w.write_all(&e.bytes());
                if e.code == 431 {
                    // closeWriteAndWait: the client may still be writing
                    let _ = conn.shutdown(std::net::Shutdown::Write);
                    std::thread::sleep(Duration::from_millis(500));
                }
                return;
            }
            Got::Req(h) => h,
        };
        // the body is due by the read timeout from the request's start
        r.until = Some(t0 + lim.read);
        let req = &head.req;
        let only_head = req.method == "HEAD";
        if head.expect == Some(false) {
            // an expectation Go doesn't meet: 417, and the end
            let resp = Response::new(417);
            let _ = w.write_all(&wire(&resp, req.minor, only_head, true));
            return;
        }
        let resp = if head.options_star {
            // Go's server answers OPTIONS * itself, with nothing
            Response::new(200)
        } else {
            handle(req)
        };
        // what Go does with an unread body before it answers: keep the
        // connection only if the body is small and comes in time
        let mut close = head.close;
        if !close {
            close = match head.body {
                Body::None => false,
                // 100-continue never sent: the body is still to come
                Body::Len(_) if head.expect == Some(true) && req.minor >= 1 => true,
                Body::Len(n) if n >= MAX_DISCARD => true,
                Body::Len(n) => {
                    std::io::copy(&mut (&mut r).take(n), &mut std::io::sink()).ok() != Some(n)
                }
                Body::Chunked => !discard_chunked(&mut r),
            };
        }
        if w.write_all(&wire(&resp, req.minor, only_head, close))
            .is_err()
            || close
        {
            return;
        }
        after_post = req.method == "POST";
    }
}

/// a chunked body read through and dropped, as Go's chunkedReader and
/// readTrailer read it: whether it ended, under MAX_DISCARD bytes
fn discard_chunked(r: &mut Conn) -> bool {
    let line = |r: &mut Conn| -> Option<Vec<u8>> {
        let mut l = Vec::new();
        r.take(4096).read_until(b'\n', &mut l).ok()?;
        l.strip_suffix(b"\n")
            .map(|l| l.strip_suffix(b"\r").unwrap_or(l).to_vec())
    };
    let mut total = 0u64;
    loop {
        let Some(l) = line(r) else {
            return false;
        };
        let l = l.trim_ascii_end();
        let size = match l.iter().position(|&b| b == b';') {
            Some(i) => l[..i].trim_ascii_end(),
            None => l,
        };
        if size.is_empty() || size.len() > 16 || !size.iter().all(u8::is_ascii_hexdigit) {
            return false;
        }
        let Ok(n) = u64::from_str_radix(std::str::from_utf8(size).unwrap_or("x"), 16) else {
            return false;
        };
        if n == 0 {
            break;
        }
        total += n;
        if total >= MAX_DISCARD {
            return false;
        }
        let mut end = [0u8; 2];
        if std::io::copy(&mut r.take(n), &mut std::io::sink()).ok() != Some(n)
            || r.read_exact(&mut end).is_err()
            || &end != b"\r\n"
        {
            return false;
        }
    }
    // the trailer, to its empty line
    loop {
        match line(r) {
            Some(l) if l.is_empty() => return true,
            Some(_) => {}
            None => return false,
        }
    }
}

/// how many connections are being served
#[derive(Default)]
pub struct Slots {
    n: Mutex<usize>,
    freed: Condvar,
}

/// one connection's slot, given back when its thread ends
struct Slot(Arc<Slots>);

impl Drop for Slot {
    fn drop(&mut self) {
        if let Ok(mut n) = self.0.n.lock() {
            *n -= 1;
        }
        self.0.freed.notify_one();
    }
}

impl Slots {
    /// wait for a free slot, of max
    fn take(self: &Arc<Self>, max: usize) -> Slot {
        let mut n = self.n.lock().unwrap_or_else(|e| e.into_inner());
        while *n >= max {
            n = self.freed.wait(n).unwrap_or_else(|e| e.into_inner());
        }
        *n += 1;
        Slot(self.clone())
    }

    /// how many are taken
    pub fn taken(&self) -> usize {
        *self.n.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Accept connections on ln, each served on a thread of its own, with
/// at most max at once (shared through slots) and these timeouts.
pub fn accept(
    ln: TcpListener,
    handle: &'static (dyn Fn(&Request) -> Response + Sync),
    slots: Arc<Slots>,
    max: usize,
    lim: Limits,
) {
    loop {
        let slot = slots.take(max);
        let Ok((c, _)) = ln.accept() else {
            continue;
        };
        let _ = std::thread::Builder::new().spawn(move || {
            let _slot = slot;
            serve_conn_with(c, handle, lim);
        });
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

    /// what read_request makes of a request's bytes: the status Go's
    /// server refuses it with and its words, or 0 for a request
    fn refused(bytes: &[u8]) -> (u16, Option<&'static str>) {
        match read_request(&mut std::io::Cursor::new(bytes.to_vec())) {
            Got::Refused(r) => (r.code, r.text),
            Got::Req(_) => (0, None),
            Got::Gone => (1, None),
        }
    }

    #[test]
    fn requests_are_refused_as_go_refuses_them() {
        let h = "Host: localhost:8475\r\n";
        let get = |extra: &str| format!("GET / HTTP/1.1\r\n{h}{extra}\r\n").into_bytes();
        // net/http's own errors are a bare 400
        assert_eq!(refused(&get("Content-Length: +0\r\n")), (400, None));
        assert_eq!(refused(&get("Content-Length: \r\n")), (400, None));
        assert_eq!(
            refused(&get("Content-Length: 0\r\nContent-Length: 5\r\n")),
            (400, None)
        );
        assert_eq!(
            refused(&get("Content-Length: 0\r\nContent-Length: 0\r\n")),
            (0, None)
        );
        assert_eq!(refused(&get("X: a\x01b\r\n")), (400, None));
        assert_eq!(refused(&get("Nocolon\r\n")), (400, None));
        assert_eq!(
            refused(format!("GET / HTTP/1.10\r\n{h}\r\n").as_bytes()),
            (400, None)
        );
        assert_eq!(
            refused(format!("GET / HTTP/1.1\r\n X: y\r\n{h}\r\n").as_bytes()),
            (400, None)
        );
        // an EOF past the request line is io.ErrUnexpectedEOF
        assert_eq!(refused(b"GET / HTTP/1.1\r\nHost: x"), (400, None));
        assert_eq!(refused(b""), (1, None));
        // a transfer coding it doesn't take, HTTP/1.1 on
        assert_eq!(refused(&get("Transfer-Encoding: gzip\r\n")), (501, None));
        assert_eq!(
            refused(&get(
                "Transfer-Encoding: chunked\r\nTransfer-Encoding: chunked\r\n"
            )),
            (501, None)
        );
        assert_eq!(
            refused(format!("GET / HTTP/1.0\r\n{h}Transfer-Encoding: gzip\r\n\r\n").as_bytes()),
            (0, None)
        );
        assert!(
            Refused::code(501)
                .bytes()
                .ends_with(b"\r\n\r\nUnsupported transfer encoding")
        );
        // the server's own checks, with words
        assert_eq!(
            refused(&get("X A: b\r\n")),
            (400, Some("invalid header name"))
        );
        assert_eq!(
            refused(b"GET / HTTP/2.0\r\n\r\n"),
            (505, Some("unsupported protocol version"))
        );
        assert_eq!(
            refused(b"GET / HTTP/0.9\r\n\r\n"),
            (505, Some("unsupported protocol version"))
        );
        assert_eq!(
            refused(b"GET / HTTP/1.1\r\n\r\n"),
            (400, Some("missing required Host header"))
        );
        // an obs-fold joins the line before it
        assert_eq!(refused(&get("X-A: a\r\n b\r\n")), (0, None));
        // too many header lines: Go's 431
        let many: String = (0..501).map(|i| format!("X-{i}: 1\r\n")).collect();
        assert_eq!(refused(&get(&many)), (431, None));
        let many: String = (0..499).map(|i| format!("X-{i}: 1\r\n")).collect();
        assert_eq!(refused(&get(&many)), (0, None));
        // OPTIONS * is the server's own
        let Got::Req(o) = read_request(&mut std::io::Cursor::new(
            format!("OPTIONS * HTTP/1.1\r\n{h}\r\n").into_bytes(),
        )) else {
            panic!()
        };
        assert!(o.options_star);
        let Got::Req(e) = read_request(&mut std::io::Cursor::new(get("Expect: tea\r\n"))) else {
            panic!()
        };
        assert_eq!(e.expect, Some(false));
    }

    /// a gateway on a loopback port with short timeouts and at most max
    /// connections: its address and its slots
    fn server(max: usize, lim: Limits) -> (std::net::SocketAddr, Arc<Slots>) {
        let ln = TcpListener::bind("127.0.0.1:0").unwrap();
        go_backlog(&ln);
        let at = ln.local_addr().unwrap();
        let slots = Arc::new(Slots::default());
        let s = slots.clone();
        let handle: &'static (dyn Fn(&Request) -> Response + Sync) = &|_: &Request| {
            let mut r = Response::new(200);
            r.body = b"ok".to_vec();
            r
        };
        std::thread::spawn(move || accept(ln, handle, s, max, lim));
        (at, slots)
    }

    const SHORT: Limits = Limits {
        header: Duration::from_millis(1000),
        read: Duration::from_millis(1000),
        idle: Duration::from_millis(1000),
    };

    /// a request on a connection of its own, and how long it took
    fn ask(at: std::net::SocketAddr) -> (Vec<u8>, Duration) {
        let t = Instant::now();
        let mut c = TcpStream::connect(at).unwrap();
        c.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut out = Vec::new();
        c.read_to_end(&mut out).unwrap();
        (out, t.elapsed())
    }

    /// when c closes (EOF or reset), from t, reading what comes
    fn closed(mut c: TcpStream, t: Instant) -> Duration {
        c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut b = [0u8; 4096];
        loop {
            match c.read(&mut b) {
                Ok(0) | Err(_) => return t.elapsed(),
                Ok(_) => {}
            }
        }
    }

    #[test]
    fn slow_clients_are_dropped_on_time_and_others_still_answer() {
        let (at, slots) = server(256, SHORT);
        let t = Instant::now();
        let mut held = Vec::new();
        // idle, kept alive after an answer, an unsent body
        for i in 0..150 {
            let mut c = TcpStream::connect(at).unwrap();
            match i % 3 {
                0 => {}
                1 => c.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n").unwrap(),
                _ => c
                    .write_all(b"GET / HTTP/1.1\r\nHost: x\r\nContent-Length: 9\r\n\r\n")
                    .unwrap(),
            }
            held.push(std::thread::spawn(move || closed(c, t)));
        }
        // and heads trickled a byte at a time
        for _ in 0..30 {
            let c = TcpStream::connect(at).unwrap();
            let mut w = c.try_clone().unwrap();
            std::thread::spawn(move || {
                for b in b"GET / HTTP/1.1\r\nX-Pad: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" {
                    if w.write_all(&[*b]).is_err() {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            });
            held.push(std::thread::spawn(move || closed(c, t)));
        }
        // meanwhile a request answers at once
        std::thread::sleep(Duration::from_millis(200));
        let (got, took) = ask(at);
        assert!(got.starts_with(b"HTTP/1.1 200 OK\r\n"), "{got:?}");
        assert!(took < Duration::from_millis(500), "{took:?}");
        assert!(slots.taken() >= 180, "{}", slots.taken());
        // and each slow one is closed at its timeout, give or take
        for h in held {
            let d = h.join().unwrap();
            assert!(
                d >= Duration::from_millis(900) && d < Duration::from_millis(1600),
                "{d:?}"
            );
        }
        std::thread::sleep(Duration::from_millis(100));
        // the one the accept loop holds for the next connection
        assert_eq!(slots.taken(), 1);
    }

    #[test]
    fn no_more_than_max_are_served_at_once() {
        let (at, slots) = server(4, SHORT);
        let idle: Vec<TcpStream> = (0..10).map(|_| TcpStream::connect(at).unwrap()).collect();
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(slots.taken(), 4);
        // a request waits for a slot: the first four close at 1 s, the
        // next four take theirs, and it comes after
        let (got, took) = ask(at);
        assert!(got.starts_with(b"HTTP/1.1 200 OK\r\n"), "{got:?}");
        assert!(took >= Duration::from_millis(1500), "{took:?}");
        assert!(slots.taken() <= 4);
        drop(idle);
    }
}
