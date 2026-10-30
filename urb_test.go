package main

import (
	"reflect"
	"strings"
	"testing"
)

// The cases of lattice's tests/lib/lattice-urls.hoon that this gateway
// serves, and the ones it refuses.
func TestDeUrb(t *testing.T) {
	ok := map[string][]string{
		"urb://~zod":                  {"index"},
		"urb://~zod/":                 {"index"},
		"urb://~zod/site/":            {"site"},
		"urb://~zod/notes/2026/intro": {"notes", "2026", "intro"},
		"urb://~zod/n/notes":          {"notes"},
	}
	for url, want := range ok {
		ship, rel, err := deUrb(url)
		if err != nil || ship.String() != "~zod" || !reflect.DeepEqual(rel, want) {
			t.Errorf("%s: %v %v %v", url, ship, rel, err)
		}
	}
	for _, url := range []string{"https://example.com", "urb://notaship/x", "urb://~notaship/x",
		"urb://~zod/x/foo", "urb://~zod/p/counter", "urb://~zod/t/apps/x", "urb://~zod/k/user/prefs",
		"urb://~zod/Caps", "urb://~zod/a//b"} {
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
		"t", func(s string) string { return "L(" + s + ")" })
	for _, want := range []string{"<h1>Hi &lt;there&gt;</h1>", `<a href="L(urb://~zod/a)">A link</a>`,
		"<ul>\n<li>one</li>\n<li>two</li>\n</ul>", "<pre>&lt;pre&gt;\n</pre>", "<blockquote>quote</blockquote>"} {
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
	g := renderMarkdown(src, "t", func(s string) string { return strings.Replace(s, "urb://~zod", "http://gw", 1) })
	for _, want := range []string{
		"<h1>Title &lt;x&gt;</h1>",
		"<p>Some <em>em</em> and <strong>strong</strong> and <code>&lt;code&gt;</code>, wrapped.</p>",
		"<ul>\n<li>one</li>\n<li><a href=\"http://gw/b\">two</a></li>\n</ul>",
		"<ol>\n<li>first</li>\n<li>second</li>\n</ol>",
		"<blockquote>quoted</blockquote>", "<hr>",
		"<pre>&lt;script&gt;alert(1)&lt;/script&gt;\n</pre>",
		"<p>&lt;script&gt;alert(2)&lt;/script&gt;</p>",
	} {
		if !strings.Contains(g, want) {
			t.Errorf("renderMarkdown lacks %q in %s", want, g)
		}
	}
	if strings.Contains(g, "<script") {
		t.Errorf("renderMarkdown let markup through: %s", g)
	}
	// with the gateway's own link rewriting: only safe targets are links
	g = renderMarkdown("[bad](javascript:alert(3)) [web](https://example.com/a?b=1&c=2) [rel](notes/intro)",
		"t", func(s string) string { return s })
	for _, want := range []string{"bad", `<a href="https://example.com/a?b=1&amp;c=2">web</a>`, `<a href="notes/intro">rel</a>`} {
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
	if !strings.Contains(g, "<pre>&lt;b&gt;a&lt;/b&gt;\n  b</pre>") {
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
		{"text", "# not a heading", "default-src 'none'", "<pre># not a heading</pre>"},
		{"css", "a{color:red}", "default-src 'none'", "<pre>a{color:red}</pre>"},
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
