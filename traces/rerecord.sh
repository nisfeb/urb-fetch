#!/bin/sh
# Records traces 1 to 10 again against the fleet and compares them with
# the committed ones (docs/rust-core.md, "Re-recording in CI"):
#
#   traces/rerecord.sh [--write]
#
# It restores the furum set to its m1c-traces snapshot (traces/fixtures.sh
# builds it), logs in to ~hus, records every trace into a scratch folder
# (wasm/record.mjs), replays each new trace into the JS (the recorder's
# own check), and compares each with the committed one by its digest
# (wasm/drift.mjs: the calls, results and events, with what changes from
# one recording to the next masked). A drift fails. --write puts the new
# traces in place of the committed ones, for the commit that updates them.
#
# It needs the fleet: the furum set claimed by FLEET_WHO, its snapshot,
# and the tools below. Without them it says what is missing and exits 2,
# which is a skip, never a pass. It leaves the set on the snapshot.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(dirname "$here")
FLEET=${FLEET:-$HOME/software/personal/hoon-development/fleet/fleet}
COOKIE=${COOKIE:-$HOME/software/personal/furum/scripts/hoon-test-kit/ship-cookie.sh}
SNAP=$HOME/fleet/snaps/furum/m1c-traces.tar.zst
write=${1:-}

skip() { echo "rerecord: SKIPPED, not passed: $*" >&2; exit 2; }
[ -x "$FLEET" ] || skip "no fleet command at $FLEET"
[ -x "$COOKIE" ] || skip "no ship-cookie.sh at $COOKIE"
[ -f "$SNAP" ] || skip "no snapshot $SNAP (traces/fixtures.sh makes it)"
[ -n "${FLEET_WHO:-}" ] || skip "FLEET_WHO is unset"
"$FLEET" ls | grep -E "^furum .*\($FLEET_WHO\)" >/dev/null || skip "the furum set is not claimed by $FLEET_WHO"
[ -z "${MESA_TRACE:-}" ] || { echo "rerecord: unset MESA_TRACE" >&2; exit 1; }

out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT
"$FLEET" reset furum m1c-traces
"$COOKIE" "$HOME/fleet/piers/hus" http://localhost:8134 "$out/hus.jar"
mkdir "$out/traces"
node "$root/wasm/record.mjs" --out "$out/traces" --hus "$out/hus.jar"
node "$root/wasm/replay.mjs" "$out"/traces/*.jsonl
"$FLEET" reset furum m1c-traces
if [ "$write" = --write ]; then
  cp "$out"/traces/*.jsonl "$here/"
  echo "rerecord: wrote the new traces to $here"
  exit 0
fi
node "$root/wasm/drift.mjs" "$here" "$out/traces"
