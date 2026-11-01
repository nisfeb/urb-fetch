//! urb_test.go's and reader_test.go's cases as vectors
//! (conformance/gateway-go.json, which gateway_vectors_test.go writes
//! from the Go), and urb.go's deUrb on conformance/deurb-go.json's
//! urls, run on the Rust gateway: every case's output must be the Go's.
//! reader_test.go's lateAnswer and onTimeAnswer need the core's reader,
//! and run in core/urb/tests/gateway.rs; here they are only counted.
#![cfg(not(target_family = "wasm"))]

use serde_json::{Value as J, json};
use urb_gateway::routes::{error_page, gateway_url};
use urb_gateway::state::one_identity;
use urb_render::gateway as render;

const PAGE: &str = "urb://~zod/p";
const PORT: &str = "8475";

fn s(b: Vec<u8>) -> J {
    J::String(String::from_utf8(b).expect("UTF-8"))
}

fn de_urb(url: &str) -> Result<J, String> {
    match render::de_urb(url.as_bytes()) {
        Ok((ship, rel)) => {
            Ok(json!({"ship": s(ship), "rel": rel.into_iter().map(s).collect::<Vec<_>>()}))
        }
        Err((status, msg)) => Err(format!("{status} {}", String::from_utf8(msg).unwrap())),
    }
}

/// one case: its output, or its error's text
fn run(f: &str, a: &[J]) -> Result<J, String> {
    let t = |i: usize| a[i].as_str().expect("text").to_string();
    let link = render::page_link(PAGE.as_bytes(), PORT);
    Ok(match f {
        "deUrb" => return de_urb(&t(0)),
        "gatewayURL" => {
            return gateway_url(t(0).as_bytes(), &t(1))
                .map(s)
                .map_err(|(st, m)| format!("{st} {}", String::from_utf8(m).unwrap()));
        }
        "urbOfHost" => s(render::urb_of_host(t(0).as_bytes(), t(1).as_bytes()).unwrap_or_default()),
        "looksHtml" => J::Bool(render::looks_html(t(0).as_bytes())),
        "rewriteHTML" => s(render::rewrite_html(t(0).as_bytes(), PORT)),
        "renderGemtext" => s(render::render_gemtext(t(0).as_bytes(), b"t", &link)),
        "renderMarkdown" => s(render::render_markdown(t(0).as_bytes(), b"t", &link)),
        "renderText" => s(render::render_text(t(0).as_bytes(), b"t")),
        "renderAs" => J::String(render::render_as(&t(0), t(1).as_bytes()).into()),
        "pageMark" => J::Bool(urb::url::PAGE_MARKS.contains(&t(0).as_str())),
        "gatewayRender" => {
            let (csp, body) = render::render(PAGE.as_bytes(), &t(0), t(1).as_bytes(), PORT);
            json!({"csp": csp, "body": s(body)})
        }
        "safeLink" => J::Bool(render::safe_link(t(0).as_bytes())),
        "errorPage" => {
            let r = error_page(
                t(0).as_bytes(),
                a[1].as_u64().expect("a status") as u16,
                t(2).as_bytes(),
            );
            let h = |k: &str| s(r.header(k).unwrap_or_default().to_vec());
            json!({"status": r.status, "headers": {
                "Content-Type": h("Content-Type"),
                "Content-Security-Policy": h("Content-Security-Policy"),
                "X-Content-Type-Options": h("X-Content-Type-Options"),
            }, "body": s(r.body)})
        }
        "oneIdentity" => {
            one_identity(&t(0), &t(1))?;
            J::Bool(true)
        }
        "isPage" => J::Bool(urb::core::keen_is_page(&t(0), a[2].as_bool().expect("ok"))),
        _ => panic!("no {f} here"),
    })
}

#[test]
fn every_gateway_vector() {
    let v: J = serde_json::from_str(include_str!("../../../conformance/gateway-go.json")).unwrap();
    let (mut ran, mut reader) = (0, 0);
    for c in v["cases"].as_array().unwrap() {
        let f = c["fn"].as_str().unwrap();
        if f == "lateAnswer" || f == "onTimeAnswer" {
            reader += 1;
            continue;
        }
        let got = run(f, c["in"].as_array().unwrap());
        match (&got, c.get("error")) {
            (Err(e), Some(want)) => assert_eq!(e, want.as_str().unwrap(), "{c}"),
            (Ok(g), None) => assert_eq!(g, &c["out"], "{c}"),
            _ => panic!("{c}: got {got:?}"),
        }
        ran += 1;
    }
    // each file's count, so no case can be skipped
    assert_eq!((ran, reader), (126, 2));
}

#[test]
fn every_deurb_go_vector() {
    let v: J = serde_json::from_str(include_str!("../../../conformance/deurb-go.json")).unwrap();
    let cases = v["cases"].as_array().unwrap();
    for c in cases {
        let got = de_urb(c["in"][0].as_str().unwrap());
        match (&got, c.get("error")) {
            (Err(e), Some(want)) => assert_eq!(e, want.as_str().unwrap(), "{c}"),
            (Ok(g), None) => assert_eq!(g, &c["out"], "{c}"),
            _ => panic!("{c}: got {got:?}"),
        }
    }
    assert_eq!(
        cases.len(),
        include_str!("../../../conformance/deurb-go.json")
            .matches("\"fn\"")
            .count()
    );
}
