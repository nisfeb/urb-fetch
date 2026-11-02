//! Each renderer's time is linear in its body's size, on bodies made to
//! make it slow: the lines in hostile.mjs that make a backtracking regex
//! slow, and the ones found since (docs/rust-core.md, "M5b's
//! findings"). Asserted twice: by the renderers' own count of their
//! steps (urb_render::cost), which is exact, at most a constant per
//! byte, and by the clock, whose time for 8 times the body must be well
//! under 64 times (quadratic) its time for the body.

use std::time::{Duration, Instant};
use urb_render::{cost, ext, gateway};

/// the bodies, each n units of its pattern
fn bodies(n: usize) -> Vec<(&'static str, String)> {
    let r = |s: &str| s.repeat(n / s.chars().count().max(1));
    vec![
        ("[", r("[")),
        ("](", r("](")),
        ("[a](", r("[a](")),
        ("*", r("*")),
        ("**", r("**")),
        ("`", r("`")),
        ("_", r("_")),
        ("#  x", format!("#{}x", r(" "))),
        ("  #", format!("{}#", r(" "))),
        ("# ## x", format!("# {} x", r("#"))),
        ("- ", r("- ")),
        ("> ", r("> ")),
        ("=>   x", format!("=> {}x", r(" "))),
        ("[aaa](", format!("[{}](", r("a"))),
        ("[a](bbb", format!("[a]({}", r("b"))),
        ("\\t<", format!("{}<", r("\t"))),
        // ITEM's backtracking in the JS: quadratic there (findings)
        ("-   a\\r", format!("- {}a\r", r(" "))),
        ("1.   a\\u2028", format!("1. {}a\u{2028}", r(" "))),
        ("- - - x", format!("-{} x", r(" -"))),
        ("`a`a", r("`a")),
        ("*a**", r("*a**")),
        ("**a*", r("**a*")),
        ("[😀", format!("[{}", r("😀"))),
        ("href=", r("href=")),
        ("href=  ", format!("href={}", r(" "))),
        ("src='urb://~", r("src='urb://~")),
        ("lines", r("a\n")),
        ("\\r\\n", r("\r\n")),
        ("``` ", r("```\n")),
        ("spaces", r("\u{3000}")),
        ("mixed", r("[*`# ](\u{a0}>-1. ")),
    ]
}

/// every renderer on one body
fn all(body: &str) {
    let b = body.as_bytes();
    let link = gateway::page_link(b"urb://~zod/a/b", "8475");
    ext::gemtext(body);
    ext::md_nodes(body, |t| ext::link_href("urb://~zod/a/b", t));
    urb_render::looks_html(body);
    for l in body.split('\n').take(4) {
        ext::spans(l);
        ext::link_href("urb://~zod/a/b", l);
    }
    gateway::render_gemtext(b, b"t", &link);
    gateway::render_markdown(b, b"t", &link);
    gateway::render_text(b, b"t");
    gateway::rewrite_html(b, "8475");
    gateway::render("urb://~zod/a/b".as_bytes(), "gmi", b, "8475");
}

fn steps(body: &str) -> u64 {
    cost::reset();
    all(body);
    cost::steps()
}

fn clock(body: &str) -> Duration {
    (0..3)
        .map(|_| {
            let t = Instant::now();
            all(body);
            t.elapsed()
        })
        .min()
        .unwrap_or_default()
}

#[test]
fn steps_are_linear_in_the_body() {
    const N: usize = 20_000;
    for ((name, small), (_, big)) in bodies(N).into_iter().zip(bodies(8 * N)) {
        let (s, b) = (steps(&small), steps(&big));
        //  a constant per byte, the same constant at both sizes
        assert!(
            b <= 64 * big.len() as u64 + 1000,
            "{name}: {b} steps for {} bytes",
            big.len()
        );
        assert!(
            b <= 9 * s + 1000,
            "{name}: {s} steps, then {b} for 8 times the body"
        );
    }
}

#[test]
fn time_is_linear_in_the_body() {
    //  big enough to time, small enough for a debug build under WASI
    let n = if cfg!(debug_assertions) {
        4_000
    } else {
        32_000
    };
    for ((name, small), (_, big)) in bodies(n).into_iter().zip(bodies(8 * n)) {
        let (s, b) = (clock(&small), clock(&big));
        //  8 times the body: 8 times the time, give or take; quadratic
        //  would be 64
        assert!(
            b <= s * 24 + Duration::from_millis(20),
            "{name}: {s:?}, then {b:?} for 8 times the body"
        );
    }
}
