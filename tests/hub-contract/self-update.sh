#!/bin/sh
# A hub updating itself, end to end (ADR-0090), as server-package.yml runs it: two debug builds of lull compiled with a
# throwaway kit key and a stand-in for GitHub's newest release (LULL_DEV_KIT_KEY, LULL_DEV_RELEASES; debug builds
# only), installed as the package. Checks that:
#   1. `lull update` downloads the next version's kit, and after the restart that version answers;
#   2. a kit that doesn't start is marked bad, the database copy goes back, the version before answers again, and
#      `lull update` says so;
#   3. a kit file changed on disk is refused at the next start, and the installed build answers.
#
#   sh tests/hub-contract/self-update.sh <lull v1> <v1> <lull v2> <v2> <v3> <private key file>
# v1, v2 and v3 are releases, oldest first; v3 is a kit whose lull exits at once.
set -eu
lull1=$1 v1=$2 lull2=$3 v2=$4 v3=$5 key=$6
here=$(cd "$(dirname "$0")" && pwd)
w=$(mktemp -d)
. "$here/update-kit.sh"

export LULL_DATA_DIR="$w/data" LULL_LISTEN=127.0.0.1:8484 LULL_SLSKD_URL=http://127.0.0.1:9 LULL_SLSKD_API_KEY=k \
  LULL_FLAGS=hub-self-update LULL_LOG=warn
start
until_health "$v1"

echo "== 1. update from $v1 to $v2"
release "$v2" "$lull2"
"$lull1" update || fail "lull update to $v2"
until_health "$v2"
[ -f "$w/data/hub/$v2/good" ] || fail "$v2 wasn't marked good"
[ -f "$w/data/backups/before-$v2.db" ] || fail "no copy of the database from before $v2"

echo "== 2. $v3 doesn't start"
printf '#!/bin/sh\nexit 3\n' >"$w/broken" && chmod 755 "$w/broken"
release "$v3" "$w/broken"
if "$lull1" update; then fail "lull update said $v3 runs"; fi
until_health "$v2"
[ -f "$w/data/hub/$v3/bad" ] || fail "$v3 wasn't marked bad"
grep -q "\"version\":\"$v3\"" "$w/data/hub/failed.json" || fail "failed.json doesn't name $v3"
[ -f "$w/data/backups/before-$v3.db" ] || fail "no copy of the database from before $v3"

echo "== 3. a kit file changed on disk"
stop
printf 'x' >>"$w/data/hub/$v2/lull"
rm "$w/stop" && start
until_health "$v1"
grep -q "a hub kit isn't used" "$w/hub.log" || fail "the changed kit wasn't refused by name"

echo "== a hub updated itself, went back from a kit that didn't start, and refused a changed one"
