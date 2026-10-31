package main

import (
	"net/http/httptest"
	"reflect"
	"strings"
	"testing"
)

// zodLink is the gateway's linker for a page at urb://~zod/p.
var zodLink = pageLink("urb://~zod/p", "8475")

// The cases of lattice's tests/lib/lattice-urls.hoon that this gateway
// serves, and the ones it refuses.
func TestDeUrb(t *testing.T) {
	ok := map[string][]string{
		"urb://~zod":                  {"index"},
		"urb://~zod/":                 {"index"},
		"urb://~zod/site/":            {"site"},
		"urb://~zod/notes/2026/intro": {"notes", "2026", "intro"},
		"urb://~zod/n/notes":          {"notes"},
		"urb://~zod/forum":            {"forum"},
		"urb://~zod/forum/b/1000":     {"forum", "b", "1000"},
		"urb://~zod/forum/b/1/x":      {"forum", "b", "1", "x"},
		"urb://~zod/n/forum/b":        {"forum", "b"},
	}
	for url, want := range ok {
		ship, rel, err := deUrb(url)
		if err != nil || ship.String() != "~zod" || !reflect.DeepEqual(rel, want) {
			t.Errorf("%s: %v %v %v", url, ship, rel, err)
		}
	}
	for _, url := range []string{"https://example.com", "urb://notaship/x", "urb://~notaship/x",
		"urb://~zod/x/foo", "urb://~zod/p/counter", "urb://~zod/t/apps/x", "urb://~zod/k/user/prefs",
		"urb://~zod/Caps", "urb://~zod/a//b", "urb://~zod/forum/b", "urb://~zod/forum/b/12", "urb://~zod/forum/b/1.000"} {
		if _, _, err := deUrb(url); err == nil {
			t.Errorf("accepted %s", url)
		}
	}
}

func TestAddresses(t *testing.T) {
	u, err := gatewayURL("urb://~sampel-palnet/n/notes", "8475")
	if err != nil || u != "http://sampel-palnet.urb.localhost:8475/n/notes" {
		t.Errorf("gatewayURL: %s %v", u, err)
	}
	if _, err := gatewayURL("urb://~notaship/x", "8475"); err == nil {
		t.Error("gatewayURL accepted a bad ship")
	}
	if got := urbOfHost("sampel-palnet.urb.localhost", "/site/"); got != "urb://~sampel-palnet/site/" {
		t.Errorf("urbOfHost: %s", got)
	}
	//  the rebinding guard: only *.urb.localhost names a ship
	for _, h := range []string{"evil.example", "zod.urb.localhost.evil.example", "a.zod.urb.localhost"} {
		if got := urbOfHost(h, "/"); got != "" {
			t.Errorf("urbOfHost(%s) = %s", h, got)
		}
	}
}

func TestRender(t *testing.T) {
	if !looksHTML("  \n<!doctype html>") || looksHTML("# heading\n<b>") {
		t.Error("looksHTML")
	}
	got := rewriteHTML(`<a href="urb://~zod/x">urb://~zod/x</a>`, "8475")
	if got != `<a href="http://zod.urb.localhost:8475/x">urb://~zod/x</a>` {
		t.Errorf("rewriteHTML: %s", got)
	}
	g := renderGemtext("# Hi <there>\n=> urb://~zod/a A link\n* one\n* two\n```\n<pre>\n```\n> quote",
		"t", zodLink)
	for _, want := range []string{"<h1>Hi &lt;there&gt;</h1>", `<a href="http://zod.urb.localhost:8475/a">A link</a>`,
		"<ul>\n<li>one</li>\n<li>two</li>\n</ul>", "<pre>\n&lt;pre&gt;</pre>", "<blockquote>quote</blockquote>"} {
		if !strings.Contains(g, want) {
			t.Errorf("renderGemtext lacks %q in %s", want, g)
		}
	}
}

// A page renders by the mark it was published under, as lattice's reader
// does. %gmi, all an older lattice publishes, keeps the body sniff.
func TestRenderAs(t *testing.T) {
	for _, c := range []struct{ mark, body, want string }{
		{"gmi", "# heading", "gmi"},
		{"gmi", "  <!doctype html><p>hi", "html"}, // a pre-74 html page
		{"html", "<p>hi", "html"},
		{"html", "no tag first", "html"},
		{"md", "# heading", "md"},
		{"md", "<div>raw</div>", "md"}, // never sniffed: escaped by the renderer
		{"text", "<b>not html</b>", "text"},
		{"js", "alert(1)", "text"},
		{"css", "body{}", "text"},
		{"noun", "x", "text"},
	} {
		if got := renderAs(c.mark, c.body); got != c.want {
			t.Errorf("renderAs(%s, %q) = %s, want %s", c.mark, c.body, got, c.want)
		}
	}
	for _, m := range []string{"text", "html", "gmi", "md", "js", "css", "noun"} {
		if !pageMarks[m] {
			t.Errorf("%%%s is not a page mark", m)
		}
	}
	if pageMarks["del"] || pageMarks[""] {
		t.Error("a tombstone or no mark counts as a page")
	}
}

