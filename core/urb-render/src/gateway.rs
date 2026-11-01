//! The Go gateway's rendering (`render.go` and `urb.go`): gemtext,
//! Markdown and text as the gateway's own HTML, a publisher's HTML
//! with its urb:// links pointed at the gateway, and the
//! Content-Security-Policy each is served under. Byte for byte.
//!
//! A Go string is bytes, and the gateway hands a page's body over as
//! it came, so everything here is bytes too: a byte that is not UTF-8
//! is read as Go reads it (U+FFFD, one byte wide, one UTF-16 unit
//! long) and written back as it was. Where a rule turns on whitespace
//! or a length it is JavaScript's, as render.go reads it. A link's
//! scheme matches in any case in ASCII only, as the extension's does
//! (urb-fetch PR #10). `rewriteHTML`'s attribute names match as Go's
//! `(?i)` matches them, where `s` is also `ſ` (U+017F).

use crate::{cost, js, ship};

/// lattice's own `+sandbox-csp`, the only policy a publisher's HTML is
/// ever served under: an opaque origin, its scripts running but
/// holding no cookie and reaching nothing of the gateway's
pub const SANDBOX: &str =
    "sandbox allow-scripts allow-forms allow-popups allow-top-navigation-by-user-activation";

/// the policy for the pages the gateway renders itself: no scripts
pub const OWN: &str = "default-src 'none'; style-src 'unsafe-inline'";

/// the policy for a page served raw (`?raw`), as text, in case a
/// browser renders it anyway
pub const RAW: &str = "sandbox";

/// where a page's link target goes, as `(href, own)`: an empty href
/// leaves it as text, and own is whether the gateway serves it (a link
/// that leaves gets `rel="noopener noreferrer"`). render.go's `linker`.
pub type Linker<'a> = dyn Fn(&[u8]) -> (Vec<u8>, bool) + 'a;

/// each ship is its own origin, `<ship>.urb.localhost`
pub const HOST_SUFFIX: &str = ".urb.localhost";

/// `mdMax`, in UTF-16 code units as the extension's `MD_MAX` counts
const MD_MAX: usize = 256 * 1024;

const CSS: &str =
    ":root{color-scheme:light dark;--fg:#1d1d1f;--bg:#fbfbf8;--mute:#6b6b70;--link:#2456c8}
