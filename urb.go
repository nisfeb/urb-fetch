package main

// Addresses: what an urb:// url names, where the gateway serves it, and
// how a page renders (render.go renders it). No network here.

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

// pageMarks are lattice's view modes (view-mode:pg). Release 74 and later
// publish each page under its mode's mark (+mode-mark), so the mark says
// the format. Older releases publish every page as %gmi
// (nisfeb/lattice#321).
var pageMarks = map[string]bool{"text": true, "html": true, "gmi": true, "md": true, "js": true, "css": true, "noun": true}

// renderAs is how a page published under mark renders, as lattice's
// reader does: "html", "md", "gmi" or "text". Only a %gmi page is
// sniffed, since %gmi is all an older publisher, an old revision or the
// vault peek says: one whose body starts with < is HTML. js, css and
// noun render as text. No mark at all is %gmi, as in wasm/mesa.mjs.
func renderAs(mark, body string) string {
	switch mark {
	case "gmi", "":
		if looksHTML(body) {
			return "html"
		}
		return "gmi"
	case "html", "md":
		return mark
	}
	return "text"
}

// looksHTML is lattice-gmi's +looks-html: the first non-blank character
// is <. It reads as the extension's looksHtml does: within the first 256
// UTF-16 code units, and blank is JavaScript's whitespace.
func looksHTML(body string) bool {
	n := 0
	for i, r := range body {
		if n += 1 + btoi(r >= 0x10000); n > 256 {
			body = body[:i]
			break
		}
	}
	return strings.HasPrefix(strings.TrimLeftFunc(body, jsSpace), "<")
}

func btoi(b bool) int {
	if b {
		return 1
	}
	return 0
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

// an attribute that may hold a link, named in any case, its value quoted
// or not, as the extension's reader follows any <a href> clicked
var urbAttr = regexp.MustCompile(`((?i:href|src|action)\s*=\s*["']?)urb://~([a-z-]+)`)

// rewriteHTML points a page's urb:// links at the gateway.
func rewriteHTML(body, port string) string {
	return urbAttr.ReplaceAllString(body, "${1}http://${2}"+hostSuffix+":"+port)
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
