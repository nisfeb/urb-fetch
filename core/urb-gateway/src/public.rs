//! The public gateway (`-public <domain>`, docs/rust-core.md "Public
//! mode"): lattice pages, read only, for anyone with a browser, at
//! `https://<ship>.<domain>/<path>`. The loopback gateway
//! ([`crate::routes`]) is left exactly as it is; this is a handler of
//! its own around the same reading and rendering.
//!
//! The domain serves third parties' pages, so it is built as a
//! user-content domain:
//!
//! - each ship is its own origin, `<ship>.<domain>`; the apex is the
//!   home page, and every other Host is 421;
//! - the gateway's own pages (the chrome) are static HTML with no
//!   script, under a policy that allows none;
//! - a publisher's HTML is only ever in a sandboxed frame, whose
//!   document (`?frame`) is served under lattice's `+sandbox-csp`, never
//!   as a top-level page; `js`, `css` and any other mark are text;
//! - no answer sets a cookie, and every answer carries HSTS, nosniff,
//!   no-referrer, COOP, `X-Robots-Tag: noindex, nofollow` and
//!   `frame-ancestors 'self'`;
//! - every write or identity action is shown, disabled, with its reason;
//! - per-client and per-publisher limits, a blocklist, and, by default,
//!   only publishers who opted in;
//! - the log holds status, duration and ship: never a path or an IP.

use crate::http::{self, Request, Response};
use crate::limits::{self, Limits, retry_secs};
use crate::lists::List;
use crate::routes::{Fail, Page};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use urb_render::gateway::{self as render, Base};
use urb_render::ship;

/// Which publishers the gateway serves. Owner's decision (spec, "What
/// the owner must decide", 3); the default is the most conservative.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Consent {
    /// only ships (or pages) on the `-optin` list; with no list, none
    OptIn,
    /// every published page, unless blocklisted
    All,
}

impl Consent {
    pub fn parse(s: &str) -> Option<Consent> {
        match s {
            "optin" => Some(Consent::OptIn),
            "all" => Some(Consent::All),
            _ => None,
        }
    }
}

/// the public gateway's settings, from its flags
#[derive(Clone, Debug)]
pub struct Config {
    /// the apex the gateway serves, lower case, no port
    pub domain: String,
    /// for tests behind a proxy on a high port: the port links name
    pub port: String,
    /// where to report abuse: shown on every page
    pub abuse: String,
    /// where to get lattice
    pub get_lattice: String,
    /// the start list's urb:// url, read for the home page; empty: none
    pub start: String,
    pub consent: Consent,
    /// the largest body served
    pub max_body: usize,
    /// take the client from `X-Forwarded-For` when the peer is loopback
    pub trust_proxy: bool,
    pub limits: limits::Config,
}

/// the start list when `-start` is unset or can't be read. The owner
/// sets the real list (spec, "What the owner must decide", 6); until
/// then there is none, rather than a guess.
pub const START_FALLBACK: &[(&str, &str)] = &[];

/// how long a page is kept, matching the engine's revision-list cache
const PAGE_TTL: Duration = Duration::from_secs(60);
/// the most the page cache holds
const CACHE_BYTES: usize = 64 * 1024 * 1024;

/// A domain `-public` takes: a lower-case DNS name with at least one
/// dot and no port, each label 1 to 63 of `a-z0-9-`, not starting or
/// ending with `-`.
pub fn valid_domain(d: &str) -> bool {
    d.len() <= 253
        && d.contains('.')
        && d.split('.').all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
}

/// What a Host names in public mode.
#[derive(Debug, PartialEq, Eq)]
pub enum Host {
    /// the apex
    Home,
    /// `<ship>.<domain>`: the ship, with its `~`
    Ship(String),
    /// anything else: 421
    Foreign,
}

/// The Host header read for domain: its port dropped, its case
/// lowered (DNS names are not case-sensitive, and browsers send lower
/// case), then the apex, one label that is a ship's name, or foreign.
pub fn host_of(host: &[u8], domain: &str) -> Host {
    let h = http::split_host_port(host)
        .unwrap_or(host)
        .to_ascii_lowercase();
    let h = h.strip_suffix(b".").unwrap_or(&h);
    if h == domain.as_bytes() {
        return Host::Home;
    }
    let Some(label) = h
        .strip_suffix(domain.as_bytes())
        .and_then(|l| l.strip_suffix(b"."))
    else {
        return Host::Foreign;
    };
    let who = [b"~", label].concat();
    if label.is_empty() || label.contains(&b'.') || !ship::parses(&who) {
        return Host::Foreign;
    }
    Host::Ship(String::from_utf8_lossy(&who).into_owned())
}

/// the gateway's counters, for `/metrics`
#[derive(Default)]
pub struct Counters {
    pub requests: AtomicU64,
    pub reads: AtomicU64,
    pub cache_hits: AtomicU64,
    pub limited: AtomicU64,
    pub blocked: AtomicU64,
    pub busy: AtomicU64,
    pub failures: AtomicU64,
}

impl Counters {
    fn add(c: &AtomicU64) {
        c.fetch_add(1, Ordering::Relaxed);
    }

    /// the Prometheus text format
    pub fn text(&self) -> String {
        let g = |c: &AtomicU64| c.load(Ordering::Relaxed);
        format!(
            "# urb-gateway public mode\n\
             urb_gateway_requests_total {}\n\
             urb_gateway_reads_total {}\n\
             urb_gateway_cache_hits_total {}\n\
             urb_gateway_limited_total {}\n\
             urb_gateway_blocked_total {}\n\
             urb_gateway_busy_total {}\n\
             urb_gateway_failures_total {}\n",
            g(&self.requests),
            g(&self.reads),
            g(&self.cache_hits),
            g(&self.limited),
            g(&self.blocked),
            g(&self.busy),
            g(&self.failures),
        )
    }
}

/// pages read in the last minute, by url
#[derive(Default)]
struct Cache {
    pages: HashMap<String, (Instant, Result<Page, Fail>)>,
    bytes: usize,
}

impl Cache {
    fn size(r: &Result<Page, Fail>) -> usize {
        match r {
            Ok(p) => p.body.len(),
            Err((_, m)) => m.len(),
        }
    }

    fn get(&mut self, url: &str, now: Instant, ttl: Duration) -> Option<Result<Page, Fail>> {
        let (at, r) = self.pages.get(url)?;
        if now.saturating_duration_since(*at) < ttl {
            return Some(r.clone());
        }
        None
    }