@media (prefers-color-scheme:dark){:root{--fg:#e8e8e3;--bg:#18181a;--mute:#9a9aa0;--link:#8ab0ff}}
body{margin:0;background:var(--bg);color:var(--fg);font:17px/1.6 system-ui,sans-serif}
main{max-width:44rem;margin:0 auto;padding:2rem 1rem}
a{color:var(--link)}p.link{margin:.3rem 0}
pre{overflow-x:auto;padding:.8rem;border:1px solid var(--mute);border-radius:6px;font-size:14px}
blockquote{margin:1rem 0;padding-left:1rem;border-left:3px solid var(--mute);color:var(--mute)}
input{font:inherit;width:100%;box-sizing:border-box;padding:.5rem}";

// ------------------------------------------------------------ Go's text

const ERROR: u32 = 0xfffd;

/// `utf8.DecodeRune(b)`: the rune at the start of b and its width, a
/// byte that starts no valid sequence being U+FFFD, one byte wide
fn decode(b: &[u8]) -> (u32, usize) {
    let c = |i: usize| b.get(i).copied().unwrap_or(0);
    let cont = |x: u8| (0x80..=0xbf).contains(&x);
    let b0 = c(0);
    let (lo, hi, n) = match b0 {
        0x00..=0x7f => return (u32::from(b0), 1),
        0xc2..=0xdf => (0x80, 0xbf, 2),
        0xe0 => (0xa0, 0xbf, 3),
        0xe1..=0xec | 0xee..=0xef => (0x80, 0xbf, 3),
        0xed => (0x80, 0x9f, 3),
        0xf0 => (0x90, 0xbf, 4),
        0xf1..=0xf3 => (0x80, 0xbf, 4),
        0xf4 => (0x80, 0x8f, 4),
        _ => return (ERROR, 1),
    };
    if b.len() < n || !(lo..=hi).contains(&c(1)) || !(2..n).all(|i| cont(c(i))) {
        return (ERROR, 1);
    }
    let mut r = u32::from(b0) & (0x7f >> n);
    for i in 1..n {
        r = (r << 6) | (u32::from(c(i)) & 0x3f);
    }
    (r, n)
}

/// `for i, r := range s`: each rune's offset, the rune and its width
fn runes(s: &[u8]) -> impl Iterator<Item = (usize, u32, usize)> + '_ {
    let mut i = 0;
    std::iter::from_fn(move || {
        if i >= s.len() {
            return None;
        }
        cost::step(1);
        let (r, w) = decode(&s[i..]);
        let at = i;
        i += w;
        Some((at, r, w))
    })
}

/// render.go's `jsSpace`
fn space(r: u32) -> bool {
    char::from_u32(r).is_some_and(js::is_space)
}

/// a JavaScript line terminator, which render.go's `jsDot` refuses
fn terminator(r: u32) -> bool {
    char::from_u32(r).is_some_and(js::is_terminator)
}

/// `strings.TrimLeftFunc(s, jsSpace)`
fn trim_left(s: &[u8]) -> &[u8] {
    for (i, r, _) in runes(s) {
        if !space(r) {
            return &s[i..];
        }
    }
    &[]
}

/// `strings.TrimRightFunc(s, jsSpace)`: a space is always a valid
/// sequence whose first byte no sequence before it can take, so
/// reading forward finds the same runes as Go's reading backward
fn trim_right(s: &[u8]) -> &[u8] {
    let mut end = 0;
    for (i, r, w) in runes(s) {
        if !space(r) {
            end = i + w;
        }
    }
    &s[..end]
}

/// `jsTrim(s)`
fn trim(s: &[u8]) -> &[u8] {
    trim_right(trim_left(s))
}

/// `units(s)`: s's length in UTF-16 code units, as render.go counts
fn units(s: &[u8]) -> usize {
    runes(s)
        .map(|(_, r, _)| if r >= 0x10000 { 2 } else { 1 })
        .sum()
}

/// `html.EscapeString(s)`, onto out
fn esc(out: &mut Vec<u8>, s: &[u8]) {
    cost::step(s.len());
    for &b in s {
        match b {
            b'&' => out.extend_from_slice(b"&amp;"),
            b'\'' => out.extend_from_slice(b"&#39;"),
            b'<' => out.extend_from_slice(b"&lt;"),
            b'>' => out.extend_from_slice(b"&gt;"),
            b'"' => out.extend_from_slice(b"&#34;"),
            _ => out.push(b),
        }
    }
}

/// a letter as Go's `(?i)` matches it, at the start of s: its width
/// there, if it is `want` in either case, or the other letter Go
/// folds it with (`ſ` for s, the Kelvin sign for k)
fn folded(s: &[u8], want: u8) -> Option<usize> {
    let (r, w) = decode(s);
    let ok = r < 0x80 && (r as u8).eq_ignore_ascii_case(&want)
        || want == b's' && r == 0x17f
        || want == b'k' && r == 0x212a;
    ok.then_some(w)
}

/// the width of `word`, matched by Go's `(?i)` at the start of s
fn folded_word(s: &[u8], word: &[u8]) -> Option<usize> {
    let mut at = 0;
    for &c in word {
        if c.is_ascii_alphabetic() {
            at += folded(&s[at..], c)?;
        } else if s.get(at) == Some(&c) {
            at += 1;
        } else {
            return None;
        }
    }
    Some(at)
}

// ------------------------------------------------------------ links

/// `safeLink(t)`: whether a page's link target may be a link: the web,
/// urb://, gemini, mail (schemes in any case, in ASCII) or a path with
/// no colon; never with a control character, a space or DEL, and never
/// protocol-relative, after JavaScript's whitespace
pub fn safe_link(t: &[u8]) -> bool {
    cost::step(t.len());
    // ctlLink, `[\x00-\x20\x7f]`
    if t.iter().any(|&b| b <= 0x20 || b == 0x7f) {
        return false;
    }
    // safeHref, `^(?:(?:[hH][tT][tT][pP][sS]?|[uU][rR][bB]|...):|[^:]*$)`
    let scheme = [&b"https:"[..], b"http:", b"urb:", b"gemini:", b"mailto:"]
        .iter()
        .any(|w| ascii_word(t, w));
    if !scheme && t.contains(&b':') {
        return false;
    }
    // hostLink, `^jsS*[/\\]{2}`
    let r = trim_left(t);
    !(r.len() >= 2 && matches!(r[0], b'/' | b'\\') && matches!(r[1], b'/' | b'\\'))
}

/// whether s starts with word, its letters in either case
fn ascii_word(s: &[u8], word: &[u8]) -> bool {
    s.len() >= word.len() && s[..word.len()].eq_ignore_ascii_case(word)
}

/// the width of `scheme`'s match, `^[a-zA-Z][a-zA-Z0-9+.-]*:`
fn scheme_len(s: &[u8]) -> Option<usize> {
    if !s.first()?.is_ascii_alphabetic() {
        return None;
    }
    let n = s
        .iter()
        .position(|&c| !(c.is_ascii_alphanumeric() || matches!(c, b'+' | b'.' | b'-')))?;
    cost::step(n);
    (s[n] == b':').then_some(n + 1)
}

/// `^urb://(~[a-z-]+)`'s group, and what follows it
fn urb_ship(s: &[u8]) -> Option<(&[u8], &[u8])> {
    let rest = s.strip_prefix(b"urb://~")?;
    let n = rest
        .iter()
        .position(|&b| !(b.is_ascii_lowercase() || b == b'-'))
        .unwrap_or(rest.len());
    cost::step(n);
    (n > 0).then(|| (&s[6..7 + n], &rest[n..]))
}

/// `gatewayURL(url, port)`: where the gateway serves an urb:// url,
/// `None` where Go gives an error (not urb://~, or no ship)
pub fn gateway_url(url: &[u8], port: &str) -> Option<Vec<u8>> {
    let rest = url.strip_prefix(b"urb://~")?;
    let (who, path) = match rest.iter().position(|&b| b == b'/') {
        Some(i) => (&rest[..i], &rest[i + 1..]),
        None => (rest, &b""[..]),
    };
    if !ship::parses(&[b"~", who].concat()) {
        return None;
    }
    Some(
        [
            b"http://",
            who,
            HOST_SUFFIX.as_bytes(),
            b":",
            port.as_bytes(),
            b"/",
            path,
        ]
        .concat(),
    )
}

/// `resolve(base, ref)`: the path of the relative reference ref on the
/// page at path base, as render.go reckons a WHATWG URL parser gives it
fn resolve(base: &[u8], r: &[u8]) -> Vec<u8> {
    let r = &r[..r.iter().position(|b| b"?#".contains(b)).unwrap_or(r.len())];
    let joined;
    let r = if r.is_empty() {
        base
    } else if r[0] != b'/' {
        let dir = base.iter().rposition(|&b| b == b'/').map_or(0, |i| i + 1);
        joined = [&base[..dir], r].concat();
        &joined
    } else {
        r
    };
    cost::step(r.len());
    let segs: Vec<&[u8]> = r
        .get(1..)
        .unwrap_or_default()
        .split(|&b| b == b'/')
        .collect();
    let mut out: Vec<&[u8]> = Vec::new();
    for (i, sg) in segs.iter().enumerate() {
        let last = i == segs.len() - 1;
        let is = |w: &[u8]| sg.eq_ignore_ascii_case(w);
        if is(b".") || is(b"%2e") {
            if last {
                out.push(b"");
            }
        } else if is(b"..") || is(b".%2e") || is(b"%2e.") || is(b"%2e%2e") {
            out.pop();
            if last {
                out.push(b"");
            }
        } else {
            out.push(sg);
        }
    }
    let mut b = vec![b'/'];
    for c in out.join(&b'/') {
        if c <= 0x20 || c >= 0x7f || b"\"<>^`{}".contains(&c) {
            b.extend_from_slice(format!("%{c:02X}").as_bytes());
        } else {
            b.push(c);
        }
    }
    b
}

/// `pageLink(page, port)`: where a link on the page at the urb://
/// address page goes, as `(href, own)`. An empty href leaves it as
/// text; own is whether the gateway serves it.
pub fn page_link<'a>(page: &'a [u8], port: &'a str) -> impl Fn(&[u8]) -> (Vec<u8>, bool) + 'a {
    move |target| link(page, port, target)
}

fn link(page: &[u8], port: &str, target: &[u8]) -> (Vec<u8>, bool) {
    let none = (Vec::new(), false);
    if !safe_link(target) {
        return none;
    }
    let t = match scheme_len(target) {
        Some(n) => [target[..n].to_ascii_lowercase(), target[n..].to_vec()].concat(),
        None => target.to_vec(),
    };
    if t.starts_with(b"mailto:") || t.starts_with(b"gemini:") {
        return (t, false);
    }
    let t = trim(&t);
    if let Some((_, rest)) = urb_ship(t)
        && (rest.is_empty() || rest[0] == b'/')
    {
        return gateway_url(t, port).map_or(none, |u| (u, true));
    }
    // webLink, `^[hH][tT][tT][pP][sS]?://[^/]`
    let web = [&b"https://"[..], b"http://"]
        .into_iter()
        .find(|w| ascii_word(t, w));
    if web.is_some_and(|w| t.len() > w.len() && t[w.len()] != b'/') {
        return (t.to_vec(), false);
    }
    if t.is_empty() || scheme_len(t).is_some() || t.starts_with(b"//") {
        return none;
    }
    // pageURL, `^urb://(~[a-z-]+)(/[^?#]*)?`
    let Some((who, rest)) = urb_ship(page) else {
        return none;
    };
    let base = if rest.first() == Some(&b'/') {
        &rest[..rest
            .iter()
            .position(|b| b"?#".contains(b))
            .unwrap_or(rest.len())]
    } else {
        b"/"
    };
    let r: Vec<u8> = t
        .iter()
        .map(|&b| if b == b'\\' { b'/' } else { b })
        .collect();
    let mut p = resolve(base, &r);
    if p == b"/" {
        p.clear();
    }
    gateway_url(&[b"urb://", who, &p].concat(), port).map_or(none, |u| (u, true))
}

/// `anchor(href, own, text)`: a link, or the text alone for no href
fn anchor(out: &mut Vec<u8>, href: &[u8], own: bool, text: &[u8]) {
    if href.is_empty() {
        return esc(out, text);
    }
    out.extend_from_slice(b"<a href=\"");
    esc(out, href);
    out.extend_from_slice(if own {
        &b"\">"[..]
    } else {
        b"\" rel=\"noopener noreferrer\">"
    });
    esc(out, text);
    out.extend_from_slice(b"</a>");
}

/// `pre(text)`: the newline after `<pre>` is the one the HTML parser
/// drops, so a first line that is blank stays
fn pre(out: &mut Vec<u8>, text: &[u8]) {
    out.extend_from_slice(b"<pre>\n");
    esc(out, text);
    out.extend_from_slice(b"</pre>");
}

/// `page(title, body)`: a body in the gateway's own minimal document
pub fn page(title: &[u8], body: &[u8]) -> Vec<u8> {
    let mut out = b"<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>"
        .to_vec();
    esc(&mut out, title);
    out.extend_from_slice(b"</title><style>");
    out.extend_from_slice(CSS.as_bytes());
    out.extend_from_slice(b"</style></head><body><main>");
    out.extend_from_slice(body);
    out.extend_from_slice(b"</main></body></html>");
    out
}

/// the body's lines, as `strings.Split(strings.ReplaceAll(body,
/// "\r\n", "\n"), "\n")`
fn lines(body: &[u8]) -> impl Iterator<Item = &[u8]> {
    cost::step(body.len());
    let mut parts = body.split(|&b| b == b'\n').peekable();
    std::iter::from_fn(move || {
        let l = parts.next()?;
        Some(match parts.peek() {
            Some(_) => l.strip_suffix(b"\r").unwrap_or(l),
            None => l,
        })
    })
}

// ------------------------------------------------------------ gemtext

enum Gem<'a> {
    Heading(usize, &'a [u8]),
    Para(&'a [u8]),
    Quote(&'a [u8]),
    Pre(Vec<u8>),
    List(Vec<&'a [u8]>),
    Link(&'a [u8], &'a [u8]),
}

/// `gemtextBlocks(body)`
fn gemtext_blocks(body: &[u8]) -> Vec<Gem<'_>> {
    let mut out = Vec::new();
    let mut pre: Option<Vec<&[u8]>> = None;
    let mut list: Option<usize> = None;
    for line in lines(body) {
        cost::step(1);
        if line.starts_with(b"```") {
            list = None;
            match pre.take() {
                Some(p) => out.push(Gem::Pre(p.join(&b'\n'))),
                None => pre = Some(Vec::new()),
            }
            continue;
        }
        if let Some(p) = &mut pre {
            p.push(line);
            continue;
        }
        let item = line.starts_with(b"* ");
        if !item {
            list = None;
        }
        if let Some(rest) = line.strip_prefix(b"=>") {
            let rest = trim(rest);
            if rest.is_empty() {
                continue;
            }
            let end = runes(rest)
                .find(|&(_, r, _)| space(r))
                .map_or(rest.len(), |(i, ..)| i);
            let href = &rest[..end];
            let label = trim(&rest[end..]);
            out.push(Gem::Link(href, if label.is_empty() { href } else { label }));
        } else if line.first() == Some(&b'#') {
            let marks = line.iter().position(|&b| b != b'#').unwrap_or(line.len());
            out.push(Gem::Heading(marks.min(3), trim(&line[marks..])));
        } else if item {
            let at = *list.get_or_insert_with(|| {
                out.push(Gem::List(Vec::new()));
                out.len() - 1
            });
            if let Gem::List(items) = &mut out[at] {
                items.push(&line[2..]);
            }
        } else if line.first() == Some(&b'>') {
            out.push(Gem::Quote(trim(&line[1..])));
        } else if !trim(line).is_empty() {
            out.push(Gem::Para(line));
        }
    }
    if let Some(p) = pre {
        out.push(Gem::Pre(p.join(&b'\n')));
    }
    out
}

/// `renderGemtext(body, title, link)`: gemtext as a page, its links
/// through link. A link left as text shows its target, as `label
/// (target)`. The title is the first h1's, else title.
pub fn render_gemtext(body: &[u8], title: &[u8], link: &Linker) -> Vec<u8> {
    let mut b = Vec::new();
    let mut title = title;
    let mut titled = false;
    let blocks = gemtext_blocks(body);
    for k in &blocks {
        match k {
            Gem::List(items) => {
                b.extend_from_slice(b"<ul>\n");
                for it in items {
                    b.extend_from_slice(b"<li>");
                    esc(&mut b, it);
                    b.extend_from_slice(b"</li>\n");
                }
                b.extend_from_slice(b"</ul>\n");
            }
            Gem::Link(href, label) => {
                b.extend_from_slice(b"<p class=\"link\">");
                let (to, own) = link(href);
                if !to.is_empty() {
                    anchor(&mut b, &to, own, label);
                } else if label == href {
                    esc(&mut b, href);
                } else {
                    esc(&mut b, &[label, &b" ("[..], href, b")"].concat());
                }
                b.extend_from_slice(b"</p>\n");
            }
            Gem::Pre(text) => {
                pre(&mut b, text);
                b.push(b'\n');
            }
            Gem::Quote(text) => {
                b.extend_from_slice(b"<blockquote>");
                esc(&mut b, text);
                b.extend_from_slice(b"</blockquote>\n");
            }
            Gem::Heading(n, text) => {
                let kind: &[u8] = [&b"h1"[..], b"h2", b"h3"][n.clamp(&1, &3) - 1];
                if *n == 1 && !titled {
                    title = text;
                    titled = true;
                }
                tagged(&mut b, kind, |b| esc(b, text));
            }
            Gem::Para(text) => tagged(&mut b, b"p", |b| esc(b, text)),
        }
    }
    page(title, &b)
}

/// `<kind>…</kind>\n`
fn tagged(out: &mut Vec<u8>, kind: &[u8], inner: impl FnOnce(&mut Vec<u8>)) {
    out.push(b'<');
    out.extend_from_slice(kind);
    out.push(b'>');
    inner(out);
    out.extend_from_slice(b"</");
    out.extend_from_slice(kind);
    out.extend_from_slice(b">\n");
}

/// `renderText(body, title)`: a text page (text, js, css) as
/// preformatted text
pub fn render_text(body: &[u8], title: &[u8]) -> Vec<u8> {
    let mut b = Vec::new();
    pre(&mut b, body);
    page(title, &b)
}

// ------------------------------------------------------------ Markdown

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Text,
    Code,
    Strong,
    Em,
    Link,
}

/// a span of a Markdown line: its text, and a link's target, set only
/// when [`safe_link`] allows it
#[derive(Clone, Copy)]
struct Span<'a> {
    kind: Kind,
    text: &'a [u8],
    target: &'a [u8],
}

enum Block<'a> {
    Heading(usize, Vec<Span<'a>>),
    Para(Vec<u8>),
    Quote(Vec<Span<'a>>),
    List(&'static [u8], Vec<Vec<Span<'a>>>),
    Rule,
    Pre(Vec<u8>),
}

/// `run(s, stop, -1)`: the length of s's prefix of runes not stop, in
/// UTF-16 units, and its end in s
fn run(s: &[u8], stop: impl Fn(u32) -> bool) -> (usize, usize) {
    let mut n = 0;
    for (i, r, _) in runes(s) {
        if stop(r) {
            return (n, i);
        }
        n += if r >= 0x10000 { 2 } else { 1 };
    }
    (n, s.len())
}

/// a line with a `[` in it, read once (as [`crate::ext`]'s is): at each
/// byte, the UTF-16 units before it, the next `]`, and the next `)` or
/// space, so each `[` costs the same
struct Scan {
    units: Vec<u32>,
    close: Vec<u32>,
    stop: Vec<u32>,
}

impl Scan {
    fn new(line: &[u8]) -> Scan {
        let n = line.len();
        let (mut units, mut close, mut stop) =
            (vec![0; n + 1], vec![n as u32; n + 1], vec![n as u32; n + 1]);
        let mut u = 0;
        let mut starts = Vec::new();
        for (i, r, w) in runes(line) {
            units[i..i + w].fill(u);
            u += if r >= 0x10000 { 2 } else { 1 };
            starts.push((i, r, w));
        }
        units[n] = u;
        for &(i, r, w) in starts.iter().rev() {
            let c2 = if r == u32::from(b']') {
                i as u32
            } else {
                close[i + w]
            };
            close[i..i + w].fill(c2);
            let s2 = if r == u32::from(b')') || space(r) {
                i as u32
            } else {
                stop[i + w]
            };
            stop[i..i + w].fill(s2);
        }
        Scan { units, close, stop }
    }

    /// `run` from byte `at` to the first stop in `next`: its end, if
    /// the stop is there and the run is 1 to `limit` units
    fn run(&self, next: &[u32], at: usize, limit: u32) -> Option<usize> {
        let e = next[at] as usize;
        let n = self.units[e] - self.units[at];
        (e < next.len() - 1 && (1..=limit).contains(&n)).then_some(e)
    }
}

/// the span render.go's `spans` matches at the start of rest, and its
/// end in rest
fn span_at<'a>(line: &'a [u8], i: usize, scan: &mut Option<Scan>) -> Option<(Span<'a>, usize)> {
    let rest = &line[i..];
    let sp = |kind, text| Span {
        kind,
        text,
        target: b"",
    };
    match rest[0] {
        b'`' => {
            let (n, e) = run(&rest[1..], |r| r == u32::from(b'`'));
            (n > 0 && e + 1 < rest.len()).then(|| (sp(Kind::Code, &rest[1..1 + e]), e + 2))
        }
        b'[' => {
            let sc = scan.get_or_insert_with(|| Scan::new(line));
            let close = sc.run(&sc.close, i + 1, 256)?;
            if line.get(close + 1) != Some(&b'(') {
                return None;
            }
            let end = sc.run(&sc.stop, close + 2, 512)?;
            if line[end] != b')' {
                return None;
            }
            let (e, tgt, f) = (close - i - 1, &line[close + 2..], end - close - 2);
            let t = &tgt[..f];
            let kind = if safe_link(t) { Kind::Link } else { Kind::Text };
            Some((
                Span {
                    kind,
                    text: &rest[1..1 + e],
                    target: if kind == Kind::Link { t } else { b"" },
                },
                1 + e + 2 + f + 1,
            ))
        }
        b'*' => {
            if rest.starts_with(b"**") {
                let (n, e) = run(&rest[2..], |r| r == u32::from(b'*'));
                if n > 0 && rest[2 + e..].starts_with(b"**") {
                    return Some((sp(Kind::Strong, &rest[2..2 + e]), 2 + e + 2));
                }
            }
            if rest.len() < 2 {
                return None;
            }
            let (first, _) = decode(&rest[1..]);
            if first == u32::from(b'*') || space(first) {
                return None;
            }
            let (_, e) = run(&rest[1..], |r| r == u32::from(b'*'));
            (1 + e < rest.len()).then(|| (sp(Kind::Em, &rest[1..1 + e]), 1 + e + 1))
        }
        _ => None,
    }
}

/// `spans(line)`: lib/markdown.js's `INLINE`, matched by hand
fn spans(line: &[u8]) -> Vec<Span<'_>> {
    let mut out = Vec::new();
    let mut scan = None;
    let (mut at, mut i) = (0, 0);
    while i < line.len() {
        cost::step(1);
        let found = match line[i] {
            b'`' | b'[' | b'*' => span_at(line, i, &mut scan),
            _ => None,
        };
        match found {
            Some((sp, end)) => {
                if i > at {
                    out.push(Span {
                        kind: Kind::Text,
                        text: &line[at..i],
                        target: b"",
                    });
                }
                out.push(sp);
                at = i + end;
                i = at;
            }
            None => i += 1,
        }
    }
    if at < line.len() {
        out.push(Span {
            kind: Kind::Text,
            text: &line[at..],
            target: b"",
        });
    }
    out
}

