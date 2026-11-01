//! nisfeb-extension's renderers, `lib/gemtext.js` and `lib/markdown.js`
//! (at the commit conformance/render-ext.json names): the blocks and
//! spans its reader builds nodes from, where a page's links go, and
//! `mdNodes`' nodes. Same names, same structures. Text is a JS string
//! here, so lengths are UTF-16 code units and whitespace is
//! JavaScript's ([`crate::js`]).

use crate::js::{self, is_space, units};
use crate::{cost, ship, whatwg};

/// `MD_MAX`: a Markdown page longer than this, in UTF-16 code units,
/// shows as text in a `<pre>`
pub const MD_MAX: usize = 256 * 1024;

// ------------------------------------------------------------ links

/// `safeLink(t)`: whether a link target may be a link. The web, urb://,
/// gemini, mail (schemes in any case, ASCII only) or a path with no
/// colon; never with a C0 control, a space or DEL in it, and never
/// protocol-relative (`//` or `\\`, after JavaScript's whitespace).
pub fn safe_link(t: &str) -> bool {
    cost::step(t.len());
    if t.chars().any(|c| c <= ' ' || c == '\u{7f}') {
        return false;
    }
    let scheme = ["https:", "http:", "urb:", "gemini:", "mailto:"]
        .iter()
        .any(|s| t.len() >= s.len() && t.as_bytes()[..s.len()].eq_ignore_ascii_case(s.as_bytes()));
    if !scheme && t.contains(':') {
        return false;
    }
    let r = js::trim_start(t).as_bytes();
    !(r.len() >= 2 && matches!(r[0], b'/' | b'\\') && matches!(r[1], b'/' | b'\\'))
}

/// the length of `/^[a-z][a-z0-9+.-]*:/i`'s match, ASCII only
fn scheme_len(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    if !b.first()?.is_ascii_alphabetic() {
        return None;
    }
    let n = b
        .iter()
        .position(|&c| !(c.is_ascii_alphanumeric() || matches!(c, b'+' | b'.' | b'-')))?;
    cost::step(n);
    (b[n] == b':').then_some(n + 1)
}

/// the scheme in lower case, as `.replace(/^[a-z][a-z0-9+.-]*:/i, s =>
/// s.toLowerCase())`
fn lower_scheme(s: &str) -> String {
    match scheme_len(s) {
        Some(n) => s[..n].to_ascii_lowercase() + &s[n..],
        None => s.to_string(),
    }
}

/// `/^urb:\/\/(~[a-z-]+)/`'s group, and what follows it
fn urb_ship(s: &str) -> Option<(&str, &str)> {
    let rest = s.strip_prefix("urb://~")?;
    let n = rest
        .bytes()
        .position(|b| !(b.is_ascii_lowercase() || b == b'-'))
        .unwrap_or(rest.len());
    cost::step(n);
    (n > 0).then(|| (&s[6..7 + n], &rest[n..]))
}

/// `linkTarget(base, href)`: where a link on the page at the urb://
/// address base goes: an urb:// address (a relative link resolved
/// against the page), an http(s) URL, or `""` for anything else.
pub fn link_target(base: &str, href: &str) -> String {
    let h = lower_scheme(js::trim(href));
    if !safe_link(&h) {
        return String::new();
    }
    if let Some((who, rest)) = urb_ship(&h)
        && (rest.is_empty() || rest.starts_with('/'))
    {
        return if ship::is_ship(who) { h } else { String::new() };
    }
    let web = h
        .strip_prefix("https://")
        .or_else(|| h.strip_prefix("http://"));
    if web.is_some_and(|r| !r.is_empty() && !r.starts_with('/')) {
        return h;
    }
    if h.is_empty() || scheme_len(&h).is_some() || h.starts_with("//") {
        return String::new();
    }
    let Some((who, rest)) = urb_ship(base) else {
        return String::new();
    };
    //  /(\/[^?#]*)?/: the page's path, if it has one
    let path = if rest.starts_with('/') {
        &rest[..rest.find(['?', '#']).unwrap_or(rest.len())]
    } else {
        ""
    };
    let p = whatwg::pathname(if path.is_empty() { "/" } else { path }, &h);
    format!("urb://{who}{}", if p == "/" { "" } else { &p })
}

