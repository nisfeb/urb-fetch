#!/bin/bash
# Gate 5's side-by-side gateway (docs/rust-core.md, "Gate 5: automated
# soak"): the Go gateway (urb-fetch) and the Rust one (urb-gateway) run
# side by side on two local ports against the fake fleet, and every 10
# minutes both are asked for the same fixed list of fleet pages. Any
# difference goes to /tmp/soak/gateway-diff.log.
#
#   core/ci/soak-gateway.sh                 run until stopped (Ctrl-C)
#   FLEET_WHO=<claim> core/ci/soak-gateway.sh --once [--out DIR]
#                                           one tick, under a held claim
#
# The claim. Left running, it never holds the furum set while it waits:
# at each tick it claims the set as lattice-loop-soak-gw only if nobody
# holds it, asks its questions and releases it again. If someone else
# holds the set, it stops both gateways at once (it looks every 5 s) and
# logs the tick as skipped. When the set was reset under it (~hus's serf
# is a new process), or after a skip, the next tick starts a new epoch:
# reset furum to m1c-traces, settle, seed, start both gateways. A soak
# round (soak.mjs) runs it with --once under the round's own claim, and
# then it takes and leaves no claim and stops its gateways at the end.
#
# The state. One state folder is seeded each epoch: the Go gateway runs
# in it as the Rust side's moon, reads one page and stops. The folder is
# copied twice. The Rust gateway opens its copy as that moon, so it
# continues a Go-written <ship>.db, as the owner's switch would. The Go
# gateway opens the other copy as a second moon: on the fake network
# one identity can't speak from two places at once (~hus keeps one flow
# per moon), so the two sides can't share a moon while both run. Moon
# names and the port in the answers are masked; so are Date and the
# framing headers (Content-Length, Transfer-Encoding, Connection), as
# gateway-diff.mjs does. Everything else is compared byte for byte.
set -u
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
FLEET=${FLEET:-$HOME/software/personal/hoon-development/fleet/fleet}
FLEET_HOME=${FLEET_HOME:-$HOME/fleet}
GO=${GO_GATEWAY:-$root/urb-fetch}
RUST=${URB_GATEWAY:-$root/core/target/release/urb-gateway}
SOAK=${SOAK_DIR:-/tmp/soak}
LOG=$SOAK/gateway-diff.log
EVERY=${EVERY:-600}
GW_WHO=lattice-loop-soak-gw
SNAP=m1c-traces
PORT_GO=8476
PORT_RUST=8477
# (0x50ac0001 << 32 | ~hus) and (0x50ac0002 << 32 | ~hus): moons no other
# fleet script uses
MOON_GO='~ritber-doznec-dozzod-dozhus'
MOON_RUST='~ritber-dozbud-dozzod-dozhus'
PAGES=(
  "hus|/" "hus|/fleet-check" "hus|/fleet-check?raw" "hus|/big" "hus|/big?raw"
  "hus|/marks/md" "hus|/marks/html" "hus|/marks/text" "hus|/marks/gmi" "hus|/marks/css"
  "hus|/n/fleet-check" "hus|/m1c-stale" "hus|/no-such-page" "hus|/p/counter"
  "hus|/forum/m1btest" "hus|/forum/m1btest/1" "rup|/" "home|/" "home|/go?url=urb://~hus/fleet-check"
)

once=0; out=
while [ $# -gt 0 ]; do
  case $1 in
    --once) once=1 ;;
    --out) out=$2; shift ;;
    *) echo "usage: $0 [--once [--out DIR]]" >&2; exit 2 ;;
  esac
  shift
done
mkdir -p "$SOAK"
[ -n "$out" ] || out=$SOAK/gateway
mkdir -p "$out"
for b in "$GO" "$RUST"; do [ -x "$b" ] || { echo "soak-gateway: no $b" >&2; exit 2; }; done

say() { echo "$(date -u +%FT%TZ) $*" | tee -a "$out/run.log"; }
holder() { cat "$FLEET_HOME/claims/furum" 2>/dev/null | tr -d '\n'; }
husPid() { pgrep -f "work --snap-dir $FLEET_HOME/piers/hus " | head -1; }

if [ $once = 1 ]; then
  WHO=${FLEET_WHO:?--once needs FLEET_WHO, the claim it runs under}
  [ "$(holder)" = "$WHO" ] || { echo "soak-gateway: the furum set is not claimed by $WHO" >&2; exit 2; }
else
  WHO=$GW_WHO
fi

pids=()
stop_gateways() {
  for p in "${pids[@]}"; do kill -INT "$p" 2>/dev/null; done
  for p in "${pids[@]}"; do
    for _ in $(seq 150); do kill -0 "$p" 2>/dev/null || break; sleep 0.1; done
    kill -KILL "$p" 2>/dev/null
  done
  pids=()
}
release() { [ $once = 0 ] && [ "$(holder)" = "$GW_WHO" ] && "$FLEET" release furum; }
cleanup() { stop_gateways; release; }
trap 'cleanup; exit 130' INT TERM
trap cleanup EXIT

