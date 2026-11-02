//! The rendering vectors (conformance/render-*.json and
//! hostile-render.json, made by the JS, the Go and by hand), run from
//! Rust: every case's output must be the vector's. render-ext.json is
//! the extension's ([`urb_render::ext`]), render-go.json the gateway's
//! ([`urb_render::gateway`]), each with its -random.json on the bodies
//! and links of conformance/random.mjs, and render-as.json and hostile-render.json
//! both's, so each of those cases runs on each, and a hostile case must
//! finish within the file's `quick` ms. render-go-bytes.json is the
//! gateway's on bytes that are mostly not UTF-8, every argument and
//! output hex. Each file's count is checked, so no case can be skipped.

mod common;

use common::{block_json, gem_json, node_json, span_json};
use serde_json::{Value as J, json};
use std::time::Instant;
use urb_render::{ext, gateway};

const PAGE: &str = "urb://~zod/a/b";
const PORT: &str = "8475";

/// each file, and its number of cases
const FILES: &[(&str, &str, usize)] = &[
    (
        "render-as",
        include_str!("../../../conformance/render-as.json"),
        192,
    ),
    (
        "render-ext",
        include_str!("../../../conformance/render-ext.json"),
        334,
    ),
    (
        "render-go",
        include_str!("../../../conformance/render-go.json"),
        347,
    ),
    (
        "hostile-render",
        include_str!("../../../conformance/hostile-render.json"),
        224,
    ),
    (
        "render-ext-random",
        include_str!("../../../conformance/render-ext-random.json"),
        3000,
    ),
    (
        "render-go-random",
        include_str!("../../../conformance/render-go-random.json"),
        3800,
    ),
];

/// a vector's argument: a string, `{"$repeat": [s, n]}`, `{"$cat":
/// [...]}`, or none (`null` or `{"$u": 1}`)
fn arg(j: &J) -> Option<String> {
    match j {
        J::String(s) => Some(s.clone()),
        J::Null => None,
        J::Object(o) if o.contains_key("$u") => None,
        J::Object(o) if o.contains_key("$repeat") => {
            let r = &o["$repeat"];
            Some(text(&r[0]).repeat(r[1].as_u64().expect("$repeat's n") as usize))
        }
        J::Object(o) if o.contains_key("$cat") => Some(
            o["$cat"]
                .as_array()
                .expect("$cat")
                .iter()
                .map(text)
                .collect(),
        ),
        _ => panic!("not an argument: {j}"),
    }
}

fn text(j: &J) -> String {
    arg(j).expect("text")
}

/// a rendered page as `{title, main}`, as conformance_test.go's
/// pageParts cuts it: page() wraps every one alike
fn parts(html: &[u8]) -> J {
    let h = String::from_utf8(html.to_vec()).expect("UTF-8");
    let title = h
        .find("<title>")
        .and_then(|a| h[a + 7..].find("</title>").map(|b| &h[a + 7..a + 7 + b]));
    let main = h.find("<main>").and_then(|a| {
        h.rfind("</main>")
            .filter(|&b| b >= a + 6)
            .map(|b| &h[a + 6..b])
    });
    match (title, main) {
        (Some(t), Some(m)) => json!({"title": t, "main": m}),
        _ => json!({"page": h}),
    }
}

fn s(b: Vec<u8>) -> J {
    J::String(String::from_utf8(b).expect("UTF-8"))
}

/// the extension's mdNodes, links by linkHref on PAGE
fn nodes(body: &str) -> Vec<ext::Node> {
    ext::md_nodes(body, |t| ext::link_href(PAGE, t))
}

const SAFE_TAGS: &[&str] = &[
    "p",
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
    "a",
    "hr",
];

fn tags(n: &ext::Node, out: &mut Vec<&'static str>) {
    if let ext::Node::El { tag, kids, .. } = n {
        out.push(tag);
        for k in kids {
            tags(k, out);
        }
    }
}

/// conformance_test.go's rawTag, `(?i)<(script|img|iframe|b|style|object|embed|svg)[\s>/]`
fn raw_tag(h: &str) -> bool {
    let l = h.to_ascii_lowercase();
    [
        "script", "img", "iframe", "b", "style", "object", "embed", "svg",
    ]
    .iter()
    .any(|t| {
        l.match_indices(&format!("<{t}"))
            .any(|(i, m)| l[i + m.len()..].starts_with([' ', '\t', '\n', '\x0c', '\r', '>', '/']))
    })
}

