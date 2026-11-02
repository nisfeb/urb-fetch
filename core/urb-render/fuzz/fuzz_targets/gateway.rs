//! a body, any bytes, rendered by the gateway as every mark: never a
//! panic, no tag but the gateway's own in what it renders itself, never
//! a link the allowlist refuses, and steps linear in the body's size
#![no_main]
use libfuzzer_sys::fuzz_target;
use urb_render::{cost, gateway};
use urb_render_fuzz::{PAGE, PORT, gateway_href, hrefs, linear};

const TAGS: &[&str] = &[
    "p",
    "p class=\"link\"",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "ul",
    "ol",
    "li",
    "blockquote",
    "pre",
    "code",
    "strong",
    "em",
    "hr",
];

/// every tag in a page the gateway renders is its own
fn own_tags(html: &[u8]) {
    let h = String::from_utf8_lossy(html);
    let main = &h[h.find("<main>").expect("main") + 6..h.rfind("</main>").expect("/main")];
    for (i, _) in main.match_indices('<') {
        let t = &main[i + 1..];
        let name = t.split('>').next().unwrap_or("");
        let name = name.strip_prefix('/').unwrap_or(name);
        assert!(
            TAGS.contains(&name) || name == "a" || name.starts_with("a href=\""),
            "a tag <{name}>"
        );
    }
}

fuzz_target!(|data: &[u8]| {
    cost::reset();
    let link = gateway::page_link(PAGE.as_bytes(), PORT);
    for html in [
        gateway::render_gemtext(data, b"t", &link),
        gateway::render_markdown(data, b"t", &link),
        gateway::render_text(data, b"t"),
    ] {
        own_tags(&html);
        for h in hrefs(&html) {
            assert!(
                gateway_href(&h),
                "an href {:?}",
                String::from_utf8_lossy(&h)
            );
        }
    }
    for mark in ["gmi", "md", "html", "text", ""] {
        let (csp, _) = gateway::render(PAGE.as_bytes(), mark, data, PORT);
        let kind = gateway::render_as(mark, data);
        assert_eq!(csp == gateway::SANDBOX, kind == "html");
        assert!(csp == gateway::SANDBOX || csp == gateway::OWN);
    }
    let out = gateway::rewrite_html(data, PORT);
    assert!(out.len() >= data.len());
    for l in data.split(|&b| b == b'\n') {
        let (h, own) = link(l);
        assert!(
            h.is_empty() || gateway_href(&h),
            "pageLink {:?}",
            String::from_utf8_lossy(&h)
        );
        assert!(!own || h.starts_with(b"http://") && h.windows(14).any(|w| w == b".urb.localhost"));
    }
    linear(data.len(), "gateway");
});
