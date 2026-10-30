package main

import (
	"context"
	"os"
	"testing"

	mesa "github.com/mopfel-winrux/libmesa/bindings/go"
)

// TestLive checks the protocol code against a real publisher, and is
// skipped unless one is named:
//
//	URB_FETCH_LIVE=~sampel-palnet URB_FETCH_STATE=<dir> go test -run Live -v
//
// It reads every page on the list twice, by keen at its id and by a peek
// of its vault file, and wants the same text. A page over 8 KB can't ride
// inline in a %snap, so its peek goes through %want and %data. Keep
// URB_FETCH_STATE between runs, or each run mines (and introduces) a new
// comet, and don't share it with a running gateway. URB_FETCH_FAKE=~ship
// reads as that ship on a fake network (urbit -F) instead.
func TestLive(t *testing.T) {
	who := os.Getenv("URB_FETCH_LIVE")
	if who == "" {
		t.Skip("set URB_FETCH_LIVE=~ship to read from a real publisher")
	}
	her, err := mesa.ParseShip(who)
	if err != nil {
		t.Fatal(err)
	}
	state := os.Getenv("URB_FETCH_STATE")
	if state == "" {
		state = t.TempDir()
	}
	r := newReader()
	node, err := open(state, "", false, os.Getenv("URB_FETCH_FAKE"), r)
	if err != nil {
		t.Fatal(err)
	}
	r.node = node
	ctx, cancel := context.WithCancel(context.Background())
	defer func() { cancel(); node.Close() }()
	go node.Run(ctx)

	revs, err := r.revList(her)
	if err != nil || len(revs) == 0 {
		t.Fatalf("revision list: %d pages, %v", len(revs), err)
	}
	b, _ := r.base(her)
	big, marks := 0, map[string]int{}
	for key, id := range revs {
		k := r.keen(her, "/g/x/1/grubbery//1/pub/page"+key+"/"+id)
		got, _ := r.peek(her, b+"/pub/vault"+key, "gmi", textOf, nil)
		v, _ := got.(string)
		if !k.isPage() || v != k.body {
			t.Errorf("%s: keen %%%s ok=%v (%d bytes), peek %d bytes", key, k.mark, k.ok, len(k.body), len(v))
		}
		marks[k.mark]++
		if len(v) > 8192 {
			big++
		}
	}
	t.Logf("%d pages read both ways, %d of them over 8 KB (%%want and %%data), by mark %v", len(revs), big, marks)
}