/// `linkHref(base, target)`: the href for a link in a page at base: a
/// reader page for urb://, the web address, or mail and gemini as
/// written (scheme in lower case). `None` leaves it as text.
pub fn link_href(base: &str, target: &str) -> Option<String> {
    if !safe_link(target) {
        return None;
    }
    let t = lower_scheme(target);
    if t.starts_with("mailto:") || t.starts_with("gemini:") {
        return Some(t);
    }
    let a = link_target(base, &t);
    if a.starts_with("urb://") {
        Some(reader_path(&a))
    } else if a.is_empty() {
        None
    } else {
        Some(a)
    }
}

/// `readerPath(addr)`: the reader page for an urb:// address, as
/// `reader.html?u=${encodeURIComponent(addr)}`
pub fn reader_path(addr: &str) -> String {
    let mut out = String::from("reader.html?u=");
    cost::step(addr.len());
    for b in addr.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

// ------------------------------------------------------------ gemtext

/// one of `gemtext`'s blocks: `{kind: 'h1' | 'h2' | 'h3', text}`,
/// `{kind: 'p' | 'quote' | 'pre', text}`, `{kind: 'list', items}` or
/// `{kind: 'link', href, label}`
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Gem {
    Heading(u8, String),
    Para(String),
    Quote(String),
    Pre(String),
    List(Vec<String>),
    Link { href: String, label: String },
}

impl Gem {
    /// the block's `kind`
    pub fn kind(&self) -> &'static str {
        match self {
            Gem::Heading(1, _) => "h1",
            Gem::Heading(2, _) => "h2",
            Gem::Heading(..) => "h3",
            Gem::Para(_) => "p",
            Gem::Quote(_) => "quote",
            Gem::Pre(_) => "pre",
            Gem::List(_) => "list",
            Gem::Link { .. } => "link",
        }
    }
}

/// `gemtext(body)`: a page's blocks, in order. A link's href is as
/// written; [`link_target`] says where it goes.
pub fn gemtext(body: &str) -> Vec<Gem> {
    let mut out = Vec::new();
    let mut pre: Option<Vec<String>> = None;
    let mut list: Option<usize> = None;
    for line in crlf_lines(body) {
        cost::step(1);
        if line.starts_with("```") {
            list = None;
            match pre.take() {
                Some(p) => out.push(Gem::Pre(p.join("\n"))),
                None => pre = Some(Vec::new()),
            }
            continue;
        }
        if let Some(p) = &mut pre {
            p.push(line.to_string());
            continue;
        }
        let item = line.starts_with("* ");
        if !item {
            list = None;
        }
        if let Some(rest) = line.strip_prefix("=>") {
            let rest = js::trim(rest);
            if rest.is_empty() {
                continue;
            }
            let href = &rest[..rest.find(is_space).unwrap_or(rest.len())];
            let label = js::trim(&rest[href.len()..]);
            let label = if label.is_empty() { href } else { label };
            out.push(Gem::Link {
                href: href.to_string(),
                label: label.to_string(),
            });
        } else if line.starts_with('#') {
            let marks = line.len() - line.trim_start_matches('#').len();
            out.push(Gem::Heading(
                marks.min(3) as u8,
                js::trim(&line[marks..]).to_string(),
            ));
        } else if item {
            let at = *list.get_or_insert_with(|| {
                out.push(Gem::List(Vec::new()));
                out.len() - 1
            });
            if let Gem::List(items) = &mut out[at] {
                items.push(line[2..].to_string());
            }
        } else if let Some(q) = line.strip_prefix('>') {
            out.push(Gem::Quote(js::trim(q).to_string()));
        } else if !js::trim(line).is_empty() {
            out.push(Gem::Para(line.to_string()));
        }
    }
    if let Some(p) = pre {
        out.push(Gem::Pre(p.join("\n")));
    }
    out
}

/// the body's lines as `String(body).replace(/\r\n/g, '\n').split('\n')`
/// gives them: a `\r` before a `\n` goes, any other stays
fn crlf_lines(body: &str) -> impl Iterator<Item = &str> {
    cost::step(body.len());
    let mut parts = body.split('\n').peekable();
    std::iter::from_fn(move || {
        let l = parts.next()?;
        Some(match parts.peek() {
            Some(_) => l.strip_suffix('\r').unwrap_or(l),
            None => l,
        })
    })
}

// ------------------------------------------------------------ Markdown

/// one of `spans`' spans: `{text}`, `{code}`, `{strong}`, `{em}`, or
/// `{link, text}`, link the target as written, there only when
/// [`safe_link`] allows it
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Span {
    Text(String),
    Code(String),
    Strong(String),
    Em(String),
    Link { link: String, text: String },
}