    fn put(&mut self, url: &str, now: Instant, r: Result<Page, Fail>) {
        let n = Cache::size(&r);
        if n > CACHE_BYTES / 4 {
            return;
        }
        if self.bytes + n > CACHE_BYTES {
            self.pages
                .retain(|_, (at, _)| now.saturating_duration_since(*at) < PAGE_TTL);
            self.bytes = self.pages.values().map(|(_, r)| Cache::size(r)).sum();
        }
        while self.bytes + n > CACHE_BYTES {
            let Some(old) = self
                .pages
                .iter()
                .min_by_key(|(_, (at, _))| *at)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            if let Some((_, r)) = self.pages.remove(&old) {
                self.bytes -= Cache::size(&r);
            }
        }
        if let Some((_, r)) = self.pages.insert(url.to_string(), (now, r)) {
            self.bytes -= Cache::size(&r);
        }
        self.bytes += n;
    }
}

/// The public gateway.
pub struct Public<'a> {
    pub cfg: Config,
    /// the identity it reads as
    pub ship: String,
    pub fetch: &'a (dyn Fn(&str) -> Result<Page, Fail> + Sync),
    /// where its lines go: status, duration and ship, nothing else
    pub log: &'a (dyn Fn(&str) + Sync),
    pub now: &'a (dyn Fn() -> Instant + Sync),
    pub limits: Limits,
    pub blocklist: List,
    pub optin: List,
    pub counters: Counters,
    base: Base,
    cache: Mutex<Cache>,
}

// ------------------------------------------------------------ policies

/// the chrome's policy: no script, no plugin, nothing loaded but its
/// own inline style; framed by nothing but its own origin
pub const CHROME: &str = "default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; \
form-action 'self'; frame-ancestors 'self'";

/// the chrome around a publisher's HTML: as [`CHROME`], and the one
/// frame, from its own origin
pub const CHROME_FRAMED: &str = "default-src 'none'; style-src 'unsafe-inline'; \
frame-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'self'";

/// a publisher's HTML, in its frame: lattice's `+sandbox-csp`, and
/// framed only by its own ship's chrome
pub fn frame_policy() -> String {
    format!("{}; frame-ancestors 'self'", render::SANDBOX)
}

/// a page as text (`?raw`)
pub fn raw_policy() -> String {
    format!("{}; frame-ancestors 'self'", render::RAW)
}

/// the iframe's sandbox: the same as the `SANDBOX` policy's
const FRAME_SANDBOX: &str = "allow-scripts allow-forms allow-popups \
allow-top-navigation-by-user-activation";

/// the headers every public answer carries
pub fn harden(r: &mut Response) {
    r.set(
        "Strict-Transport-Security",
        "max-age=31536000; includeSubDomains",
    );
    r.set("X-Content-Type-Options", "nosniff");
    r.set("Referrer-Policy", "no-referrer");
    r.set("Cross-Origin-Opener-Policy", "same-origin");
    r.set("Cross-Origin-Resource-Policy", "same-origin");
    r.set("X-Robots-Tag", "noindex, nofollow");
    r.headers.remove("Set-Cookie");
    match r.header("Content-Security-Policy").map(<[u8]>::to_vec) {
        Some(p) if !p.windows(15).any(|w| w == b"frame-ancestors") => {
            r.set(
                "Content-Security-Policy",
                [&p[..], b"; frame-ancestors 'self'"].concat(),
            );
        }
        Some(_) => {}
        None => r.set(
            "Content-Security-Policy",
            "default-src 'none'; frame-ancestors 'self'",
        ),
    }
}

fn esc(s: &str) -> String {
    String::from_utf8_lossy(&render::html_escape(s.as_bytes())).into_owned()
}

/// a rendered document's body: what [`render::page`] put inside
/// `<main>`. The title before it is escaped, so the first
/// `</style></head><body><main>` is the document's own.
fn inner(doc: &[u8]) -> Vec<u8> {
    let head = b"</style></head><body><main>";
    let tail = b"</main></body></html>";
    let at = doc.windows(head.len()).position(|w| w == head);
    match at.and_then(|i| doc[i + head.len()..].strip_suffix(tail)) {
        Some(b) => b.to_vec(),
        None => render::html_escape(doc),
    }
}

const STYLE: &str = "<style>\
header.gw,footer.gw{font-size:14px;color:var(--mute)}\
header.gw{border-bottom:1px solid var(--mute);padding-bottom:.6rem;margin-bottom:1.2rem}\
header.gw .where{font-family:ui-monospace,monospace;word-break:break-all}\
ul.acts{list-style:none;padding:0;margin:.5rem 0;display:flex;flex-wrap:wrap;gap:.4rem 1rem}\
ul.acts button{font:inherit;padding:.1rem .6rem}\
footer.gw{border-top:1px solid var(--mute);margin-top:2rem;padding-top:.6rem}\
p.note{border-left:3px solid var(--mute);padding-left:.8rem}\
iframe.page{width:100%;height:80vh;border:1px solid var(--mute);border-radius:6px}\
</style>";

/// the write and identity actions, each shown disabled with its reason
pub const ACTIONS: &[(&str, &str)] = &[
    ("Bookmark", "Connect a ship to bookmark"),
    ("Comment", "Connect a ship to comment"),
    ("Follow", "Connect a ship to follow"),
];

