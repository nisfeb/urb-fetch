# libmesa is built in its submodule first: the Go binding links its static
# archives from there. libmesa pins zig 0.15.2; point ZIG at that version.
ZIG ?= zig
LIBMESA := third_party/libmesa/zig-out/lib/libmesa.a

urb-fetch: $(LIBMESA) $(wildcard *.go) go.mod go.sum
	go build -o $@ .

# -Dno-host: the gateway brings its own host (the Go binding's), so
# libmesa's libuv, curl and OpenSSL are not needed
$(LIBMESA):
	git submodule update --init
	cd third_party/libmesa && $(ZIG) build -Doptimize=ReleaseFast -Dno-host=true

test: $(LIBMESA)
	go test ./...

clean:
	rm -f urb-fetch

.PHONY: test clean
