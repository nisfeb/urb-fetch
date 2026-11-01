//! urb:// rendering (docs/rust-core.md, M5b): which renderer a page
//! gets, and the renderers, in the two forms that exist today.
//!
//! - [`ext`] is nisfeb-extension's `lib/gemtext.js` and
//!   `lib/markdown.js`: the parsed structures its reader builds nodes
//!   from, with the same names.
//! - [`gateway`] is the Go gateway's `render.go` and `urb.go`: the HTML
//!   it serves, byte for byte, and the Content-Security-Policy it
//!   serves it under.
//!
//! Each is a port, and its source is the specification: where this
//! crate would behave differently, this crate is wrong. The two
//! sources still differ in a few places (docs/rust-core.md, "M5b's
//! findings"), so each module ports its own. Pure Rust: no C, no IO,
//! and nothing from the `urb` crate. Every body is hostile input:
//! nothing here panics on one, and each renderer's time is linear in
//! its size.

pub mod cost;
pub mod ext;
pub mod gateway;
pub mod js;
pub mod ship;
mod whatwg;

/// wasm/mesa.mjs's `looksHtml` (lattice-gmi's `+looks-html`): the
/// first non-blank character of the body's first 256 UTF-16 units is
/// `<`, blank being JavaScript's whitespace.
pub fn looks_html(body: &str) -> bool {
    let mut units = 0;
    for c in body.chars() {
        if units >= 256 {
            return false;
        }
        units += c.len_utf16();
        cost::step(1);
        if !js::is_space(c) {
            return c == '<';
        }
    }
    false
}

/// wasm/mesa.mjs's `renderAs(mark, body)`: how a page published under
/// mark renders, `"html"`, `"md"`, `"gmi"` or `"text"`. No mark (a /k/
/// or /t/ read) is %gmi, and only %gmi is sniffed. A mark of `""` is
/// `"text"` here, as in the JS; the gateway reads it as no mark
/// ([`gateway::render_as`]).
pub fn render_as(mark: Option<&str>, body: &str) -> &'static str {
    match mark {
        None | Some("gmi") => {
            if looks_html(body) {
                "html"
            } else {
                "gmi"
            }
        }
        Some("html") => "html",
        Some("md") => "md",
        Some(_) => "text",
    }
}
