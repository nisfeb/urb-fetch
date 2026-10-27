module github.com/nisfeb/urb-fetch

go 1.26.0

require (
	github.com/mopfel-winrux/libmesa/bindings/go v0.0.0
	modernc.org/sqlite v1.61.0
)

require (
	github.com/dustin/go-humanize v1.1.0 // indirect
	github.com/google/uuid v1.6.0 // indirect
	github.com/mattn/go-isatty v0.0.24 // indirect
	github.com/ncruces/go-strftime v1.1.0 // indirect
	github.com/remyoudompheng/bigfft v0.0.0-20230129092748-24d4a6f8daec // indirect
	golang.org/x/sys v0.49.0 // indirect
	modernc.org/libc v1.79.0 // indirect
	modernc.org/mathutil v1.7.1 // indirect
	modernc.org/memory v1.12.1 // indirect
)

// libmesa's cgo flags point at its own zig-out, so the binding is used
// from the pinned submodule, built in place (see the Makefile).
replace github.com/mopfel-winrux/libmesa/bindings/go => ./third_party/libmesa/bindings/go