/// after `^jsS*`, `[-*+]` or `[0-9]+[.)]`: the rest, and whether it was
/// a number's
fn marker(line: &[u8]) -> Option<(&[u8], bool)> {
    let s = trim_left(line);
    match s.first()? {
        b'-' | b'*' | b'+' => Some((&s[1..], false)),
        b'0'..=b'9' => {
            let n = s
                .iter()
                .position(|c| !c.is_ascii_digit())
                .unwrap_or(s.len());
            cost::step(n);
            matches!(s.get(n), Some(b'.' | b')')).then(|| (&s[n + 1..], true))
        }
        _ => None,
    }
}

/// `mdItem`, `^jsS*(?:[-*+]|[0-9]+[.)])jsS+(jsDot*)$`: the item's
/// text, and whether `mdOrdered` matches too
fn item(line: &[u8]) -> Option<(&[u8], bool)> {
    let (after, ordered) = marker(line)?;
    let text = trim_left(after);
    if text.len() == after.len() || runes(text).any(|(_, r, _)| terminator(r)) {
        return None;
    }
    Some((text, ordered))
}

/// `mdRule`
fn rule(line: &[u8]) -> bool {
    let s = trim_left(line);
    let Some(&m) = s.first().filter(|c| b"-*_".contains(c)) else {
        return false;
    };
    let mut n = 0;
    for (_, r, _) in runes(s) {
        if r == u32::from(m) {
            n += 1;
        } else if !space(r) {
            return false;
        }
    }
    n >= 3
}

