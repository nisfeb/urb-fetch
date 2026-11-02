// cometkey writes a comet's keyfile to the path it is given, mined
// offline under ~marzod and written as mesa.LoadComet writes one
// (created, never overwritten, mode 0600): gateway-diff.mjs's comet
// case, which must not fetch the live comet-stars list.
package main

import (
	"fmt"
	"os"

	mesa "github.com/mopfel-winrux/libmesa/bindings/go"
)

func main() {
	if len(os.Args) != 2 {
		fmt.Fprintln(os.Stderr, "usage: cometkey PATH")
		os.Exit(2)
	}
	id, err := mesa.MineComet([]mesa.Ship{mesa.MustShip("~marzod")})
	if err == nil {
		var f *os.File
		if f, err = os.OpenFile(os.Args[1], os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o600); err == nil {
			_, err = f.WriteString(id.Keyfile() + "\n")
			if cerr := f.Close(); err == nil {
				err = cerr
			}
		}
	}
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	fmt.Println(id.Ship())
}
