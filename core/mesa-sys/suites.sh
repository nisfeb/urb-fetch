#!/bin/sh
# Runs libmesa's own test suites (build.zig's twelve) against a build.
#
#   suites.sh cargo native|wasi [debug|release]
#     libmesa as mesa-sys builds it, each test compiled with the same
#     compiler and flags (build.rs's suite.env) and linked against
#     mesa-sys's archives
#   suites.sh zig native|wasi [debug|release]
#     libmesa as its own build.zig builds it, for comparison
#
# zig's debug builds libmesa with UBSan. "suites.sh cargo native ubsan"
# does the same with ours (MESA_SYS_UBSAN=1, in a target dir of its own).
#
# Each prints one line per test, "<name> pass|fail|no-build", and exits
# non-zero if the lines differ from suites.expected for that target.
#
# Under WASI the tests that seed rand() with getpid() get wasi-libc's
# emulated getpid (zig-wasi-tests.diff does the same for zig), and run
# with a 1 MiB stack, as wasm/build.sh links mesa.wasm. A wasi build
# needs WASI_SDK_PATH; a zig run needs ZIG (0.15.2); both need wasmtime.
#
# Native builds are for baseline x86-64 in both, and each run prints its
# C compiler: zig uses its own clang (20 in zig 0.15.2), and mesa-sys the
# clang on PATH. The comparison is pass, fail or no-build per test.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../.." && pwd)
upstream=$repo/third_party/libmesa
how=${1:?cargo or zig}
where=${2:?native or wasi}
mode=${3:-debug}

TESTS="pact-test:pact_test ship-test:ship_test lss-test:lss_test
bitset-test:bitset_test noun-test:noun_test crypto-test:crypto_test
ident-test:ident_test azimuth-test:azimuth_test engine-test:engine_test
store-test:store_test gall-test:gall_test aes-siv-test:aes_siv_test"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
results=$work/results

cargo_suites() {
  set -- -p mesa-sys --message-format=json-render-diagnostics
  [ "$mode" = release ] && set -- "$@" --release
  if [ "$mode" = ubsan ]; then
    export MESA_SYS_UBSAN=1 CARGO_TARGET_DIR="$repo/core/target/ubsan"
  fi
  [ "$where" = wasi ] && set -- "$@" --target wasm32-wasip1
  # rust-toolchain.toml's rustc, through rustup: a system cargo ahead of
  # rustup's on PATH would build with another (and may lack wasip1)
  want=$(sed -n 's/^channel = "\(.*\)"/\1/p' "$repo/core/rust-toolchain.toml")
  have=$(cd "$repo/core" && rustc --version | cut -d' ' -f2)
  if [ "$have" != "$want" ]; then
    echo "suites: rustc $have, not the pinned $want: put rustup's cargo first on PATH" >&2
    exit 1
  fi
  # the build's JSON goes to a file first, so a failed build fails here
  # (sh has no pipefail)
  if ! (cd "$repo/core" && cargo build "$@" > "$work/build.json"); then
    echo "suites: cargo build failed" >&2
    exit 1
  fi
  out=$(python3 -I -c 'import json,sys
for l in open(sys.argv[1]):
    m=json.loads(l)
    if m.get("reason")=="build-script-executed" and "mesa-sys" in m["package_id"]:
        print(m["out_dir"])' "$work/build.json" | tail -1)
  [ -n "$out" ] || { echo "suites: no mesa-sys build" >&2; exit 1; }
  # MESA_CC, MESA_CFLAGS, MESA_LIBDIR, MESA_ROOT, MESA_WASM
  . "$out/suite.env"
  echo "suites: rustc $have; C: $("$MESA_CC" --version | head -1)"
  libs="$MESA_LIBDIR/libmesa.a $MESA_LIBDIR/libmesa-urcrypt.a $MESA_LIBDIR/libmesa-sqlite.a"
  for t in $TESTS; do
    name=${t%%:*} file=${t#*:}
    exe=$work/$name
    if [ "$MESA_WASM" = 1 ]; then
      # shellcheck disable=SC2086
      if "$MESA_CC" $MESA_CFLAGS -D_WASI_EMULATED_GETPID "$upstream/test/$file.c" $libs \
          -lwasi-emulated-getpid -Wl,-z,stack-size=1048576 -o "$exe.wasm" 2> "$work/$name.log"; then
        (cd "$work" && wasmtime run --dir=. "$exe.wasm") >> "$work/$name.log" 2>&1 &&
          echo "$name pass" || echo "$name fail"
      else
        echo "$name no-build"
      fi
    else
      # shellcheck disable=SC2086
      if "$MESA_CC" $MESA_CFLAGS "$upstream/test/$file.c" $libs -lm -lpthread \
          -o "$exe" 2> "$work/$name.log"; then
        (cd "$work" && "$exe") >> "$work/$name.log" 2>&1 &&
          echo "$name pass" || echo "$name fail"
      else
        echo "$name no-build"
      fi
    fi
  done > "$results"
}

zig_suites() {
  zig=${ZIG:?point ZIG at zig 0.15.2}
  src=$work/libmesa
  mkdir -p "$src"
  git -C "$upstream" archive HEAD | tar -x -C "$src"
  set -- -Dno-host=true
  [ "$mode" = release ] && set -- "$@" -Doptimize=ReleaseFast
  # zig builds for the build machine's CPU unless told; mesa-sys builds
  # for baseline x86-64, so the comparison does too
  [ "$where" = native ] && set -- "$@" -Dcpu=baseline
  echo "suites: zig $("$zig" version); C: $("$zig" cc --version | head -1)"
  if [ "$where" = wasi ]; then
    patch -s -d "$src" -p1 < "$repo/wasm/libmesa-wasm32.diff"
    patch -s -d "$src" -p1 < "$here/zig-wasi-tests.diff"
    set -- "$@" -Dtarget=wasm32-wasi -fwasmtime
  fi
  for t in $TESTS; do
    name=${t%%:*}
    if (cd "$src" && "$zig" build "$name" "$@") > "$work/$name.log" 2>&1; then
      echo "$name pass"
    elif grep -Eq "compile exe $name .*[0-9]+ errors?" "$work/$name.log"; then
      echo "$name no-build"
    else
      echo "$name fail"
    fi
  done > "$results"
}

case $how in
  cargo) cargo_suites ;;
  zig) zig_suites ;;
  *) echo "suites: cargo or zig, not $how" >&2; exit 2 ;;
esac

cat "$results"
for f in "$work"/*.log; do
  n=$(basename "$f" .log)
  if ! grep -q "^$n pass" "$results"; then
    echo "--- $n" >&2
    tail -20 "$f" >&2
  fi
done
grep "^$where " "$here/suites.expected" | cut -d' ' -f2- > "$work/expected"
if diff -u "$work/expected" "$results" >&2; then
  echo "suites: $how $where $mode matches suites.expected"
else
  echo "suites: $how $where $mode differs from suites.expected" >&2
  exit 1
fi
