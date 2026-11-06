#!/bin/sh
# Asks the queue (ADR-0095) to test a waiting topic alone, ahead of older ones: `mise run queue:first <topic>`, for a
# fix to what every batch trips on. It marks the topic with refs/heads/queue-first/<topic>, which every run's plan
# reads until the topic leaves queue/, and starts a run. A dispatch's input couldn't carry this: Actions keeps one
# pending run, and any push's kick replaces it before it starts.
set -eu
topic=${1:?usage: queue-first.sh <topic>}
git fetch -q origin "refs/heads/queue/$topic" 2>/dev/null \
  || { echo "queue/$topic isn't waiting: ship it first (mise run ship $topic)" >&2; exit 1; }
git push -q origin "FETCH_HEAD:refs/heads/queue-first/$topic"
gh workflow run queue.yml --repo "$(gh repo view --json nameWithOwner --jq .nameWithOwner)" --ref main
echo "queue/$topic goes first in the next run"