/// `mdHeading`, `^(#{1,6})jsS`: the number of #s
fn heading(line: &[u8]) -> Option<usize> {
    let n = line.iter().position(|&b| b != b'#').unwrap_or(line.len());
    cost::step(n);
    ((1..=6).contains(&n) && n < line.len() && space(decode(&line[n..]).0)).then_some(n)
}

/// `headingText(rest)`
fn heading_text(rest: &[u8]) -> &[u8] {
    let t = trim_right(rest);
    let end = t.iter().rposition(|&b| b != b'#').map_or(0, |i| i + 1);
    trim(&t[..end])
}

/// `markdownBlocks(body)`. A paragraph keeps its joined text, and is
/// cut into spans as it is written.
fn markdown_blocks(body: &[u8]) -> Vec<Block<'_>> {
    let mut out: Vec<Block> = Vec::new();
    let mut para: Vec<&[u8]> = Vec::new();
    let mut list: Option<usize> = None;
    let flush = |out: &mut Vec<Block>, para: &mut Vec<&[u8]>, list: &mut Option<usize>| {
        if !para.is_empty() {
            out.push(Block::Para(para.join(&b' ')));
        }
        para.clear();
        *list = None;
    };
    let mut pre: Option<Vec<&[u8]>> = None;
    for line in lines(body) {
        cost::step(1);
        if trim(line).starts_with(b"```") {
            match pre.take() {
                Some(p) => out.push(Block::Pre(p.join(&b'\n'))),
                None => {
                    flush(&mut out, &mut para, &mut list);
                    pre = Some(Vec::new());
                }
            }
            continue;
        }
        if let Some(p) = &mut pre {
            p.push(line);
            continue;
        }
        if let Some((text, ordered)) = item(line)
            && !rule(line)
        {
            let kind: &'static [u8] = if ordered { b"ol" } else { b"ul" };
            let same = list.is_some_and(|at| matches!(&out[at], Block::List(k, _) if *k == kind));
            if !para.is_empty() || !same {
                flush(&mut out, &mut para, &mut list);
                out.push(Block::List(kind, Vec::new()));
                list = Some(out.len() - 1);
            }
            if let Some(Block::List(_, items)) = list.map(|at| &mut out[at]) {
                items.push(spans(text));
            }
            continue;
        }
        let t = trim(line);
        if t.is_empty() {
            flush(&mut out, &mut para, &mut list);
        } else if let Some(n) = heading(line) {
            flush(&mut out, &mut para, &mut list);
            out.push(Block::Heading(n, spans(heading_text(&line[n..]))));
        } else if rule(line) {
            flush(&mut out, &mut para, &mut list);
            out.push(Block::Rule);
        } else if t[0] == b'>' {
            flush(&mut out, &mut para, &mut list);
            out.push(Block::Quote(spans(trim(&t[1..]))));
        } else {
            if list.is_some() {
                flush(&mut out, &mut para, &mut list);
            }
            para.push(t);
        }
    }
    if let Some(p) = pre {
        out.push(Block::Pre(p.join(&b'\n')));
    }
    flush(&mut out, &mut para, &mut list);
    out
}

