package main

import (
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"reflect"
	"regexp"
	"strings"
	"testing"
	"time"
)

// The conformance vectors the Go implements (docs/rust-core.md, "The
// conformance suite"; each file's "about" says what its cases are):
// render-as.json and hostile-render.json, written by the JS and by hand,
// and the Go's own, deurb-go.json and render-go.json, which
// UPDATE_VECTORS=1 writes from what the Go gives today. The JS runs the
// rest (conformance/run.mjs).

type vectors struct {
	About     string  `json:"about"`
	Extension string  `json:"extension,omitempty"`
	Quick     int     `json:"quick,omitempty"`
	Cases     []vcase `json:"cases"`
}

type vcase struct {
	Fn    string            `json:"fn"`
	In    []json.RawMessage `json:"in"`
	Out   json.RawMessage   `json:"out,omitempty"`
	Error *string           `json:"error,omitempty"`
}

const (
	vectorPage = "urb://~zod/a/b"
	vectorPort = "8475"
)

// a vector's argument as text: a string, {"$repeat": [s, n]}, {"$cat":
// [...]}, or none (null or {"$u": 1}) as ""
func vtext(t *testing.T, raw json.RawMessage) string {
	var s string
	if json.Unmarshal(raw, &s) == nil {
		return s
	}
	var o map[string]json.RawMessage
	if err := json.Unmarshal(raw, &o); err != nil {
		if string(raw) == "null" {
			return ""
		}
		t.Fatalf("not text: %s", raw)
	}
	if r, ok := o["$repeat"]; ok {
		var rep [2]json.RawMessage
		var n int
		if json.Unmarshal(r, &rep) != nil || json.Unmarshal(rep[1], &n) != nil {
			t.Fatalf("bad $repeat: %s", r)
		}
		return strings.Repeat(vtext(t, rep[0]), n)
	}
	if c, ok := o["$cat"]; ok {
		var parts []json.RawMessage
		if json.Unmarshal(c, &parts) != nil {
			t.Fatalf("bad $cat: %s", c)
		}
		var b strings.Builder
		for _, p := range parts {
			b.WriteString(vtext(t, p))
		}
		return b.String()
	}
	if _, ok := o["$u"]; ok {
		return ""
	}
	t.Fatalf("not text: %s", raw)
	return ""
}

var (
	titleOf = regexp.MustCompile(`(?s)<title>(.*?)</title>`)
	mainOf  = regexp.MustCompile(`(?s)<main>(.*)</main>`)
	rawTag  = regexp.MustCompile(`(?i)<(script|img|iframe|b|style|object|embed|svg)[\s>/]`)
)

// a rendered page as {title, main}: page() wraps every one alike
func pageParts(html string) map[string]string {
	t, m := titleOf.FindStringSubmatch(html), mainOf.FindStringSubmatch(html)
	if t == nil || m == nil {
		return map[string]string{"page": html}
	}
	return map[string]string{"title": t[1], "main": m[1]}
}