/// one of `markdown`'s blocks: `{kind: 'h1'..'h6' | 'p' | 'quote',
/// spans}`, `{kind: 'ul' | 'ol', items: [spans]}`, `{kind: 'hr'}` or
/// `{kind: 'pre', text}`
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    Heading(u8, Vec<Span>),
    Para(Vec<Span>),
    Quote(Vec<Span>),
    List {
        ordered: bool,
        items: Vec<Vec<Span>>,
    },
    Rule,
    Pre(String),
}

impl Block {
    /// the block's `kind`
    pub fn kind(&self) -> &'static str {
        match self {
            Block::Heading(n, _) => {
                ["h1", "h2", "h3", "h4", "h5", "h6"][(*n as usize).clamp(1, 6) - 1]
            }
            Block::Para(_) => "p",
            Block::Quote(_) => "quote",
            Block::List { ordered: false, .. } => "ul",
            Block::List { ordered: true, .. } => "ol",
            Block::Rule => "hr",
            Block::Pre(_) => "pre",
        }
    }
}

/// a line with a `[` in it, read once so that each `[` costs the same:
/// at each byte, the UTF-16 units before it, the next `]`, and the
/// next `)` or space. A scan from each `[` to its `]` and on to its
/// `)` would look at up to 768 units, and a line of `[` or `[a](` at
/// every one of them, which is linear but slow.
struct Scan {
    units: Vec<u32>,
    close: Vec<u32>,
    stop: Vec<u32>,
}

impl Scan {
    fn new(line: &str) -> Scan {
        let n = line.len();
        cost::step(n);
        let (mut units, mut close, mut stop) =
            (vec![0; n + 1], vec![n as u32; n + 1], vec![n as u32; n + 1]);
        let mut u = 0;
        for (i, c) in line.char_indices() {
            units[i..i + c.len_utf8()].fill(u);
            u += c.len_utf16() as u32;
        }
        units[n] = u;
        for (i, c) in line.char_indices().rev() {
            let w = c.len_utf8();
            let c2 = if c == ']' { i as u32 } else { close[i + w] };
            close[i..i + w].fill(c2);
            let s2 = if c == ')' || is_space(c) {
                i as u32
            } else {
                stop[i + w]
            };
            stop[i..i + w].fill(s2);
        }
        Scan { units, close, stop }
    }

    /// from byte `at`, the end of the run up to the first stop in
    /// `next`, if the stop is there and the run is 1 to `limit` units
    fn run(&self, next: &[u32], at: usize, limit: u32) -> Option<usize> {
        let e = next[at] as usize;
        let n = self.units[e] - self.units[at];
        (e < next.len() - 1 && (1..=limit).contains(&n)).then_some(e)
    }
}

/// the span `INLINE` matches at byte `i` of `line`, and its end:
/// `` `([^`]+)` ``, `\[([^\]]{1,256})\]\(([^)\s]{1,512})\)`,
/// `\*\*([^*]+)\*\*` or `\*([^*\s][^*]*)\*`, tried in that order
fn inline_at(line: &str, i: usize, scan: &mut Option<Scan>) -> Option<(Span, usize)> {
    let rest = &line[i..];
    let b = rest.as_bytes();
    match b[0] {
        b'`' => {
            let e = rest[1..].find('`')?;
            cost::step(e);
            (e > 0).then(|| (Span::Code(rest[1..1 + e].to_string()), i + e + 2))
        }
        b'[' => {
            //  `[^\]]{1,256}` then `](`, then `[^)\s]{1,512}` then `)`
            let sc = scan.get_or_insert_with(|| Scan::new(line));
            let close = sc.run(&sc.close, i + 1, 256)?;
            if line.as_bytes().get(close + 1) != Some(&b'(') {
                return None;
            }
            let end = sc.run(&sc.stop, close + 2, 512)?;
            if line.as_bytes()[end] != b')' {
                return None;
            }
            let (e, tgt, f) = (close - i, &line[close + 2..], end - close - 2);
            let (text, target) = (&rest[1..e], &tgt[..f]);
            let span = if safe_link(target) {
                Span::Link {
                    link: target.to_string(),
                    text: text.to_string(),
                }
            } else {
                Span::Text(text.to_string())
            };
            Some((span, i + e + 2 + f + 1))
        }
        b'*' => {
            if b.get(1) == Some(&b'*')
                && let Some(e) = rest[2..].find('*')
            {
                cost::step(e);
                if e > 0 && b.get(2 + e + 1) == Some(&b'*') {
                    return Some((Span::Strong(rest[2..2 + e].to_string()), i + 2 + e + 2));
                }
            }
            let first = rest[1..].chars().next()?;
            if first == '*' || is_space(first) {
                return None;
            }
            let e = rest[1..].find('*')?;
            cost::step(e);
            Some((Span::Em(rest[1..1 + e].to_string()), i + 1 + e + 1))
        }
        _ => None,
    }
}