/// `renderMarkdown(body, title, link)`: a small, safe subset of
/// Markdown, as the extension reads it, all text escaped. A body over
/// `mdMax` is one `<pre>` of it as written. The title is the first
/// h1's text, else title.
pub fn render_markdown(body: &[u8], title: &[u8], link: &Linker) -> Vec<u8> {
    if units(body) > MD_MAX {
        return render_text(body, title);
    }
    let line = |b: &mut Vec<u8>, ss: &[Span]| {
        for s in ss {
            match s.kind {
                Kind::Code | Kind::Strong | Kind::Em => {
                    let tag: &[u8] = match s.kind {
                        Kind::Code => b"code",
                        Kind::Strong => b"strong",
                        _ => b"em",
                    };
                    b.push(b'<');
                    b.extend_from_slice(tag);
                    b.push(b'>');
                    esc(b, s.text);
                    b.extend_from_slice(b"</");
                    b.extend_from_slice(tag);
                    b.push(b'>');
                }
                Kind::Link => {
                    let (href, own) = link(s.target);
                    anchor(b, &href, own, s.text);
                }
                Kind::Text => esc(b, s.text),
            }
        }
    };
    let plain = |ss: &[Span]| {
        ss.iter()
            .flat_map(|s| s.text.iter().copied())
            .collect::<Vec<u8>>()
    };
    let mut b = Vec::new();
    let mut title = title.to_vec();
    let mut titled = false;
    for k in &markdown_blocks(body) {
        match k {
            Block::Pre(text) => {
                pre(&mut b, text);
                b.push(b'\n');
            }
            Block::Rule => b.extend_from_slice(b"<hr>\n"),
            Block::List(kind, items) => {
                b.push(b'<');
                b.extend_from_slice(kind);
                b.extend_from_slice(b">\n");
                for it in items {
                    b.extend_from_slice(b"<li>");
                    line(&mut b, it);
                    b.extend_from_slice(b"</li>\n");
                }
                b.extend_from_slice(b"</");
                b.extend_from_slice(kind);
                b.extend_from_slice(b">\n");
            }
            Block::Quote(ss) => {
                b.extend_from_slice(b"<blockquote>");
                line(&mut b, ss);
                b.extend_from_slice(b"</blockquote>\n");
            }
            Block::Para(text) => {
                let ss = spans(text);
                tagged(&mut b, b"p", |b| line(b, &ss));
            }
            Block::Heading(n, ss) => {
                let kind = format!("h{n}");
                if *n == 1 && !titled {
                    title = plain(ss);
                    titled = true;
                }
                tagged(&mut b, kind.as_bytes(), |b| line(b, ss));
            }
        }
    }
    page(&title, &b)
}