// one case run: its output, or its error
func runVector(t *testing.T, c vcase) (any, error) {
	arg := func(i int) string { return vtext(t, c.In[i]) }
	link := pageLink(vectorPage, vectorPort)
	switch c.Fn {
	case "deUrb":
		ship, segs, err := deUrb(arg(0))
		if err != nil {
			var h *httpError
			if errors.As(err, &h) {
				return nil, fmt.Errorf("%d %s", h.status, h.msg)
			}
			return nil, err
		}
		return map[string]any{"ship": ship.String(), "rel": segs}, nil
	case "renderAs":
		return renderAs(arg(0), arg(1)), nil
	case "looksHtml":
		return looksHTML(arg(0)), nil
	case "renderGemtext":
		return pageParts(renderGemtext(arg(0), "t", link)), nil
	case "renderMarkdown":
		return pageParts(renderMarkdown(arg(0), "t", link)), nil
	case "renderText":
		return pageParts(renderText(arg(0), "t")), nil
	case "rewriteHTML":
		return rewriteHTML(arg(0), vectorPort), nil
	case "page":
		return page(arg(0), arg(1)), nil
	case "safeLink":
		return safeLink(arg(0)), nil
	case "pageLink":
		href, own := link(arg(0))
		return map[string]any{"href": href, "own": own}, nil
	case "mdLink":
		for _, s := range spans("[" + arg(0) + "](" + arg(1) + ")") {
			if s.kind == "link" {
				return true, nil
			}
		}
		return false, nil
	case "mdFirst":
		main := pageParts(renderMarkdown(arg(0), "t", link))["main"]
		return regexp.MustCompile(`^<([a-z0-9]+)`).FindStringSubmatch(main)[1], nil
	case "escapes", "renders":
		var out string
		switch arg(0) {
		case "md":
			out = renderMarkdown(arg(1), "t", link)
		case "gmi":
			out = renderGemtext(arg(1), "t", link)
		default:
			out = renderText(arg(1), "t")
		}
		if c.Fn == "renders" {
			return true, nil
		}
		return !rawTag.MatchString(mainOf.FindStringSubmatch(out)[1]), nil
	}
	return nil, fmt.Errorf("no %s in the Go", c.Fn)
}

func readVectors(t *testing.T, name string) vectors {
	b, err := os.ReadFile("conformance/" + name + ".json")
	if err != nil {
		t.Fatal(err)
	}
	var v vectors
	if err := json.Unmarshal(b, &v); err != nil {
		t.Fatalf("%s: %v", name, err)
	}
	return v
}

func checkVectors(t *testing.T, name string, only map[string]bool) {
	v := readVectors(t, name)
	fails, ran := 0, 0
	for i, c := range v.Cases {
		if only != nil && !only[c.Fn] {
			continue
		}
		ran++
		start := time.Now()
		got, err := runVector(t, c)
		ms := time.Since(start).Milliseconds()
		ok := false
		switch {
		case c.Error != nil:
			ok = err != nil && (v.Quick > 0 && strings.Contains(err.Error(), *c.Error) || err.Error() == *c.Error)
		case err == nil:
			var want any
			_ = json.Unmarshal(c.Out, &want)
			g, _ := json.Marshal(got)
			var have any
			_ = json.Unmarshal(g, &have)
			ok = reflect.DeepEqual(want, have)
		}
		if v.Quick > 0 && ms > int64(v.Quick) {
			ok = false
		}
		if !ok {
			if fails++; fails <= 5 {
				in, _ := json.Marshal(c.In)
				t.Errorf("%s %d %s(%.100s) in %d ms: want %.200s%s, got %.200v %v", name, i, c.Fn, in, ms, c.Out, deref(c.Error), got, err)
			}
		}
	}
	if fails > 5 {
		t.Errorf("%s: %d of %d cases fail", name, fails, ran)
	}
	t.Logf("%s: %d cases", name, ran)
}

func deref(s *string) string {
	if s == nil {
		return ""
	}
	return *s
}