func TestRenderMarkdown(t *testing.T) {
	src := "# Title <x>\n\nSome *em* and **strong** and `<code>`,\nwrapped.\n\n" +
		"- one\n- [two](urb://~zod/b)\n\n1. first\n2. second\n\n> quoted\n\n---\n\n" +
		"```\n<script>alert(1)</script>\n```\n\n<script>alert(2)</script>\n\n" +
		"[bad](javascript:alert(3)) [web](https://example.com/a?b=1&c=2) [rel](notes/intro)"
	g := renderMarkdown(src, "t", zodLink)
	for _, want := range []string{
		"<h1>Title &lt;x&gt;</h1>",
		"<p>Some <em>em</em> and <strong>strong</strong> and <code>&lt;code&gt;</code>, wrapped.</p>",
		"<ul>\n<li>one</li>\n<li><a href=\"http://zod.urb.localhost:8475/b\">two</a></li>\n</ul>",
		"<ol>\n<li>first</li>\n<li>second</li>\n</ol>",
		"<blockquote>quoted</blockquote>", "<hr>",
		"<pre>\n&lt;script&gt;alert(1)&lt;/script&gt;</pre>",
		"<p>&lt;script&gt;alert(2)&lt;/script&gt;</p>",
	} {
		if !strings.Contains(g, want) {
			t.Errorf("renderMarkdown lacks %q in %s", want, g)
		}
	}
	if strings.Contains(g, "<script") {
		t.Errorf("renderMarkdown let markup through: %s", g)
	}
	// only safe targets are links, and a relative one is on the page's ship
	g = renderMarkdown("[bad](javascript:alert(3)) [web](https://example.com/a?b=1&c=2) [rel](notes/intro)",
		"t", zodLink)
	for _, want := range []string{"bad", `<a href="https://example.com/a?b=1&amp;c=2" rel="noopener noreferrer">web</a>`,
		`<a href="http://zod.urb.localhost:8475/notes/intro">rel</a>`} {
		if !strings.Contains(g, want) {
			t.Errorf("renderMarkdown lacks %q in %s", want, g)
		}
	}
	if strings.Contains(g, `href="javascript`) {
		t.Errorf("renderMarkdown linked javascript: %s", g)
	}
}

func TestRenderText(t *testing.T) {
	g := renderText("<b>a</b>\n  b", "t")
	if !strings.Contains(g, "<pre>\n&lt;b&gt;a&lt;/b&gt;\n  b</pre>") {
		t.Errorf("renderText: %s", g)
	}
}

// The gateway serves a publisher's HTML only in the sandbox, whatever
// mark said it was HTML, and renders everything else itself.
func TestGatewayRender(t *testing.T) {
	g := &gateway{port: "8475"}
	for _, c := range []struct{ mark, body, csp, want string }{
		{"html", `<a href="urb://~zod/x">x</a>`, sandbox, `href="http://zod.urb.localhost:8475/x"`},
		{"gmi", "<!doctype html><p>old", sandbox, "<p>old"},
		{"gmi", "# old gemtext\n=> urb://~zod/y y", "default-src 'none'", `<a href="http://zod.urb.localhost:8475/y">y</a>`},
		{"md", "# md\n[z](urb://~zod/z)\n<script>x</script>", "default-src 'none'", `<a href="http://zod.urb.localhost:8475/z">z</a>`},
		{"text", "# not a heading", "default-src 'none'", "<pre>\n# not a heading</pre>"},
		{"css", "a{color:red}", "default-src 'none'", "<pre>\na{color:red}</pre>"},
	} {
		csp, out := g.render("urb://~zod/p", c.mark, c.body)
		if !strings.HasPrefix(csp, c.csp) || !strings.Contains(out, c.want) {
			t.Errorf("%%%s: csp %q, body %s", c.mark, csp, out)
		}
		if c.csp != sandbox && strings.Contains(out, "<script>") {
			t.Errorf("%%%s: markup outside the sandbox: %s", c.mark, out)
		}
	}
}