// ------------------------------------------------------------ pages

/// `looksHTML(body)`: the first non-blank character within the first
/// 256 UTF-16 units is `<`
pub fn looks_html(body: &[u8]) -> bool {
    let mut n = 0;
    for (_, r, _) in runes(body) {
        n += if r >= 0x10000 { 2 } else { 1 };
        if n > 256 {
            return false;
        }
        if !space(r) {
            return r == u32::from(b'<');
        }
    }
    false
}

/// `renderAs(mark, body)`: `"html"`, `"md"`, `"gmi"` or `"text"`. No
/// mark (`""`) is %gmi, and only %gmi is sniffed; js, css, noun and
/// any other mark render as text.
pub fn render_as(mark: &str, body: &[u8]) -> &'static str {
    match mark {
        "gmi" | "" => {
            if looks_html(body) {
                "html"
            } else {
                "gmi"
            }
        }
        "html" => "html",
        "md" => "md",
        _ => "text",
    }
}

/// `rewriteHTML(body, port)`: a page's urb:// links pointed at the
/// gateway, as `((?i:href|src|action)\s*=\s*["']?)urb://~([a-z-]+)`
/// replaced by `${1}http://${2}.urb.localhost:<port>`
pub fn rewrite_html(body: &[u8], port: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len());
    let (mut last, mut i) = (0, 0);
    while i < body.len() {
        cost::step(1);
        match attr_at(&body[i..]) {
            Some((g1, g2)) => {
                out.extend_from_slice(&body[last..i + g1]);
                out.extend_from_slice(b"http://");
                out.extend_from_slice(&body[i + g1 + 7..i + g2]);
                out.extend_from_slice(HOST_SUFFIX.as_bytes());
                out.push(b':');
                out.extend_from_slice(port.as_bytes());
                i += g2;
                last = i;
            }
            None => i += 1,
        }
    }
    out.extend_from_slice(&body[last..]);
    out
}

