#!/bin/sh
# Builds the shims' modules into shim/build (docs/rust-core.md, M6):
#
#   mesa.wasm       the urb core (core/urb-wasm), for mesa.mjs
#   render-wasm.js  render.wasm (core/render-wasm) as base64, beside the
#                   shims (not committed), for
#                   render.js to compile synchronously
#
# and prints their sizes, raw and gzipped, against the budget: 1.5 times
# wasm/build.sh's mesa.wasm, 1.7 MB and 600 KB gzipped. Needs rustup's
# toolchain and WASI_SDK_PATH, as core/ci/test.sh does.
#
#   shim/build.sh            build
#   shim/build.sh <dir>      build, then stage a lib/ in dir: the shims
#                            and the modules, as an extension takes them
set -eu
here=$(cd "$(dirname "$0")" && pwd)
core=$here/../core
out=$here/build
PATH=${CARGO_HOME:-$HOME/.cargo}/bin:$PATH
export PATH
(cd "$core" && cargo build --profile wasm --target wasm32-wasip1 -p urb-wasm -p render-wasm)
t=$core/target/wasm32-wasip1/wasm
mkdir -p "$out"
cp "$t/urb_wasm.wasm" "$out/mesa.wasm"
cp "$t/render_wasm.wasm" "$out/render.wasm"
{
  echo '//  render.wasm (core/render-wasm), built by shim/build.sh: do not edit.'
  printf 'export default "'
  base64 -w0 "$out/render.wasm"
  echo '"'
} > "$here/render-wasm.js"

size() { echo "$(wc -c < "$1") $(gzip -9c "$1" | wc -c)"; }
set -- "${1:-}" $(size "$out/mesa.wasm") $(size "$out/render.wasm")
echo "mesa.wasm    $2 bytes, $3 gzipped"
echo "render.wasm  $4 bytes, $5 gzipped"
echo "together     $(($2 + $4)) bytes, $(($3 + $5)) gzipped (budget 2547093, 897543)"
if [ $(($2 + $4)) -gt 2547093 ] || [ $(($3 + $5)) -gt 897543 ]; then
  echo "over the budget" >&2
  exit 1
fi

if [ -n "$1" ]; then
  mkdir -p "$1"
  cp "$here"/mesa.mjs "$here"/pub.mjs "$here"/furum.mjs "$here"/forum.mjs \
    "$here"/gemtext.js "$here"/markdown.js "$here"/render.js "$here"/wasi.js \
    "$out/mesa.wasm" "$here/render-wasm.js" "$1"/
  echo "staged in $1"
fi