// the Go's own vectors, as the Go gives them today
func writeGoVectors(t *testing.T) {
	var urls []string
	b, _ := os.ReadFile("conformance/inputs/urls.json")
	if err := json.Unmarshal(b, &urls); err != nil {
		t.Fatal(err)
	}
	var render struct{ Bodies, Lines, Links []string }
	b, _ = os.ReadFile("conformance/inputs/render.json")
	if err := json.Unmarshal(b, &render); err != nil {
		t.Fatal(err)
	}
	str := func(s string) json.RawMessage { b, _ := json.Marshal(s); return b }
	fill := func(cases []vcase) []vcase {
		for i := range cases {
			if cases[i].Out != nil {
				continue // render-go-bytes', filled by runBytes
			}
			got, err := runVector(t, cases[i])
			if err != nil {
				e := err.Error()
				cases[i].Error = &e
				continue
			}
			cases[i].Out, _ = json.Marshal(got)
		}
		return cases
	}
	save := func(name, about string, cases []vcase) {
		b, _ := json.MarshalIndent(vectors{About: about, Cases: fill(cases)}, "", " ")
		if err := os.WriteFile("conformance/"+name+".json", append(b, '\n'), 0o644); err != nil {
			t.Fatal(err)
		}
		t.Logf("wrote %s.json: %d cases", name, len(cases))
	}
	var du []vcase
	for _, u := range urls {
		du = append(du, vcase{Fn: "deUrb", In: []json.RawMessage{str(u)}})
	}
	save("deurb-go", "urb.go's deUrb(url): {ship, rel} for a page the gateway reads, or \"<status> <message>\"", du)
	bodies := append([]string{}, render.Bodies...)
	bodies = append(bodies, "<a href=\"urb://~nec/x\">x</a> <img SRC='urb://~zod/i.png'> <form action=urb://~bud/f>",
		"<A HREF = \"urb://~nec\">", "href=urb://~nec", "<a href=\"URB://~nec\">")
	var rc []vcase
	for _, b := range bodies {
		for _, fn := range []string{"renderGemtext", "renderMarkdown", "renderText", "rewriteHTML"} {
			rc = append(rc, vcase{Fn: fn, In: []json.RawMessage{str(b)}})
		}
	}
	for _, l := range render.Links {
		rc = append(rc, vcase{Fn: "safeLink", In: []json.RawMessage{str(l)}}, vcase{Fn: "pageLink", In: []json.RawMessage{str(l)}})
	}
	rc = append(rc, vcase{Fn: "page", In: []json.RawMessage{str("a <title>"), str("<p>x</p>")}})
	var random struct{ Bodies, Links []string }
	b, _ = os.ReadFile("conformance/inputs/random.json")
	if err := json.Unmarshal(b, &random); err != nil {
		t.Fatal(err)
	}
	var rr []vcase
	for _, b := range random.Bodies {
		rr = append(rr, vcase{Fn: "renderGemtext", In: []json.RawMessage{str(b)}}, vcase{Fn: "renderMarkdown", In: []json.RawMessage{str(b)}})
	}
	for _, l := range random.Links {
		rr = append(rr, vcase{Fn: "safeLink", In: []json.RawMessage{str(l)}}, vcase{Fn: "pageLink", In: []json.RawMessage{str(l)}})
	}
	for _, b := range random.Bodies {
		rr = append(rr, vcase{Fn: "renderText", In: []json.RawMessage{str(b)}}, vcase{Fn: "rewriteHTML", In: []json.RawMessage{str(b)}})
	}
	for _, b := range randomHTML() {
		rr = append(rr, vcase{Fn: "rewriteHTML", In: []json.RawMessage{str(b)}})
	}
	save("render-go", fmt.Sprintf("urb.go's and render.go's HTML: renderGemtext, renderMarkdown and renderText(body, \"t\", pageLink(%q, %q)) as the page's {title, main} (page() wraps each alike), rewriteHTML(body, %q), safeLink, pageLink's {href, own}, and page(title, body) whole", vectorPage, vectorPort, vectorPort), rc)
	save("render-go-random", fmt.Sprintf("the same, on random bodies and links (conformance/random.mjs, inputs/random.json): renderGemtext and renderMarkdown(body, \"t\", pageLink(%q, %q)) and renderText(body, \"t\") as {title, main}, safeLink, pageLink's {href, own}, and rewriteHTML(body, %q), then rewriteHTML on random attribute soup (randomHTML in conformance_test.go)", vectorPage, vectorPort, vectorPort), rr)
	save("render-go-bytes", fmt.Sprintf("the same on random bytes, most not UTF-8 (randomBytes in conformance_test.go), every argument and every output hex: renderGemtext, renderMarkdown and renderText(body, \"t\", pageLink(%q, %q)) as the whole page, rewriteHTML(body, %q), looksHtml, renderAs(mark, body), safeLink, and pageLink's {href, own}", vectorPage, vectorPort, vectorPort), bytesCases(t))
}

func TestConformance(t *testing.T) {
	if os.Getenv("UPDATE_VECTORS") != "" {
		writeGoVectors(t)
	}
	checkVectors(t, "deurb-go", nil)
	checkVectors(t, "render-go", nil)
	checkVectors(t, "render-go-random", nil)
	checkBytes(t)
	checkVectors(t, "render-as", nil)
	checkVectors(t, "hostile-render", nil)
}