/// `spans(line)`: a line's spans, as `INLINE` (a global regex) cuts it
pub fn spans(line: &str) -> Vec<Span> {
    let mut out = Vec::new();
    let mut scan = None;
    let (mut at, mut i) = (0, 0);
    while i < line.len() {
        cost::step(1);
        //  every span starts with an ASCII byte, so at a character
        let found = match line.as_bytes()[i] {
            b'`' | b'[' | b'*' => inline_at(line, i, &mut scan),
            _ => None,
        };
        match found {
            Some((span, end)) => {
                if i > at {
                    out.push(Span::Text(line[at..i].to_string()));
                }
                out.push(span);
                at = end;
                i = end;
            }
            None => i += 1,
        }
    }
    if at < line.len() {
        out.push(Span::Text(line[at..].to_string()));
    }
    out
}

/// `/^\s*\d+[.)]/`'s match, or `[-*+]`'s, after the leading space: the
/// rest of the line after the marker, and whether it is a number's
fn marker(line: &str) -> Option<(&str, bool)> {
    let s = js::trim_start(line);
    let b = s.as_bytes();
    match b.first()? {
        b'-' | b'*' | b'+' => Some((&s[1..], false)),
        b'0'..=b'9' => {
            let n = b
                .iter()
                .position(|c| !c.is_ascii_digit())
                .unwrap_or(b.len());
            cost::step(n);
            matches!(b.get(n), Some(b'.' | b')')).then(|| (&s[n + 1..], true))
        }
        _ => None,
    }
}

/// `ITEM.exec(line)`, `/^\s*(?:[-*+]|\d+[.)])\s+(.*)$/`: the item's
/// text, and whether `ORDERED` (`/^\s*\d+[.)]\s/`) matches too. The
/// space after the marker is taken whole, and what follows must hold
/// no line terminator, as `.*$` asks; read in one pass, where the
/// regex backtracks over the space once for each of its characters.
fn item(line: &str) -> Option<(&str, bool)> {
    let (after, ordered) = marker(line)?;
    let text = js::trim_start(after);
    if text.len() == after.len() {
        return None;
    }
    cost::step(text.len());
    if text.chars().any(js::is_terminator) {
        return None;
    }
    Some((text, ordered))
}

/// `RULE.test(line)`: `/^\s*(?:(?:-\s*){3,}|(?:\*\s*){3,}|(?:_\s*){3,})$/`
fn rule(line: &str) -> bool {
    let s = js::trim_start(line);
    let Some(m) = s.chars().next().filter(|c| matches!(c, '-' | '*' | '_')) else {
        return false;
    };
    cost::step(s.len());
    s.chars().all(|c| c == m || is_space(c)) && s.chars().filter(|&c| c == m).count() >= 3
}

/// `HEADING.exec(line)`, `/^(#{1,6})\s/`: the number of #s
fn heading(line: &str) -> Option<usize> {
    let n = line.len() - line.trim_start_matches('#').len();
    cost::step(n);
    ((1..=6).contains(&n) && line[n..].chars().next().is_some_and(is_space)).then_some(n)
}

/// `headingText(rest)`: a heading's text, less the #s that close it
fn heading_text(rest: &str) -> &str {
    js::trim(js::trim_end(rest).trim_end_matches('#'))
}