// A page's links stay on the web, urb:// or this ship: no script
// targets, and no protocol-relative // or \\ that leaves the origin.
func TestSafeLinks(t *testing.T) {
	for _, s := range []string{"https://a.example/x", "http://a", "urb://~zod/x", "gemini://g", "mailto:a@b", "notes/intro", "/notes", "#top", ""} {
		if !safeLink(s) {
			t.Errorf("refused %q", s)
		}
	}
	for _, s := range []string{"javascript:alert(1)", "data:text/html,x", "//evil.example", `\\evil.example`, `/\evil.example`, `\/evil.example`, " //evil.example"} {
		if safeLink(s) {
			t.Errorf("allowed %q", s)
		}
	}
	g := renderGemtext("=> //evil.example evil\n=> javascript:alert(1) js\n=> /ok fine", "t", zodLink)
	if strings.Contains(g, "evil.example\"") || strings.Contains(g, `href="javascript`) ||
		!strings.Contains(g, `<a href="http://zod.urb.localhost:8475/ok">fine</a>`) {
		t.Errorf("renderGemtext links: %s", g)
	}
	m := renderMarkdown("[evil](//evil.example) [back](\\\\evil.example)", "t", zodLink)
	if strings.Contains(m, "<a ") {
		t.Errorf("renderMarkdown links: %s", m)
	}
}

// The gateway's own pages run no scripts and aren't sniffed.
func TestOwnPageHeaders(t *testing.T) {
	w := httptest.NewRecorder()
	(&gateway{}).error(w, "urb://~zod/x", fail(404, "<b>gone</b>"))
	h := w.Result().Header
	if h.Get("Content-Security-Policy") != own || h.Get("X-Content-Type-Options") != "nosniff" || w.Code != 404 {
		t.Errorf("error page: %d %v", w.Code, h)
	}
	if strings.Contains(w.Body.String(), "<b>gone") {
		t.Error("error page did not escape")
	}
	if oneIdentity("~zod", "moon.key") == nil || oneIdentity("~zod", "") != nil || oneIdentity("", "moon.key") != nil {
		t.Error("oneIdentity")
	}
}

// Control characters anywhere in a target make it text: a browser strips
// them, so \x01//evil.com would leave the ship.
func TestSafeLinkControls(t *testing.T) {
	for _, s := range []string{"\x01//evil.com", "\x1f//evil.com", "\x00//evil.com", "\x7f//evil.com", " //evil.com", "/\t/evil.com",
		"java\tscript:alert(1)", "java\x00script:alert(1)", "\x01javascript:alert(1)", "https://a.b/\nx", "urb://~zod/x\x01"} {
		if safeLink(s) {
			t.Errorf("allowed %q", s)
		}
	}
	if m := renderMarkdown("[x](\x01//evil.com)", "t", zodLink); strings.Contains(m, "<a ") {
		t.Errorf("control link kept: %s", m)
	}
}

// Schemes match in any case; the rest of the hostile class stays text.
func TestSafeLinkSchemesAndHostile(t *testing.T) {
	for _, s := range []string{"HTTPS://a.b", "Http://a.b", "HTTP://a.b", "URB://~zod/x", "Mailto:a@b.c", "GEMINI://a.b", "javascript&#58;alert(1)"} {
		if !safeLink(s) {
			t.Errorf("refused %q", s)
		}
	}
	for _, s := range []string{"javascript:alert(1)", "JaVaScRiPt:alert(1)", "data:text/html,<script>", "DATA:text/html,x", "vbscript:x",
		"//evil.example", `\\evil.example`, `/\evil.example`, "file:///etc/passwd", "chrome-extension://x/y"} {
		if safeLink(s) {
			t.Errorf("allowed %q", s)
		}
	}
	m := renderMarkdown("[s](HTTPS://a.b) [click](javascript:alert(1)) [data](data:text/html;base64,PHNjcmlwdD4=) [off](//evil.example/x)", "t", zodLink)
	if !strings.Contains(m, `<a href="https://a.b" rel="noopener noreferrer">s</a>`) || strings.Count(m, "<a ") != 1 {
		t.Errorf("renderMarkdown: %s", m)
	}
}

// A link's text and target are capped, so they stay text past the cap.
func TestMarkdownLinkCaps(t *testing.T) {
	id := zodLink
	ok := renderMarkdown("["+strings.Repeat("a", 256)+"](/"+strings.Repeat("b", 511)+")", "t", id)
	if !strings.Contains(ok, "<a ") {
		t.Error("link at the caps dropped")
	}
	for _, body := range []string{"[a](/" + strings.Repeat("b", 512) + ")", "[" + strings.Repeat("a", 257) + "](/x)"} {
		if m := renderMarkdown(body, "t", id); strings.Contains(m, "<a ") {
			t.Errorf("over-cap link kept: %.80s", m)
		}
	}
}
