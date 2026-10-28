// urb-fetch is a local gateway to lattice pages on the Urbit network,
// with no Urbit ship: it runs as a comet (or your moon) through libmesa
// and serves urb://~ship/page at http://ship.urb.localhost:8475/page.
package main

import (
	"context"
	"crypto/rand"
	"encoding/binary"
	"errors"
	"flag"
	"fmt"
	"html"
	"log"
	"net"
	"net/http"
	"os"
	"os/signal"
	"path/filepath"
	"strconv"
	"strings"
	"time"

	mesa "github.com/mopfel-winrux/libmesa/bindings/go"
	_ "modernc.org/sqlite"
)

func logf(format string, args ...any) { log.Printf(format, args...) }

func main() {
	listen := flag.String("listen", "localhost:8475", "address to serve on (keep it on loopback); localhost means both 127.0.0.1 and ::1")
	state := flag.String("state", defaultState(), "where the identity and its network state live")
	moon := flag.String("moon", "", "run as this moon: the keyfile its parent's |moon printed")
	newMoon := flag.Bool("new-moon", false, "the moon has never run anywhere: start its state empty")
	relayAt := flag.String("relay", "", "instead of the gateway, run the relay for libmesa in a browser (wasm/mesa.mjs) on this address, e.g. localhost:8476")
	origins := flag.String("relay-origins", "chrome-extension://*", "comma-separated Origin patterns the relay accepts (clients that send none, such as Node, are always accepted)")
	relayFake := flag.Bool("relay-fake", false, "for tests: relay to a fake network (urbit -F) on this machine, galaxy g at 127.0.0.1:31337+g")
	flag.Parse()

	if err := os.MkdirAll(*state, 0o700); err != nil {
		log.Fatal(err)
	}
	if *relayAt != "" {
		lns, _ := listenAll(*relayAt)
		rl, err := newRelay(*state, strings.Split(*origins, ","), *relayFake)
		if err != nil {
			log.Fatal(err)
		}
		srv := &http.Server{Handler: rl, ReadHeaderTimeout: 10 * time.Second}
		for _, ln := range lns[1:] {
			go srv.Serve(ln)
		}
		logf("relaying at ws://%s/", *relayAt)
		log.Fatal(srv.Serve(lns[0]))
	}

	lns, port := listenAll(*listen)
	r := newReader()
	node, err := open(*state, *moon, *newMoon, r)
	if err != nil {
		log.Fatal(err)
	}
	r.node = node
	logf("running as %s", node.Ship())

	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt)
	defer stop()
	go func() {
		if err := node.Run(ctx); err != nil && !errors.Is(err, context.Canceled) {
			logf("node: %v", err)
		}
		stop()
	}()
	srv := &http.Server{Handler: &gateway{r, port}, ReadHeaderTimeout: 10 * time.Second}
	for _, ln := range lns[1:] {
		go srv.Serve(ln)
	}
	go func() {
		<-ctx.Done()
		srv.Close()
	}()
	logf("serving http://urb.localhost:%s/", port)
	if err := srv.Serve(lns[0]); !errors.Is(err, http.ErrServerClosed) {
		log.Fatal(err)
	}
	node.Close()
}

// listenAll listens on addr, and on both loopbacks for localhost:
// browsers resolve localhost and *.localhost to either, ::1 often first.
// It returns the listeners and the port.
func listenAll(addr string) ([]net.Listener, string) {
	host, port, err := net.SplitHostPort(addr)
	if err != nil {
		log.Fatalf("%s: %v", addr, err)
	}
	addrs := []string{addr}
	if host == "localhost" {
		addrs = []string{net.JoinHostPort("127.0.0.1", port), net.JoinHostPort("::1", port)}
	}
	var lns []net.Listener
	for _, a := range addrs {
		ln, err := net.Listen("tcp", a)
		if err != nil && len(lns) > 0 {
			logf("not serving on %s: %v", a, err) // e.g. IPv6 is off
			continue
		}
		if err != nil {
			log.Fatal(err)
		}
		lns = append(lns, ln)
	}
	return lns, port
}

func defaultState() string {
	if d := os.Getenv("XDG_STATE_HOME"); d != "" {
		return filepath.Join(d, "urb-fetch")
	}
	home, _ := os.UserHomeDir()
	return filepath.Join(home, ".local", "state", "urb-fetch")
}

// open loads the identity (a comet, mined on first use, or a moon), its
// store and its UDP port, and opens the node.
func open(state, moon string, newMoon bool, app mesa.App) (*mesa.Node, error) {
	var id *mesa.Identity
	var err error
	if moon != "" {
		id, err = mesa.LoadKeyfile(moon)
	} else {
		id, err = mesa.LoadComet(filepath.Join(state, "comet.key"))
	}
	if err != nil {
		return nil, err
	}
	db := filepath.Join(state, strings.TrimPrefix(id.Ship().String(), "~")+".db")
	_, statErr := os.Stat(db)
	fresh := os.IsNotExist(statErr)
	if moon != "" && fresh && !newMoon {
		return nil, fmt.Errorf("no state for %s in %s. Pass -new-moon only if this moon has "+
			"never run anywhere; a moon that has run must be breached first (|moon-breach on its parent)",
			id.Ship(), state)
	}
	st, err := mesa.OpenSQLite("sqlite", db)
	if err != nil {
		return nil, err
	}
	p, err := udpPort(db + ".port")
	if err != nil {
		return nil, err
	}
	return mesa.Open(mesa.Config{Identity: id, Store: st, App: app, Port: p, Breached: moon != "" && fresh})
}

