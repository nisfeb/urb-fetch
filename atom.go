package main

// atomSize counts an atom's bytes in place: Noun.Bytes copies them out,
// up to 64 MiB for a refused noun, and the binding has no Size of its
// own. libmesa.a, which the binding links, has mesa_met3.

/*
#include <stddef.h>
#include <stdint.h>
size_t mesa_met3(uint64_t atom);
*/
import "C"

import (
	"unsafe"

	mesa "github.com/mopfel-winrux/libmesa/bindings/go"
)

// A Noun is one mesa_noun handle (a uint64_t), so it is read as one.
// These fail to compile if the binding's Noun ever differs in size.
var (
	_ [unsafe.Sizeof(mesa.Noun{}) - 8]byte
	_ [8 - unsafe.Sizeof(mesa.Noun{})]byte
)

// atomSize is the number of significant bytes in the atom n.
func atomSize(n mesa.Noun) int {
	return int(C.mesa_met3(C.uint64_t(*(*uint64)(unsafe.Pointer(&n)))))
}
