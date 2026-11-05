#!/bin/sh
# The queue's steps (ADR-0095), run by .github/workflows/queue.yml on main. Nothing else moves main.
#   plan   build the candidate: every waiting queue/<topic> branch rebased, oldest first, onto main
#          and onto the ones before it; write which checks its diff needs to $GITHUB_OUTPUT
#   land   fast-forward main to the tested candidate; mark and delete the branches it holds
#   reject report a red candidate on its branches, or split a red batch into single tries
# Needs git with push rights (the job's token), gh, and in CI: REPO, RUN_URL; land and reject also
# CANDIDATE and BRANCHES (plan's outputs), and for reject FAILED (the red checks' names).
set -eu

die() { echo "queue: $*" >&2; exit 1; }
short() { echo "$1" | cut -c1-7; }
REPO=${REPO:-${GITHUB_REPOSITORY:-}}
RUN_URL=${RUN_URL:-${GITHUB_SERVER_URL:-https://github.com}/$REPO/actions/runs/${GITHUB_RUN_ID:-0}}
out() { echo "$1=$2" >> "${GITHUB_OUTPUT:-/dev/stdout}"; }

# The status every queue/<topic> tip and every landed commit carries. Only GitHub Actions sets it, and
# main's ruleset requires it from GitHub Actions.
CONTEXT=queue
SPLIT="failed in a batch; trying alone"
MAX_BATCH=10
# More commits than a topic holds: a branch made from history main no longer has (a rewrite), which a rebase
# would replay whole.
MAX_TOPIC=100

status() { # <sha> <state> <description> [url]
  gh api -X POST "repos/$REPO/statuses/$1" -f context="$CONTEXT" -f state="$2" \
    -f description="$(printf '%s' "$3" | cut -c1-140)" -f target_url="${4:-$RUN_URL}" >/dev/null
}

# The newest `queue` status on a commit, as "state<TAB>description" ("" when it has none).
current() {
  gh api "repos/$REPO/commits/$1/statuses" \
    --jq "[.[] | select(.context == \"$CONTEXT\")][0] | select(.) | \"\(.state)\t\(.description)\""
}

# Which checks a diff needs, by the paths the workflows filtered on before the queue (ADR-0095's table).
checks() { # <base> <candidate>
  files=$(git diff --name-only "$1" "$2")
  has() { printf '%s\n' "$files" | grep -Eq "$1"; }
  all=false; has '^(\.github/|scripts/queue\.sh$)' && all=true
  want() { if $all || has "$2"; then out "$1" true; else out "$1" false; fi; }
  # ci: everything but words and pictures.
  if $all || printf '%s\n' "$files" | grep -Evq '^(docs/|design/|fastlane/|site/)|\.md$'; then out ci true; else out ci false; fi
  want ui_gates '^(web/|tests/ui/)'
  want ui '^(web/|crates/server/src/web\.rs$|tests/ui/|scripts/web-dist/|Dockerfile$)'
  want app '^(app/|crates/|web/desktop/|web/assets/icons/|rust-toolchain\.toml$)'
  want android '^(app/gen/android/|app/src/mobile/|app/build\.rs$|app/Cargo\.(toml|lock)$|app/tauri\.android\.conf\.json$|scripts/android-|rust-toolchain\.toml$)'
  want slskd '^(deploy/compose\.yaml$|app/build\.rs$|crates/soulseek/|tests/slskd-contract/)'
  # What reaches the image: the hub's code, the web UI it embeds, and how both are built.
  want image '^(crates/|web/|Cargo\.(toml|lock)$|rust-toolchain\.toml$|Dockerfile$|\.dockerignore$|scripts/vendor-web\.sh$|scripts/web-dist/)'
  want pages '^(site/|web/assets/app-icon\.svg$|web/fonts/nunito-latin\.woff2$)'
  # What every release build compiles: main builds the bundles then, to warm their caches (ADR-0097).
  want warm '^(Cargo\.lock|app/Cargo\.lock|app/Cargo\.toml|rust-toolchain\.toml)$'
}

plan() {
  git fetch -q origin '+refs/heads/queue/*:refs/remotes/origin/queue/*' '+refs/heads/main:refs/remotes/origin/main'
  # One run at a time, so any other run's candidate is left from a run that was cancelled.
  for ref in $(git ls-remote origin 'refs/heads/queue-candidate/*' | cut -f2); do
    [ "$ref" = "refs/heads/queue-candidate/$GITHUB_RUN_ID" ] || git push -q origin --delete "$ref" || true
  done
  base=$(git rev-parse origin/main)
  # Oldest first, by when the tip was committed.
  waiting=$(git for-each-ref --sort=committerdate --format='%(refname:strip=4) %(objectname)' refs/remotes/origin/queue/)
  [ -n "$waiting" ] || { echo "nothing waiting"; out candidate ""; return; }

  # A red tip waits for its owner's next push. Tips left from a red batch go one at a time.
  eligible="" split=false
  while read -r topic tip; do
    st=$(current "$tip")
    case "$st" in
      failure*|error*) echo "queue/$topic: red at $(short "$tip"), waiting for a new push"; continue ;;
      "pending	$SPLIT"*) split=true ;;
    esac
    eligible="$eligible$topic $tip
"
  done <<EOF
$waiting
EOF
  [ -n "$eligible" ] || { out candidate ""; return; }
  if $split; then eligible=$(printf '%s' "$eligible" | head -n 1); else eligible=$(printf '%s' "$eligible" | head -n "$MAX_BATCH"); fi

  cand=$base branches=""
  while read -r topic tip; do
    [ -n "$topic" ] || continue
    if ! fork=$(git merge-base "$base" "$tip") || [ "$(git rev-list --count "$fork..$tip")" -gt "$MAX_TOPIC" ]; then
      echo "queue/$topic: not made from main's history"
      status "$tip" failure "not made from main's history: rebase your commits onto origin/main and push again"
      continue
    fi
    git checkout -q --detach "$tip"
    if git rebase -q --onto "$cand" "$fork" 2>/dev/null; then
      new=$(git rev-parse HEAD)
      if [ "$new" = "$cand" ]; then
        echo "queue/$topic: already in main"
        git push -q origin --delete "refs/heads/queue/$topic" || true
        status "$tip" success "already in main"
        continue
      fi
      cand=$new branches="$branches$topic:$tip "
      status "$tip" pending "testing with $(git rev-list --count "$base..$cand") commits on main"
    else
      git rebase --abort
      echo "queue/$topic: conflicts with main"
      status "$tip" failure "conflicts with main: rebase on origin/main and push again"
    fi
  done <<EOF
$eligible
EOF
  [ "$cand" != "$base" ] || { out candidate ""; return; }

  # The candidate needs a ref for the check jobs to fetch; a push with the job's token starts nothing.
  git push -q origin "$cand:refs/heads/queue-candidate/$GITHUB_RUN_ID"
  git log --format='  %h %s' "$base..$cand"
  out candidate "$cand"
  out base "$base"
  out branches "${branches% }"
  checks "$base" "$cand"
}

