package main

import (
	"encoding/json"
	"errors"
	"fmt"
	"net/http/httptest"
	"os"
	"reflect"
	"strings"
	"testing"
	"time"
)

// gateway-go.json: the cases of urb_test.go and reader_test.go as
// vectors, so that the Rust gateway (core/urb-gateway, M8) runs them
// too. Each test's own inputs, and what the Go gives for them today:
// UPDATE_VECTORS=1 writes the file, and TestGatewayVectors holds the Go
// to it. Pages render as the tests render them, for urb://~zod/p on
// port 8475 with the title "t".

const gwPage, gwPort = "urb://~zod/p", "8475"

// the tests' inputs, case by case
func gatewayCases() []vcase {
	var cs []vcase
	add := func(fn string, in ...any) {
		var raw []json.RawMessage
		for _, a := range in {
			b, _ := json.Marshal(a)
			raw = append(raw, b)
		}
		cs = append(cs, vcase{Fn: fn, In: raw})
	}
	// TestDeUrb
	for _, u := range []string{"urb://~zod", "urb://~zod/", "urb://~zod/site/", "urb://~zod/notes/2026/intro",
		"urb://~zod/n/notes", "urb://~zod/forum", "urb://~zod/forum/b/1000", "urb://~zod/forum/b/1/x", "urb://~zod/n/forum/b",
		"https://example.com", "urb://notaship/x", "urb://~notaship/x", "urb://~zod/x/foo", "urb://~zod/p/counter",
		"urb://~zod/t/apps/x", "urb://~zod/k/user/prefs", "urb://~zod/Caps", "urb://~zod/a//b", "urb://~zod/forum/b",
		"urb://~zod/forum/b/12", "urb://~zod/forum/b/1.000"} {
		add("deUrb", u)
	}
	// TestAddresses
	add("gatewayURL", "urb://~sampel-palnet/n/notes", gwPort)
	add("gatewayURL", "urb://~notaship/x", gwPort)
	add("urbOfHost", "sampel-palnet.urb.localhost", "/site/")
	for _, h := range []string{"evil.example", "zod.urb.localhost.evil.example", "a.zod.urb.localhost"} {
		add("urbOfHost", h, "/")
	}
	// TestRender
	add("looksHtml", "  \n<!doctype html>")
	add("looksHtml", "# heading\n<b>")
	add("rewriteHTML", `<a href="urb://~zod/x">urb://~zod/x</a>`)
	add("renderGemtext", "# Hi <there>\n=> urb://~zod/a A link\n* one\n* two\n```\n<pre>\n```\n> quote")
	// TestRenderAs
	for _, c := range [][2]string{{"gmi", "# heading"}, {"gmi", "  <!doctype html><p>hi"}, {"html", "<p>hi"},
		{"html", "no tag first"}, {"md", "# heading"}, {"md", "<div>raw</div>"}, {"text", "<b>not html</b>"},
		{"js", "alert(1)"}, {"css", "body{}"}, {"noun", "x"}} {
		add("renderAs", c[0], c[1])
	}
	for _, m := range []string{"text", "html", "gmi", "md", "js", "css", "noun", "del", ""} {
		add("pageMark", m)
	}
	// TestRenderMarkdown
	add("renderMarkdown", "# Title <x>\n\nSome *em* and **strong** and `<code>`,\nwrapped.\n\n"+
		"- one\n- [two](urb://~zod/b)\n\n1. first\n2. second\n\n> quoted\n\n---\n\n"+
		"```\n<script>alert(1)</script>\n```\n\n<script>alert(2)</script>\n\n"+
		"[bad](javascript:alert(3)) [web](https://example.com/a?b=1&c=2) [rel](notes/intro)")
	add("renderMarkdown", "[bad](javascript:alert(3)) [web](https://example.com/a?b=1&c=2) [rel](notes/intro)")
	// TestRenderText
	add("renderText", "<b>a</b>\n  b")
	// TestGatewayRender
	for _, c := range [][2]string{{"html", `<a href="urb://~zod/x">x</a>`}, {"gmi", "<!doctype html><p>old"},
		{"gmi", "# old gemtext\n=> urb://~zod/y y"}, {"md", "# md\n[z](urb://~zod/z)\n<script>x</script>"},
		{"text", "# not a heading"}, {"css", "a{color:red}"}} {
		add("gatewayRender", c[0], c[1])
	}
	// TestSafeLinks
	for _, s := range []string{"https://a.example/x", "http://a", "urb://~zod/x", "gemini://g", "mailto:a@b", "notes/intro",
		"/notes", "#top", "", "javascript:alert(1)", "data:text/html,x", "//evil.example", `\\evil.example`, `/\evil.example`,
		`\/evil.example`, " //evil.example"} {
		add("safeLink", s)
	}
	add("renderGemtext", "=> //evil.example evil\n=> javascript:alert(1) js\n=> /ok fine")
	add("renderMarkdown", "[evil](//evil.example) [back](\\\\evil.example)")
	// TestOwnPageHeaders
	add("errorPage", "urb://~zod/x", 404, "<b>gone</b>")
	for _, c := range [][2]string{{"~zod", "moon.key"}, {"~zod", ""}, {"", "moon.key"}, {"", ""}} {
		add("oneIdentity", c[0], c[1])
	}
	// TestSafeLinkControls
	for _, s := range []string{"\x01//evil.com", "\x1f//evil.com", "\x00//evil.com", "\x7f//evil.com", " //evil.com", "/\t/evil.com",
		"java\tscript:alert(1)", "java\x00script:alert(1)", "\x01javascript:alert(1)", "https://a.b/\nx", "urb://~zod/x\x01"} {
		add("safeLink", s)
	}
	add("renderMarkdown", "[x](\x01//evil.com)")
	// TestSafeLinkSchemesAndHostile
	for _, s := range []string{"HTTPS://a.b", "Http://a.b", "HTTP://a.b", "URB://~zod/x", "Mailto:a@b.c", "GEMINI://a.b",
		"javascript&#58;alert(1)", "javascript:alert(1)", "JaVaScRiPt:alert(1)", "data:text/html,<script>", "DATA:text/html,x",
		"vbscript:x", "//evil.example", `\\evil.example`, `/\evil.example`, "file:///etc/passwd", "chrome-extension://x/y"} {
		add("safeLink", s)
	}
	add("renderMarkdown", "[s](HTTPS://a.b) [click](javascript:alert(1)) [data](data:text/html;base64,PHNjcmlwdD4=) [off](//evil.example/x)")
	// TestMarkdownLinkCaps
	add("renderMarkdown", "["+strings.Repeat("a", 256)+"](/"+strings.Repeat("b", 511)+")")
	add("renderMarkdown", "[a](/"+strings.Repeat("b", 512)+")")
	add("renderMarkdown", "["+strings.Repeat("a", 257)+"](/x)")
	// reader_test.go: TestKeenIsPage
	for _, k := range []keenResult{{mark: "gmi", body: "x", ok: true}, {mark: "md", body: "x", ok: true},
		{mark: "html", body: "x", ok: true}, {mark: "text", body: "x", ok: true}, {mark: "js", body: "x", ok: true},
		{mark: "css", body: "x", ok: true}, {mark: "noun", body: "x", ok: true}, {mark: "del", ok: true}, {mark: "md"}, {},
		{mark: "json", ok: true}} {
		add("isPage", k.mark, k.body, k.ok)
	}
	// reader_test.go: TestLateAnswer, a revision list that comes after
	// its caller gave up, and an answer that comes in time
	add("lateAnswer", map[string]string{"/a": "0v1"})
	add("onTimeAnswer", "body")
	return cs
}

