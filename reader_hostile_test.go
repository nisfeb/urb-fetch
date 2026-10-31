package main

import (
	"context"
	"strings"
	"testing"
	"time"

	mesa "github.com/mopfel-winrux/libmesa/bindings/go"
)

// hostile is a publisher that answers every grubbery peek with whatever
// answer makes of the asker's wire (or file, if file is set), and each
// %want with want's answer, if it is set.
type hostile struct {
	mesa.BaseApp
	answer func(wire mesa.Noun) mesa.Value
	file   func(wire mesa.Noun, folder, name string) mesa.Value
	want   func(transfer mesa.Noun) mesa.Value
}

func (h *hostile) Poke(n *mesa.Node, her mesa.Ship, bone uint64, agent, mark string, noun mesa.Noun) error {
	var v mesa.Value
	switch {
	case mark == "grubbery-load" && h.file != nil:
		dest := noun.Head().Tail().Tail()
		v = h.file(noun.Head().Head(), dest.Head().Path(), dest.Tail().Text())
	case mark == "grubbery-load":
		v = h.answer(noun.Head().Head())
	case mark == "grubbery-transfer" && h.want != nil && noun.Tail().Head().Text() == "want":
		v = h.want(noun)
	}
	if v == nil {
		return nil
	}
	b, err := n.OpenFlow(her)
	if err != nil {
		return err
	}
	return n.Poke(her, b, "grubbery", "grubbery-transfer", v)
}