/// the attribute `rewrite_html` matches at the start of s: the end of
/// its group 1 (where `urb://~` starts) and of the ship's name
fn attr_at(s: &[u8]) -> Option<(usize, usize)> {
    let mut at = [&b"href"[..], b"src", b"action"]
        .iter()
        .find_map(|w| folded_word(s, w))?;
    let ws = |s: &[u8], at: usize| {
        let n = s[at..]
            .iter()
            .position(|b| !b"\t\n\x0c\r ".contains(b))
            .unwrap_or(s.len() - at);
        cost::step(n);
        at + n
    };
    at = ws(s, at);
    if s.get(at) != Some(&b'=') {
        return None;
    }
    at = ws(s, at + 1);
    if matches!(s.get(at), Some(b'"' | b'\'')) {
        at += 1;
    }
    if !s[at..].starts_with(b"urb://~") {
        return None;
    }
    let ship = &s[at + 7..];
    let n = ship
        .iter()
        .position(|&b| !(b.is_ascii_lowercase() || b == b'-'))
        .unwrap_or(ship.len());
    cost::step(n);
    (n > 0).then_some((at, at + 7 + n))
}

/// `render(url, mark, body)`: a page as the gateway serves it, by the
/// mark it was published under, with its Content-Security-Policy. A
/// publisher's HTML is only ever served in the sandbox; everything
/// else the gateway renders itself, with no scripts, titled by its url.
pub fn render(url: &[u8], mark: &str, body: &[u8], port: &str) -> (&'static str, Vec<u8>) {
    let link = page_link(url, port);
    match render_as(mark, body) {
        "html" => (SANDBOX, rewrite_html(body, port)),
        "md" => (OWN, render_markdown(body, url, &link)),
        "gmi" => (OWN, render_gemtext(body, url, &link)),
        _ => (OWN, render_text(body, url)),
    }
}

