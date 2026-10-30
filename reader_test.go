package main

import (
	"testing"
	"time"
)

// An answer that comes after its caller gave up still fills the cache;
// one that comes in time goes to the caller and nowhere else.
func TestLateAnswer(t *testing.T) {
	r := newReader()
	var late any
	keep := func(v any) { late = v }

	r.peeks["/gone"] = &peek{done: make(chan any, 1), late: keep, gone: time.Now()}
	r.finish("/gone", map[string]string{"/a": "0v1"})
	if m, _ := late.(map[string]string); m["/a"] != "0v1" {
		t.Errorf("late answer lost: %v", late)
	}
	if _, ok := r.peeks["/gone"]; ok {
		t.Error("an answered peek is still waiting")
	}

	late = nil
	done := make(chan any, 1)
	r.peeks["/here"] = &peek{done: done, late: keep}
	r.finish("/here", "body")
	if v := <-done; v != "body" || late != nil {
		t.Errorf("on-time answer: caller got %v, late got %v", v, late)
	}
}

// A keen hit is any lattice view-mode mark (release 74 and later), or
// %gmi (every page before). A %del tombstone, a miss, or a cell is not.
func TestKeenIsPage(t *testing.T) {
	for _, m := range []string{"gmi", "md", "html", "text", "js", "css", "noun"} {
		if !(keenResult{mark: m, body: "x", ok: true}).isPage() {
			t.Errorf("%%%s keen rejected", m)
		}
	}
	for _, k := range []keenResult{{mark: "del", ok: true}, {mark: "md"}, {}, {mark: "json", ok: true}} {
		if k.isPage() {
			t.Errorf("%+v accepted", k)
		}
	}
}
