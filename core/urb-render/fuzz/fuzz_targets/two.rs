//! a body read by both renderers: the same blocks, spans, sniff,
//! safeLink and link, as tests/two.rs asks, the one known difference
//! (an urb:// name that is not a ship) aside
#![no_main]
use libfuzzer_sys::fuzz_target;
use urb_render::{ext, gateway, js, ship};
use urb_render_fuzz::{PAGE, PORT};

fn gateway_of(addr: &str) -> String {
    match addr.strip_prefix("urb://~") {
        Some(rest) => {
            let (who, path) = rest.split_once('/').unwrap_or((rest, ""));
            format!("http://{who}.urb.localhost:{PORT}/{path}")
        }
        None => addr.to_string(),
    }
}

fn md_href(t: &str) -> String {
    if !ext::safe_link(t) {
        return String::new();
    }
    let lower = match t.find(':') {
        Some(n)
            if t.as_bytes()[0].is_ascii_alphabetic()
                && t[..n]
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"+.-".contains(&b)) =>
        {
            t[..n].to_ascii_lowercase() + &t[n..]
        }
        _ => t.to_string(),
    };
    if lower.starts_with("mailto:") || lower.starts_with("gemini:") {
        return lower;
    }
    ext::link_target(PAGE, &lower)
}

fn not_a_ship(link: &str) -> bool {
    let t = link.trim_matches(js::is_space);
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

fuzz_target!(|data: &[u8]| {
    let Ok(body) = std::str::from_utf8(data) else {
        return;
    };
    assert_eq!(ext::gemtext(body), gateway::parsed::gemtext(data));
    assert_eq!(ext::markdown(body), gateway::parsed::markdown(data));
    assert_eq!(urb_render::looks_html(body), gateway::looks_html(data));
    let page = gateway::page_link(PAGE.as_bytes(), PORT);
    for l in body.split('\n') {
        assert_eq!(ext::spans(l), gateway::parsed::spans(l.as_bytes()));
        assert_eq!(
            ext::safe_link(l),
            gateway::safe_link(l.as_bytes()),
            "safeLink {l:?}"
        );
        let (e, g) = (
            gateway_of(&md_href(l)),
            String::from_utf8_lossy(&page(l.as_bytes()).0).into_owned(),
        );
        assert!(
            e == g || not_a_ship(l) && g.is_empty(),
            "link {l:?}: ext {e:?}, gateway {g:?}"
        );
    }
});
