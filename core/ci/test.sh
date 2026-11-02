#!/bin/sh
# fmt, clippy and cargo test, natively and under WASI, as CI's core job
# runs them, with the toolchain rust-toolchain.toml pins. A cargo that
# is not rustup's (a distribution's /usr/bin/cargo) ignores the pin:
# native tests may pass on it while WASI fails with "can't find crate
# for core". So rustup's bin comes first on PATH, and the run stops at
# once if rustc is still not the pinned one. Extra arguments go to
# each cargo test, e.g. --release. wasmtime must be on PATH (see
# tools.sh).
set -eu
core=$(cd "$(dirname "$0")/.." && pwd)
cd "$core"
PATH=${CARGO_HOME:-$HOME/.cargo}/bin:$PATH
export PATH
want=$(sed -n 's/^channel = "\(.*\)"/\1/p' rust-toolchain.toml)
if ! rustc --version 2>/dev/null | grep -q "^rustc $want "; then
  echo "test.sh: rust-toolchain.toml pins rustc $want, but $(command -v cargo) runs $(rustc --version 2>&1)" >&2
  echo "test.sh: install rustup, then: rustup toolchain install (in core/)" >&2
  exit 1
fi
if ! rustup target list --installed | grep -qx wasm32-wasip1; then
  echo "test.sh: rustc $want has no wasm32-wasip1; run: rustup toolchain install (in core/)" >&2
  exit 1
fi
command -v wasmtime >/dev/null || { echo "test.sh: no wasmtime on PATH (see ci/tools.sh)" >&2; exit 1; }
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo clippy --all-targets --target wasm32-wasip1 -- -D warnings
cargo test "$@"
cargo test --target wasm32-wasip1 "$@"
