//! `render.wasm` (docs/rust-core.md, M6): urb-render's extension half,
//! and the pure helpers mesa.mjs exports (deUrb, uv, da), for the
//! shims whose calls are synchronous: gemtext.js, markdown.js, and
//! mesa.mjs's renderAs, looksHtml, deUrb, uv and da. The extension's
//! service worker imports them, and a module service worker can't wait
//! on a top-level await, so the shim compiles this module synchronously
//! from bytes it carries. It has no imports at all, WASI's included.
//!
//! Every export takes its arguments as one JSON array, through a buffer
//! from `render_alloc`, and leaves its answer as JSON in the result
//! buffer (`render_out_ptr`, `render_out_len`): 0 for an answer, 1 for
//! an error's message, as the JS throws it.

use serde_json::{Map, Value as J, json};
use std::cell::RefCell;
use urb_render::ext::{self, Block, Gem, Node, Span};

thread_local! {
    static OUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

#[unsafe(no_mangle)]
pub extern "C" fn render_alloc(n: usize) -> *mut u8 {
    Box::into_raw(vec![0u8; n].into_boxed_slice()).cast()
}

#[unsafe(no_mangle)]
pub extern "C" fn render_out_ptr() -> *const u8 {
    OUT.with(|o| o.borrow().as_ptr())
}

#[unsafe(no_mangle)]
pub extern "C" fn render_out_len() -> usize {
    OUT.with(|o| o.borrow().len())
}

/// the arguments the shim handed in, taken
unsafe fn args(p: *mut u8, n: usize) -> Result<Vec<J>, String> {
    let b = unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(p, n)) };
    match serde_json::from_slice(&b) {
        Ok(J::Array(a)) => Ok(a),
        _ => Err("render.wasm: arguments are a JSON array".into()),
    }
}

fn answer(r: Result<J, String>) -> i32 {
    let (code, text) = match r {
        Ok(j) => (0, j.to_string()),
        Err(e) => (1, e),
    };
    OUT.with(|o| *o.borrow_mut() = text.into_bytes());
    code
}

fn text(a: &[J], i: usize) -> Result<&str, String> {
    a.get(i)
        .and_then(J::as_str)
        .ok_or_else(|| format!("render.wasm: argument {i} is not text"))
}

macro_rules! export {
    ($name:ident, |$a:ident| $body:expr) => {
        /// # Safety
        /// `p` and `n` must be what `render_alloc` gave, filled.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(p: *mut u8, n: usize) -> i32 {
            answer(unsafe { args(p, n) }.and_then(|$a| $body))
        }
    };
}

pub fn gem_json(g: &Gem) -> J {
    match g {
        Gem::Heading(_, t) | Gem::Para(t) | Gem::Quote(t) | Gem::Pre(t) => {
            json!({"kind": g.kind(), "text": t})
        }
        Gem::List(items) => json!({"kind": "list", "items": items}),
        Gem::Link { href, label } => json!({"kind": "link", "href": href, "label": label}),
    }
}

pub fn span_json(s: &Span) -> J {
    match s {
        Span::Text(t) => json!({"text": t}),
        Span::Code(t) => json!({"code": t}),
        Span::Strong(t) => json!({"strong": t}),
        Span::Em(t) => json!({"em": t}),
        Span::Link { link, text } => json!({"link": link, "text": text}),
    }
}

fn spans_json(ss: &[Span]) -> J {
    J::Array(ss.iter().map(span_json).collect())
}

pub fn block_json(b: &Block) -> J {
    match b {
        Block::Heading(_, ss) | Block::Para(ss) | Block::Quote(ss) => {
            json!({"kind": b.kind(), "spans": spans_json(ss)})
        }
        Block::List { items, .. } => {
            json!({"kind": b.kind(), "items": items.iter().map(|i| spans_json(i)).collect::<Vec<_>>()})
        }
        Block::Rule => json!({"kind": "hr"}),
        Block::Pre(t) => json!({"kind": "pre", "text": t}),
    }
}

/// a node for the shim to build: text, or `{tag, href, rel, kids}`
pub fn node_json(n: &Node) -> J {
    match n {
        Node::Text(t) => J::String(t.clone()),
        Node::El {
            tag,
            href,
            rel,
            kids,
        } => {
            let mut o = Map::new();
            o.insert("tag".into(), json!(tag));
            if let Some(h) = href {
                o.insert("href".into(), json!(h));
            }
            if *rel {
                o.insert("rel".into(), J::Bool(true));
            }
            o.insert(
                "kids".into(),
                J::Array(kids.iter().map(node_json).collect()),
            );
            J::Object(o)
        }
    }
}

