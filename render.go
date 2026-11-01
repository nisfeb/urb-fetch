package main

// Rendering: gemtext, Markdown and text as the gateway's own HTML, read
// by the same rules as nisfeb-extension's reader (lib/gemtext.js,
// lib/markdown.js, reader.js), so a page reads the same in both. The
// extension parses in JavaScript, so where a rule turns on whitespace,
// "any character" or a length, it is JavaScript's: its whitespace (\s,
// trim), its line terminators (.), and lengths in UTF-16 code units.

import (
	"fmt"
	"html"
	"regexp"
	"strings"
	"unicode/utf8"
)

// linker says where a page's link target goes: href "" leaves it as
// text, and own is whether the gateway serves it (a link that leaves
// gets rel="noopener noreferrer").
type linker func(target string) (href string, own bool)

// jsSpace is JavaScript's whitespace, what \s matches and trim() trims.
func jsSpace(r rune) bool {
	switch r {
	case '\t', '\n', '\v', '\f', '\r', ' ', 0xa0, 0x1680, 0x2028, 0x2029, 0x202f, 0x205f, 0x3000, 0xfeff:
		return true
	}
	return r >= 0x2000 && r <= 0x200a
}

func jsTrim(s string) string { return strings.TrimFunc(s, jsSpace) }

// units is s's length in UTF-16 code units, as JavaScript counts it.
func units(s string) int {
	n := 0
	for _, r := range s {
		n++
		if r >= 0x10000 {
			n++
		}
	}
	return n
}

// jsS and jsDot are \s and . in a JavaScript regex, for RE2.
const (
	jsS   = `[\t\n\v\f\r \x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}\x{feff}]`
	jsDot = `[^\n\r\x{2028}\x{2029}]`
)

var (
	mdHeading = regexp.MustCompile(`^(#{1,6})` + jsS)
	mdItem    = regexp.MustCompile(`^` + jsS + `*(?:[-*+]|[0-9]+[.)])` + jsS + `+(` + jsDot + `*)$`)
	mdOrdered = regexp.MustCompile(`^` + jsS + `*[0-9]+[.)]` + jsS)
	mdRule    = regexp.MustCompile(`^` + jsS + `*(?:(?:-` + jsS + `*){3,}|(?:\*` + jsS + `*){3,}|(?:_` + jsS + `*){3,})$`)
	// Schemes match in any case, but in ASCII only, as JavaScript's
	// /i does: Go's (?i) also folds U+017F (ſ) into s and U+212A (the
	// Kelvin sign) into k, so httpſ://x would be a link here and text
	// in the extension.
	safeHref = regexp.MustCompile(`^(?:(?:[hH][tT][tT][pP][sS]?|[uU][rR][bB]|[gG][eE][mM][iI][nN][iI]|[mM][aA][iI][lL][tT][oO]):|[^:]*$)`)
	ctlLink  = regexp.MustCompile(`[\x00-\x20\x7f]`)
	hostLink = regexp.MustCompile(`^` + jsS + `*[/\\]{2}`)
	scheme   = regexp.MustCompile(`^[a-zA-Z][a-zA-Z0-9+.-]*:`)
	urbLink  = regexp.MustCompile(`^urb://~[a-z-]+(/|$)`)
	webLink  = regexp.MustCompile(`^[hH][tT][tT][pP][sS]?://[^/]`)
	pageURL  = regexp.MustCompile(`^urb://(~[a-z-]+)(/[^?#]*)?`)
)

// mdMax: a Markdown page longer than this, in UTF-16 code units as the
// extension's MD_MAX counts it, shows as text in a <pre>.
const mdMax = 256 * 1024

// safeLink is whether a page's link target may be a link: the web,
// urb://, gemini, mail, or a path on this ship. A protocol-relative //
// (or \\, which browsers read as //, after JavaScript's whitespace)
// would leave the ship's origin. A control character (C0, space, DEL)
// anywhere makes it text, since a browser strips them; schemes match
// in any case, in ASCII.
func safeLink(t string) bool {
	return !ctlLink.MatchString(t) && safeHref.MatchString(t) && !hostLink.MatchString(t)
}

