//! main.go's `gateway`: `ServeHTTP` and what it calls, byte for byte.
//! The reading is handed in, so the routes run without a network in
//! the tests and the vectors.

use crate::http::{self, Request, Response};
use std::time::Instant;
use urb_render::gateway as render;

/// A page as `Reader.Fetch` gives it: its key in the vault, the mark it
/// was published under, its body, and how it was read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Page {
    pub key: String,
    pub mark: String,
    pub body: Vec<u8>,
    pub how: String,
}

/// a failure the gateway reports with a status code (urb.go's
/// `httpError`): the code and the message
pub type Fail = (u16, Vec<u8>);

/// The gateway: the port its links name, the ship it runs as, the
/// reading (of an url urb.go's deUrb already took), and its log.
pub struct Gateway<'a> {
    pub port: String,
    pub ship: String,
    pub fetch: &'a (dyn Fn(&str) -> Result<Page, Fail> + Sync),
    pub log: &'a (dyn Fn(&str) + Sync),
}

/// the policy for the gateway's own pages: no scripts at all
pub const OWN: &str = render::OWN;

/// `ownPage(h)`: the headers of a page the gateway renders itself
fn own_page(r: &mut Response) {
    r.set("Content-Type", "text/html; charset=utf-8");
    r.set("Content-Security-Policy", OWN);
    r.set("X-Content-Type-Options", "nosniff");
}

/// `(%.1fs)` of the time since t
fn secs(t: Instant) -> String {
    format!("{:.1}", t.elapsed().as_secs_f64())
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

impl Gateway<'_> {
    /// `ServeHTTP`
    pub fn handle(&self, req: &Request) -> Response {
        if req.method != "GET" && req.method != "HEAD" {
            return http::error("read only", 405);
        }
        let host = http::split_host_port(&req.host).unwrap_or(&req.host);
        if let Some(url) = render::urb_of_host(host, &req.path) {
            return self.serve(req, &url);
        }
        if host == b"urb.localhost" || host == b"localhost" || host == b"127.0.0.1" {
            if req.path == b"/go" {
                return self.redirect(req);
            }
            return self.home();
        }
        // any other name pointing here is DNS rebinding: refuse it
        http::error("unknown host", 421)
    }

    /// `serve`: the page an url names, rendered, or its error page
    fn serve(&self, req: &Request, url: &[u8]) -> Response {
        let t = Instant::now();
        let got = render::de_urb(url).and_then(|_| {
            // deUrb took it, so it is ASCII
            (self.fetch)(&text(url))
        });
        let pg = match got {
            Ok(pg) => pg,
            Err((status, msg)) => {
                (self.log)(&format!("{}: {} ({}s)", text(url), text(&msg), secs(t)));
                return self.error(url, status, &msg);
            }
        };
        (self.log)(&format!(
            "{} -> {} %{} by {}, {} bytes ({}s)",
            text(url),
            pg.key,
            pg.mark,
            pg.how,
            pg.body.len(),
            secs(t)
        ));
        let mut r = Response::new(200);
        r.set("X-Urb-Url", url);
        r.set("X-Urb-Mark", pg.mark.as_bytes());
        r.set("X-Content-Type-Options", "nosniff");
        r.set("Cache-Control", "no-cache");
        if req.has(b"raw") {
            r.set("Content-Type", "text/plain; charset=utf-8");
            // in case a browser renders it anyway
            r.set("Content-Security-Policy", render::RAW);
            r.body = pg.body;
            return r;
        }
        let (csp, body) = render::render(url, &pg.mark, &pg.body, &self.port);
        r.set("Content-Type", "text/html; charset=utf-8");
        r.set("Content-Security-Policy", csp);
        r.body = body;
        r
    }

    /// `redirect`: /go?url=urb://... to where the gateway serves it
    fn redirect(&self, req: &Request) -> Response {
        let raw = req.get(b"url");
        match gateway_url(http::trim_space(&raw), &self.port) {
            Ok(to) => http::redirect(&req.method, &to, 302),
            Err((status, msg)) => self.error(&raw, status, &msg),
        }
    }

    /// `home`
    fn home(&self) -> Response {
        let mut r = Response::new(200);
        own_page(&mut r);
        let body = [
            b"<h1>urb-fetch</h1>\n<p>Read a page published with lattice on the Urbit network, as "
                .as_slice(),
            &render::html_escape(self.ship.as_bytes()),
            b".</p>\n<form action=\"/go\"><input name=\"url\" placeholder=\"urb://~sampel-palnet/page\" autofocus></form>",
        ]
        .concat();
        r.body = render::page(b"urb-fetch", &body);
        r
    }

    /// `error`: the gateway's own error page
    pub fn error(&self, url: &[u8], status: u16, msg: &[u8]) -> Response {
        error_page(url, status, msg)
    }
}