land() {
  [ -n "${CANDIDATE:-}" ] || die "no candidate"
  git fetch -q origin "refs/heads/queue-candidate/$GITHUB_RUN_ID"
  status "$CANDIDATE" success "tested in the queue"
  # A fast-forward only: if main moved (a push past the queue), nothing is lost and the next run retries.
  if ! git push -q origin "$CANDIDATE:refs/heads/main"; then
    echo "main moved during the run; trying again"
    git push -q origin --delete "refs/heads/queue-candidate/$GITHUB_RUN_ID" || true
    out landed false
    retry
    return
  fi
  out landed true
  for b in $BRANCHES; do
    topic=${b%%:*} tip=${b#*:}
    status "$tip" success "landed in main at $(short "$CANDIDATE")"
    # Only the tip that was tested: a push meanwhile stays queued.
    git push -q origin --delete --force-with-lease="refs/heads/queue/$topic:$tip" "refs/heads/queue/$topic" \
      || echo "queue/$topic moved since it was tested; it stays queued"
  done
  git push -q origin --delete "refs/heads/queue-candidate/$GITHUB_RUN_ID" || true
  next
}

reject() {
  [ -n "${CANDIDATE:-}" ] || die "no candidate"
  set -- $BRANCHES
  if [ $# -gt 1 ]; then
    for b; do status "${b#*:}" pending "$SPLIT"; done
    retry
  else
    failed=$(echo $FAILED | sed 's/ /, /g')
    status "${1#*:}" failure "${failed:-a check} failed or ran out of time; open the run"
    next
  fi
  git push -q origin --delete "refs/heads/queue-candidate/$GITHUB_RUN_ID" || true
}

# Ends the run as soon as one check fails, instead of after the slowest (ui takes about 15 minutes): the verdict and
# the next run come at once. Returns when no check is left running, green or not yet created.
watch() {
  [ -n "${CANDIDATE:-}" ] || die "no candidate"
  while :; do
    jobs=$(gh api "repos/$REPO/actions/runs/$GITHUB_RUN_ID/jobs?per_page=100" \
      --jq '.jobs[] | select(.name | test("^(plan|watch|land|reject)$") | not) | [.name, .status, (.conclusion // "")] | @tsv')
    failed=$(printf '%s\n' "$jobs" | awk -F'\t' '$3 == "failure" || $3 == "cancelled" { split($1, a, " / "); print a[1] }' | sort -u | tr '\n' ' ')
    if [ -n "$failed" ]; then
      FAILED=$failed reject
      gh run cancel "$GITHUB_RUN_ID" --repo "$REPO"
      return
    fi
    printf '%s\n' "$jobs" | awk -F'\t' '$2 != "completed"' | grep -q . || return 0
    sleep 20
  done
}

# Another run, after this one: the concurrency group holds it until this run ends.
retry() { gh workflow run queue.yml --repo "$REPO" --ref main; }

# After a verdict, the branches still waiting (left from a split batch, or pushed during the run) get the next
# run at once; their own pushes' runs may have been dropped while this one held the group.
next() {
  if [ -n "$(git ls-remote origin 'refs/heads/queue/*')" ]; then retry; fi
}

case "${1:-}" in
  plan) plan ;;
  land) land ;;
  reject) reject ;;
  watch) watch ;;
  *) die "usage: queue.sh plan|watch|land|reject" ;;
esac
