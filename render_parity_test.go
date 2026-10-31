package main

import (
	"encoding/json"
	"math/rand"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
)

// TestExtensionParity reads random bodies and link targets with the
// gateway's renderers and with nisfeb-extension's (testdata/
// ext_parity.mjs runs them in node), and wants the same blocks, spans,
// sniff and link for each. It needs node and the extension's checkout,
// at $NISFEB_EXTENSION or ../nisfeb-extension, and skips without them.
func TestExtensionParity(t *testing.T) {
	ext := os.Getenv("NISFEB_EXTENSION")
	if ext == "" {
		ext = "../nisfeb-extension"
	}
	if _, err := os.Stat(filepath.Join(ext, "lib", "markdown.js")); err != nil {
		t.Skipf("no nisfeb-extension at %s", ext)
	}
	if _, err := exec.LookPath("node"); err != nil {
		t.Skip("no node")
	}
	if _, err := os.Stat("wasm/mesa.mjs"); err != nil {
		t.Skip("no wasm/mesa.mjs")
	}

	rng := rand.New(rand.NewSource(1))
	pick := func(parts []string, n int) string {
		var b strings.Builder
		for range n {
			b.WriteString(parts[rng.Intn(len(parts))])
		}
		return b.String()
	}
	chars := strings.Split("# # * * ` [ ] ( ) _ - + > 1 .      \t \r \n \n \u00a0 \u0085 \ufeff \u2028 \v x y / : \\ é \U0001F600 < = h t p s \x01 a", " ")
	chars = append(chars, " ", "](", "**", strings.Repeat("a", 100), strings.Repeat("\U0001F600", 60))
	links := []string{"urb://~zod", "urb://~nec/x", "URB://~zod/y", "HTTPS://a.b/c?d#e", "http:/x", "http://", "mailto:a@b",
		"Gemini://g", "javascript:x", "/", "//", "\\", "..", ".", "x", "y/z", "?q", "#f", "%41", "%2e", "%2E", "é",
		"\U0001F600", ":", "a:b", "~", "'", "(", "!", "*", "\"", "<", "{", "|", "^", "`", "[", "]", "@", "$", "&", "+", ",", ";", "=", "\x01", " "}
	var cases []string
	for range 6000 {
		cases = append(cases, pick(chars, rng.Intn(70)))
	}
	for range 400 {
		cases = append(cases, pick(chars, rng.Intn(900)))
	}
	for range 6000 {
		cases = append(cases, pick(links, 1+rng.Intn(4)))
	}
	smile := strings.Repeat("\U0001F600", 128)
	cases = append(cases, "["+strings.Repeat("a", 256)+"](x)", "["+smile+"](x)", "["+smile+"a](x)",
		"[a](/"+strings.Repeat("\U0001F600", 255)+"a)", "[a](/"+smile+smile+")", strings.Repeat(" ", 255)+"<",
		strings.Repeat("\u3000", 256)+"<")

	file := filepath.Join(t.TempDir(), "cases.json")
	raw, _ := json.Marshal(cases)
	if err := os.WriteFile(file, raw, 0o644); err != nil {
		t.Fatal(err)
	}
	out, err := exec.Command("node", "testdata/ext_parity.mjs", ext, file).Output()
	if err != nil {
		t.Fatalf("node: %v", err)
	}
	var js []map[string]any
	if err := json.Unmarshal(out, &js); err != nil {
		t.Fatal(err)
	}

	link := pageLink("urb://~zod/a/b", "8475")
	bad := 0
	for i, c := range cases {
		got := map[string]any{"md": mdJSON(markdownBlocks(c)), "gmi": gmiJSON(gemtextBlocks(c)),
			"html": looksHTML(c), "safe": safeLink(c), "link": urbOf(link(c))}
		//  through JSON, so both sides have the same types
		var g map[string]any
		b, _ := json.Marshal(got)
		json.Unmarshal(b, &g)
		for _, k := range []string{"md", "gmi", "html", "safe", "link"} {
			if !reflect.DeepEqual(g[k], js[i][k]) {
				if bad++; bad <= 5 {
					t.Errorf("%s of %q:\n go %v\n js %v", k, c, g[k], js[i][k])
				}
			}
		}
	}
	if bad > 0 {
		t.Errorf("%d differences in %d cases", bad, len(cases))
	}
}

// urbOf is the urb:// address a gateway link names, or the link as is.
func urbOf(href string, _ bool) string {
	rest, ok := strings.CutPrefix(href, "http://")
	who, path, found := strings.Cut(rest, hostSuffix+":8475")
	if !ok || !found {
		return href
	}
	if path == "/" {
		path = ""
	}
	return "urb://~" + who + path
}

func spansJSON(ss []span) []map[string]string {
	out := []map[string]string{}
	for _, s := range ss {
		if s.kind == "link" {
			out = append(out, map[string]string{"link": s.target, "text": s.text})
		} else {
			out = append(out, map[string]string{s.kind: s.text})
		}
	}
	return out
}

// mdJSON and gmiJSON are the blocks as lib/markdown.js and lib/gemtext.js
// give them.
func mdJSON(bs []mdBlock) []map[string]any {
	out := []map[string]any{}
	for _, b := range bs {
		m := map[string]any{"kind": b.kind}
		switch b.kind {
		case "pre":
			m["text"] = b.text
		case "hr":
		case "ul", "ol":
			items := [][]map[string]string{}
			for _, it := range b.items {
				items = append(items, spansJSON(it))
			}
			m["items"] = items
		default:
			m["spans"] = spansJSON(b.spans)
		}
		out = append(out, m)
	}
	return out
}

func gmiJSON(bs []gemBlock) []map[string]any {
	out := []map[string]any{}
	for _, b := range bs {
		m := map[string]any{"kind": b.kind}
		switch b.kind {
		case "list":
			m["items"] = b.items
		case "link":
			m["href"], m["label"] = b.href, b.label
		default:
			m["text"] = b.text
		}
		out = append(out, m)
	}
	return out
}