// xorshift32 from a fixed seed, so the random files are the same each
// time they are written
type xorshift uint32

func (s *xorshift) next() uint32 {
	*s ^= *s << 13
	*s ^= *s >> 17
	*s ^= *s << 5
	return uint32(*s)
}

func (s *xorshift) pick(xs []string, n uint32) string {
	var b strings.Builder
	for i := uint32(0); i < n; i++ {
		b.WriteString(xs[s.next()%uint32(len(xs))])
	}
	return b.String()
}

// random attributes for rewriteHTML: each a word of its own in any
// case (and ſ) or a near miss, whitespace RE2's \s takes and some it
// doesn't, = or not, either quote or none, and urb:// links to names
// of its class and outside it, among bits of the tags around them
func randomHTML() []string {
	words := []string{"href", "HREF", "Href", "hReF", "src", "SRC", "ſrc", "\u017fRC", "action", "ACTION", "Action", "hre", "ref",
		"xhref", "srcset", "data-src", "a"}
	ws := []string{"", "", "", " ", "  ", "\t", "\n", "\r", "\f", "\v", "\u00a0"}
	eq := []string{"=", "=", "=", "=", "==", "", ":"}
	quote := []string{"", "", "\"", "\"", "'", "'", "`", "\"\"", "' "}
	pre := []string{"urb://~", "urb://~", "urb://~", "urb://~", "URB://~", "Urb://~", "urb://", "urb:/~", "urb://~~", "http://~"}
	name := []string{"zod", "nec", "sampel-palnet", "-", "--", "zod-", "Zod", "zOd", "1", "", "é", "zod/x", "nec?q", "zod.x"}
	fill := []string{"", " ", "<a ", "<img ", "<form ", ">", "\">", "'>", "x", "\n", "<a href=", "é", "\U0001F600", "&amp;"}
	s := xorshift(0x6d2b79f5)
	var out []string
	for i := 0; i < 1000; i++ {
		var b strings.Builder
		for n := s.next() % 6; n > 0; n-- {
			b.WriteString(s.pick(fill, 1))
			b.WriteString(s.pick(words, 1))
			b.WriteString(s.pick(ws, 1))
			b.WriteString(s.pick(eq, 1))
			b.WriteString(s.pick(ws, 1))
			b.WriteString(s.pick(quote, 1))
			b.WriteString(s.pick(pre, 1))
			b.WriteString(s.pick(name, 1+s.next()%2))
			b.WriteString(s.pick(fill, 1))
		}
		out = append(out, b.String())
	}
	return out
}

// random bytes, most of them not UTF-8: lone leads and continuations,
// truncated sequences, surrogates, overlongs, 0xFF, among the marks
// the renderers and rewriteHTML turn on
func randomBytes() (bodies, links []string) {
	parts := []string{"\xff", "\xfe", "\x80", "\xbf", "\xc5", "\xc5\xbf", "\xc0\xaf", "\xc1\xbf", "\xe2\x80", "\xe2\x80\xa8",
		"\xed\xa0\x80", "\xed\xbf\xbf", "\xef\xbb\xbf", "\xf0\x9f\x98", "\xf0\x9f\x98\x80", "\xf4\x90\x80\x80", "\xf8\x88\x80\x80\x80",
		"\xc2\xa0", "\x00", "\x01", "\x7f", "#", "# ", "* ", "- ", "1. ", "> ", "=> ", "```", "**", "`", "[", "](", ")", "_", " ",
		" ", "\t", "\n", "\n", "\r", "<", ">", "&", "'", "\"", "=", "href=", "SRC='", "href=\"urb://~", "src=urb://~", "ACTION = 'urb://~", "urb://~", "zod", "nec", "abc",
		"https://", "httpſ://", "mailto:a@b", "/", "\\", ":", "x", "<html>", "<!doctype html>", "a"}
	lparts := []string{"urb://~zod", "urb://~nec/", "https://a.b/", "http://", "mailto:a@b", "gemini://g", "javascript:x", "/", "//",
		"\\", "..", "x", "%2e", " ", "\t", "\xff", "\x80", "\xc5", "\xc5\xbf", "\xed\xa0\x80", "\xc0\xaf", "\xe2\x80\xa8", "\xc2\xa0",
		"\x01", "\x7f", "\xf0\x9f\x98"}
	s := xorshift(0x1b873593)
	for i := 0; i < 300; i++ {
		bodies = append(bodies, s.pick(parts, s.next()%60))
	}
	for i := 0; i < 20; i++ {
		bodies = append(bodies, s.pick(parts, s.next()%600))
	}
	for i := 0; i < 300; i++ {
		links = append(links, s.pick(lparts, 1+s.next()%4))
	}
	return bodies, links
}