/// `markdown(body)`: a page's blocks, in order
pub fn markdown(body: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut para: Vec<&str> = Vec::new();
    let mut list: Option<usize> = None;
    let flush = |out: &mut Vec<Block>, para: &mut Vec<&str>, list: &mut Option<usize>| {
        if !para.is_empty() {
            out.push(Block::Para(spans(&para.join(" "))));
        }
        para.clear();
        *list = None;
    };
    let mut pre: Option<Vec<&str>> = None;
    for line in crlf_lines(body) {
        cost::step(1);
        if js::trim(line).starts_with("```") {
            match pre.take() {
                Some(p) => out.push(Block::Pre(p.join("\n"))),
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
            let same = list.is_some_and(
                |at| matches!(&out[at], Block::List { ordered: o, .. } if *o == ordered),
            );
            if !para.is_empty() || !same {
                flush(&mut out, &mut para, &mut list);
                out.push(Block::List {
                    ordered,
                    items: Vec::new(),
                });
                list = Some(out.len() - 1);
            }
            if let Some(Block::List { items, .. }) = list.map(|at| &mut out[at]) {
                items.push(spans(text));
            }
            continue;
        }
        let t = js::trim(line);
        if t.is_empty() {
            flush(&mut out, &mut para, &mut list);
        } else if let Some(n) = heading(line) {
            flush(&mut out, &mut para, &mut list);
            out.push(Block::Heading(n as u8, spans(heading_text(&line[n..]))));
        } else if rule(line) {
            flush(&mut out, &mut para, &mut list);
            out.push(Block::Rule);
        } else if let Some(q) = t.strip_prefix('>') {
            flush(&mut out, &mut para, &mut list);
            out.push(Block::Quote(spans(js::trim(q))));
        } else {
            if list.is_some() {
                flush(&mut out, &mut para, &mut list);
            }
            para.push(t);
        }
    }
    if let Some(p) = pre {
        out.push(Block::Pre(p.join("\n")));
    }
    flush(&mut out, &mut para, &mut list);
    out
}

/// one of `mdNodes`' nodes, as a DOM would hold it: a text node, or an
/// element of a fixed name with its children. Only an `a` has
/// attributes: its href, and `rel="noopener noreferrer"` when the href
/// is not a reader page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    Text(String),
    El {
        tag: &'static str,
        href: Option<String>,
        rel: bool,
        kids: Vec<Node>,
    },
}

fn el(tag: &'static str, kids: Vec<Node>) -> Node {
    Node::El {
        tag,
        href: None,
        rel: false,
        kids,
    }
}

/// an element whose text is set as textContent: a text node, or none
/// for `""`
fn el_text(tag: &'static str, text: &str) -> Node {
    el(
        tag,
        if text.is_empty() {
            Vec::new()
        } else {
            vec![Node::Text(text.to_string())]
        },
    )
}

/// `mdNodes(doc, body, hrefFor)`: the page's nodes, text always set as
/// text. `href_for(target)` says where a safe link goes from the page,
/// or `None` (or `""`) to leave it as text. A body over [`MD_MAX`] is
/// one `<pre>` of it as written.
pub fn md_nodes(body: &str, mut href_for: impl FnMut(&str) -> Option<String>) -> Vec<Node> {
    if units(body) > MD_MAX {
        return vec![el_text("pre", body)];
    }
    let mut line = |tag: &'static str, ss: &[Span]| {
        let kids = ss
            .iter()
            .map(|s| match s {
                Span::Code(t) => el_text("code", t),
                Span::Strong(t) => el_text("strong", t),
                Span::Em(t) => el_text("em", t),
                Span::Text(t) => Node::Text(t.clone()),
                Span::Link { link, text } => match href_for(link).filter(|h| !h.is_empty()) {
                    None => Node::Text(text.clone()),
                    //  a link's text is never empty
                    Some(href) => Node::El {
                        tag: "a",
                        rel: !href.starts_with("reader.html"),
                        href: Some(href),
                        kids: vec![Node::Text(text.clone())],
                    },
                },
            })
            .collect();
        el(tag, kids)
    };
    markdown(body)
        .iter()
        .map(|b| match b {
            Block::Pre(t) => el_text("pre", t),
            Block::Rule => el("hr", Vec::new()),
            Block::List { items, .. } => {
                el(b.kind(), items.iter().map(|it| line("li", it)).collect())
            }
            Block::Quote(ss) => line("blockquote", ss),
            Block::Heading(_, ss) | Block::Para(ss) => line(b.kind(), ss),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reader_paths_as_encode_uri_component() {
        // node: encodeURIComponent("urb://~zod/a b'()!*~é😀?#&=+$,;@[]")
        assert_eq!(
            reader_path("urb://~zod/a b'()!*~é😀?#&=+$,;@[]"),
            "reader.html?u=urb%3A%2F%2F~zod%2Fa%20b'()!*~%C3%A9%F0%9F%98%80%3F%23%26%3D%2B%24%2C%3B%40%5B%5D"
        );
    }

    #[test]
    fn crlf_only_before_a_newline() {
        assert_eq!(crlf_lines("a\r\nb\r").collect::<Vec<_>>(), ["a", "b\r"]);
        assert_eq!(crlf_lines("a\r\r\n").collect::<Vec<_>>(), ["a\r", ""]);
    }
}
