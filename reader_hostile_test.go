package main

import (
	"context"
	"testing"
	"time"

	mesa "github.com/mopfel-winrux/libmesa/bindings/go"
)

// hostile is a publisher that answers every grubbery peek with whatever
// answer makes of the asker's wire.
type hostile struct {
	mesa.BaseApp
	answer func(wire mesa.Noun) mesa.Value
}

func (h *hostile) Poke(n *mesa.Node, her mesa.Ship, bone uint64, agent, mark string, noun mesa.Noun) error {
	if mark != "grubbery-load" {
		return nil
	}
	b, err := n.OpenFlow(her)
	if err != nil {
		return err
	}
	return n.Poke(her, b, "grubbery", "grubbery-transfer", h.answer(noun.Head().Head()))
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
