package main

// The reader: lattice's published pages over libmesa, read the way
// lattice's own reader reads them (nex/lattice/app.hoon: +peer-base,
// +peer-revs-cached, +read-page-body-rev, +read-page-or-index).
//
// A page's body is bound in the publisher's scry namespace at
//   /g/x/1/grubbery//1/pub/page/<path>/<id>
// where the id is salted, so it is learned from the publisher's revision
// list, the grubbery file <base>/pub/revs. That file is read with
// grubbery's own cross-ship peek: a %grubbery-load poke to the
// publisher's %grubbery, answered by a %grubbery-transfer poke back.

import (
	"crypto/rand"
	"encoding/hex"
	"fmt"
	"strings"
	"sync"
	"time"

	mesa "github.com/mopfel-winrux/libmesa/bindings/go"
)

const (
	// where lattice lives on a ship: +desk-base, then +app-base
	deskBase = "/apps/shell.shell/desks/lattice.desk/desk/data/lattice.lattice_app"
	appBase  = "/apps/lattice.lattice_app"

	revsTTL     = 60 * time.Second // +revs-ttl
	keenTimeout = 10 * time.Second // +mesa-timeout
	peekTimeout = 30 * time.Second // +remote-timeout
)

// revList is a publisher's /pub/revs, page -> id, and when it was read.
type revList struct {
	at   time.Time
	revs map[string]string
}

// a peek waiting for its %grubbery-transfer
type peek struct {
	seq  int
	bone uint64
	conv func(mesa.Noun) any // runs in the hook: nouns are borrowed
	jobe string              // the snap's content hash, while a %want is out
	done chan any            // the converted file, or nil
}

type keenResult struct {
	mark, body string
	ok         bool
}

// Reader is the libmesa App. Its peek, keen and flow maps belong to the
// node's goroutine (hooks and Node.Do); request goroutines wait on
// channels. bases and revs are shared with them under mu.
type Reader struct {
	mesa.BaseApp
	node  *mesa.Node
	nonce string
	seq   int
	flows map[mesa.Ship]uint64
	peeks map[string]*peek
	keens map[string][]chan keenResult

	mu    sync.Mutex
	bases map[mesa.Ship]string
	revs  map[mesa.Ship]revList
}

func newReader() *Reader {
	b := make([]byte, 4)
	rand.Read(b)
	return &Reader{
		//  wires never repeat across runs: a publisher can deliver a reply
		//  meant for an earlier process late, and it must not match ours
		nonce: hex.EncodeToString(b),
		flows: map[mesa.Ship]uint64{},
		peeks: map[string]*peek{},
		keens: map[string][]chan keenResult{},
		bases: map[mesa.Ship]string{},
		revs:  map[mesa.Ship]revList{},
	}
}

// ------------------------------------------------------------ the hooks

// Poke takes the publishers' %grubbery-transfer answers to our peeks.
func (r *Reader) Poke(n *mesa.Node, her mesa.Ship, bone uint64, agent, mark string, noun mesa.Noun) error {
	if agent != "grubbery" {
		return mesa.Flub
	}
	if mark != "grubbery-transfer" {
		return nil
	}
	wire, tag, rest := noun.Head().Path(), noun.Tail().Head().Text(), noun.Tail().Tail()
	if wire == "/want" {
		//  +process-want answers on the fixed wire /want, not ours, so
		//  match the content to the snap that asked for it, as grubbery's
		//  own reader does. A %miss means the 5-minute pin expired.
		var oldest *peek
		var oldW string
		for w, p := range r.peeks {
			if p.jobe == "" {
				continue
			}
			if tag == "data" {
				if v := fileOf(p.jobe, rest, p.conv); v != nil {
					r.finish(w, v)
				}
			} else if oldest == nil || p.seq < oldest.seq {
				oldest, oldW = p, w
			}
		}
		if oldest != nil {
			r.finish(oldW, nil)
		}
		return nil
	}
	p := r.peeks[wire]
	if p == nil {
		return nil // a late answer to an earlier run, or to a timed-out peek
	}
	if tag != "snap" { // %veto, %miss
		r.finish(wire, nil)
		return nil
	}
	//  [dest snap-id snap=(unit [cass pace refs]) data=(unit silo)]
	dest, snapID := rest.Head(), rest.Tail().Head()
	snapU, dataU := rest.Tail().Tail().Head(), rest.Tail().Tail().Tail()
	jobe := ""
	if snapU.IsCell() {
		jobe = paceJobe(snapU.Tail())
	}
	switch {
	case jobe == "":
		r.finish(wire, nil) // nothing there
	case !dataU.IsCell():
		//  too big to inline: ask for the content, pinned for 5 minutes
		p.jobe = jobe
		if err := n.Poke(her, p.bone, "grubbery", "grubbery-transfer",
			mesa.Tuple(noun.Head(), "want", dest, snapID)); err != nil {
			r.finish(wire, nil)
		}
	default:
		r.finish(wire, fileOf(jobe, dataU.Tail(), p.conv))
	}
	return nil
}