var byteMarks = []string{"html", "md", "gmi", "text", ""}

// render-go-bytes.json's cases, every argument hex
func bytesCases(t *testing.T) []vcase {
	bodies, links := randomBytes()
	hx := func(s string) json.RawMessage { b, _ := json.Marshal(hex.EncodeToString([]byte(s))); return b }
	var cs []vcase
	for i, b := range bodies {
		for _, fn := range []string{"renderGemtext", "renderMarkdown", "renderText", "rewriteHTML", "looksHtml"} {
			cs = append(cs, vcase{Fn: fn, In: []json.RawMessage{hx(b)}})
		}
		cs = append(cs, vcase{Fn: "renderAs", In: []json.RawMessage{hx(byteMarks[i%len(byteMarks)]), hx(b)}})
	}
	for _, l := range links {
		cs = append(cs, vcase{Fn: "safeLink", In: []json.RawMessage{hx(l)}}, vcase{Fn: "pageLink", In: []json.RawMessage{hx(l)}})
	}
	for i := range cs {
		cs[i].Out, _ = json.Marshal(runBytes(t, cs[i]))
	}
	return cs
}

// one render-go-bytes case run: its arguments and every string it
// gives are hex
func runBytes(t *testing.T, c vcase) any {
	arg := func(i int) string {
		var h string
		if json.Unmarshal(c.In[i], &h) != nil {
			t.Fatalf("not hex: %s", c.In[i])
		}
		b, err := hex.DecodeString(h)
		if err != nil {
			t.Fatalf("not hex: %s", h)
		}
		return string(b)
	}
	hs := func(s string) string { return hex.EncodeToString([]byte(s)) }
	link := pageLink(vectorPage, vectorPort)
	switch c.Fn {
	case "renderGemtext":
		return hs(renderGemtext(arg(0), "t", link))
	case "renderMarkdown":
		return hs(renderMarkdown(arg(0), "t", link))
	case "renderText":
		return hs(renderText(arg(0), "t"))
	case "rewriteHTML":
		return hs(rewriteHTML(arg(0), vectorPort))
	case "looksHtml":
		return looksHTML(arg(0))
	case "renderAs":
		return renderAs(arg(0), arg(1))
	case "safeLink":
		return safeLink(arg(0))
	case "pageLink":
		href, own := link(arg(0))
		return map[string]any{"href": hs(href), "own": own}
	}
	t.Fatalf("no %s in render-go-bytes", c.Fn)
	return nil
}

func checkBytes(t *testing.T) {
	v := readVectors(t, "render-go-bytes")
	fails := 0
	for i, c := range v.Cases {
		g, _ := json.Marshal(runBytes(t, c))
		var want, have any
		_ = json.Unmarshal(c.Out, &want)
		_ = json.Unmarshal(g, &have)
		if !reflect.DeepEqual(want, have) {
			if fails++; fails <= 5 {
				t.Errorf("render-go-bytes %d %s(%.100s): want %.200s, got %.200s", i, c.Fn, c.In, c.Out, g)
			}
		}
	}
	if fails > 5 {
		t.Errorf("render-go-bytes: %d of %d cases fail", fails, len(v.Cases))
	}
	t.Logf("render-go-bytes: %d cases", len(v.Cases))
}
