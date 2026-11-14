#!/bin/sh
# The hub the functional UI tests run against, and the checks made on it besides the tests: `sh tests/ui/hub.sh <step>`.
#   up        load the image `mise run image:save` made, make the fixture library, start the stub slskd and the hub on
#             :8080 (run-hub.sh); the tests' own global setup waits for it
#   contract  the hub contract (ADR-0090; server-package.yml runs it on the Linux package): what any installed hub must do
#   stub      fail when the hub called a slskd route the stub doesn't answer. It answers what the hub asks slskd in these
#             tests; a call it doesn't know would be a 404 and show on every page as a Soulseek banner (seen 2026-11-06
#             as layout shift on every route), so it fails here by name instead
#   logs      the hub's log and the stub's, for after a failure
# Needs Docker and Linux (host networking). Files go in $RUNNER_TEMP, else $TMPDIR.
set -eu
cd "$(dirname "$0")"
tmp=${RUNNER_TEMP:-${TMPDIR:-/tmp}}
tar=$tmp/lull-image.tar # where image-save.sh puts it
stub_log=$tmp/slskd-stub.log

case "${1:-}" in
  up)
    docker load -i "$tar"
    node fixtures/make-library.mjs "$tmp/music"
    # No terminal and no pipe left open, so the step ends and the stub stays up for the steps after it.
    nohup node stub-slskd.mjs 5030 ui-test-key > "$stub_log" 2>&1 < /dev/null &
    sh run-hub.sh lull:candidate "$tmp/music" "$tmp/hub"
    ;;
  contract)
    code=$(docker exec lull-ui lull pair | sed -n 's/.*enter the code \([A-Z0-9]*\).*/\1/p')
    [ -n "$code" ] || { echo "hub.sh: lull pair gave no code" >&2; exit 1; }
    node ../hub-contract/run.mjs "${LULL_URL:-http://127.0.0.1:8080}" --pair "$code" --youtube
    ;;
  stub)
    [ -f "$stub_log" ] || { echo "hub.sh: no stub log at $stub_log: hub.sh up didn't run" >&2; exit 1; }
    if grep 'unhandled' "$stub_log" | sort | uniq -c | grep .; then
      echo "::error::the hub called slskd routes tests/ui/stub-slskd.mjs doesn't answer (above): answer them there"
      exit 1
    fi
    ;;
  logs)
    docker logs lull-ui 2>&1 | tail -200
    cat "$stub_log"
    ;;
  *) echo "usage: hub.sh up|contract|stub|logs" >&2; exit 2 ;;
esac
