//! a body (and each of its lines as a link target) read by the
//! extension's renderers: never a panic, never a link the allowlist
//! refuses, and steps linear in the body's size
#![no_main]
use libfuzzer_sys::fuzz_target;
use urb_render::ext::{self, Gem, Node, Span};
use urb_render::{cost, js};
use urb_render_fuzz::{PAGE, clean, ext_href, linear};

fn nodes(n: &Node) {
    if let Node::El {
        tag, href, kids, ..
    } = n
    {
        assert_eq!(href.is_some(), *tag == "a", "{tag} with {href:?}");
        if let Some(h) = href {
            assert!(ext_href(h), "an href {h:?}");
        }
        kids.iter().for_each(nodes);
    }
}

fn target(t: &str) {
    let to = ext::link_target(PAGE, t);
    assert!(
        to.is_empty()
            || to.starts_with("urb://~")
            || to.starts_with("http://")
            || to.starts_with("https://"),
        "linkTarget {to:?}"
    );
    assert!(clean(to.as_bytes()), "linkTarget {to:?}");
    if let Some(h) = ext::link_href(PAGE, t) {
        assert!(ext_href(&h), "linkHref {h:?}");
    }
}

fuzz_target!(|data: &[u8]| {
    let body = String::from_utf8_lossy(data);
    cost::reset();
    for g in ext::gemtext(&body) {
        if let Gem::Link { href, .. } = g {
            target(&href);
        }
    }
    let ns = ext::md_nodes(&body, |t| ext::link_href(PAGE, t));
    ns.iter().for_each(nodes);
    if js::units(&body) > ext::MD_MAX {
        assert!(matches!(&ns[..], [Node::El { tag: "pre", .. }]));
    }
    for l in body.split('\n') {
        for s in ext::spans(l) {
            if let Span::Link { link, .. } = s {
                assert!(ext::safe_link(&link), "a span links {link:?}");
            }
        }
        target(l);
    }
    urb_render::render_as(None, &body);
    linear(data.len(), "ext");
});
