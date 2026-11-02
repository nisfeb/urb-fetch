//! What the tests share: the extension's structures as JSON, as its
//! JS gives them (and conformance/lib.mjs's document gives mdNodes').

#![allow(dead_code)]

use serde_json::{Map, Value as J, json};
use urb_render::ext::{Block, Gem, Node, Span};

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

pub fn node_json(n: &Node) -> J {
    match n {
        Node::Text(t) => json!({"text": t}),
        Node::El {
            tag,
            href,
            rel,
            kids,
        } => {
            let mut o = Map::new();
            o.insert("tag".into(), json!(tag));
            let mut attrs = Map::new();
            if let Some(h) = href {
                attrs.insert("href".into(), json!(h));
            }
            if *rel {
                attrs.insert("rel".into(), json!("noopener noreferrer"));
            }
            if !attrs.is_empty() {
                o.insert("attrs".into(), J::Object(attrs));
            }
            o.insert(
                "kids".into(),
                J::Array(kids.iter().map(node_json).collect()),
            );
            J::Object(o)
        }
    }
}
