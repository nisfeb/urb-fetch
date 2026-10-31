#!/bin/sh
# Fetches the two sources libmesa's build.zig.zon pins and mesa-sys
# compiles: urcrypt (its blake3, chacha and ed25519 subset) and the
# SQLite amalgamation. Each archive is checked against its SHA-256
# before it is unpacked into deps/. The build never downloads anything.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
deps=${MESA_SYS_DEPS:-$here/deps}

# the URLs are build.zig.zon's, at libmesa 2cbb076
urcrypt_url=https://github.com/urbit/urcrypt/archive/a4c229e9380636ace36325b6c8fe04eaee32a2ef.tar.gz
urcrypt_sha=a2a0cadf9b982a0e0aa12d22c27bdc76d8a25834f7f578b1f2b11c8b950e4ee5
sqlite_url=https://www.sqlite.org/2024/sqlite-autoconf-3460100.tar.gz
sqlite_sha=67d3fe6d268e6eaddcae3727fce58fcc8e9c53869bdd07a0c61e38ddf2965071

fetch() { # name url sha
  if [ -f "$deps/$1/.sha256" ] && [ "$(cat "$deps/$1/.sha256")" = "$3" ]; then
    return
  fi
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT
  curl -fsSL -o "$tmp/a.tar.gz" "$2"
  got=$(sha256sum "$tmp/a.tar.gz" | cut -d' ' -f1)
  if [ "$got" != "$3" ]; then
    echo "fetch-deps: $1: sha256 $got, want $3" >&2
    exit 1
  fi
  rm -rf "${deps:?}/$1"
  mkdir -p "$deps/$1"
  tar -xzf "$tmp/a.tar.gz" -C "$deps/$1" --strip-components=1
  echo "$3" > "$deps/$1/.sha256"
  rm -rf "$tmp"
  trap - EXIT
  echo "fetch-deps: $1 ok"
}

fetch urcrypt "$urcrypt_url" "$urcrypt_sha"
fetch sqlite "$sqlite_url" "$sqlite_sha"