fn main_of(html: &[u8]) -> String {
    parts(html)["main"].as_str().expect("main").to_string()
}

/// the gateway's page for (mark, body), as conformance_test.go renders it
fn go_page(mark: &str, body: &str) -> Vec<u8> {
    let link = gateway::page_link(PAGE.as_bytes(), PORT);
    match mark {
        "md" => gateway::render_markdown(body.as_bytes(), b"t", &link),
        "gmi" => gateway::render_gemtext(body.as_bytes(), b"t", &link),
        _ => gateway::render_text(body.as_bytes(), b"t"),
    }
}

/// one case's outputs: the extension's and the gateway's, for a
/// function both have, or the one's that has it
fn run(f: &str, a: &[Option<String>]) -> Vec<J> {
    let t = |i: usize| a[i].clone().unwrap_or_default();
    let link = gateway::page_link(PAGE.as_bytes(), PORT);
    match f {
        "renderAs" => vec![
            json!(urb_render::render_as(a[0].as_deref(), &t(1))),
            json!(gateway::render_as(
                a[0].as_deref().unwrap_or(""),
                t(1).as_bytes()
            )),
        ],
        "looksHtml" => vec![
            json!(urb_render::looks_html(&t(0))),
            json!(gateway::looks_html(t(0).as_bytes())),
        ],
        // the extension's
        "gemtext" => vec![J::Array(ext::gemtext(&t(0)).iter().map(gem_json).collect())],
        "markdown" => vec![J::Array(
            ext::markdown(&t(0)).iter().map(block_json).collect(),
        )],
        "mdNodes" => vec![J::Array(nodes(&t(0)).iter().map(node_json).collect())],
        "spans" => vec![J::Array(ext::spans(&t(0)).iter().map(span_json).collect())],
        "linkTarget" => vec![json!(ext::link_target(&t(0), &t(1)))],
        "linkHref" => vec![json!(ext::link_href(&t(0), &t(1)))],
        "readerPath" => vec![json!(ext::reader_path(&t(0)))],
        // the gateway's
        "renderGemtext" => vec![parts(&go_page("gmi", &t(0)))],
        "renderMarkdown" => vec![parts(&go_page("md", &t(0)))],
        "renderText" => vec![parts(&go_page("text", &t(0)))],
        "rewriteHTML" => vec![s(gateway::rewrite_html(t(0).as_bytes(), PORT))],
        "page" => vec![s(gateway::page(t(0).as_bytes(), t(1).as_bytes()))],
        "pageLink" => {
            let (href, own) = link(t(0).as_bytes());
            vec![json!({"href": String::from_utf8(href).expect("UTF-8"), "own": own})]
        }
        // both's
        "safeLink" => vec![
            json!(ext::safe_link(&t(0))),
            json!(gateway::safe_link(t(0).as_bytes())),
        ],
        "mdLink" => {
            let line = format!("[{}]({})", t(0), t(1));
            let link = |ss: Vec<ext::Span>| ss.iter().any(|s| matches!(s, ext::Span::Link { .. }));
            vec![
                json!(link(ext::spans(&line))),
                json!(link(gateway::parsed::spans(line.as_bytes()))),
            ]
        }
        "mdFirst" => {
            let first = match &nodes(&t(0))[0] {
                ext::Node::El { tag, .. } => *tag,
                ext::Node::Text(_) => "#text",
            };
            let main = main_of(&go_page("md", &t(0)));
            let tag: String = main[1..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect();
            vec![json!(first), json!(tag)]
        }
        "escapes" => {
            let ext_ok = t(0) != "md" || {
                let mut ts = Vec::new();
                for n in nodes(&t(1)) {
                    tags(&n, &mut ts);
                }
                ts.iter().all(|t| SAFE_TAGS.contains(t))
            };
            vec![
                json!(ext_ok),
                json!(!raw_tag(&main_of(&go_page(&t(0), &t(1))))),
            ]
        }
        "renders" => {
            if t(0) == "md" {
                nodes(&t(1));
            } else {
                ext::gemtext(&t(1));
            }
            go_page(&t(0), &t(1));
            vec![json!(true), json!(true)]
        }
        _ => panic!("no {f} here"),
    }
}

#[test]
fn every_rendering_vector() {
    let mut bad = 0;
    for (name, src, count) in FILES {
        let v: J = serde_json::from_str(src).expect(name);
        let cases = v["cases"].as_array().expect("cases");
        assert_eq!(cases.len(), *count, "{name}'s cases");
        let quick = v["quick"].as_u64();
        let mut fails = 0;
        for (i, c) in cases.iter().enumerate() {
            let f = c["fn"].as_str().expect("fn");
            let a: Vec<Option<String>> = c["in"].as_array().expect("in").iter().map(arg).collect();
            let t = Instant::now();
            let got = run(f, &a);
            let ms = t.elapsed().as_millis() as u64;
            let slow = quick.is_some_and(|q| ms > q);
            if slow || got.iter().any(|g| *g != c["out"]) {
                fails += 1;
                if fails <= 5 {
                    let shown: String = c["in"].to_string().chars().take(120).collect();
                    let want: String = c["out"].to_string().chars().take(300).collect();
                    let have: Vec<String> = got
                        .iter()
                        .map(|g| g.to_string().chars().take(300).collect())
                        .collect();
                    eprintln!(
                        "  {name} {i} {f}({shown}) in {ms} ms\n    want {want}\n    got  {have:?}"
                    );
                }
            }
        }
        eprintln!(
            "{}  {name}: {}/{}",
            if fails > 0 { "FAIL" } else { "ok  " },
            cases.len() - fails,
            cases.len()
        );
        bad += fails;
    }
    assert_eq!(bad, 0, "rendering vectors fail");
}

/// render-go-bytes.json and its number of cases
const BYTES: (&str, usize) = (
    include_str!("../../../conformance/render-go-bytes.json"),
    2520,
);

fn unhex(j: &J) -> Vec<u8> {
    let h = j.as_str().expect("hex").as_bytes();
    assert!(h.len().is_multiple_of(2), "odd hex");
    let d = |c: u8| (c as char).to_digit(16).expect("hex digit") as u8;
    h.chunks(2).map(|p| d(p[0]) << 4 | d(p[1])).collect()
}

fn hex(b: &[u8]) -> J {
    J::String(b.iter().map(|x| format!("{x:02x}")).collect())
}

/// one render-go-bytes case: the gateway's output, strings as hex
fn run_bytes(f: &str, a: &[Vec<u8>]) -> J {
    let link = gateway::page_link(PAGE.as_bytes(), PORT);
    match f {
        "renderGemtext" => hex(&gateway::render_gemtext(&a[0], b"t", &link)),
        "renderMarkdown" => hex(&gateway::render_markdown(&a[0], b"t", &link)),
        "renderText" => hex(&gateway::render_text(&a[0], b"t")),
        "rewriteHTML" => hex(&gateway::rewrite_html(&a[0], PORT)),
        "looksHtml" => json!(gateway::looks_html(&a[0])),
        "renderAs" => json!(gateway::render_as(
            std::str::from_utf8(&a[0]).expect("a mark"),
            &a[1]
        )),
        "safeLink" => json!(gateway::safe_link(&a[0])),
        "pageLink" => {
            let (href, own) = link(&a[0]);
            json!({"href": hex(&href), "own": own})
        }
        _ => panic!("no {f} in render-go-bytes"),
    }
}

#[test]
fn every_bytes_vector() {
    let (src, count) = BYTES;
    let v: J = serde_json::from_str(src).expect("render-go-bytes");
    let cases = v["cases"].as_array().expect("cases");
    assert_eq!(cases.len(), count, "render-go-bytes' cases");
    let mut fails = 0;
    for (i, c) in cases.iter().enumerate() {
        let f = c["fn"].as_str().expect("fn");
        let a: Vec<Vec<u8>> = c["in"].as_array().expect("in").iter().map(unhex).collect();
        let got = run_bytes(f, &a);
        if got != c["out"] {
            fails += 1;
            if fails <= 5 {
                let want: String = c["out"].to_string().chars().take(300).collect();
                let have: String = got.to_string().chars().take(300).collect();
                eprintln!("  render-go-bytes {i} {f}\n    want {want}\n    got  {have}");
            }
        }
    }
    eprintln!(
        "{}  render-go-bytes: {}/{}",
        if fails > 0 { "FAIL" } else { "ok  " },
        cases.len() - fails,
        cases.len()
    );
    assert_eq!(fails, 0, "render-go-bytes vectors fail");
}