claim() { # 0 when the set is ours
  [ $once = 1 ] && return 0
  local h; h=$(holder)
  [ -z "$h" ] && { "$FLEET" claim furum "$GW_WHO"; sleep 0.5; h=$(holder); }
  [ "$h" = "$GW_WHO" ]
}

start() { # bin moon state port log
  "$1" -fake "$2" -state "$3" -listen "127.0.0.1:$4" > "$5" 2>&1 &
  local p=$!
  for _ in $(seq 600); do grep -q 'serving http://' "$5" && { pids+=("$p"); return 0; }; kill -0 "$p" 2>/dev/null || break; sleep 0.1; done
  kill -INT "$p" 2>/dev/null
  return 1
}

epoch_pid=
epoch() {
  local e; e=$out/epoch-$(date -u +%Y%m%dT%H%M%SZ)
  mkdir -p "$e"
  stop_gateways
  "$FLEET" --as "$WHO" reset furum "$SNAP" > "$e/reset.log" 2>&1 || { say "epoch: fleet reset failed"; return 1; }
  node --input-type=module -e "import {settle} from '$here/settle.mjs'; settle()" >> "$e/reset.log" 2>&1 || { say "epoch: settle failed"; return 1; }
  # the seed: Go as the Rust side's moon reads one page and stops
  start "$GO" "$MOON_RUST" "$e/seed" "$PORT_GO" "$e/seed.log" || { say "epoch: the seed gateway did not start"; return 1; }
  curl -s -o /dev/null --max-time 90 -H "Host: hus.urb.localhost:$PORT_GO" "http://127.0.0.1:$PORT_GO/fleet-check"
  stop_gateways
  cp -a "$e/seed" "$e/go"; cp -a "$e/seed" "$e/rust"
  start "$GO" "$MOON_GO" "$e/go" "$PORT_GO" "$e/go.log" || { say "epoch: Go did not start"; return 1; }
  start "$RUST" "$MOON_RUST" "$e/rust" "$PORT_RUST" "$e/rust.log" || { say "epoch: Rust did not start"; stop_gateways; return 1; }
  epoch_pid=$(husPid)
  say "epoch $e: Go as $MOON_GO on $PORT_GO, Rust as $MOON_RUST on $PORT_RUST (a copy of the Go-written seed)"
}

# one answer, normalised: status line, headers but Date and framing,
# body; moon names and the port masked
answer() { # port host path
  local h
  case $2 in home) h=urb.localhost:$1 ;; *) h=$2.urb.localhost:$1 ;; esac
  curl -s -i --max-time 90 -H "Host: $h" "http://127.0.0.1:$1$3" 2>&1 \
    | tr -d '\r' \
    | grep -viE '^(date|content-length|transfer-encoding|connection):' \
    | sed -E "s/$MOON_GO|$MOON_RUST/~MOON/g; s/:($PORT_GO|$PORT_RUST)\b/:PORT/g"
}

tick() {
  local t d n=0 bad=0 a b
  t=$(date -u +%Y%m%dT%H%M%SZ)
  d=$out/tick-$t; mkdir -p "$d"
  for pg in "${PAGES[@]}"; do
    local host=${pg%%|*} path=${pg#*|} name
    name=$(echo "$host$path" | tr -c 'a-z0-9\n' '-' | sed 's/-*$//')
    answer "$PORT_GO" "$host" "$path" > "$d/$name.go" & a=$!
    answer "$PORT_RUST" "$host" "$path" > "$d/$name.rust" & b=$!
    wait $a $b
    n=$((n + 1))
    if ! diff -q "$d/$name.go" "$d/$name.rust" > /dev/null; then
      bad=$((bad + 1))
      { echo "== $t $host$path differs (tick $d)"
        diff -u --label go --label rust "$d/$name.go" "$d/$name.rust" | head -80; } >> "$LOG"
    fi
  done
  if [ $bad -gt 0 ]; then say "tick: $((n - bad)) of $n the same, differences in $LOG"; else say "tick: $n of $n the same"; fi
  echo "$t tick $((n - bad))/$n same" >> "$SOAK/gateway-ticks.log"
  return $((bad > 0))
}

if [ $once = 1 ]; then
  epoch || exit 1
  tick; r=$?
  stop_gateways
  exit $r
fi

say "soak-gateway: every $EVERY s, claim $GW_WHO when furum is free; log $LOG"
while :; do
  if claim; then
    if [ ${#pids[@]} -eq 0 ] || [ "$(husPid)" != "$epoch_pid" ]; then epoch || { release; sleep 60; continue; }; fi
    tick
    release
  else
    [ ${#pids[@]} -gt 0 ] && stop_gateways
    say "tick skipped: furum is claimed by $(holder)"
    echo "$(date -u +%Y%m%dT%H%M%SZ) skipped: claimed by $(holder)" >> "$SOAK/gateway-ticks.log"
  fi
  # wait for the next tick, stopping the gateways the moment anyone
  # else claims the set
  for _ in $(seq $((EVERY / 5))); do
    h=$(holder)
    if [ ${#pids[@]} -gt 0 ] && [ -n "$h" ] && [ "$h" != "$GW_WHO" ]; then
      stop_gateways; say "furum claimed by $h: gateways stopped"
    fi
    sleep 5
  done
done
