package main

import (
	"strings"
	"testing"
	"time"
)

// The gateway renders by the extension's rules (nisfeb-extension 800ff22,
// lib/gemtext.js, lib/markdown.js and reader.js). Each case here is one
// place the two differed.

func TestLooksHTMLAsTheExtension(t *testing.T) {
	for body, want := range map[string]bool{
		"\u00a0<p>":                         true, // JavaScript's whitespace, not only " \t\r\n"
		"\f\v\ufeff<p>":                     true,
		"\u0085<p>":                         false, // NEL is not JavaScript whitespace
		strings.Repeat(" ", 255) + "<":      true,
		strings.Repeat(" ", 256) + "<":      false,
		strings.Repeat("\u3000", 255) + "<": true, // 256 UTF-16 units, 766 bytes
	} {
		if got := looksHTML(body); got != want {
			t.Errorf("looksHTML(%.20q) = %v", body, got)
		}
	}
	if renderAs("", "# x") != "gmi" || renderAs("", "<p>") != "html" {
		t.Error("no mark should read as gmi, as in mesa.mjs")
	}
}

// A pre's first line stays, even blank: the parser drops one newline
// after <pre>, and the extension sets textContent.
func TestPreKeepsItsFirstLine(t *testing.T) {
	if g := renderGemtext("```\n\nx\n```", "t", zodLink); !strings.Contains(g, "<pre>\n\nx</pre>") {
		t.Errorf("gemtext: %s", g)
	}
	if m := renderMarkdown("```\n\nx\n```", "t", zodLink); !strings.Contains(m, "<pre>\n\nx</pre>") {
		t.Errorf("markdown: %s", m)
	}
	if s := renderText("\nx", "t"); !strings.Contains(s, "<pre>\n\nx</pre>") {
		t.Errorf("text: %s", s)
	}
}

func TestLinksAsTheExtension(t *testing.T) {
	gw := "http://zod.urb.localhost:8475"
	for target, want := range map[string]string{
		"urb://~nec/x":      "http://nec.urb.localhost:8475/x",
		"URB://~nec/x":      "http://nec.urb.localhost:8475/x", // the scheme lower-cased
		"Mailto:a@b.c":      "mailto:a@b.c",
		"gemini://g.x/":     "gemini://g.x/",
		"HTTPS://a.b/c?d#e": "https://a.b/c?d#e",
		"x":                 gw + "/x", // relative to the page, urb://~zod/p
		"../../c?q=1#f":     gw + "/c", // its path only
		"#top":              gw + "/p",
		"/":                 gw + "/",
		"a\\b":              gw + "/a/b", // \ is / in a path
		"%2e%2e/%41é{":      gw + "/%41%C3%A9%7B",
		"http:/x":           "", // the web needs a host
		"urb://~notaship/x": "",
		"urb:x":             "",
		"\\\\evil.example":  "",
		"\x01x:-nec/page":   "", // the extension's gemtext reads this as urb://~zod-nec/page
		"javascript:x":      "",
	} {
		if got, _ := zodLink(target); got != want {
			t.Errorf("link(%q) = %q, want %q", target, got, want)
		}
	}
	g := renderGemtext("=> javascript:x run it\n=> javascript:y\n=> https://a.b web\n=> /n here", "t", zodLink)
	for _, want := range []string{
		`<p class="link">run it (javascript:x)</p>`, `<p class="link">javascript:y</p>`,
		`<a href="https://a.b" rel="noopener noreferrer">web</a>`, `<a href="` + gw + `/n">here</a>`,
	} {
		if !strings.Contains(g, want) {
			t.Errorf("gemtext lacks %s in %s", want, g)
		}
	}
	m := renderMarkdown("[mail](mailto:a@b.c) [ship](urb://~nec) [up](../x)", "t", zodLink)
	for _, want := range []string{`<a href="mailto:a@b.c" rel="noopener noreferrer">mail</a>`,
		`<a href="http://nec.urb.localhost:8475/">ship</a>`, `<a href="` + gw + `/x">up</a>`} {
		if !strings.Contains(m, want) {
			t.Errorf("markdown lacks %s in %s", want, m)
		}
	}
}

// The page's title is its first h1's text, as the reader sets it.
func TestTitles(t *testing.T) {
	for _, c := range []struct{ got, want string }{
		{renderGemtext("## two\n# One <b>\n# Again", "urb://~zod/p", zodLink), "<title>One &lt;b&gt;</title>"},
		{renderGemtext("no heading", "urb://~zod/p", zodLink), "<title>urb://~zod/p</title>"},
		{renderMarkdown("# A *b* `c` [d](x)", "urb://~zod/p", zodLink), "<title>A b c d</title>"},
		{renderMarkdown("## only h2", "urb://~zod/p", zodLink), "<title>urb://~zod/p</title>"},
	} {
		if !strings.Contains(c.got, c.want) {
			t.Errorf("want %s in %.200s", c.want, c.got)
		}
	}
}