// one gateway case run by the Go
func runGatewayVector(t *testing.T, c vcase) (any, error) {
	var a []any
	for _, r := range c.In {
		var v any
		if err := json.Unmarshal(r, &v); err != nil {
			t.Fatal(err)
		}
		a = append(a, v)
	}
	s := func(i int) string { x, _ := a[i].(string); return x }
	link := pageLink(gwPage, gwPort)
	status := func(err error) error {
		var h *httpError
		if errors.As(err, &h) {
			return fmt.Errorf("%d %s", h.status, h.msg)
		}
		return err
	}
	switch c.Fn {
	case "deUrb":
		ship, segs, err := deUrb(s(0))
		if err != nil {
			return nil, status(err)
		}
		return map[string]any{"ship": ship.String(), "rel": segs}, nil
	case "gatewayURL":
		u, err := gatewayURL(s(0), s(1))
		return u, status(err)
	case "urbOfHost":
		return urbOfHost(s(0), s(1)), nil
	case "looksHtml":
		return looksHTML(s(0)), nil
	case "rewriteHTML":
		return rewriteHTML(s(0), gwPort), nil
	case "renderGemtext":
		return renderGemtext(s(0), "t", link), nil
	case "renderMarkdown":
		return renderMarkdown(s(0), "t", link), nil
	case "renderText":
		return renderText(s(0), "t"), nil
	case "renderAs":
		return renderAs(s(0), s(1)), nil
	case "pageMark":
		return pageMarks[s(0)], nil
	case "gatewayRender":
		csp, out := (&gateway{port: gwPort}).render(gwPage, s(0), s(1))
		return map[string]any{"csp": csp, "body": out}, nil
	case "safeLink":
		return safeLink(s(0)), nil
	case "errorPage":
		w := httptest.NewRecorder()
		code, _ := a[1].(float64)
		(&gateway{}).error(w, s(0), fail(int(code), "%s", s(2)))
		h := w.Result().Header
		return map[string]any{"status": w.Code, "headers": map[string]string{
			"Content-Type":            h.Get("Content-Type"),
			"Content-Security-Policy": h.Get("Content-Security-Policy"),
			"X-Content-Type-Options":  h.Get("X-Content-Type-Options"),
		}, "body": w.Body.String()}, nil
	case "oneIdentity":
		if err := oneIdentity(s(0), s(1)); err != nil {
			return nil, err
		}
		return true, nil
	case "isPage":
		ok, _ := a[2].(bool)
		return keenResult{mark: s(0), body: s(1), ok: ok}.isPage(), nil
	case "lateAnswer":
		//  TestLateAnswer's first half: what a revision list's late answer
		//  leaves, and whether its peek still waits
		r := newReader()
		var late any
		r.peeks["/gone"] = &peek{done: make(chan any, 1), late: func(v any) { late = v }, gone: time.Now()}
		list := map[string]string{}
		for k, v := range a[0].(map[string]any) {
			list[k] = v.(string)
		}
		r.finish("/gone", list)
		_, waiting := r.peeks["/gone"]
		return map[string]any{"kept": late, "waiting": waiting}, nil
	case "onTimeAnswer":
		//  its second half: an answer in time goes to the caller only
		r := newReader()
		var late any
		done := make(chan any, 1)
		r.peeks["/here"] = &peek{done: done, late: func(v any) { late = v }}
		r.finish("/here", s(0))
		return map[string]any{"caller": <-done, "late": late}, nil
	}
	return nil, fmt.Errorf("no %s in the Go", c.Fn)
}

