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