// udpPort is one UDP port per identity, kept across runs. Publishers send
// their answers to the address they last saw: a new port each run leaves
// them resending to a dead one, with newer answers queued behind it (a
// 79 s stall, measured).
func udpPort(file string) (int, error) {
	if b, err := os.ReadFile(file); err == nil {
		return strconv.Atoi(strings.TrimSpace(string(b)))
	}
	var b [2]byte
	rand.Read(b[:])
	p := 20000 + int(binary.BigEndian.Uint16(b[:]))%40000
	return p, os.WriteFile(file, []byte(strconv.Itoa(p)+"\n"), 0o600)
}

// ------------------------------------------------------------ the gateway

type gateway struct {
	r    *Reader
	port string
}

// sandbox is lattice's own +sandbox-csp for a peer's HTML: an opaque
// origin, so its scripts run but hold no cookie and reach nothing of ours.
const sandbox = "sandbox allow-scripts allow-forms allow-popups allow-top-navigation-by-user-activation"

func (g *gateway) ServeHTTP(w http.ResponseWriter, req *http.Request) {
	if req.Method != http.MethodGet && req.Method != http.MethodHead {
		http.Error(w, "read only", http.StatusMethodNotAllowed)
		return
	}
	host, _, err := net.SplitHostPort(req.Host)
	if err != nil {
		host = req.Host
	}
	switch url := urbOfHost(host, req.URL.Path); {
	case url != "":
		g.serve(w, req, url)
	case host == "urb.localhost" || host == "localhost" || host == "127.0.0.1":
		if req.URL.Path == "/go" {
			g.redirect(w, req)
			return
		}
		g.home(w)
	default:
		//  any other name pointing here is DNS rebinding: refuse it
		http.Error(w, "unknown host", http.StatusMisdirectedRequest)
	}
}

func (g *gateway) serve(w http.ResponseWriter, req *http.Request, url string) {
	t := time.Now()
	key, body, how, err := g.r.Fetch(url)
	if err != nil {
		g.error(w, url, err)
		logf("%s: %v (%.1fs)", url, err, time.Since(t).Seconds())
		return
	}
	isHTML := looksHTML(body)
	logf("%s -> %s by %s, %d bytes (%.1fs)", url, key, how, len(body), time.Since(t).Seconds())
	h := w.Header()
	h.Set("X-Urb-Url", url)
	h.Set("X-Content-Type-Options", "nosniff")
	h.Set("Cache-Control", "no-cache")
	switch {
	case req.URL.Query().Has("raw"):
		h.Set("Content-Type", "text/plain; charset=utf-8")
	case isHTML:
		h.Set("Content-Type", "text/html; charset=utf-8")
		h.Set("Content-Security-Policy", sandbox)
		body = rewriteHTML(body, g.port)
	default:
		h.Set("Content-Type", "text/html; charset=utf-8")
		h.Set("Content-Security-Policy", "default-src 'none'; style-src 'unsafe-inline'")
		body = renderGemtext(body, url, func(target string) string {
			if u, err := gatewayURL(target, g.port); err == nil {
				return u
			}
			return target
		})
	}
	fmt.Fprint(w, body)
}

// redirect sends /go?url=urb://... to where the gateway serves it.
func (g *gateway) redirect(w http.ResponseWriter, req *http.Request) {
	u, err := gatewayURL(strings.TrimSpace(req.URL.Query().Get("url")), g.port)
	if err != nil {
		g.error(w, req.URL.Query().Get("url"), err)
		return
	}
	http.Redirect(w, req, u, http.StatusFound)
}

func (g *gateway) home(w http.ResponseWriter) {
	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	fmt.Fprint(w, page("urb-fetch", `<h1>urb-fetch</h1>
<p>Read a page published with lattice on the Urbit network, as `+html.EscapeString(g.r.node.Ship().String())+`.</p>
<form action="/go"><input name="url" placeholder="urb://~sampel-palnet/page" autofocus></form>`))
}

func (g *gateway) error(w http.ResponseWriter, url string, err error) {
	status := http.StatusBadGateway
	var he *httpError
	if errors.As(err, &he) {
		status = he.status
	}
	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	w.WriteHeader(status)
	fmt.Fprint(w, page(url, "<h1>"+strconv.Itoa(status)+"</h1><p>"+html.EscapeString(err.Error())+"</p>"))
}