func TestGatewayVectors(t *testing.T) {
	if os.Getenv("UPDATE_VECTORS") != "" {
		cs := gatewayCases()
		for i := range cs {
			got, err := runGatewayVector(t, cs[i])
			if err != nil {
				e := err.Error()
				cs[i].Error = &e
				continue
			}
			cs[i].Out, _ = json.Marshal(got)
		}
		b, _ := json.MarshalIndent(vectors{About: "urb_test.go's and reader_test.go's cases, each test's own inputs, as the Go " +
			"gives them (gateway_vectors_test.go): pages for " + gwPage + " on port " + gwPort + " titled \"t\" (whole, as strings); " +
			"gatewayRender's {csp, body}; errorPage(url, status, msg)'s {status, headers, body}; isPage(mark, body, ok); " +
			"lateAnswer: a revision list's answer after its caller gave up, {kept, waiting}; onTimeAnswer: an answer in time, {caller, late}; " +
			"an error as \"<status> <message>\" where urb.go's fail() gives one", Cases: cs}, "", " ")
		if err := os.WriteFile("conformance/gateway-go.json", append(b, '\n'), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	v := readVectors(t, "gateway-go")
	for i, c := range v.Cases {
		got, err := runGatewayVector(t, c)
		ok := false
		switch {
		case c.Error != nil:
			ok = err != nil && err.Error() == *c.Error
		case err == nil:
			var want, have any
			_ = json.Unmarshal(c.Out, &want)
			g, _ := json.Marshal(got)
			_ = json.Unmarshal(g, &have)
			ok = reflect.DeepEqual(want, have)
		}
		if !ok {
			t.Errorf("gateway-go %d %s: want %s%s, got %v %v", i, c.Fn, c.Out, deref(c.Error), got, err)
		}
	}
	t.Logf("gateway-go: %d cases", len(v.Cases))
}
