#!/bin/sh
# A listener's logins across an update (ADR-0091), as server-package.yml runs it. The last release before the vault
# (0.2.6), built from its tag, holds the logins as plain rows; the candidate, with the `secrets` switch on, moves them
# into its file vault. Checks that:
#   1. a candidate that moves them and then fails its health window goes back to the release with every login in place;
#   2. updating to a candidate that starts asks for nothing again: every login is still set, none is left in lull.db,
#      the vault is 0600 in a 0700 folder, and the copy of the database from before the update loses them once the
#      update is marked good.
# The switch reaches only the candidate: the release refuses a switch it doesn't know, so it lives in the LULL_CONFIG
# file, which each start reads, and is taken out again before the release starts.
#
#   sh tests/hub-contract/secrets-update.sh <the release's lull> <its version> <candidate lull, built as v2> <v2> <v3> <key>
# Both lulls are debug builds with the throwaway kit key and the stand-in release (self-update.sh).
set -eu
lull1=$1 v1=$2 lull2=$3 v2=$4 v3=$5 key=$6
here=$(cd "$(dirname "$0")" && pwd)
w=$(mktemp -d)
. "$here/update-kit.sh"

TOKEN=canary-listenbrainz-$$-token
ACOUSTID=canary-acoustid-$$-key
flags() { printf 'LULL_FLAGS=%s\n' "$1" >"$w/lull.conf"; }
holds() { grep -qa "$2" "$1" 2>/dev/null; } # <file> <text>
# The database and its write-ahead log: a new row sits in the log until a checkpoint copies it into lull.db.
db_holds() { holds "$w/data/lull.db" "$1" || holds "$w/data/lull.db-wal" "$1"; } # <text>
set_logins() {
  for check in LULL_LISTENBRAINZ_TOKEN LULL_ACOUSTID_KEY; do
    "$lull1" config get "$check" | grep -q '(set)' || fail "$check isn't set: the listener would have to type it again"
  done
}

export LULL_DATA_DIR="$w/data" LULL_LISTEN=127.0.0.1:8484 LULL_SLSKD_URL=http://127.0.0.1:9 LULL_SLSKD_API_KEY=k \
  LULL_CONFIG="$w/lull.conf" LULL_LOG=info
flags hub-self-update
start
until_health "$v1"
"$lull1" config set LULL_LISTENBRAINZ_TOKEN "$TOKEN" >/dev/null
"$lull1" config set LULL_ACOUSTID_KEY "$ACOUSTID" >/dev/null
set_logins
db_holds "$TOKEN" || fail "$v1 didn't keep the token in its database, so this test proves nothing"

echo "== 1. the candidate moves the logins, then fails its health window"
# Built as $v2, published as $v3: it never answers /health as $v3.
cp "$lull2" "$w/wrong-version" && release "$v3" "$w/wrong-version"
flags hub-self-update,secrets
"$lull1" update >/dev/null 2>&1 &
updating=$!
for _ in $(seq 1 90); do holds "$w/data/secrets/secrets.json" "$TOKEN" && break; sleep 1; done
holds "$w/data/secrets/secrets.json" "$TOKEN" || fail "the candidate didn't move the token into its vault"
flags hub-self-update
# The release to update to next is published while this one is still in its window: the release, back, asks
# which is newest at once (`lull update` above follows it) and keeps the answer a minute.
release "$v2" "$lull2"
until_health "$v1" 300
kill "$updating" 2>/dev/null || true # its answer is not what this checks
until_marked "$v3" bad
set_logins
db_holds "$TOKEN" || fail "$v1 came back without its token"

echo "== 2. the update to $v2 starts, and nothing is asked again"
flags hub-self-update,secrets
"$lull1" update || fail "lull update to $v2"
until_health "$v2"
until_marked "$v2" good
set_logins
for canary in "$TOKEN" "$ACOUSTID"; do
  db_holds "$canary" && fail "$canary is still in lull.db or its log"
  holds "$w/data/secrets/secrets.json" "$canary" || fail "$canary isn't in the vault"
done
[ "$(stat -c %a "$w/data/secrets")" = 700 ] || fail "the vault's folder is $(stat -c %a "$w/data/secrets"), not 700"
[ "$(stat -c %a "$w/data/secrets/secrets.json")" = 600 ] || fail "the vault is $(stat -c %a "$w/data/secrets/secrets.json"), not 600"
for _ in $(seq 1 30); do holds "$w/data/backups/before-$v2.db" "$TOKEN" || break; sleep 1; done
holds "$w/data/backups/before-$v2.db" "$TOKEN" && fail "the copy from before $v2 still holds the token after $v2 was marked good"
[ "$(stat -c %a "$w/data/backups/before-$v2.db")" = 600 ] || fail "the copy from before $v2 isn't 600"
for f in "$w"/data/*.db* "$w"/data/backups/*; do
  holds "$f" "$TOKEN" && fail "$f holds the token"
done

stop
echo "== $v1's logins survived a failed update and moved, with nothing asked, on one that started"