export!(render_gemtext, |a| Ok(J::Array(
    ext::gemtext(text(&a, 0)?).iter().map(gem_json).collect()
)));

export!(render_markdown, |a| Ok(J::Array(
    ext::markdown(text(&a, 0)?).iter().map(block_json).collect()
)));

export!(render_spans, |a| Ok(spans_json(&ext::spans(text(&a, 0)?))));

export!(render_safe_link, |a| Ok(J::Bool(ext::safe_link(text(
    &a, 0
)?))));

export!(render_link_target, |a| Ok(J::String(ext::link_target(
    text(&a, 0)?,
    text(&a, 1)?
))));

export!(render_link_href, |a| Ok(ext::link_href(
    text(&a, 0)?,
    text(&a, 1)?
)
.map_or(J::Null, J::String)));

export!(render_reader_path, |a| Ok(J::String(ext::reader_path(
    text(&a, 0)?
))));

// mdNodes(doc, body, hrefFor), in two calls: the targets hrefFor is
// asked of, in order, and then the nodes, given its answers in that
// order (null or "" for none). Its answers never change what it is
// asked next, so each target is asked once, as the JS asks it.
export!(render_md_targets, |a| {
    let mut asked = Vec::new();
    ext::md_nodes(text(&a, 0)?, |t| {
        asked.push(J::String(t.to_string()));
        None
    });
    Ok(J::Array(asked))
});

export!(render_md_nodes, |a| {
    let hrefs: Vec<Option<String>> = match a.get(1) {
        Some(J::Array(h)) => h
            .iter()
            .map(|x| x.as_str().filter(|s| !s.is_empty()).map(String::from))
            .collect(),
        _ => return Err("render.wasm: no answers".into()),
    };
    let mut at = 0;
    let nodes = ext::md_nodes(text(&a, 0)?, |_| {
        at += 1;
        hrefs.get(at - 1).cloned().flatten()
    });
    if at != hrefs.len() {
        return Err(format!(
            "render.wasm: {} answers for {at} targets",
            hrefs.len()
        ));
    }
    Ok(J::Array(nodes.iter().map(node_json).collect()))
});

export!(render_render_as, |a| Ok(J::String(
    urb_render::render_as(a.first().and_then(J::as_str), text(&a, 1)?).into()
)));

export!(render_looks_html, |a| Ok(J::Bool(urb_render::looks_html(
    text(&a, 0)?
))));

// deUrb(url): {ship, rel, mount, post} (post in decimal, or null), or
// its UrbError's message
export!(render_de_urb, |a| {
    let u = urb::url::de_urb(text(&a, 0)?).map_err(|e| e.message)?;
    let (mount, post) = match &u.mount {
        urb::url::Mount::N => ("n", None),
        urb::url::Mount::K => ("k", None),
        urb::url::Mount::P => ("p", None),
        urb::url::Mount::T => ("t", None),
        urb::url::Mount::Forum { post } => ("forum", post.as_ref().map(|p| p.decimal())),
    };
    Ok(json!({"ship": u.ship, "rel": u.rel, "mount": mount, "post": post}))
});

// uv(n), n in decimal
export!(render_uv, |a| {
    let d = text(&a, 0)?;
    let mut le: Vec<u8> = Vec::new();
    for c in d.bytes() {
        if !c.is_ascii_digit() {
            return Err(format!("render.wasm: not a decimal: {d}"));
        }
        let mut carry = u32::from(c - b'0');
        for b in le.iter_mut() {
            let v = u32::from(*b) * 10 + carry;
            *b = v as u8;
            carry = v >> 8;
        }
        while carry > 0 {
            le.push(carry as u8);
            carry >>= 8;
        }
    }
    Ok(J::String(urb::text::uv(&urb::Int::from_le(&le))))
});

// da(ms), ms a whole number of milliseconds, in decimal
export!(render_da, |a| {
    let ms = a
        .first()
        .and_then(J::as_u64)
        .ok_or("render.wasm: da takes whole milliseconds")?;
    Ok(J::String(urb::text::da(ms).to_string()))
});

// pub.mjs's pageKey(name): '/<name>', or null
export!(render_page_key, |a| Ok(urb::publish::page_key(text(
    &a, 0
)?)
.map_or(J::Null, J::String)));
