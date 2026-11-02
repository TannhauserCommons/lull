#!/bin/sh
# Starts the hub under test from a built image: `sh run-hub.sh <image> <music dir> <work dir>`.
# Host networking, so the stub slskd (stub-slskd.mjs) on 127.0.0.1:5030 is reachable. The work dir
# holds the hub's data and transfers. Prints nothing on success; the hub is `lull-ui` on :8080.
# Everything outside this machine is refused through a dead proxy, so MusicBrainz, Cover Art
# Archive and the rest fail the same way on every run instead of changing the pictures.
set -eu
image=$1 music=$2 work=$3
repo=$(cd "$(dirname "$0")/../.." && pwd)

mkdir -p "$work/data" "$work/transfers/complete" "$work/transfers/incomplete"
chmod -R a+rwX "$work"

# A key a build does not know yet would stop the hub (unknown setting), so pass it only once the
# code reads it: pairing (ADR-0002) and folder mode (ADR-0003) land after this script.
optional() {
  if grep -rqF "\"$1\"" "$repo/crates/core/src"; then set -- -e "$1=$2"; echo "$@"; fi
}

docker rm -f lull-ui >/dev/null 2>&1 || true
# shellcheck disable=SC2046
docker run -d --name lull-ui --network host \
  -e LULL_LISTEN=127.0.0.1:8080 \
  -e LULL_LOG=info \
  -e LULL_SLSKD_URL=http://127.0.0.1:5030 \
  -e LULL_SLSKD_API_KEY=ui-test-key \
  -e LULL_TRANSFERS_DIR=/transfers \
  -e LULL_MUSIC_DIR=/music \
  -e HTTP_PROXY=http://127.0.0.1:9 -e HTTPS_PROXY=http://127.0.0.1:9 -e NO_PROXY=127.0.0.1,localhost \
  $(optional LULL_SETUP_CODE ui-test-setup) \
  $(optional LULL_LIBRARY folder) \
  -v "$music:/music:ro" -v "$work/data:/data" -v "$work/transfers:/transfers" \
  "$image" >/dev/null
