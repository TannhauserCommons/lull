#!/bin/sh
# Lands HEAD on main through the queue (ADR-0095): `mise run ship <topic>`. Pushes HEAD to
# queue/<topic> and waits for the queue's verdict on that commit: landed (exit 0), or red with the
# run to open (exit 1). The topic says what the change holds (`loops-in-css`); pushing the same topic
# again replaces what waits there.
set -eu

die() { echo "ship: $*" >&2; exit 1; }

topic=${1:-}
echo "$topic" | grep -Eq '^[a-z0-9][a-z0-9.-]*$' || die "name the change: mise run ship <topic> (lower case, digits, dashes)"
git diff --quiet && git diff --cached --quiet || die "commit or set aside your changes first"
git fetch -q origin main
[ -n "$(git rev-list origin/main..HEAD)" ] || die "nothing on HEAD that main doesn't have"
# The queue rebases; a conflict is cheaper to find here.
git merge-tree --write-tree origin/main HEAD >/dev/null 2>&1 || die "HEAD conflicts with origin/main: rebase on it first"

tip=$(git rev-parse HEAD)
repo=$(gh repo view --json nameWithOwner --jq .nameWithOwner)
git push -q --force origin "HEAD:refs/heads/queue/$topic"
echo "queued $(git rev-parse --short HEAD) as queue/$topic; waiting for the queue (about as long as its slowest check)"

# The queue's status on the pushed commit is its verdict. Polled: nothing else reaches this shell.
last=""
deadline=$(( $(date +%s) + 2 * 3600 ))
while [ "$(date +%s)" -lt "$deadline" ]; do
  st=$(gh api "repos/$repo/commits/$tip/statuses" \
    --jq '[.[] | select(.context == "queue")][0] | select(.) | "\(.state)\t\(.description)\t\(.target_url)"' 2>/dev/null || true)
  state=$(printf '%s' "$st" | cut -f1) desc=$(printf '%s' "$st" | cut -f2) url=$(printf '%s' "$st" | cut -f3)
  [ "$desc" = "$last" ] || { [ -z "$desc" ] || echo "  $desc"; last=$desc; }
  case "$state" in
    # The candidate's own "tested in the queue" comes just before main moves; only "landed" (or "already in main")
    # says it did.
    success) case "$desc" in landed*|already*) echo "landed: $desc"; exit 0 ;; esac ;;
    failure|error) echo "red: $desc" >&2; echo "  $url" >&2; exit 1 ;;
  esac
  sleep 20
done
die "no verdict after two hours: see https://github.com/$repo/actions/workflows/queue.yml"