// Done: a nacked peek fails every peek waiting on that flow.
func (r *Reader) Done(n *mesa.Node, her mesa.Ship, bone uint64, err *mesa.Nack) {
	if err == nil {
		return
	}
	logf("nack from %s: %v", her, err)
	for w, p := range r.peeks {
		if p.bone == bone {
			r.finish(w, nil)
		}
	}
}

// Scry takes the answer to a keen.
func (r *Reader) Scry(n *mesa.Node, her mesa.Ship, path, mark string, noun mesa.Noun, ok bool) {
	res := keenResult{mark: mark, ok: ok && noun.IsAtom()}
	if res.ok {
		res.body = noun.Text()
	}
	for _, ch := range r.keens[path] {
		ch <- res
	}
	delete(r.keens, path)
}

func (r *Reader) finish(wire string, v any) {
	if p := r.peeks[wire]; p != nil {
		delete(r.peeks, wire)
		p.done <- v
	}
}

// ------------------------------------------------------------ nouns

// entries are the nodes of a hoon map or set, a treap of [n l r].
func entries(n mesa.Noun) []mesa.Noun {
	var out []mesa.Noun
	for stack := []mesa.Noun{n}; len(stack) > 0; {
		t := stack[len(stack)-1]
		stack = stack[:len(stack)-1]
		if t.IsCell() {
			out = append(out, t.Head())
			stack = append(stack, t.Tail().Head(), t.Tail().Tail())
		}
	}
	return out
}

// paceJobe is the content hash a snap's pace names, or "".
func paceJobe(snap mesa.Noun) string {
	pace := snap.Tail().Head()
	if t := pace.Head().Text(); (t == "firm" || t == "temp") && pace.Tail().IsCell() {
		return string(pace.Tail().Tail().Bytes())
	}
	return ""
}

// fileOf is the file a pace's jobe names, out of a silo [nouns jects],
// converted, or nil.
func fileOf(jobe string, silo mesa.Noun, conv func(mesa.Noun) any) any {
	for _, e := range entries(silo.Tail()) {
		if string(e.Head().Bytes()) != jobe {
			continue
		}
		ject := e.Tail().Tail()
		if ject.Head().Text() != "leaf" {
			return nil // a directory
		}
		lobe := string(ject.Tail().Head().Bytes())
		for _, m := range entries(silo.Head()) {
			if string(m.Head().Bytes()) == lobe {
				return conv(m.Tail().Tail())
			}
		}
	}
	return nil
}

// jsonOf reads a $json noun.
func jsonOf(n mesa.Noun) any {
	if !n.IsCell() {
		return nil
	}
	p := n.Tail()
	switch n.Head().Text() {
	case "o":
		m := map[string]any{}
		for _, kv := range entries(p) {
			m[kv.Head().Text()] = jsonOf(kv.Tail())
		}
		return m
	case "a":
		var a []any
		for _, it := range p.List() {
			a = append(a, jsonOf(it))
		}
		return a
	case "b":
		v, _ := p.Uint64()
		return v == 0
	}
	return p.Text() // %n keeps its text, %s is text
}

// revsOf reads /pub/revs, {"/page": {"rev": 1, "id": "0v..."}}, as
// page -> id.
func revsOf(n mesa.Noun) any {
	m, ok := jsonOf(n).(map[string]any)
	if !ok {
		return nil
	}
	out := map[string]string{}
	for k, v := range m {
		row, _ := v.(map[string]any)
		if id, _ := row["id"].(string); id != "" {
			out[k] = id
		}
	}
	return out
}

func textOf(n mesa.Noun) any {
	if n.IsCell() {
		return nil
	}
	return n.Text()
}

// ------------------------------------------------------------ requests

// peek reads one grubbery file, [folder]/[name], from her.
func (r *Reader) peek(her mesa.Ship, folder, name string, conv func(mesa.Noun) any) any {
	done := make(chan any, 1)
	var wire string
	err := r.node.Do(func() error {
		r.seq++
		wire = fmt.Sprintf("/urb-fetch/%s/%d", r.nonce, r.seq)
		bone, ok := r.flows[her]
		if !ok {
			b, err := r.node.OpenFlow(her)
			if err != nil {
				return err
			}
			bone = b
		}
		//  [[wire dest=[%& rail]] %peek case=~ deep=%.y]
		load := mesa.Tuple(mesa.Cell{H: mesa.Path(wire), T: mesa.Tuple(true, mesa.Path(folder), name)},
			"peek", nil, true)
		if err := r.node.Poke(her, bone, "grubbery", "grubbery-load", load); err != nil {
			return err // the transaction undoes a flow it opened
		}
		r.flows[her] = bone
		r.peeks[wire] = &peek{seq: r.seq, bone: bone, conv: conv, done: done}
		return nil
	})
	if err != nil {
		logf("peek %s %s/%s: %v", her, folder, name, err)
		return nil
	}
	select {
	case v := <-done:
		return v
	case <-time.After(peekTimeout):
		r.node.Do(func() error { delete(r.peeks, wire); return nil })
		return nil
	}
}

