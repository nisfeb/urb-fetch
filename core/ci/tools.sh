#!/bin/sh
# Installs the pinned tools the core's CI needs into a directory, each
# checked against its SHA-256: the wasi-sdk (C for wasm32-wasip1),
# wasmtime (runs the WASI tests), zig 0.15.2 (libmesa's own build, for
# comparison and for wasm/build.sh) and vere (check.mjs's urbit eval).
#
#   core/ci/tools.sh <dir>
#
# It prints the environment to use. Under GitHub Actions it also writes
# it to GITHUB_ENV and GITHUB_PATH. Linux x86_64 only.
set -eu
dir=${1:?a directory}
mkdir -p "$dir"
dir=$(cd "$dir" && pwd)

get() { # name url sha
  [ -f "$dir/$1.ok" ] && [ "$(cat "$dir/$1.ok")" = "$3" ] && return
  tmp=$(mktemp -d)
  curl -fsSL -o "$tmp/a" "$2"
  got=$(sha256sum "$tmp/a" | cut -d' ' -f1)
  if [ "$got" != "$3" ]; then
    echo "tools: $1: sha256 $got, want $3" >&2
    rm -rf "$tmp"
    exit 1
  fi
  rm -rf "${dir:?}/$1"
  mkdir -p "$dir/$1"
  case $2 in
    *.tar.xz) tar -xJf "$tmp/a" -C "$dir/$1" --strip-components=1 ;;
    *) tar -xzf "$tmp/a" -C "$dir/$1" ;;
  esac
  rm -rf "$tmp"
  echo "$3" > "$dir/$1.ok"
}

get wasi-sdk https://github.com/WebAssembly/wasi-sdk/releases/download/wasi-sdk-34/wasi-sdk-34.0-x86_64-linux.tar.gz \
  b761e3a0721dbae9c09a0059e5fdb2bf917d1b4a8a7b430fb3b5aafb0984b2c4
get wasmtime https://github.com/bytecodealliance/wasmtime/releases/download/v50.0.1/wasmtime-v50.0.1-x86_64-linux.tar.xz \
  685466455dd42a14774a86c36c8f2e45cbb9bd6d92a850139315df60a75498a5
get zig https://ziglang.org/download/0.15.2/zig-x86_64-linux-0.15.2.tar.xz \
  02aa270f183da276e5b5920b1dac44a63f1a49e55050ebde3aecc9eb82f93239
get vere https://github.com/urbit/vere/releases/download/vere-v4.6/linux-x86_64.tgz \
  f332e96d66cb767f847fbf15e04b4f953fd078e610b449fbbc26629ea312a3ae

# the wasi-sdk unpacks with its version in the name
sdk=$(ls -d "$dir"/wasi-sdk/wasi-sdk-* 2>/dev/null | head -1)
[ -n "$sdk" ] || sdk=$dir/wasi-sdk
env="WASI_SDK_PATH=$sdk
ZIG=$dir/zig/zig
URBIT=$dir/vere/vere-v4.6-linux-x86_64"
echo "$env"
echo "PATH=$dir/wasmtime:\$PATH"
if [ -n "${GITHUB_ENV:-}" ]; then
  echo "$env" >> "$GITHUB_ENV"
  echo "$dir/wasmtime" >> "$GITHUB_PATH"
fi