func startNode(t *testing.T, g uint64, app mesa.App) *mesa.Node {
	t.Helper()
	n, err := mesa.Open(mesa.Config{Identity: mesa.FakeIdentity(mesa.Ship{Lo: g}), Store: mesa.NewMemoryStore(), App: app})
	if err != nil {
		t.Fatalf("open: %v", err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	go n.Run(ctx)
	t.Cleanup(func() { cancel(); n.Close() })
	return n
}

// snapOf is a %snap answer holding one file, value, inline.
func snapOf(wire mesa.Noun, value mesa.Value) mesa.Value {
	jobe, lobe := uint64(7), uint64(9)
	pace := mesa.Tuple("firm", nil, jobe)
	nouns := mesa.Tuple(mesa.Tuple(lobe, nil, value), nil, nil)
	jects := mesa.Tuple(mesa.Tuple(jobe, nil, "leaf", lobe, nil), nil, nil)
	return mesa.Tuple(wire, "snap", nil, uint64(5), mesa.Some(mesa.Tuple(nil, pace, nil)), mesa.Some(mesa.Tuple(nouns, jects)))
}

// A hostile publisher's answers read as nothing, at once: shared
// structure past maxCells, $json nested past maxDepth, and wrong shapes.
// None panics, and none waits out the peek's timeout.
func TestHostileAnswers(t *testing.T) {
	h := &hostile{}
	r := newReader()
	r.node = startNode(t, 220, r)
	pub := startNode(t, 221, h)

	num := mesa.Tuple("n", "1")
	deep := func(n int) mesa.Value {
		v := num
		for range n {
			v = mesa.Tuple("a", mesa.List(v))
		}
		return v
	}
	// 2^n copies of v as a tree, which jam sends as n+1 cells
	shared := func(v mesa.Value, n int) mesa.Value {
		for range n {
			v = mesa.Cell{H: v, T: v}
		}
		return v
	}
	read := func(answer func(mesa.Noun) mesa.Value) (any, bool, time.Duration) {
		h.answer = answer
		t0 := time.Now()
		v, ok := r.peek(pub.Ship(), "/x", "y", jsonOf, nil)
		return v, ok, time.Since(t0)
	}

	if v, ok, _ := read(func(w mesa.Noun) mesa.Value { return snapOf(w, deep(63)) }); !ok || v == nil {
		t.Fatalf("a $json 64 deep: %v %v", v, ok)
	}
	for name, answer := range map[string]func(mesa.Noun) mesa.Value{
		"shared structure": func(w mesa.Noun) mesa.Value { return snapOf(w, shared(num, 18)) },
		"a shared snap":    func(w mesa.Noun) mesa.Value { return mesa.Tuple(w, "snap", nil, shared(uint64(1), 18), nil, nil) },
		"deep $json":       func(w mesa.Noun) mesa.Value { return snapOf(w, deep(64)) },
		"deeper $json":     func(w mesa.Noun) mesa.Value { return snapOf(w, deep(5000)) },
		"an atom for a snap": func(w mesa.Noun) mesa.Value {
			return mesa.Tuple(w, "snap", uint64(5))
		},
		"a cell for a tag": func(w mesa.Noun) mesa.Value { return mesa.Tuple(w, mesa.Tuple("snap", 1), 1) },
		"a cell for the pace": func(w mesa.Noun) mesa.Value {
			return mesa.Tuple(w, "snap", nil, uint64(5), mesa.Some(mesa.Tuple(nil, mesa.Tuple(mesa.Tuple("firm", 1), nil, 7), nil)), nil)
		},
		"a silo of atoms": func(w mesa.Noun) mesa.Value {
			return mesa.Tuple(w, "snap", nil, uint64(5), mesa.Some(mesa.Tuple(nil, mesa.Tuple("firm", nil, 7), nil)), mesa.Some(mesa.Tuple(1, 2)))
		},
		"a veto": func(w mesa.Noun) mesa.Value { return mesa.Tuple(w, "veto", nil) },
	} {
		v, ok, took := read(answer)
		if v != nil || !ok || took > 5*time.Second {
			t.Errorf("%s: read %v, answered %v, in %s", name, v, ok, took)
		}
	}
	//  and it still reads a good answer
	if v, ok, _ := read(func(w mesa.Noun) mesa.Value { return snapOf(w, mesa.Tuple("s", "still")) }); v != "still" || !ok {
		t.Errorf("after: %v %v", v, ok)
	}
}

// A revision list over maxCells is an empty one, as lattice reads it:
// the page still reads, by the vault peek, and the publisher is not one
// that "shares no lattice pages".
func TestRevsOverTheCap(t *testing.T) {
	h := &hostile{}
	r := newReader()
	r.node = startNode(t, 223, r)
	pub := startNode(t, 224, h)
	items := make([]mesa.Value, maxCells/2)
	for i := range items {
		items[i] = mesa.Tuple("n", "1")
	}
	huge := mesa.Tuple("a", mesa.List(items...))
	h.file = func(w mesa.Noun, folder, name string) mesa.Value {
		switch {
		case folder == deskBase+"/pub" && name == "revs":
			return snapOf(w, huge)
		case folder == deskBase+"/pub/vault/page" && name == "gmi":
			return snapOf(w, "# by the vault")
		}
		return mesa.Tuple(w, "snap", nil, uint64(5), nil, nil)
	}
	pg, err := r.Fetch("urb://" + pub.Ship().String() + "/page")
	if err != nil || pg.Body != "# by the vault" || pg.How != "peek" {
		t.Fatalf("read %+v, %v", pg, err)
	}
	r.mu.Lock()
	defer r.mu.Unlock()
	if r.bases[pub.Ship()] != deskBase || len(r.revs[pub.Ship()].revs) != 0 {
		t.Errorf("base %q, revs %v", r.bases[pub.Ship()], r.revs[pub.Ship()].revs)
	}
}

// A /want answer is matched only to %wants out to the ship that sent
// it: another ship's %miss leaves ours waiting.
func TestWantByShip(t *testing.T) {
	asked, send := make(chan bool, 1), make(chan bool)
	h := &hostile{}
	r := newReader()
	r.node = startNode(t, 225, r)
	pub := startNode(t, 226, h)
	other := startNode(t, 227, &hostile{})
	body := strings.Repeat("z", 50_000)
	h.answer = func(w mesa.Noun) mesa.Value {
		//  too big to inline: no data, so the reader %wants it
		return mesa.Tuple(w, "snap", nil, uint64(5), mesa.Some(mesa.Tuple(nil, mesa.Tuple("firm", nil, uint64(7)), nil)), nil)
	}
	h.want = func(mesa.Noun) mesa.Value {
		asked <- true
		select { // never hold the publisher's loop for ever
		case <-send:
		case <-time.After(5 * time.Second):
		}
		silo := mesa.Tuple(mesa.Tuple(mesa.Tuple(uint64(9), nil, body), nil, nil),
			mesa.Tuple(mesa.Tuple(uint64(7), nil, "leaf", uint64(9), nil), nil, nil))
		return mesa.Tuple(mesa.Path("/want"), "data", silo)
	}
	got := make(chan any, 1)
	go func() {
		v, _ := r.peek(pub.Ship(), "/x", "y", textOf, nil)
		got <- v
	}()
	select {
	case <-asked:
	case <-time.After(10 * time.Second):
		t.Fatal("no %want")
	}
	if err := other.Do(func() error {
		b, err := other.OpenFlow(r.node.Ship())
		if err != nil {
			return err
		}
		return other.Poke(r.node.Ship(), b, "grubbery", "grubbery-transfer", mesa.Tuple(mesa.Path("/want"), "miss", nil))
	}); err != nil {
		t.Fatal(err)
	}
	select {
	case v := <-got:
		t.Fatalf("another ship's %%miss settled our peek: %v", v)
	case <-time.After(time.Second):
	}
	close(send)
	select {
	case v := <-got:
		if s, _ := v.(string); len(s) != 50_000 {
			t.Errorf("read %.40v", v)
		}
	case <-time.After(10 * time.Second):
		t.Fatal("the data never landed")
	}
}