// Lengths are UTF-16 code units, as the extension's are.
func TestMarkdownCapsInUnits(t *testing.T) {
	ok := renderMarkdown("# x\n"+strings.Repeat("\U0001F600", (mdMax-4)/2), "t", zodLink)
	if !strings.Contains(ok, "<h1>x</h1>") {
		t.Error("a body at the cap is not rendered")
	}
	over := renderMarkdown("# x\n"+strings.Repeat("\U0001F600", (mdMax-4)/2)+"y", "t", zodLink)
	if strings.Contains(over, "<h1>") || !strings.Contains(over, "<pre>\n# x\n") {
		t.Error("a body over the cap is not shown as text")
	}
	if m := renderMarkdown("<b>"+strings.Repeat("a", mdMax), "t", zodLink); !strings.Contains(m, "<pre>\n&lt;b&gt;") {
		t.Error("an over-cap body is not escaped")
	}
	smile := func(n int) string { return strings.Repeat("\U0001F600", n) }
	if m := renderMarkdown("["+smile(128)+"](/x)", "t", zodLink); !strings.Contains(m, "<a ") {
		t.Error("link text of 256 units dropped")
	}
	if m := renderMarkdown("["+smile(128)+"a](/x)", "t", zodLink); strings.Contains(m, "<a ") {
		t.Error("link text of 257 units kept")
	}
	if m := renderMarkdown("[a](/"+smile(255)+"a)", "t", zodLink); !strings.Contains(m, "<a ") {
		t.Error("a target of 512 units dropped")
	}
	if m := renderMarkdown("[a](/"+smile(256)+")", "t", zodLink); strings.Contains(m, "<a ") {
		t.Error("a target of 513 units kept")
	}
}

// Whitespace is JavaScript's, and . does not cross a line terminator.
func TestMarkdownWhitespace(t *testing.T) {
	for body, want := range map[string]string{
		"#\u00a0Title":         "<h1>Title</h1>",
		"-\u3000item":          "<li>item</li>",
		"1.\vfirst":            "<ol>\n<li>first</li>",
		"*\u00a0*\u00a0*":      "<hr>",
		"#\u0085x":             "<p>#\u0085x</p>", // NEL is not whitespace
		"- a\rb":               "<p>- a\rb</p>",   // . stops at \r, so not an item
		"- a\u2028b":           "<p>- a\u2028b</p>",
		"a *\u00a0b*":          "<p>a *\u00a0b*</p>", // em must not start with whitespace
		"[a](b\u00a0c)":        "<p>[a](b\u00a0c)</p>",
		"\u00a0> quoted":       "<blockquote>quoted</blockquote>",
		"\ufeff```\ncode\n```": "<pre>\ncode</pre>",
	} {
		if m := renderMarkdown(body, "t", zodLink); !strings.Contains(m, want) {
			t.Errorf("%q: want %q in %.300s", body, want, m)
		}
	}
	if g := renderGemtext("=>\u00a0/a\u00a0the\u3000label", "t", zodLink); !strings.Contains(g, `>the`+"\u3000"+`label</a>`) {
		t.Errorf("gemtext link: %s", g)
	}
}

// Hostile bodies cost time linear in their size: no regex backtracks.
func TestRenderTime(t *testing.T) {
	for name, body := range map[string]string{
		"spaces after #":  "#" + strings.Repeat(" ", 200_000) + "x",
		"open brackets":   strings.Repeat("[", 200_000),
		"half links":      strings.Repeat("](", 100_000),
		"long link texts": strings.Repeat("["+strings.Repeat("a", 255), 700),
		"stars":           strings.Repeat("* ", 100_000),
		"backticks":       "`" + strings.Repeat("a", 200_000),
		"list markers":    strings.Repeat("- ", 100_000) + "\r",
	} {
		t0 := time.Now()
		renderMarkdown(body, "t", zodLink)
		renderGemtext(body, "t", zodLink)
		if d := time.Since(t0); d > 2*time.Second {
			t.Errorf("%s: %s", name, d)
		}
	}
}

// rewriteHTML rewrites any urb:// link the extension's reader would
// follow: an attribute named in any case, its value quoted or not.
func TestRewriteHTMLAttributes(t *testing.T) {
	got := rewriteHTML(`<A HREF="urb://~zod/x">a</A><a href=urb://~nec/y>b</a><img src='urb://~bud/z'>`, "8475")
	for _, want := range []string{`HREF="http://zod.urb.localhost:8475/x"`, `href=http://nec.urb.localhost:8475/y`,
		`src='http://bud.urb.localhost:8475/z'`} {
		if !strings.Contains(got, want) {
			t.Errorf("rewriteHTML lacks %s in %s", want, got)
		}
	}
}