// keen reads one binding of her scry namespace: (mark, body, ok).
func (r *Reader) keen(her mesa.Ship, path string) keenResult {
	ch := make(chan keenResult, 1)
	err := r.node.Do(func() error {
		r.keens[path] = append(r.keens[path], ch)
		if len(r.keens[path]) > 1 {
			return nil // already asked
		}
		return r.node.Scry(her, path)
	})
	if err != nil {
		return keenResult{}
	}
	select {
	case res := <-ch:
		return res
	case <-time.After(keenTimeout):
		//  a miss never answers: stop asking, as +keen-page-raw %yawns
		r.node.Do(func() error {
			left := r.keens[path][:0]
			for _, c := range r.keens[path] {
				if c != ch {
					left = append(left, c)
				}
			}
			if len(left) > 0 {
				r.keens[path] = left
				return nil
			}
			delete(r.keens, path)
			return r.node.Unscry(her, path)
		})
		return keenResult{}
	}
}

// base is where her runs lattice, asked rather than guessed (+peer-base).
// The question is a read of the revision list, which it keeps.
func (r *Reader) base(her mesa.Ship) (string, error) {
	r.mu.Lock()
	b, ok := r.bases[her]
	r.mu.Unlock()
	if ok {
		return b, nil
	}
	for _, b := range []string{deskBase, appBase} {
		if v, ok := r.peek(her, b+"/pub", "revs", revsOf).(map[string]string); ok {
			r.mu.Lock()
			r.bases[her], r.revs[her] = b, revList{time.Now(), v}
			r.mu.Unlock()
			return b, nil
		}
	}
	return "", fail(502, "%s did not answer a lattice read: offline, not running lattice release 35 or later, or not sharing anything", her)
}

// revList is her revision list, cached for revsTTL (+peer-revs-cached).
func (r *Reader) revList(her mesa.Ship) (map[string]string, error) {
	b, err := r.base(her)
	if err != nil {
		return nil, err
	}
	r.mu.Lock()
	l, ok := r.revs[her]
	r.mu.Unlock()
	if ok && time.Since(l.at) < revsTTL {
		return l.revs, nil
	}
	v, _ := r.peek(her, b+"/pub", "revs", revsOf).(map[string]string)
	if v == nil {
		v = map[string]string{}
	}
	r.mu.Lock()
	r.revs[her] = revList{time.Now(), v}
	r.mu.Unlock()
	return v, nil
}

// page is one published page (+read-page-body-rev): a keen at the id the
// list names, else a peek of the vault file. "" when it isn't there.
func (r *Reader) page(her mesa.Ship, key string, revs map[string]string) (body, how string) {
	if id, ok := revs[key]; ok {
		res := r.keen(her, "/g/x/1/grubbery//1/pub/page"+key+"/"+id)
		if res.ok && res.mark == "gmi" {
			return res.body, "keen"
		}
		r.mu.Lock()
		delete(r.revs, her) // a miss means the list is stale
		r.mu.Unlock()
	}
	b, err := r.base(her)
	if err != nil {
		return "", ""
	}
	if s, ok := r.peek(her, b+"/pub/vault"+key, "gmi", textOf).(string); ok {
		return s, "peek"
	}
	return "", ""
}

// Fetch reads the page an urb:// url names (+read-page-or-index): the
// page itself, or the index under it.
func (r *Reader) Fetch(url string) (key, body, how string, err error) {
	her, rel, err := deUrb(url)
	if err != nil {
		return "", "", "", err
	}
	revs, err := r.revList(her)
	if err != nil {
		return "", "", "", err
	}
	keyOf := func(p []string) string { return "/" + strings.Join(p, "/") }
	hit := func(p []string) bool { _, ok := revs[keyOf(p)]; return ok }
	dex := append(append([]string{}, rel...), "index")
	var tries [][]string
	switch {
	case len(rel) == 0 || rel[len(rel)-1] == "index":
		tries = [][]string{rel}
	case !hit(rel) && hit(dex):
		tries = [][]string{dex}
	case len(revs) > 0 && !hit(rel):
		tries = [][]string{rel}
	default:
		tries = [][]string{rel, dex}
	}
	for _, p := range tries {
		if body, how := r.page(her, keyOf(p), revs); how != "" {
			return keyOf(p), body, how, nil
		}
	}
	return "", "", "", fail(404, "%s is not published", url)
}