/// the gateway's parsed blocks and spans, in [`crate::ext`]'s shapes
/// (as render_parity_test.go's `gmiJSON` and `mdJSON` give them), for
/// the tests and fuzzers that hold the two renderers together. Bytes
/// that are not UTF-8 read as U+FFFD.
#[doc(hidden)]
pub mod parsed {
    use super::{Block, Gem, Kind, Span, gemtext_blocks, markdown_blocks};
    use crate::ext;

    fn s(b: &[u8]) -> String {
        String::from_utf8_lossy(b).into_owned()
    }

    /// render.go's `spans(line)`
    pub fn spans(line: &[u8]) -> Vec<ext::Span> {
        super::spans(line).iter().map(span).collect()
    }

    fn span(x: &Span) -> ext::Span {
        match x.kind {
            Kind::Text => ext::Span::Text(s(x.text)),
            Kind::Code => ext::Span::Code(s(x.text)),
            Kind::Strong => ext::Span::Strong(s(x.text)),
            Kind::Em => ext::Span::Em(s(x.text)),
            Kind::Link => ext::Span::Link {
                link: s(x.target),
                text: s(x.text),
            },
        }
    }

    fn spans_of(ss: &[Span]) -> Vec<ext::Span> {
        ss.iter().map(span).collect()
    }

    /// render.go's `gemtextBlocks(body)`
    pub fn gemtext(body: &[u8]) -> Vec<ext::Gem> {
        gemtext_blocks(body)
            .iter()
            .map(|g| match g {
                Gem::Heading(n, t) => ext::Gem::Heading(*n as u8, s(t)),
                Gem::Para(t) => ext::Gem::Para(s(t)),
                Gem::Quote(t) => ext::Gem::Quote(s(t)),
                Gem::Pre(t) => ext::Gem::Pre(s(t)),
                Gem::List(items) => ext::Gem::List(items.iter().map(|i| s(i)).collect()),
                Gem::Link(href, label) => ext::Gem::Link {
                    href: s(href),
                    label: s(label),
                },
            })
            .collect()
    }

    /// render.go's `markdownBlocks(body)`
    pub fn markdown(body: &[u8]) -> Vec<ext::Block> {
        markdown_blocks(body)
            .iter()
            .map(|b| match b {
                Block::Heading(n, ss) => ext::Block::Heading(*n as u8, spans_of(ss)),
                Block::Para(text) => ext::Block::Para(spans(text)),
                Block::Quote(ss) => ext::Block::Quote(spans_of(ss)),
                Block::List(kind, items) => ext::Block::List {
                    ordered: *kind == b"ol",
                    items: items.iter().map(|i| spans_of(i)).collect(),
                },
                Block::Rule => ext::Block::Rule,
                Block::Pre(t) => ext::Block::Pre(s(t)),
            })
            .collect()
    }
}
