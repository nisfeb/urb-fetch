#!/bin/sh
# Builds build/mesa.wasm: libmesa for wasm32-wasi, with libmesa-wasm32.diff
# applied to a copy of the submodule, linked with glue.c as a WASI reactor.
# libmesa pins zig 0.15.2; point ZIG at that version.
set -eu
ZIG=${ZIG:-zig}
here=$(cd "$(dirname "$0")" && pwd)
out=$here/build

rm -rf "$out/libmesa"
mkdir -p "$out/libmesa"
git -C "$here/../third_party/libmesa" archive HEAD | tar -x -C "$out/libmesa"
patch -s -d "$out/libmesa" -p1 < "$here/libmesa-wasm32.diff"
(cd "$out/libmesa" && "$ZIG" build -Dtarget=wasm32-wasi -Dno-host=true -Doptimize=ReleaseFast)

lib=$out/libmesa/zig-out/lib
"$ZIG" cc -target wasm32-wasi -mexec-model=reactor -O2 \
  -I "$out/libmesa/include" "$here/glue.c" \
  "$lib/libmesa.a" "$lib/libmesa-urcrypt.a" "$lib/libmesa-sqlite.a" \
  -Wl,-z,stack-size=1048576 -o "$out/mesa.wasm"
echo "built $out/mesa.wasm"