/// urb.go's `gatewayURL(url, port)`: where the gateway serves an urb://
/// url, or its error
pub fn gateway_url(u: &[u8], port: &str) -> Result<Vec<u8>, Fail> {
    render::gateway_url(u, port).ok_or_else(|| {
        if u.starts_with(b"urb://~") {
            (400, [b"no ship in ", u].concat())
        } else {
            (400, [b"not an urb:// url: ", u].concat())
        }
    })
}

/// main.go's `gateway.error`, for a failure with its status
pub fn error_page(url: &[u8], status: u16, msg: &[u8]) -> Response {
    let mut r = Response::new(status);
    own_page(&mut r);
    let body = [
        b"<h1>".as_slice(),
        status.to_string().as_bytes(),
        b"</h1><p>",
        &render::html_escape(msg),
        b"</p>",
    ]
    .concat();
    r.body = render::page(url, &body);
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gw(f: &(dyn Fn(&str) -> Result<Page, Fail> + Sync)) -> Gateway<'_> {
        Gateway {
            port: "8475".into(),
            ship: "~zod".into(),
            fetch: f,
            log: &|_| {},
        }
    }

    fn req(method: &str, host: &str, path: &str, q: &str) -> Request {
        Request {
            method: method.into(),
            host: host.into(),
            path: path.into(),
            raw_query: q.into(),
            minor: 1,
        }
    }

    #[test]
    fn routes() {
        let f = |url: &str| -> Result<Page, Fail> {
            match url {
                "urb://~zod/hello" => Ok(Page {
                    key: "/hello".into(),
                    mark: "md".into(),
                    body: b"# hi".to_vec(),
                    how: "keen".into(),
                }),
                u => Err((404, format!("{u} is not published").into_bytes())),
            }
        };
        let g = gw(&f);
        let r = g.handle(&req("GET", "zod.urb.localhost:8475", "/hello", ""));
        assert_eq!(r.status, 200);
        assert_eq!(r.header("X-Urb-Mark"), Some(&b"md"[..]));
        assert!(r.body.windows(11).any(|w| w == b"<h1>hi</h1>"));
        let r = g.handle(&req("GET", "zod.urb.localhost:8475", "/hello", "raw"));
        assert_eq!(
            (r.body.as_slice(), r.header("Content-Security-Policy")),
            (&b"# hi"[..], Some(&b"sandbox"[..]))
        );
        assert_eq!(
            g.handle(&req("POST", "zod.urb.localhost", "/a", "")).status,
            405
        );
        assert_eq!(g.handle(&req("GET", "evil.example", "/", "")).status, 421);
        assert_eq!(
            g.handle(&req("GET", "zod.urb.localhost", "/nope", ""))
                .status,
            404
        );
        assert_eq!(
            g.handle(&req("GET", "zod.urb.localhost", "/forum/b", ""))
                .status,
            501
        );
        let r = g.handle(&req("GET", "localhost:8475", "/go", "url=+urb://~nec/x+"));
        assert_eq!(
            (r.status, r.header("Location")),
            (302, Some(&b"http://nec.urb.localhost:8475/x"[..]))
        );
        assert_eq!(
            g.handle(&req("GET", "localhost:8475", "/go", "url=x"))
                .status,
            400
        );
    }
}
