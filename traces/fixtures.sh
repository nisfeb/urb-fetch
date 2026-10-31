#!/bin/sh
# Builds the furum fleet set's m1c-traces snapshot, which wasm/record.mjs
# records traces 1 to 9 against: hus-lattice (~hus on lattice 81, with
# board m1btest and the marks/* pages; ~rup on furum 7) and then
#
#   ~hus  pages index (gmi) and big (md, 16 KB), shared over urbit
#         know entries fleet/granted and fleet/withheld
#         group m1c: trace 3's moon may peek fleet/granted and the page
#         fleet-check's code grub (a group, not /share-file: its notice
#         to a ship that never sent ~hus anything goes over ames, and
#         ~hus then drops that ship's mesa; docs/rust-core.md)
#   ~rup  furum board m1c (poster by default), post 0, comment 0
#
#   traces/fixtures.sh
#
# It claims nothing: claim the furum set first (fleet claim furum WHO).
# FLEET is the fleet command, COOKIE hoon-test-kit's ship-cookie.sh.
set -eu
FLEET=${FLEET:-$HOME/software/personal/hoon-development/fleet/fleet}
COOKIE=${COOKIE:-$HOME/software/personal/furum/scripts/hoon-test-kit/ship-cookie.sh}
jar=$(mktemp -d)
trap 'rm -rf "$jar"' EXIT

"$FLEET" reset furum hus-lattice
"$COOKIE" "$HOME/fleet/piers/hus" http://localhost:8134 "$jar/hus"
"$COOKIE" "$HOME/fleet/piers/rup" http://localhost:8133 "$jar/rup"
hus() { curl -fsS -b "$jar/hus" -X POST "http://localhost:8134/apps/lattice/$1" --data-binary "$2"; echo; }
rup() { curl -fsS -o /dev/null -w "%{http_code}\n" -H "cookie: $(awk '$6 ~ /^urbauth-/ {print $6"="$7}' "$jar/rup")" \
          -X POST "http://localhost:8133/apps/furum/$1" --data "$2"; }
B=/apps/shell.shell/desks/lattice.desk/desk/data/lattice.lattice_app

hus 'page-save?name=index&type=gmi' "$(printf '# ~hus\n\nThe fleet'"'"'s lattice, for urb-fetch'"'"'s traces.\n=> /fleet-check fleet-check')"
hus 'page-share?name=index&mode=urbit' ''
big=$( { echo '# A page over 15 KB'; echo; i=0; while [ $i -lt 220 ]; do
  echo "Line $i: the quick brown fox jumps over the lazy dog, for the %want leg."; i=$((i + 1)); done; } )
hus 'page-save?name=big&type=md' "$big"
hus 'page-share?name=big&mode=urbit' ''
hus 'know-save?key=fleet/granted' 'A know entry granted to the trace-3 reader.'
hus 'know-save?key=fleet/withheld' 'A know entry nobody is granted.'
hus 'share-group-save?name=m1c' "{\"ships\":[\"~natdul-molwes-dozzod-dozhus\"],\"peek\":[\"$B/know/vault/fleet/granted\",\"$B/page/fleet-check\"],\"make\":[]}"

rup create 'name=m1c&title=M1c+traces&description=furum+for+urb-fetch+trace+4&default-role=poster'
rup b/~rup/m1c/submit 'title=The+host%27s+first+post&body=for+trace+4'
rup b/~rup/m1c/0/comment 'body=the+host+comments'

"$FLEET" snap furum m1c-traces