impl<'a> Public<'a> {
    pub fn new(
        cfg: Config,
        ship: String,
        fetch: &'a (dyn Fn(&str) -> Result<Page, Fail> + Sync),
        log: &'a (dyn Fn(&str) + Sync),
        now: &'a (dyn Fn() -> Instant + Sync),
        blocklist: List,
        optin: List,
    ) -> Public<'a> {
        let mut base = Base::public(&cfg.domain);
        if !cfg.port.is_empty() {
            base.port = format!(":{}", cfg.port);
        }
        Public {
            limits: Limits::new(cfg.limits),
            cfg,
            ship,
            fetch,
            log,
            now,
            blocklist,
            optin,
            counters: Counters::default(),
            base,
            cache: Mutex::new(Cache::default()),
        }
    }

    /// `https://<domain>/`
    fn home_url(&self) -> String {
        format!("https://{}{}/", self.cfg.domain, self.base.port)
    }

    /// Every request: the answer, hardened, and its one log line.
    pub fn handle(&self, req: &Request) -> Response {
        let t = (self.now)();
        Counters::add(&self.counters.requests);
        let (who, mut r) = self.route(req, t);
        harden(&mut r);
        if r.header("Cache-Control").is_none() {
            r.set("Cache-Control", "no-store");
        }
        let ms = (self.now)().saturating_duration_since(t).as_millis();
        (self.log)(&format!(
            "{} {} {ms}ms",
            r.status,
            who.as_deref().unwrap_or("-")
        ));
        r
    }

    /// the answer, and the ship it was for
    fn route(&self, req: &Request, t: Instant) -> (Option<String>, Response) {
        if req.method != "GET" && req.method != "HEAD" {
            return (None, http::error("read only", 405));
        }
        let host = host_of(&req.host, &self.cfg.domain);
        if host == Host::Foreign {
            return (None, http::error("unknown host", 421));
        }
        let fwd: Vec<&[u8]> = req.headers("X-Forwarded-For").collect();
        let ip = limits::client(req.peer, &fwd, self.cfg.trust_proxy)
            .unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
        if let Err(wait) = self.limits.request(ip, t) {
            Counters::add(&self.counters.limited);
            let mut r = http::error("too many requests: wait a little, then try again", 429);
            r.set("Retry-After", retry_secs(wait).to_string());
            return (None, r);
        }
        if req.path == b"/robots.txt" {
            let mut r = Response::new(200);
            r.set("Content-Type", "text/plain; charset=utf-8");
            r.set("Cache-Control", "public, max-age=300");
            r.body = b"User-agent: *\nDisallow: /\n".to_vec();
            return (None, r);
        }
        match host {
            Host::Home => (None, self.apex(req, t)),
            Host::Ship(who) => {
                let r = self.ship_page(req, &who, t);
                (Some(who), r)
            }
            Host::Foreign => unreachable!(),
        }
    }

    // -------------------------------------------------------- the apex

    fn apex(&self, req: &Request, t: Instant) -> Response {
        match req.path.as_slice() {
            b"/" => self.home(t),
            b"/go" => self.go(req),
            _ => self.chrome_error(
                None,
                404,
                "There is nothing here. Pages live at https://<ship>.<domain>/<path>.",
                false,
            ),
        }
    }

    /// `/go?url=urb://~ship/page` (or `~ship/page`): to where it is served
    fn go(&self, req: &Request) -> Response {
        let raw = req.get(b"url");
        let u = http::trim_space(&raw);
        let u = if u.starts_with(b"~") {
            [b"urb://", u].concat()
        } else {
            u.to_vec()
        };
        match render::gateway_url_at(&u, &self.base) {
            Some(to) if u.starts_with(b"urb://~") => {
                let mut r = http::redirect(&req.method, &to, 302);
                r.set("Content-Security-Policy", CHROME);
                r
            }
            _ => self.chrome_error(
                None,
                400,
                &format!(
                    "{} is not an urb:// address. Addresses look like urb://~sampel-palnet/page.",
                    String::from_utf8_lossy(&raw)
                ),
                false,
            ),
        }
    }

    /// the start list's links, as HTML: read through the engine, or the
    /// compiled-in list
    fn start_list(&self, t: Instant) -> Vec<u8> {
        if !self.cfg.start.is_empty()
            && let Ok(pg) = self.read_cached(&self.cfg.start, t)
        {
            let url = self.cfg.start.as_bytes();
            let (csp, doc) = render::render_at(url, &pg.mark, &pg.body, &self.base);
            if csp == render::OWN {
                return inner(&doc);
            }
        }
        if START_FALLBACK.is_empty() {
            return b"<p>No start list yet.</p>".to_vec();
        }
        let mut out = b"<ul>".to_vec();
        for (title, url) in START_FALLBACK {
            if let Some(href) = render::gateway_url_at(url.as_bytes(), &self.base) {
                out.extend_from_slice(b"<li><a href=\"");
                out.extend_from_slice(&render::html_escape(&href));
                out.extend_from_slice(b"\">");
                out.extend_from_slice(esc(title).as_bytes());
                out.extend_from_slice(b"</a></li>");
            }
        }
        out.extend_from_slice(b"</ul>");
        out
    }

    fn home(&self, t: Instant) -> Response {
        let d = esc(&self.cfg.domain);
        let mut body = format!(
            "{STYLE}<h1>{d}</h1>\
<p>This site shows pages published with lattice on the Urbit network, to anyone with a browser. \
It only reads: to comment, bookmark or follow, connect a ship of your own.</p>\
<form action=\"/go\" method=\"get\"><label>An urb:// address \
<input name=\"url\" placeholder=\"urb://~sampel-palnet/page\"></label></form>\
<h2>Start here</h2>"
        )
        .into_bytes();
        body.extend_from_slice(&self.start_list(t));
        body.extend_from_slice(self.footer(None).as_bytes());
        let mut r = Response::new(200);
        r.set("Content-Type", "text/html; charset=utf-8");
        r.set("Content-Security-Policy", CHROME);
        r.set("Cache-Control", "public, max-age=300");
        r.body = render::page(self.cfg.domain.as_bytes(), &body);
        r
    }

    // -------------------------------------------------------- the chrome

    fn footer(&self, url: Option<&str>) -> String {
        let open = match url {
            Some(u) => format!("<a href=\"{}\">Open in lattice</a> &middot; ", esc(u)),
            None => String::new(),
        };
        let get = if self.cfg.get_lattice.is_empty() {
            String::new()
        } else {
            format!(
                "<a href=\"{}\" rel=\"noopener noreferrer\">Get lattice</a> &middot; ",
                esc(&self.cfg.get_lattice)
            )
        };
        format!(
            "<footer class=\"gw\"><p>{open}{get}Report abuse: {}</p></footer>",
            esc(&self.cfg.abuse)
        )
    }

    /// the strip at the top of a ship's page: where it is, the way
    /// home, and the actions, each disabled with its reason
    fn header(&self, url: &str) -> String {
        let mut acts = String::from("<ul class=\"acts\">");
        for (name, why) in ACTIONS {
            acts.push_str(&format!(
                "<li><button type=\"button\" disabled title=\"{why}\">{name}</button> <small>{why}</small></li>"
            ));
        }
        acts.push_str("</ul>");
        format!(
            "<header class=\"gw\"><a href=\"{}\">{}</a> &middot; <span class=\"where\">{}</span>{acts}</header>",
            esc(&self.home_url()),
            esc(&self.cfg.domain),
            esc(url),
        )
    }

    /// a ship's page in the chrome
    fn chrome(&self, url: &str, status: u16, csp: &str, main: &[u8]) -> Response {
        let mut body = format!("{STYLE}{}", self.header(url)).into_bytes();
        body.extend_from_slice(main);
        body.extend_from_slice(self.footer(Some(url)).as_bytes());
        let mut r = Response::new(status);
        r.set("Content-Type", "text/html; charset=utf-8");
        r.set("Content-Security-Policy", csp);
        r.body = render::page(url.as_bytes(), &body);
        r
    }

    /// an error, in the chrome (a ship's, given its url), its words
    /// escaped; later adds a retry link
    fn chrome_error(&self, url: Option<&str>, status: u16, words: &str, later: bool) -> Response {
        let mut main = format!("<h1>{status}</h1><p>{}</p>", esc(words));
        if later {
            main.push_str("<p><a href=\"\">Try again</a></p>");
        }
        match url {
            Some(u) => self.chrome(u, status, CHROME, main.as_bytes()),
            None => {
                let mut body = format!(
                    "{STYLE}<header class=\"gw\"><a href=\"{}\">{}</a></header>{main}",
                    esc(&self.home_url()),
                    esc(&self.cfg.domain)
                )
                .into_bytes();
                body.extend_from_slice(self.footer(None).as_bytes());
                let mut r = Response::new(status);
                r.set("Content-Type", "text/html; charset=utf-8");
                r.set("Content-Security-Policy", CHROME);
                r.body = render::page(self.cfg.domain.as_bytes(), &body);
                r
            }
        }
    }

    // -------------------------------------------------------- reading

    /// a read through the page cache: a page read in the last minute is
    /// answered from it, and only a read that reaches the engine counts
    /// against its publisher's limits
    fn read_cached(&self, url: &str, t: Instant) -> Result<Page, Fail> {
        if let Some(r) = self
            .cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(url, t, PAGE_TTL)
        {
            Counters::add(&self.counters.cache_hits);
            return r;
        }
        let who = url
            .strip_prefix("urb://")
            .and_then(|r| r.split('/').next())
            .unwrap_or_default();
        let slot = match self.limits.cold(who, t) {
            Ok(s) => s,
            Err(wait) => {
                Counters::add(&self.counters.busy);
                return Err((503, format!("{}", retry_secs(wait)).into_bytes()));
            }
        };
        Counters::add(&self.counters.reads);
        let mut got = (self.fetch)(url);
        drop(slot);
        if let Ok(pg) = &got
            && pg.body.len() > self.cfg.max_body
        {
            got = Err((
                502,
                format!(
                    "the page is {} MB, over the reader's {} MB",
                    pg.body.len() / (1024 * 1024),
                    self.cfg.max_body / (1024 * 1024)
                )
                .into_bytes(),
            ));
        }
        // only pages and "not published" are kept: an unanswered read is
        // tried again by the next visitor
        if got.is_ok() || matches!(&got, Err((404, _))) {
            self.cache.lock().unwrap_or_else(|e| e.into_inner()).put(
                url,
                (self.now)(),
                got.clone(),
            );
        }
        got
    }

    fn ship_page(&self, req: &Request, who: &str, t: Instant) -> Response {
        // as urbOfHost gives it: the ship, then the path as it came
        let url = [b"urb://", who.as_bytes(), &req.path[..]].concat();
        let url = String::from_utf8_lossy(&url).into_owned();
        let (ship, rel) = match render::de_urb(url.as_bytes()) {
            Ok(x) => x,
            Err((status, msg)) => return self.refused(&url, status, &msg),
        };
        let ship = String::from_utf8_lossy(&ship).into_owned();
        let rel: Vec<String> = rel
            .iter()
            .map(|s| String::from_utf8_lossy(s).into_owned())
            .collect();
        let path: Vec<&str> = rel.iter().map(String::as_str).collect();
        if self.blocklist.holds(&ship, &path) {
            Counters::add(&self.counters.blocked);
            return self.chrome_error(
                Some(&url),
                451,
                &format!(
                    "This page is not available on {}. To report a page, or to ask about this one: {}",
                    self.cfg.domain, self.cfg.abuse
                ),
                false,
            );
        }
        if self.cfg.consent == Consent::OptIn && !self.optin.holds(&ship, &path) {
            return self.chrome_error(
                Some(&url),
                403,
                &format!(
                    "{ship} has not chosen to have its pages read on {}. Open it in lattice to read it.",
                    self.cfg.domain
                ),
                false,
            );
        }
        let frame = req.has(b"frame");
        if frame {
            // the frame is only ever a frame: never a page of its own,
            // and only in its own ship's chrome
            let dest = req.headers("Sec-Fetch-Dest").next();
            let site = req.headers("Sec-Fetch-Site").next();
            if dest == Some(b"document") || site.is_some_and(|s| s != b"same-origin") {
                return self.chrome_error(
                    Some(&url),
                    403,
                    "This address is the frame a page is shown in, and is not a page itself.",
                    false,
                );
            }
        }
        let pg = match self.read_cached(&url, t) {
            Ok(pg) => pg,
            Err((503, secs)) => {
                let mut r = self.chrome_error(
                    Some(&url),
                    503,
                    &format!("{ship} is being read a lot just now. Try again in a few seconds."),
                    true,
                );
                r.set("Retry-After", secs);
                return r;
            }
            Err((status, msg)) => {
                Counters::add(&self.counters.failures);
                let words = String::from_utf8_lossy(&msg).into_owned();
                let words = if status == 404 {
                    format!(
                        "{words}. If it was shared with your ship rather than published, \
                         connect that ship to read it."
                    )
                } else {
                    words
                };
                return self.chrome_error(Some(&url), status, &words, status == 504);
            }
        };
        let kind = render::render_as(&pg.mark, &pg.body);
        let mut r = if frame {
            if kind != "html" {
                return self.chrome_error(Some(&url), 404, "This page has no frame.", false);
            }
            let mut r = Response::new(200);
            r.set("Content-Type", "text/html; charset=utf-8");
            r.set("Content-Security-Policy", frame_policy());
            r.body = render::rewrite_html_at(&pg.body, &self.base);
            r
        } else if req.has(b"raw") {
            let mut r = Response::new(200);
            r.set("Content-Type", "text/plain; charset=utf-8");
            r.set("Content-Security-Policy", raw_policy());
            r.body = pg.body;
            r
        } else if kind == "html" {
            let main = format!(
                "<p class=\"note\">This page is from {}, not from {}.</p>\
<iframe class=\"page\" sandbox=\"{FRAME_SANDBOX}\" referrerpolicy=\"no-referrer\" \
title=\"{}\" src=\"?frame\"></iframe>",
                esc(&ship),
                esc(&self.cfg.domain),
                esc(&url)
            );
            self.chrome(&url, 200, CHROME_FRAMED, main.as_bytes())
        } else {
            // gemtext, Markdown and text the gateway renders itself, as
            // the loopback gateway does; any other mark is text
            let (_, doc) = render::render_at(url.as_bytes(), &pg.mark, &pg.body, &self.base);
            self.chrome(&url, 200, CHROME, &inner(&doc))
        };
        r.set("X-Urb-Mark", pg.mark.as_bytes());
        r.set("Cache-Control", "public, max-age=60");
        r
    }

    /// an address the gateway does not read, in the chrome
    fn refused(&self, url: &str, status: u16, msg: &[u8]) -> Response {
        let m = String::from_utf8_lossy(msg);
        let words = if m.contains("is a lattice forum board") {
            "Reading boards on the web is coming. Open in lattice to read it now.".to_string()
        } else if m.starts_with("/k/") || m.starts_with("/t/") {
            "This page is private. Connect the ship it was shared with.".to_string()
        } else if m.starts_with("/p/") {
            "This is one of lattice's live pages, which only lattice shows. Open in lattice to read it."
                .to_string()
        } else {
            m.into_owned()
        };
        self.chrome_error(Some(url), status, &words, false)
    }

    /// `/metrics`, for the loopback-only listener
    pub fn metrics(&self, req: &Request) -> Response {
        if req.path != b"/metrics" {
            return http::error("not found", 404);
        }
        let mut r = Response::new(200);
        r.set("Content-Type", "text/plain; version=0.0.4; charset=utf-8");
        r.set("Cache-Control", "no-store");
        r.body = self.counters.text().into_bytes();
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;

    const HOSTILE: &str = "<!doctype html><html><head><base href=\"https://evil.example/\">\
<meta http-equiv=\"refresh\" content=\"0;url=https://evil.example/\"></head><body>\
<script>parent.document.body.innerHTML='owned';document.cookie='a=b'</script>\
<form action=\"https://evil.example/steal\"><input name=p></form>\
<a href=\"urb://~nec/x\">nec</a></body></html>";

    fn cfg() -> Config {
        Config {
            domain: "test.example".into(),
            port: String::new(),
            abuse: "abuse@test.example".into(),
            get_lattice: "https://lattice.test.example/get".into(),
            start: String::new(),
            consent: Consent::All,
            max_body: 4 * 1024 * 1024,
            trust_proxy: false,
            limits: limits::Config::default(),
        }
    }

    fn page(mark: &str, body: &str) -> Result<Page, Fail> {
        Ok(Page {
            key: "/k".into(),
            mark: mark.into(),
            body: body.as_bytes().to_vec(),
            how: "keen".into(),
        })
    }

    /// the fleet's pages, by url
    fn pages(url: &str) -> Result<Page, Fail> {
        match url {
            "urb://~zod" | "urb://~zod/" => page("md", "# home"),
            "urb://~zod/gmi" => page("gmi", "# hi\n=> urb://~nec/x nec\n=> /rel rel"),
            "urb://~zod/md" => page("md", "# md\n\n<script>alert(1)</script> [x](urb://~nec/y)"),
            "urb://~zod/txt" => page("txt", "plain <b>text</b>"),
            "urb://~zod/js" => page(
                "js",
                "alert(document.cookie)</pre><script>alert(1)</script>",
            ),
            "urb://~zod/css" => page("css", "body{background:url(javascript:alert(1))}"),
            "urb://~zod/noun" => page("noun", "<script>x</script>"),
            "urb://~zod/html" => page("html", HOSTILE),
            "urb://~zod/sniffed" => page("gmi", HOSTILE),
            "urb://~zod/start" => page("gmi", "=> urb://~nec/a A page\n=> urb://~zod/md Another"),
            "urb://~zod/slow" => Err((
                504,
                b"~zod did not answer within 30s. It may be offline".to_vec(),
            )),
            u => Err((404, format!("{u} is not published").into_bytes())),
        }
    }

    struct Rig {
        log: Arc<Mutex<Vec<String>>>,
        reads: Arc<AtomicUsize>,
    }

    fn rig(cfg: Config, block: Option<&str>, optin: Option<&str>) -> (Public<'static>, Rig) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let reads = Arc::new(AtomicUsize::new(0));
        let (l2, r2) = (log.clone(), reads.clone());
        let fetch: &'static (dyn Fn(&str) -> Result<Page, Fail> + Sync) =
            Box::leak(Box::new(move |u: &str| {
                r2.fetch_add(1, Ordering::SeqCst);
                pages(u)
            }));
        let logf: &'static (dyn Fn(&str) + Sync) =
            Box::leak(Box::new(move |l: &str| l2.lock().unwrap().push(l.into())));
        let list = |text: Option<&str>, name: &str| {
            let p = text.map(|t| {
                let p = std::env::temp_dir().join(format!(
                    "urb-public-{}-{name}-{}",
                    std::process::id(),
                    reads_id()
                ));
                std::fs::write(&p, t).unwrap();
                p
            });
            let l = List::new(p.clone(), Box::new(|_| {}));
            // read once; a list keeps what it held when its file goes
            if let Some(p) = p {
                std::fs::remove_file(p).unwrap();
            }
            l
        };
        let p = Public::new(
            cfg,
            "~sampel-palnet".into(),
            fetch,
            logf,
            &Instant::now,
            list(block, "block"),
            list(optin, "optin"),
        );
        (p, Rig { log, reads })
    }

    fn reads_id() -> usize {
        static N: AtomicUsize = AtomicUsize::new(0);
        N.fetch_add(1, Ordering::SeqCst)
    }

    fn get(host: &str, path: &str, q: &str) -> Request {
        Request {
            method: "GET".into(),
            host: host.into(),
            path: path.into(),
            raw_query: q.into(),
            minor: 1,
            peer: Some("192.0.2.1".parse().unwrap()),
            headers: Vec::new(),
        }
    }

    fn with(mut r: Request, k: &str, v: &str) -> Request {
        r.headers.push((k.into(), v.into()));
        r
    }

    fn body(r: &Response) -> String {
        String::from_utf8_lossy(&r.body).into_owned()
    }

    fn csp(r: &Response) -> String {
        String::from_utf8_lossy(r.header("Content-Security-Policy").unwrap_or_default()).into()
    }

    #[test]
    fn domains_are_lower_case_dns_names_with_a_dot() {
        for d in ["test.example", "a-b.c0.example", "x.y"] {
            assert!(valid_domain(d), "{d}");
        }
        for d in [
            "example",
            "Test.example",
            "test.example:443",
            "-a.example",
            "a..b",
            "a.b.",
            "a_b.example",
            "",
        ] {
            assert!(!valid_domain(d), "{d}");
        }
    }

    #[test]
    fn the_host_rule() {
        let d = "test.example";
        let comet = "batdux-sibnem-hobfyn-ripsyn--rollec-midmul-namlet-litzod";
        assert_eq!(comet.len(), 56);
        let cases = [
            ("test.example", Host::Home),
            ("test.example:8443", Host::Home),
            ("zod.test.example", Host::Ship("~zod".into())),
            (
                "sampel-palnet.test.example",
                Host::Ship("~sampel-palnet".into()),
            ),
            (
                "doznec-dozzod-dozhus.test.example",
                Host::Ship("~doznec-dozzod-dozhus".into()),
            ),
            ("ZOD.Test.Example", Host::Ship("~zod".into())),
            ("a.zod.test.example", Host::Foreign),
            ("www.test.example", Host::Foreign),
            ("dozzod.test.example", Host::Foreign),
            ("zod.urb.localhost", Host::Foreign),
            ("urb.localhost", Host::Foreign),
            ("localhost", Host::Foreign),
            ("127.0.0.1", Host::Foreign),
            ("zodtest.example", Host::Foreign),
            ("evil.example", Host::Foreign),
            ("test.example.evil.example", Host::Foreign),
            (".test.example", Host::Foreign),
        ];
        for (h, want) in cases {
            assert_eq!(host_of(h.as_bytes(), d), want, "{h}");
        }
        assert_eq!(
            host_of(format!("{comet}.test.example").as_bytes(), d),
            Host::Ship(format!("~{comet}"))
        );
        let (p, _) = rig(cfg(), None, None);
        for h in [
            "zod.urb.localhost:8475",
            "localhost:8475",
            "127.0.0.1",
            "evil.example",
        ] {
            assert_eq!(p.handle(&get(h, "/", "")).status, 421, "{h}");
        }
        assert_eq!(p.handle(&get("zod.test.example", "/gmi", "")).status, 200);
    }

    #[test]
    fn links_point_at_public_hosts() {
        let b = Base::public("test.example");
        assert_eq!(
            render::gateway_url_at(b"urb://~nec/a/b", &b).unwrap(),
            b"https://nec.test.example/a/b"
        );
        let (p, _) = rig(cfg(), None, None);
        let g = body(&p.handle(&get("zod.test.example", "/gmi", "")));
        assert!(g.contains("href=\"https://nec.test.example/x\""), "{g}");
        assert!(g.contains("href=\"https://zod.test.example/rel\""), "{g}");
        let r = p.handle(&get("zod.test.example", "/html", "frame").clone());
        assert!(body(&r).contains("href=\"https://nec.test.example/x\""));
    }

    #[test]
    fn every_answer_has_the_user_content_headers_and_no_cookie() {
        let (p, _) = rig(cfg(), Some("~bus\n"), None);
        let reqs = [
            get("test.example", "/", ""),
            get("test.example", "/go", "url=urb://~zod/gmi"),
            get("test.example", "/go", "url=nope"),
            get("test.example", "/nothing", ""),
            get("test.example", "/robots.txt", ""),
            get("zod.test.example", "/robots.txt", ""),
            get("zod.test.example", "/gmi", ""),
            get("zod.test.example", "/md", ""),
            get("zod.test.example", "/txt", ""),
            get("zod.test.example", "/html", ""),
            get("zod.test.example", "/html", "frame"),
            get("zod.test.example", "/gmi", "raw"),
            get("zod.test.example", "/missing", ""),
            get("zod.test.example", "/forum/b", ""),
            get("bus.test.example", "/", ""),
            get("evil.example", "/", ""),
            Request {
                method: "POST".into(),
                ..get("zod.test.example", "/gmi", "")
            },
        ];
        for q in reqs {
            let r = p.handle(&q);
            let h = |k| String::from_utf8_lossy(r.header(k).unwrap_or_default()).into_owned();
            let what = format!(
                "{} {:?}?{:?}: {}",
                r.status,
                String::from_utf8_lossy(&q.host),
                String::from_utf8_lossy(&q.path),
                String::from_utf8_lossy(&q.raw_query)
            );
            assert_eq!(
                h("Strict-Transport-Security"),
                "max-age=31536000; includeSubDomains",
                "{what}"
            );
            assert_eq!(h("X-Content-Type-Options"), "nosniff", "{what}");
            assert_eq!(h("Referrer-Policy"), "no-referrer", "{what}");
            assert_eq!(h("Cross-Origin-Opener-Policy"), "same-origin", "{what}");
            assert_eq!(h("X-Robots-Tag"), "noindex, nofollow", "{what}");
            assert!(csp(&r).contains("frame-ancestors 'self'"), "{what}");
            assert!(r.header("Set-Cookie").is_none(), "{what}");
            assert!(r.header("Cache-Control").is_some(), "{what}");
        }
        let r = p.handle(&get("zod.test.example", "/robots.txt", ""));
        assert_eq!(body(&r), "User-agent: *\nDisallow: /\n");
    }

    #[test]
    fn the_chrome_has_no_script_and_the_actions_are_disabled() {
        let (p, _) = rig(cfg(), None, None);
        for path in [
            "/gmi", "/md", "/txt", "/js", "/css", "/noun", "/html", "/sniffed", "/missing",
            "/forum/b", "/slow",
        ] {
            let r = p.handle(&get("zod.test.example", path, ""));
            let b = body(&r).to_ascii_lowercase();
            assert!(!b.contains("<script"), "{path}: {b}");
            assert!(!b.contains("<base"), "{path}");
            assert!(!b.contains("http-equiv"), "{path}");
            assert!(!b.contains("evil.example"), "{path}");
            assert!(csp(&r).starts_with("default-src 'none'"), "{path}");
            assert!(!csp(&r).contains("script-src"), "{path}");
            for (name, why) in ACTIONS {
                assert!(
                    body(&r).contains(&format!(
                        "<button type=\"button\" disabled title=\"{why}\">{name}</button>"
                    )),
                    "{path}"
                );
            }
            assert!(
                body(&r).contains("Report abuse: abuse@test.example"),
                "{path}"
            );
            assert!(
                body(&r).contains(&format!("<a href=\"urb://~zod{path}\">Open in lattice</a>")),
                "{path}"
            );
            assert!(body(&r).contains("Get lattice"), "{path}");
        }
        let home = body(&p.handle(&get("test.example", "/", "")));
        assert!(!home.to_ascii_lowercase().contains("<script"));
    }

    #[test]
    fn a_publishers_html_is_only_in_the_sandboxed_frame() {
        let (p, _) = rig(cfg(), None, None);
        for path in ["/html", "/sniffed"] {
            let r = p.handle(&get("zod.test.example", path, ""));
            let b = body(&r);
            assert_eq!(r.status, 200);
            assert!(
                b.contains("This page is from ~zod, not from test.example."),
                "{b}"
            );
            assert!(b.contains(
                "<iframe class=\"page\" sandbox=\"allow-scripts allow-forms allow-popups allow-top-navigation-by-user-activation\""
            ));
            assert!(b.contains("src=\"?frame\""));
            assert!(!b.contains("allow-same-origin"));
            assert_eq!(csp(&r), CHROME_FRAMED);
            // the frame: the publisher's HTML, under the sandbox policy
            let f = p.handle(&with(
                get("zod.test.example", path, "frame"),
                "Sec-Fetch-Dest",
                "iframe",
            ));
            assert_eq!(f.status, 200);
            assert!(body(&f).contains("<script>parent.document"));
            assert_eq!(
                csp(&f),
                "sandbox allow-scripts allow-forms allow-popups allow-top-navigation-by-user-activation; frame-ancestors 'self'"
            );
            // never a page of its own, nor framed from another site
            let top = with(
                get("zod.test.example", path, "frame"),
                "Sec-Fetch-Dest",
                "document",
            );
            assert_eq!(p.handle(&top).status, 403);
            let far = with(
                get("zod.test.example", path, "frame"),
                "Sec-Fetch-Site",
                "cross-site",
            );
            assert_eq!(p.handle(&far).status, 403);
            let same = with(
                get("zod.test.example", path, "frame"),
                "Sec-Fetch-Site",
                "same-origin",
            );
            assert_eq!(p.handle(&same).status, 200);
        }
        // a page that is not HTML has no frame
        assert_eq!(
            p.handle(&get("zod.test.example", "/gmi", "frame")).status,
            404
        );
        // ?raw is text, under the sandbox
        let r = p.handle(&get("zod.test.example", "/html", "raw"));
        assert_eq!(
            r.header("Content-Type"),
            Some(&b"text/plain; charset=utf-8"[..])
        );
        assert_eq!(csp(&r), "sandbox; frame-ancestors 'self'");
    }

    #[test]
    fn marks_render_as_the_loopback_gateway_renders_them() {
        let (p, _) = rig(cfg(), None, None);
        let b = body(&p.handle(&get("zod.test.example", "/md", "")));
        assert!(
            b.contains("<h1>md</h1>") && b.contains("&lt;script&gt;"),
            "{b}"
        );
        let b = body(&p.handle(&get("zod.test.example", "/gmi", "")));
        assert!(b.contains("<h1>hi</h1>"), "{b}");
        for path in ["/js", "/css", "/noun", "/txt"] {
            let b = body(&p.handle(&get("zod.test.example", path, "")));
            assert!(b.contains("<pre>"), "{path}: {b}");
            assert!(!b.contains("<script>") && !b.contains("<b>"), "{path}");
        }
        let r = p.handle(&get("zod.test.example", "/js", ""));
        assert_eq!(
            r.header("Content-Type"),
            Some(&b"text/html; charset=utf-8"[..])
        );
    }

    #[test]
    fn errors_have_the_gateways_words_in_the_chrome() {
        let (p, _) = rig(cfg(), None, None);
        let r = p.handle(&get("zod.test.example", "/missing", ""));
        assert_eq!(r.status, 404);
        assert!(body(&r).contains("urb://~zod/missing is not published."));
        assert_eq!(r.header("Cache-Control"), Some(&b"no-store"[..]));
        let r = p.handle(&get("zod.test.example", "/slow", ""));
        assert_eq!(r.status, 504);
        assert!(body(&r).contains("<a href=\"\">Try again</a>"));
        let r = p.handle(&get("zod.test.example", "/forum/b", ""));
        assert_eq!(r.status, 501);
        assert!(
            body(&r)
                .contains("Reading boards on the web is coming. Open in lattice to read it now.")
        );
        for m in ["k", "t"] {
            let r = p.handle(&get("zod.test.example", &format!("/{m}/a"), ""));
            assert_eq!(r.status, 501);
            assert!(
                body(&r).contains("This page is private. Connect the ship it was shared with.")
            );
        }
        assert_eq!(p.handle(&get("zod.test.example", "/Bad", "")).status, 400);
    }

    #[test]
    fn the_home_page_and_go() {
        let (p, r) = rig(cfg(), None, None);
        let h = p.handle(&get("test.example", "/", ""));
        assert_eq!(h.status, 200);
        assert_eq!(h.header("Cache-Control"), Some(&b"public, max-age=300"[..]));
        let b = body(&h);
        assert!(b.contains("<form action=\"/go\" method=\"get\">") && b.contains("name=\"url\""));
        assert!(
            b.contains("No start list yet."),
            "no -start, no compiled-in list"
        );
        assert_eq!(r.reads.load(Ordering::SeqCst), 0);
        for (q, to) in [
            ("url=urb://~zod/gmi", "https://zod.test.example/gmi"),
            ("url=%20urb://~nec/a/b%20", "https://nec.test.example/a/b"),
            ("url=~nec/a", "https://nec.test.example/a"),
            ("url=urb://~nec", "https://nec.test.example/"),
        ] {
            let g = p.handle(&get("test.example", "/go", q));
            assert_eq!(
                (g.status, g.header("Location")),
                (302, Some(to.as_bytes())),
                "{q}"
            );
        }
        for q in [
            "url=nope",
            "url=urb://~dozzod/x",
            "url=https://evil.example/",
            "",
        ] {
            assert_eq!(p.handle(&get("test.example", "/go", q)).status, 400, "{q}");
        }
        // with a start list
        let (p, _) = rig(
            Config {
                start: "urb://~zod/start".into(),
                ..cfg()
            },
            None,
            None,
        );
        let b = body(&p.handle(&get("test.example", "/", "")));
        assert!(
            b.contains("<a href=\"https://nec.test.example/a\">A page</a>"),
            "{b}"
        );
        // one that can't be read falls back
        let (p, _) = rig(
            Config {
                start: "urb://~zod/slow".into(),
                ..cfg()
            },
            None,
            None,
        );
        assert!(body(&p.handle(&get("test.example", "/", ""))).contains("No start list yet."));
    }

    #[test]
    fn clients_are_limited_and_told_when_to_come_back() {
        let (p, _) = rig(
            Config {
                limits: limits::Config {
                    burst: 3,
                    ..limits::Config::default()
                },
                ..cfg()
            },
            None,
            None,
        );
        for _ in 0..3 {
            assert_eq!(p.handle(&get("zod.test.example", "/gmi", "")).status, 200);
        }
        let r = p.handle(&get("zod.test.example", "/gmi", ""));
        assert_eq!((r.status, r.header("Retry-After")), (429, Some(&b"1"[..])));
        // another client is not
        let mut other = get("zod.test.example", "/gmi", "");
        other.peer = Some("192.0.2.2".parse().unwrap());
        assert_eq!(p.handle(&other).status, 200);
        // X-Forwarded-For is not trusted without -trust-proxy
        let mut lo = with(
            get("zod.test.example", "/gmi", ""),
            "X-Forwarded-For",
            "192.0.2.77",
        );
        lo.peer = Some("127.0.0.1".parse().unwrap());
        for _ in 0..3 {
            assert_eq!(p.handle(&lo).status, 200);
        }
        let mut lo2 = with(
            get("zod.test.example", "/gmi", ""),
            "X-Forwarded-For",
            "192.0.2.78",
        );
        lo2.peer = lo.peer;
        assert_eq!(p.handle(&lo2).status, 429, "one client: the proxy");
    }

    #[test]
    fn behind_a_trusted_proxy_each_forwarded_client_has_its_own_bucket() {
        let (p, _) = rig(
            Config {
                trust_proxy: true,
                limits: limits::Config {
                    burst: 1,
                    ..limits::Config::default()
                },
                ..cfg()
            },
            None,
            None,
        );
        let at = |ip: &str| {
            let mut r = with(get("zod.test.example", "/gmi", ""), "X-Forwarded-For", ip);
            r.peer = Some("127.0.0.1".parse().unwrap());
            p.handle(&r).status
        };
        assert_eq!(at("192.0.2.10"), 200);
        assert_eq!(at("192.0.2.10"), 429);
        assert_eq!(at("192.0.2.11"), 200);
    }

    #[test]
    fn a_publisher_is_read_at_most_so_often_and_the_cache_does_not_count() {
        let (p, r) = rig(
            Config {
                limits: limits::Config {
                    ship_rate: 2,
                    burst: 100,
                    ..limits::Config::default()
                },
                ..cfg()
            },
            None,
            None,
        );
        assert_eq!(p.handle(&get("zod.test.example", "/gmi", "")).status, 200);
        assert_eq!(p.handle(&get("zod.test.example", "/md", "")).status, 200);
        // cached: answered, and the engine not asked
        for _ in 0..5 {
            assert_eq!(p.handle(&get("zod.test.example", "/gmi", "")).status, 200);
        }
        assert_eq!(r.reads.load(Ordering::SeqCst), 2);
        let busy = p.handle(&get("zod.test.example", "/txt", ""));
        assert_eq!(busy.status, 503);
        assert!(busy.header("Retry-After").is_some());
        assert_eq!(r.reads.load(Ordering::SeqCst), 2);
        // another publisher is not held up
        assert_eq!(p.handle(&get("nec.test.example", "/xx", "")).status, 404);
        let m = p.metrics(&get("127.0.0.1", "/metrics", ""));
        let m = body(&m);
        assert!(
            m.contains("urb_gateway_reads_total 3")
                && m.contains("urb_gateway_cache_hits_total 5")
                && m.contains("urb_gateway_busy_total 1"),
            "{m}"
        );
    }

    #[test]
    fn a_body_over_the_cap_is_refused() {
        let (p, _) = rig(
            Config {
                max_body: 10,
                ..cfg()
            },
            None,
            None,
        );
        let r = p.handle(&get("zod.test.example", "/gmi", ""));
        assert_eq!(r.status, 502);
        assert!(
            body(&r).contains("over the reader&#39;s 0 MB"),
            "{}",
            body(&r)
        );
    }

    #[test]
    fn the_blocklist_answers_451_before_the_engine_is_asked() {
        let (p, r) = rig(cfg(), Some("~bus\n~zod/md\n~zod/gm\n"), None);
        let b = p.handle(&get("bus.test.example", "/anything", ""));
        assert_eq!(b.status, 451);
        assert!(body(&b).contains("abuse@test.example"));
        assert_eq!(p.handle(&get("zod.test.example", "/md", "")).status, 451);
        assert_eq!(p.handle(&get("zod.test.example", "/n/md", "")).status, 451);
        assert_eq!(
            p.handle(&get("zod.test.example", "/md", "frame")).status,
            451
        );
        assert_eq!(r.reads.load(Ordering::SeqCst), 0);
        // on segment boundaries: ~zod/gm is not ~zod/gmi
        assert_eq!(p.handle(&get("zod.test.example", "/gmi", "")).status, 200);
    }

    #[test]
    fn by_default_only_publishers_who_opted_in_are_served() {
        let (p, r) = rig(
            Config {
                consent: Consent::OptIn,
                ..cfg()
            },
            None,
            None,
        );
        let x = p.handle(&get("zod.test.example", "/gmi", ""));
        assert_eq!(x.status, 403);
        assert!(body(&x).contains("~zod has not chosen to have its pages read on test.example."));
        assert_eq!(r.reads.load(Ordering::SeqCst), 0);
        let (p, _) = rig(
            Config {
                consent: Consent::OptIn,
                ..cfg()
            },
            Some("~zod/md\n"),
            Some("~zod\n~nec/aa\n"),
        );
        assert_eq!(p.handle(&get("zod.test.example", "/gmi", "")).status, 200);
        // the blocklist wins
        assert_eq!(p.handle(&get("zod.test.example", "/md", "")).status, 451);
        assert_eq!(p.handle(&get("nec.test.example", "/aa", "")).status, 404);
        assert_eq!(p.handle(&get("nec.test.example", "/bb", "")).status, 403);
    }

    #[test]
    fn the_log_holds_no_path_query_or_address() {
        let (p, r) = rig(
            Config {
                trust_proxy: true,
                ..cfg()
            },
            Some("~bus\n"),
            None,
        );
        let mut reqs = vec![
            get("zod.test.example", "/gmi", "secret=1"),
            get("zod.test.example", "/missing-page-xyz", ""),
            get("zod.test.example", "/html", "frame"),
            get("bus.test.example", "/blocked-path", ""),
            get("test.example", "/go", "url=urb://~zod/go-target"),
            get("evil.example", "/foreign-path", ""),
        ];
        let mut fwd = with(
            get("zod.test.example", "/fwd-path", ""),
            "X-Forwarded-For",
            "198.51.100.23",
        );
        fwd.peer = Some("127.0.0.1".parse().unwrap());
        reqs.push(fwd);
        for q in &reqs {
            p.handle(q);
        }
        let log = r.log.lock().unwrap();
        assert_eq!(log.len(), reqs.len());
        for l in log.iter() {
            for bad in [
                "/",
                "?",
                "secret",
                "missing",
                "blocked",
                "go-target",
                "foreign",
                "fwd",
                "192.0.2",
                "198.51",
                "127.0",
            ] {
                assert!(!l.contains(bad), "{l:?} holds {bad:?}");
            }
            let parts: Vec<&str> = l.split(' ').collect();
            assert_eq!(parts.len(), 3, "{l}");
            assert!(
                parts[0].parse::<u16>().is_ok() && parts[2].ends_with("ms"),
                "{l}"
            );
        }
        assert!(log[0].starts_with("200 ~zod "), "{}", log[0]);
        assert!(log[5].starts_with("421 - "), "{}", log[5]);
    }

    #[test]
    fn head_and_other_methods() {
        let (p, _) = rig(cfg(), None, None);
        let r = p.handle(&Request {
            method: "HEAD".into(),
            ..get("zod.test.example", "/gmi", "")
        });
        assert_eq!(r.status, 200);
        for m in ["POST", "PUT", "DELETE", "OPTIONS"] {
            assert_eq!(
                p.handle(&Request {
                    method: m.into(),
                    ..get("zod.test.example", "/gmi", "")
                })
                .status,
                405
            );
        }
    }
}
