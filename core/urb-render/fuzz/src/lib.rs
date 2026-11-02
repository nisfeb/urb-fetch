//! What the renderers' fuzz targets check: that each link they give is
//! one the allowlist lets through, and that their steps are linear in
//! the body's size.

use urb_render::cost;

/// the page every link is on
pub const PAGE: &str = "urb://~zod/a/b";
pub const PORT: &str = "8475";

/// at most this many steps a byte, and this many more: the same bound
/// as tests/linear.rs
pub fn linear(len: usize, what: &str) {
    let s = cost::steps();
    assert!(
        s <= 64 * len as u64 + 1000,
        "{what}: {s} steps for {len} bytes"
    );
}

/// a target with no C0 control, space or DEL, not protocol-relative
pub fn clean(t: &[u8]) -> bool {
    !t.iter().any(|&b| b <= 0x20 || b == 0x7f)
        && !(t.len() >= 2 && matches!(t[0], b'/' | b'\\') && matches!(t[1], b'/' | b'\\'))
}

/// whether t starts with one of the schemes, in lower case
pub fn scheme(t: &[u8], schemes: &[&str]) -> bool {
    schemes.iter().any(|s| t.starts_with(s.as_bytes()))
}

/// an href the extension's reader may set: a reader page (a path in
/// the extension), the web, mail or gemini
pub fn ext_href(h: &str) -> bool {
    clean(h.as_bytes())
        && (h.starts_with("reader.html?u=urb%3A%2F%2F~")
            || scheme(h.as_bytes(), &["http://", "https://", "mailto:", "gemini:"]))
}

/// each `<a href="…">` in the gateway's HTML, unescaped of what
/// html.EscapeString escapes
pub fn hrefs(html: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let pat = b"<a href=\"";
    let mut i = 0;
    while let Some(at) = html[i..].windows(pat.len()).position(|w| w == pat) {
        let start = i + at + pat.len();
        let end = start
            + html[start..]
                .iter()
                .position(|&b| b == b'"')
                .expect("a closed href");
        let h = String::from_utf8_lossy(&html[start..end])
            .replace("&#34;", "\"")
            .replace("&#39;", "'")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&amp;", "&");
        out.push(h.into_bytes());
        i = end;
    }
    out
}

/// an href the gateway may write: its own origin for a ship, the web,
/// mail or gemini
pub fn gateway_href(h: &[u8]) -> bool {
    clean(h) && scheme(h, &["http://", "https://", "mailto:", "gemini:"])
}
