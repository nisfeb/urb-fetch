package main

import (
	"bufio"
	"io"
	"net"
	"net/http"
	"strings"
	"testing"
	"time"
)

// serve runs newServer with a handler that answers "ok" on a loopback
// port, and returns its address.
func serve(t *testing.T) string {
	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	srv := newServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/slow" {
			time.Sleep(12 * time.Second) // a read nobody answers soon
		}
		io.WriteString(w, "ok")
	}))
	go srv.Serve(ln)
	t.Cleanup(func() { srv.Close() })
	return ln.Addr().String()
}

// closedBy reads c until it closes, and fails unless that is by limit.
// It returns what came.
func closedBy(t *testing.T, c net.Conn, limit time.Duration) string {
	t.Helper()
	c.SetReadDeadline(time.Now().Add(limit))
	b, err := io.ReadAll(c)
	if err != nil {
		t.Fatalf("still open after %s: %v (got %q)", limit, err, b)
	}
	return string(b)
}

// No connection is held for a client that sends nothing, or nothing
// more, or sends slowly (the gateway's timeouts).
func TestServerTimeouts(t *testing.T) {
	const req = "GET / HTTP/1.1\r\nHost: localhost\r\n"
	cases := []struct {
		name, send, want string
		trickle          bool
	}{
		// a new connection that sends nothing
		{name: "idle", send: ""},
		// a head sent a byte at a time
		{name: "trickled head", send: req + "X-Pad: " + strings.Repeat("a", 200) + "\r\n\r\n", trickle: true},
		// a kept-alive connection after its answer
		{name: "kept alive", send: req + "\r\n", want: "200 OK"},
		// a body announced and never sent: answered once the read gives up
		{name: "unsent body", send: req + "Content-Length: 10\r\n\r\n", want: "200 OK"},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			t.Parallel()
			conn, err := net.Dial("tcp", serve(t))
			if err != nil {
				t.Fatal(err)
			}
			defer conn.Close()
			if c.trickle {
				go func() {
					for i := range len(c.send) {
						if _, err := conn.Write([]byte{c.send[i]}); err != nil {
							return
						}
						time.Sleep(150 * time.Millisecond)
					}
				}()
			} else {
				io.WriteString(conn, c.send)
			}
			got := closedBy(t, conn, 25*time.Second)
			if !strings.Contains(got, c.want) {
				t.Errorf("got %q, want %q in it", got, c.want)
			}
		})
	}
}

// Requests on one connection each answer, a slow one too: the read
// timeouts don't bound the handler.
func TestServerKeepsAlive(t *testing.T) {
	t.Parallel()
	conn, err := net.Dial("tcp", serve(t))
	if err != nil {
		t.Fatal(err)
	}
	defer conn.Close()
	r := bufio.NewReader(conn)
	for _, path := range []string{"/", "/slow", "/"} {
		io.WriteString(conn, "GET "+path+" HTTP/1.1\r\nHost: localhost\r\n\r\n")
		conn.SetReadDeadline(time.Now().Add(15 * time.Second))
		resp, err := http.ReadResponse(r, nil)
		if err != nil {
			t.Fatal(err)
		}
		b, _ := io.ReadAll(resp.Body)
		if string(b) != "ok" {
			t.Fatalf("got %q", b)
		}
	}
}
