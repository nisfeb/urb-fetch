package main

// Addresses and rendering: what an urb:// url names, where the gateway
// serves it, and gemtext as HTML. No network here.

import (
	"fmt"
	"html"
	"regexp"
	"strings"

	mesa "github.com/mopfel-winrux/libmesa/bindings/go"
)

// httpError is a failure the gateway reports with a status code.
type httpError struct {
	status int
	msg    string
}

func (e *httpError) Error() string { return e.msg }

func fail(status int, format string, args ...any) error {
	return &httpError{status, fmt.Sprintf(format, args...)}
}

var knot = regexp.MustCompile(`^[a-z0-9~._-]+$`)

// deUrb resolves a published page's url the way lattice's +de-urb does
// (code/lib/lattice-urls.hoon): the ship and the page's path in the pub
// vault. /p/, /k/ and /t/ name the publisher's grubbery tree, which a
// stranger can't read, so they are refused.
func deUrb(url string) (mesa.Ship, []string, error) {
	rest, ok := strings.CutPrefix(url, "urb://")
	if !ok {
		return mesa.Ship{}, nil, fail(400, "not an urb:// url: %s", url)
	}
	who, rest, _ := strings.Cut(rest, "/")
	ship, err := mesa.ParseShip(who)
	if err != nil {
		return mesa.Ship{}, nil, fail(400, "no ship in %s", url)
	}
	rest = strings.TrimSuffix(rest, "/") // urb://~s/site/ is urb://~s/site
	if rest == "" {
		return ship, []string{"index"}, nil
	}
	segs := strings.Split(rest, "/")
	for _, s := range segs {
		if !knot.MatchString(s) {
			return mesa.Ship{}, nil, fail(400, "bad path in %s", url)
		}
	}
	switch {
	case len(segs[0]) > 1: // the frozen legacy form
		return ship, segs, nil
	case segs[0] == "n":
		return ship, segs[1:], nil
	case strings.Contains("pkt", segs[0]):
		return mesa.Ship{}, nil, fail(501, "/%s/ addresses read the publisher's tree, which this gateway does not do", segs[0])
	}
	return mesa.Ship{}, nil, fail(404, "/%s/ is not an urb:// mount", segs[0])
}

// looksHTML is lattice-gmi's +looks-html: the first non-blank character
// is <. Lattice grows every published page under %gmi whatever its
// format (nisfeb/lattice#321), so the body has to be sniffed.
func looksHTML(body string) bool {
	if len(body) > 256 {
		body = body[:256]
	}
	return strings.HasPrefix(strings.TrimLeft(body, " \t\r\n"), "<")
}

// hostSuffix: each ship is its own origin, <ship>.urb.localhost, so one
// ship's pages can't script another's. Browsers send *.localhost to the
// loopback address.
const hostSuffix = ".urb.localhost"

// gatewayURL is where the gateway serves an urb:// url.
func gatewayURL(url, port string) (string, error) {
	rest, ok := strings.CutPrefix(url, "urb://~")
	if !ok {
		return "", fail(400, "not an urb:// url: %s", url)
	}
	who, path, _ := strings.Cut(rest, "/")
	if _, err := mesa.ParseShip("~" + who); err != nil {
		return "", fail(400, "no ship in %s", url)
	}
	return "http://" + who + hostSuffix + ":" + port + "/" + path, nil
}

// urbOfHost is the urb:// url a gateway request names, or "" when the
// host isn't a ship's.
func urbOfHost(host, path string) string {
	who, ok := strings.CutSuffix(host, hostSuffix)
	if !ok || strings.Contains(who, ".") {
		return ""
	}
	return "urb://~" + who + path
}

var urbAttr = regexp.MustCompile(`((?:href|src|action)\s*=\s*["'])urb://~([a-z-]+)`)

// rewriteHTML points a page's urb:// links at the gateway.
func rewriteHTML(body, port string) string {
	return urbAttr.ReplaceAllString(body, "${1}http://${2}"+hostSuffix+":"+port)
}

// renderGemtext renders gemtext (gemini's text/gemini) as a page, with
// link targets passed through link.
func renderGemtext(body, title string, link func(string) string) string {
	var b strings.Builder
	esc := html.EscapeString
	pre, list := false, false
	for _, line := range strings.Split(strings.ReplaceAll(body, "\r\n", "\n"), "\n") {
		if strings.HasPrefix(line, "```") {
			if list {
				b.WriteString("</ul>\n")
				list = false
			}
			if pre {
				b.WriteString("</pre>\n")
			} else {
				b.WriteString("<pre>")
			}
			pre = !pre
			continue
		}
		if pre {
			b.WriteString(esc(line) + "\n")
			continue
		}
		item := strings.HasPrefix(line, "* ")
		if list && !item {
			b.WriteString("</ul>\n")
			list = false
		}
		switch {
		case strings.HasPrefix(line, "=>"):
			f := strings.Fields(line[2:])
			if len(f) == 0 {
				continue
			}
			label := strings.TrimSpace(strings.TrimPrefix(strings.TrimSpace(line[2:]), f[0]))
			if label == "" {
				label = f[0]
			}
			fmt.Fprintf(&b, "<p class=\"link\"><a href=\"%s\">%s</a></p>\n", esc(link(f[0])), esc(label))
		case strings.HasPrefix(line, "#"):
			lvl := len(line) - len(strings.TrimLeft(line, "#"))
			lvl = min(lvl, 3)
			fmt.Fprintf(&b, "<h%d>%s</h%d>\n", lvl, esc(strings.TrimSpace(strings.TrimLeft(line, "#"))), lvl)
		case item:
			if !list {
				b.WriteString("<ul>\n")
				list = true
			}
			b.WriteString("<li>" + esc(line[2:]) + "</li>\n")
		case strings.HasPrefix(line, ">"):
			b.WriteString("<blockquote>" + esc(strings.TrimSpace(line[1:])) + "</blockquote>\n")
		case strings.TrimSpace(line) != "":
			b.WriteString("<p>" + esc(line) + "</p>\n")
		}
	}
	if pre {
		b.WriteString("</pre>\n")
	}
	if list {
		b.WriteString("</ul>\n")
	}
	return page(title, b.String())
}

// page wraps a body in the gateway's own minimal document.
func page(title, body string) string {
	return `<!doctype html><html lang="en"><head><meta charset="utf-8">` +
		`<meta name="viewport" content="width=device-width, initial-scale=1">` +
		`<title>` + html.EscapeString(title) + `</title><style>` + css + `</style></head>` +
		`<body><main>` + body + `</main></body></html>`
}

const css = `:root{color-scheme:light dark;--fg:#1d1d1f;--bg:#fbfbf8;--mute:#6b6b70;--link:#2456c8}
@media (prefers-color-scheme:dark){:root{--fg:#e8e8e3;--bg:#18181a;--mute:#9a9aa0;--link:#8ab0ff}}
body{margin:0;background:var(--bg);color:var(--fg);font:17px/1.6 system-ui,sans-serif}
main{max-width:44rem;margin:0 auto;padding:2rem 1rem}
a{color:var(--link)}p.link{margin:.3rem 0}
pre{overflow-x:auto;padding:.8rem;border:1px solid var(--mute);border-radius:6px;font-size:14px}
blockquote{margin:1rem 0;padding-left:1rem;border-left:3px solid var(--mute);color:var(--mute)}
input{font:inherit;width:100%;box-sizing:border-box;padding:.5rem}`
