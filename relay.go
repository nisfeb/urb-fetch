package main

// The relay carries datagrams for libmesa running in a browser
// (wasm/mesa.mjs), which can't send UDP. The browser sends each datagram
// over a WebSocket, and the relay sends it on from a UDP port it keeps for
// that ship. Datagrams to that port go back down the WebSocket.
//
// The first message names the ship. Then each frame from the browser is
// [kind:1][galaxy:1][ip:4][port:2][datagram], where kind 1 sends to that
// galaxy (looked up here) and kind 2 to ip:port. Each frame to the
// browser is [ip:4][port:2][datagram]. All big-endian.
//
// A ship's UDP port stays open while the relay runs and is kept across
// runs, since publishers answer the address they last saw. While no
// browser is attached, its datagrams are dropped, and senders resend.
//
// ponytail: anyone who can reach the relay may claim any ship and send
// datagrams anywhere. That is fine on loopback. Before hosting it for
// others, it needs proof that a client holds the ship's key, and rate
// limits.

import (
	"context"
	"encoding/binary"
	"encoding/json"
	"fmt"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"sync"
	"time"

	"github.com/coder/websocket"
	mesa "github.com/mopfel-winrux/libmesa/bindings/go"
)

const (
	relayFrameMax = 4096 // a mesa packet is at most 1472 bytes
	galaxyTTL     = 5 * time.Minute
)

type relay struct {
	origins []string
	file    string // ship -> UDP port, as JSON

	mu       sync.Mutex
	ports    map[string]int
	ships    map[string]*relayShip
	galaxies map[byte]galaxyAddr
}

type relayShip struct {
	name string
	conn *net.UDPConn

	mu sync.Mutex
	ws *websocket.Conn // the browser attached now, if any
}

type galaxyAddr struct {
	addr *net.UDPAddr
	at   time.Time
}

func newRelay(state string, origins []string) (*relay, error) {
	rl := &relay{
		origins:  origins,
		file:     filepath.Join(state, "relay-ports.json"),
		ports:    map[string]int{},
		ships:    map[string]*relayShip{},
		galaxies: map[byte]galaxyAddr{},
	}
	if b, err := os.ReadFile(rl.file); err == nil {
		if err := json.Unmarshal(b, &rl.ports); err != nil {
			return nil, fmt.Errorf("%s: %w", rl.file, err)
		}
	}
	return rl, nil
}

func (rl *relay) ServeHTTP(w http.ResponseWriter, req *http.Request) {
	ws, err := websocket.Accept(w, req, &websocket.AcceptOptions{OriginPatterns: rl.origins})
	if err != nil {
		return // Accept has answered
	}
	ws.SetReadLimit(relayFrameMax)
	ctx := req.Context()

	typ, msg, err := ws.Read(ctx)
	if err != nil {
		return
	}
	who, err := mesa.ParseShip(string(msg))
	if typ != websocket.MessageText || err != nil {
		ws.Close(websocket.StatusPolicyViolation, "the first message names the ship")
		return
	}
	rs, err := rl.ship(who.String())
	if err != nil {
		logf("relay: %s: %v", who, err)
		ws.Close(websocket.StatusInternalError, "no UDP port")
		return
	}
	rs.attach(ws)
	defer rs.detach(ws)

	for {
		typ, msg, err := ws.Read(ctx)
		if err != nil {
			return
		}
		if typ != websocket.MessageBinary || len(msg) < 8 {
			continue
		}
		to := rl.dest(msg[0], msg[1], msg[2:6], binary.BigEndian.Uint16(msg[6:8]))
		if to != nil {
			rs.conn.WriteToUDP(msg[8:], to)
		}
	}
}

// dest is where a frame goes: a galaxy's address, or ip:port
func (rl *relay) dest(kind, galaxy byte, ip []byte, port uint16) *net.UDPAddr {
	switch kind {
	case 1:
		return rl.galaxy(galaxy)
	case 2:
		if port == 0 {
			return nil
		}
		return &net.UDPAddr{IP: net.IP(append([]byte(nil), ip...)), Port: int(port)}
	}
	return nil
}

// galaxy looks up <galaxy>.urbit.org, kept for five minutes
func (rl *relay) galaxy(g byte) *net.UDPAddr {
	rl.mu.Lock()
	hit, ok := rl.galaxies[g]
	rl.mu.Unlock()
	if ok && time.Since(hit.at) < galaxyTTL {
		return hit.addr
	}
	name := mesa.Ship{Lo: uint64(g)}.String()[1:] + ".urbit.org"
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	ips, err := net.DefaultResolver.LookupIP(ctx, "ip4", name)
	if err != nil || len(ips) == 0 {
		logf("relay: can't find galaxy %s: %v", name, err)
		return hit.addr // the last one known, if any
	}
	addr := &net.UDPAddr{IP: ips[0], Port: 13337 + int(g)}
	rl.mu.Lock()
	rl.galaxies[g] = galaxyAddr{addr, time.Now()}
	rl.mu.Unlock()
	return addr
}

// ship is the ship's UDP side, opened on its kept port on first use
func (rl *relay) ship(name string) (*relayShip, error) {
	rl.mu.Lock()
	defer rl.mu.Unlock()
	if rs := rl.ships[name]; rs != nil {
		return rs, nil
	}
	conn, err := net.ListenUDP("udp4", &net.UDPAddr{Port: rl.ports[name]})
	if err != nil && rl.ports[name] != 0 {
		logf("relay: %s's port %d: %v; taking another", name, rl.ports[name], err)
		conn, err = net.ListenUDP("udp4", &net.UDPAddr{})
	}
	if err != nil {
		return nil, err
	}
	if p := conn.LocalAddr().(*net.UDPAddr).Port; p != rl.ports[name] {
		rl.ports[name] = p
		b, _ := json.Marshal(rl.ports)
		if err := os.WriteFile(rl.file, b, 0o600); err != nil {
			logf("relay: %v", err)
		}
	}
	rs := &relayShip{name: name, conn: conn}
	rl.ships[name] = rs
	go rs.listen()
	logf("relay: %s on UDP port %d", name, rl.ports[name])
	return rs, nil
}

// listen passes each datagram to the browser attached, if any
func (rs *relayShip) listen() {
	buf := make([]byte, 6+65536)
	for {
		n, from, err := rs.conn.ReadFromUDP(buf[6:])
		if err != nil {
			logf("relay: %s: %v", rs.name, err)
			return
		}
		ip := from.IP.To4()
		if ip == nil {
			continue
		}
		copy(buf[0:4], ip)
		binary.BigEndian.PutUint16(buf[4:6], uint16(from.Port))
		rs.mu.Lock()
		ws := rs.ws
		rs.mu.Unlock()
		if ws == nil {
			continue
		}
		ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		ws.Write(ctx, websocket.MessageBinary, buf[:6+n])
		cancel()
	}
}

// attach makes ws the ship's browser. An identity runs in one place, so a
// browser already attached is let go.
func (rs *relayShip) attach(ws *websocket.Conn) {
	rs.mu.Lock()
	old := rs.ws
	rs.ws = ws
	rs.mu.Unlock()
	if old != nil {
		old.Close(websocket.StatusPolicyViolation, "the ship connected again")
	}
}

func (rs *relayShip) detach(ws *websocket.Conn) {
	rs.mu.Lock()
	if rs.ws == ws {
		rs.ws = nil
	}
	rs.mu.Unlock()
	ws.CloseNow()
}