// pageLink is where a link on the page at the urb:// address page goes,
// as the extension's reader sends a Markdown link (reader.js's mdHref,
// then lib/gemtext.js's linkTarget), here for gemtext too: a target
// safeLink refuses is text; the scheme is lower-cased; mail and gemini
// stay as written; urb:// goes to the gateway; the web, with a host,
// stays as written; any other scheme is text; and a relative link is
// resolved against the page, its path only, on the page's ship.
func pageLink(page, port string) linker {
	return func(target string) (string, bool) {
		if !safeLink(target) {
			return "", false
		}
		t := target
		if m := scheme.FindString(t); m != "" {
			t = strings.ToLower(m) + t[len(m):]
		}
		if strings.HasPrefix(t, "mailto:") || strings.HasPrefix(t, "gemini:") {
			return t, false
		}
		t = jsTrim(t) // linkTarget trims, in JavaScript's whitespace
		switch {
		case urbLink.MatchString(t):
			u, err := gatewayURL(t, port)
			return u, err == nil
		case webLink.MatchString(t):
			return t, false
		case t == "" || scheme.MatchString(t) || strings.HasPrefix(t, "//"):
			return "", false
		}
		m := pageURL.FindStringSubmatch(page)
		if m == nil {
			return "", false
		}
		base := m[2]
		if base == "" {
			base = "/"
		}
		p := resolve(base, strings.ReplaceAll(t, `\`, "/"))
		if p == "/" {
			p = ""
		}
		u, err := gatewayURL("urb://"+m[1]+p, port)
		return u, err == nil
	}
}

// resolve is the path of the relative reference ref on the page at
// path base, as a WHATWG URL parser (the extension's) gives it: query
// and fragment dropped, dot segments (. .. and their %2e forms) removed,
// and what the parser escapes in a path escaped, the rest left as
// written. A URL parser reads \ as / in a web path; the caller has.
func resolve(base, ref string) string {
	if i := strings.IndexAny(ref, "?#"); i >= 0 {
		ref = ref[:i]
	}
	switch {
	case ref == "":
		ref = base
	case !strings.HasPrefix(ref, "/"):
		ref = base[:strings.LastIndex(base, "/")+1] + ref
	}
	segs := strings.Split(ref[1:], "/")
	var out []string
	for i, sg := range segs {
		last := i == len(segs)-1
		switch strings.ToLower(sg) {
		case ".", "%2e":
			if last {
				out = append(out, "")
			}
		case "..", ".%2e", "%2e.", "%2e%2e":
			if len(out) > 0 {
				out = out[:len(out)-1]
			}
			if last {
				out = append(out, "")
			}
		default:
			out = append(out, sg)
		}
	}
	var b strings.Builder
	b.WriteByte('/')
	for _, c := range []byte(strings.Join(out, "/")) {
		if c <= 0x20 || c >= 0x7f || strings.IndexByte("\"<>^`{}", c) >= 0 {
			fmt.Fprintf(&b, "%%%02X", c)
		} else {
			b.WriteByte(c)
		}
	}
	return b.String()
}

// anchor is a link to href with text, or the text alone when href is "".
func anchor(href string, own bool, text string) string {
	esc := html.EscapeString
	switch {
	case href == "":
		return esc(text)
	case own:
		return fmt.Sprintf(`<a href="%s">%s</a>`, esc(href), esc(text))
	}
	return fmt.Sprintf(`<a href="%s" rel="noopener noreferrer">%s</a>`, esc(href), esc(text))
}

// preformatted text: the newline after <pre> is the one the HTML parser
// drops, so a first line that is blank stays.
func pre(text string) string { return "<pre>\n" + html.EscapeString(text) + "</pre>" }

// ------------------------------------------------------------ gemtext

type gemBlock struct {
	kind        string // h1 h2 h3 p quote pre list link
	text        string
	items       []string
	href, label string
}

// gemtextBlocks is lib/gemtext.js's gemtext: a page's blocks, in order.
func gemtextBlocks(body string) []gemBlock {
	var out []gemBlock
	var preLines []string
	inPre, list := false, -1
	for _, line := range strings.Split(strings.ReplaceAll(body, "\r\n", "\n"), "\n") {
		if strings.HasPrefix(line, "```") {
			list = -1
			if inPre {
				out = append(out, gemBlock{kind: "pre", text: strings.Join(preLines, "\n")})
				preLines, inPre = nil, false
			} else {
				inPre = true
			}
			continue
		}
		if inPre {
			preLines = append(preLines, line)
			continue
		}
		item := strings.HasPrefix(line, "* ")
		if !item {
			list = -1
		}
		switch {
		case strings.HasPrefix(line, "=>"):
			rest := jsTrim(line[2:])
			if rest == "" {
				continue
			}
			href := rest
			if i := strings.IndexFunc(rest, jsSpace); i >= 0 {
				href = rest[:i]
			}
			label := jsTrim(rest[len(href):])
			if label == "" {
				label = href
			}
			out = append(out, gemBlock{kind: "link", href: href, label: label})
		case strings.HasPrefix(line, "#"):
			marks := len(line) - len(strings.TrimLeft(line, "#"))
			out = append(out, gemBlock{kind: fmt.Sprintf("h%d", min(marks, 3)), text: jsTrim(line[marks:])})
		case item:
			if list < 0 {
				out = append(out, gemBlock{kind: "list"})
				list = len(out) - 1
			}
			out[list].items = append(out[list].items, line[2:])
		case strings.HasPrefix(line, ">"):
			out = append(out, gemBlock{kind: "quote", text: jsTrim(line[1:])})
		case jsTrim(line) != "":
			out = append(out, gemBlock{kind: "p", text: line})
		}
	}
	if inPre {
		out = append(out, gemBlock{kind: "pre", text: strings.Join(preLines, "\n")})
	}
	return out
}

// renderGemtext renders gemtext (gemini's text/gemini) as a page, its
// links through link. A link left as text shows its target, as
// "label (target)". The title is the first h1's, else title.
func renderGemtext(body, title string, link linker) string {
	var b strings.Builder
	esc := html.EscapeString
	titled := false
	for _, k := range gemtextBlocks(body) {
		switch k.kind {
		case "list":
			b.WriteString("<ul>\n")
			for _, it := range k.items {
				b.WriteString("<li>" + esc(it) + "</li>\n")
			}
			b.WriteString("</ul>\n")
		case "link":
			b.WriteString(`<p class="link">`)
			if href, own := link(k.href); href != "" {
				b.WriteString(anchor(href, own, k.label))
			} else if k.label == k.href {
				b.WriteString(esc(k.href))
			} else {
				b.WriteString(esc(k.label + " (" + k.href + ")"))
			}
			b.WriteString("</p>\n")
		case "pre":
			b.WriteString(pre(k.text) + "\n")
		case "quote":
			b.WriteString("<blockquote>" + esc(k.text) + "</blockquote>\n")
		default:
			if k.kind == "h1" && !titled {
				title, titled = k.text, true
			}
			fmt.Fprintf(&b, "<%s>%s</%s>\n", k.kind, esc(k.text), k.kind)
		}
	}
	return page(title, b.String())
}

// renderText renders a text page (text, js, css) as preformatted text.
func renderText(body, title string) string {
	return page(title, pre(body))
}

// ------------------------------------------------------------ Markdown

// a span of a Markdown line: text, code, strong, em or a link (target
// set only when safeLink allows it)
type span struct {
	kind, text, target string
}

type mdBlock struct {
	kind  string // h1..h6 p quote ul ol hr pre
	spans []span
	items [][]span
	text  string
}

// run is the length of s's prefix of characters not stop, in UTF-16
// code units, stopping past limit units (-1 for none), and its end in s.
func run(s string, stop func(rune) bool, limit int) (n, end int) {
	for i, r := range s {
		if stop(r) {
			return n, i
		}
		if n++; r >= 0x10000 {
			n++
		}
		if limit >= 0 && n > limit {
			return n, i
		}
	}
	return n, len(s)
}

func is(c rune) func(rune) bool { return func(r rune) bool { return r == c } }

// spans is lib/markdown.js's spans: its regex
// `([^`]+)`|\[([^\]]{1,256})\]\(([^)\s]{1,512})\)|\*\*([^*]+)\*\*|\*([^*\s][^*]*)\*
// matched by hand, with its lengths in UTF-16 code units and its \s
// JavaScript's. Each scan is bounded by the regex's own caps.
func spans(line string) []span {
	var out []span
	text := func(s string) {
		if s != "" {
			out = append(out, span{kind: "text", text: s})
		}
	}
	at := 0
	for i := 0; i < len(line); i++ {
		rest := line[i:]
		var sp span
		var end int // the match's end in rest, 0 for none
		switch rest[0] {
		case '`':
			if n, e := run(rest[1:], is('`'), -1); n > 0 && e+1 < len(rest) {
				sp, end = span{kind: "code", text: rest[1 : 1+e]}, e+2
			}
		case '[':
			n, e := run(rest[1:], is(']'), 256)
			if n < 1 || n > 256 || !strings.HasPrefix(rest[1+e:], "](") {
				break
			}
			tgt := rest[1+e+2:]
			m, f := run(tgt, func(r rune) bool { return r == ')' || jsSpace(r) }, 512)
			if m < 1 || m > 512 || !strings.HasPrefix(tgt[f:], ")") {
				break
			}
			sp, end = span{kind: "text", text: rest[1 : 1+e]}, 1+e+2+f+1
			if t := tgt[:f]; safeLink(t) {
				sp.kind, sp.target = "link", t
			}
		case '*':
			if strings.HasPrefix(rest, "**") {
				if n, e := run(rest[2:], is('*'), -1); n > 0 && strings.HasPrefix(rest[2+e:], "**") {
					sp, end = span{kind: "strong", text: rest[2 : 2+e]}, 2+e+2
					break
				}
			}
			if len(rest) < 2 {
				break
			}
			first, _ := utf8.DecodeRuneInString(rest[1:])
			if first == '*' || jsSpace(first) {
				break
			}
			if _, e := run(rest[1:], is('*'), -1); 1+e < len(rest) {
				sp, end = span{kind: "em", text: rest[1 : 1+e]}, 1+e+1
			}
		}
		if end == 0 {
			continue
		}
		text(line[at:i])
		out = append(out, sp)
		at = i + end
		i = at - 1
	}
	text(line[at:])
	return out
}

// headingText is a heading's text less the #s that close it, as
// lib/markdown.js's headingText takes it.
func headingText(rest string) string {
	t := strings.TrimRightFunc(rest, jsSpace)
	return jsTrim(strings.TrimRight(t, "#"))
}

// markdownBlocks is lib/markdown.js's markdown: a page's blocks, in order.
func markdownBlocks(body string) []mdBlock {
	var out []mdBlock
	var para, preLines []string
	list := -1
	flush := func() {
		if len(para) > 0 {
			out = append(out, mdBlock{kind: "p", spans: spans(strings.Join(para, " "))})
		}
		para, list = nil, -1
	}
	inPre := false
	for _, line := range strings.Split(strings.ReplaceAll(body, "\r\n", "\n"), "\n") {
		if strings.HasPrefix(jsTrim(line), "```") {
			if inPre {
				out = append(out, mdBlock{kind: "pre", text: strings.Join(preLines, "\n")})
				preLines, inPre = nil, false
			} else {
				flush()
				inPre = true
			}
			continue
		}
		if inPre {
			preLines = append(preLines, line)
			continue
		}
		if m := mdItem.FindStringSubmatch(line); m != nil && !mdRule.MatchString(line) {
			kind := "ul"
			if mdOrdered.MatchString(line) {
				kind = "ol"
			}
			if len(para) > 0 || list < 0 || out[list].kind != kind {
				flush()
				out = append(out, mdBlock{kind: kind})
				list = len(out) - 1
			}
			out[list].items = append(out[list].items, spans(m[1]))
			continue
		}
		h := mdHeading.FindStringSubmatch(line)
		switch t := jsTrim(line); {
		case t == "":
			flush()
		case h != nil:
			flush()
			out = append(out, mdBlock{kind: fmt.Sprintf("h%d", len(h[1])), spans: spans(headingText(line[len(h[1]):]))})
		case mdRule.MatchString(line):
			flush()
			out = append(out, mdBlock{kind: "hr"})
		case strings.HasPrefix(t, ">"):
			flush()
			out = append(out, mdBlock{kind: "quote", spans: spans(jsTrim(t[1:]))})
		default:
			if list >= 0 {
				flush()
			}
			para = append(para, t)
		}
	}
	if inPre {
		out = append(out, mdBlock{kind: "pre", text: strings.Join(preLines, "\n")})
	}
	flush()
	return out
}

// renderMarkdown renders a small, safe subset of Markdown, as the
// extension reads it, with links through link: headings, paragraphs,
// lists, quotes, rules and fenced code, and inline code, links, bold and
// italics. Everything else shows as written: all text is escaped, so raw
// HTML shows as text. A body over mdMax is one <pre> of it as written.
// The title is the first h1's text, else title.
func renderMarkdown(body, title string, link linker) string {
	if units(body) > mdMax {
		return page(title, pre(body))
	}
	esc := html.EscapeString
	line := func(ss []span) string {
		var o strings.Builder
		for _, s := range ss {
			switch s.kind {
			case "code", "strong", "em":
				o.WriteString("<" + s.kind + ">" + esc(s.text) + "</" + s.kind + ">")
			case "link":
				href, own := link(s.target)
				o.WriteString(anchor(href, own, s.text))
			default:
				o.WriteString(esc(s.text))
			}
		}
		return o.String()
	}
	plain := func(ss []span) string {
		var o strings.Builder
		for _, s := range ss {
			o.WriteString(s.text)
		}
		return o.String()
	}
	var b strings.Builder
	titled := false
	for _, k := range markdownBlocks(body) {
		switch k.kind {
		case "pre":
			b.WriteString(pre(k.text) + "\n")
		case "hr":
			b.WriteString("<hr>\n")
		case "ul", "ol":
			b.WriteString("<" + k.kind + ">\n")
			for _, it := range k.items {
				b.WriteString("<li>" + line(it) + "</li>\n")
			}
			b.WriteString("</" + k.kind + ">\n")
		case "quote":
			b.WriteString("<blockquote>" + line(k.spans) + "</blockquote>\n")
		default:
			if k.kind == "h1" && !titled {
				title, titled = plain(k.spans), true
			}
			fmt.Fprintf(&b, "<%s>%s</%s>\n", k.kind, line(k.spans), k.kind)
		}
	}
	return page(title, b.String())
}
