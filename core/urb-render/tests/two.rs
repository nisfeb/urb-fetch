//! The two renderers held together: on random bodies and links, the
//! extension's blocks, spans, sniff and links (urb_render::ext) and the
//! gateway's (urb_render::gateway) must agree, as render_parity_test.go
//! holds the Go to the JS. The one difference left, a link to an
//! urb:// name that is not a real ship (docs/rust-core.md, "M5b's
//! findings"), is the only one allowed, and only where it applies.

use urb_render::{ext, gateway, ship};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn pick<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        xs[(self.next() % xs.len() as u64) as usize]
    }
}

const CHARS: &[&str] = &[
    "#",
    "#",
    "*",
    "*",
    "`",
    "[",
    "]",
    "(",
    ")",
    "_",
    "-",
    "+",
    ">",
    "1",
    ".",
    " ",
    " ",
    " ",
    "\t",
    "\r",
    "\n",
    "\n",
    "\u{a0}",
    "\u{85}",
    "\u{feff}",
    "\u{2028}",
    "\u{2029}",
    "\u{3000}",
    "\u{b}",
    "\u{c}",
    "x",
    "y",
    "/",
    ":",
    "\\",
    "é",
    "😀",
    "<",
    "=",
    "h",
    "t",
    "p",
    "s",
    "\u{1}",
    "\u{7f}",
    "a",
    "ſ",
    "\u{212a}",
    "&",
    "'",
    "\"",
    "](",
    "**",
    "=>",
    "```",
    "* ",
    "- ",
    "1. ",
    "10) ",
    "# ",
    "###### ",
    "> ",
    "urb://~",
    "zod",
    "nec",
    "abc",
    "marzod",
    "dozzod",
    "sampel-palnet",
    "https://",
    "HTTP",
    "mailto:",
    "gemini:",
    "%2e",
    "..",
    "?",
    "#",
    "|",
];

const LINKS: &[&str] = &[
    "urb://~zod",
    "urb://~nec/x",
    "URB://~zod/y",
    "urb://~abc/x",
    "urb://~dozzod/y",
    "urb://~doznec",
    "urb://~marzod",
    "urb://~sampel-palnet/p",
    "urb://~zod-",
    "HTTPS://a.b/c?d#e",
    "http:/x",
    "http://",
    "httpſ://e.x",
    "mailto:a@b",
    "Gemini://g",
    "javascript:x",
    "/",
    "//",
    "\\",
    "..",
    ".",
    "x",
    "y/z",
    "?q",
    "#f",
    "%41",
    "%2e",
    "%2E",
    "é",
    "😀",
    ":",
    "a:b",
    "~",
    "'",
    "(",
    ")",
    "!",
    "*",
    "\"",
    "<",
    "{",
    "|",
    "^",
    "`",
    "[",
    "]",
    "@",
    "$",
    "&",
    "+",
    ",",
    ";",
    "=",
    "\u{1}",
    " ",
    "\u{a0}",
    "\u{3000}",
    "\u{212a}",
    "\u{7f}",
    "a/../../b",
    "./",
    "%2e%2E/",
    "\t",
    "ſ",
];

/// where the gateway serves where the extension's link goes: an
/// urb:// address at its ship's origin, anything else as it is
fn gateway_of(addr: &str) -> String {
    match addr.strip_prefix("urb://~") {
        Some(rest) => {
            let (who, path) = rest.split_once('/').unwrap_or((rest, ""));
            format!("http://{who}.urb.localhost:8475/{path}")
        }
        None => addr.to_string(),
    }
}

/// reader.js's mdHref: where the extension sends a link, as an urb://
/// address
fn md_href(t: &str) -> String {
    if !ext::safe_link(t) {
        return String::new();
    }
    let lower = match t.find(':') {
        Some(n)
            if t[..n]
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"+.-".contains(&b))
                && t.as_bytes()[0].is_ascii_alphabetic() =>
        {
            t[..n].to_ascii_lowercase() + &t[n..]
        }
        _ => t.to_string(),
    };
    if lower.starts_with("mailto:") || lower.starts_with("gemini:") {
        return lower;
    }
    ext::link_target("urb://~zod/a/b", &lower)
}

/// an urb:// link to a name the extension takes for a ship and the
/// gateway does not
fn not_a_ship(link: &str) -> bool {
    let t = link.trim_matches(urb_render::js::is_space);
    let Some(rest) = t
        .get(..6)
        .filter(|s| s.eq_ignore_ascii_case("urb://"))
        .map(|_| &t[6..])
    else {
        return false;
    };
    let who = rest.split('/').next().unwrap_or("");
    ship::is_ship(who) && !ship::parses(who.as_bytes())
}

#[test]
fn the_extensions_and_the_gateways_agree() {
    let mut rng = Rng(0x2545_f491_4f6c_dd1d);
    let page = gateway::page_link(b"urb://~zod/a/b", "8475");
    let mut allowed = 0;
    //  a quarter as many in a debug build, which WASI runs too
    let scale = if cfg!(debug_assertions) { 4 } else { 1 };
    for i in 0..60_000 / scale {
        let n = (rng.next() % if i % 10 == 0 { 400 } else { 40 }) as usize;
        let body: String = (0..n).map(|_| rng.pick(CHARS)).collect();
        let b = body.as_bytes();
        assert_eq!(
            ext::gemtext(&body),
            gateway::parsed::gemtext(b),
            "gemtext of {body:?}"
        );
        assert_eq!(
            ext::markdown(&body),
            gateway::parsed::markdown(b),
            "markdown of {body:?}"
        );
        assert_eq!(
            ext::spans(&body),
            gateway::parsed::spans(b),
            "spans of {body:?}"
        );
        assert_eq!(
            urb_render::looks_html(&body),
            gateway::looks_html(b),
            "looksHtml of {body:?}"
        );
        assert_eq!(
            ext::safe_link(&body),
            gateway::safe_link(b),
            "safeLink of {body:?}"
        );
        let (e, g) = (
            gateway_of(&md_href(&body)),
            String::from_utf8_lossy(&page(b).0).into_owned(),
        );
        if e != g {
            assert!(
                not_a_ship(&body) && g.is_empty(),
                "link of {body:?}: ext {e:?}, gateway {g:?}"
            );
            allowed += 1;
        }
    }
    for _ in 0..200_000 / scale {
        let n = 1 + (rng.next() % 4) as usize;
        let link: String = (0..n).map(|_| rng.pick(LINKS)).collect();
        let b = link.as_bytes();
        assert_eq!(
            ext::safe_link(&link),
            gateway::safe_link(b),
            "safeLink of {link:?}"
        );
        let (e, g) = (
            gateway_of(&md_href(&link)),
            String::from_utf8_lossy(&page(b).0).into_owned(),
        );
        if e != g {
            assert!(
                not_a_ship(&link) && g.is_empty(),
                "link of {link:?}: ext {e:?}, gateway {g:?}"
            );
            allowed += 1;
        }
    }
    assert!(allowed > 0);
    eprintln!("{allowed} links to names that are not ships");
}
