#!/bin/sh
# The queue's steps (ADR-0095), run by .github/workflows/queue.yml on main. Nothing else moves main.
#   plan   build the candidate: every waiting queue/<topic> branch rebased, oldest first, onto main
#          and onto the ones before it; write which checks its diff needs to $GITHUB_OUTPUT
#   land   fast-forward main to the tested candidate; mark and delete the branches it holds
#   reject report a red candidate on its branches, or split a red batch into single tries
#   rescue start the next run when this one broke off before it could (queue.yml's last job)
#   dispatch <workflow>  start a workflow on main, asking again when GitHub fails
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

# gh, asked again when GitHub's answer is an error of its own: now and then the API answers 5xx, or nothing at all (run
# 42065508846: "unexpected end of JSON input" right after main moved). A refusal (4xx, save 429) is final at once.
# Every call here is safe to repeat: a status set twice, or a run asked for twice, changes nothing.
GH_PAUSES=${GH_PAUSES:-2 8 30}
try_gh() {
  gh_err=$(mktemp)
  for pause in $GH_PAUSES ''; do
    if gh_out=$(gh "$@" 2>"$gh_err"); then
      rm -f "$gh_err"; [ -z "$gh_out" ] || printf '%s\n' "$gh_out"; return 0
    fi
    cat "$gh_err" >&2
    if grep -q 'HTTP 4' "$gh_err" && ! grep -q 'HTTP 429' "$gh_err"; then break; fi
    [ -n "$pause" ] || break
    echo "queue: gh $1 failed; asking again in ${pause}s" >&2
    sleep "$pause"
  done
  rm -f "$gh_err"
  return 1
}

status() { # <sha> <state> <description> [url]
  try_gh api -X POST "repos/$REPO/statuses/$1" -f context="$CONTEXT" -f state="$2" \
    -f description="$(printf '%s' "$3" | cut -c1-140)" -f target_url="${4:-$RUN_URL}" >/dev/null
}

# The newest `queue` status on a commit, as "state<TAB>description" ("" when it has none).
current() {
  try_gh api "repos/$REPO/commits/$1/statuses" \
    --jq "[.[] | select(.context == \"$CONTEXT\")][0] | select(.) | \"\(.state)\t\(.description)\""
}

# Which checks a diff needs, by the paths the workflows filtered on before the queue (ADR-0095's table).
checks() { # <base> <candidate>
  files=$(git diff --name-only "$1" "$2")
  has() { printf '%s\n' "$files" | grep -Eq "$1"; }
  all=false; has '^(\.github/|scripts/queue\.sh$)' && all=true
  want() { if $all || has "$2"; then out "$1" true; else out "$1" false; fi; }
  # ci: everything but words and pictures, and the share Worker (its own check), save its host list, which the hub
  # compiles in.
  if $all || has '^share/hosts\.json$' || printf '%s\n' "$files" | grep -Evq '^(docs/|design/|fastlane/|site/|share/)|\.md$'; then
    out ci true
  else
    out ci false
  fi
  want ui_gates '^(web/|tests/ui/)'
  want ui '^(web/|crates/server/src/web\.rs$|tests/ui/|scripts/web-dist/|Dockerfile$)'
  want app '^(app/|crates/|web/desktop/|web/assets/icons/|rust-toolchain\.toml$)'
  want android '^(app/gen/android/|app/src/mobile/|app/build\.rs$|app/Cargo\.(toml|lock)$|app/tauri\.android\.conf\.json$|scripts/android-|rust-toolchain\.toml$)'
  want slskd '^(deploy/compose\.yaml$|app/build\.rs$|app/slskd/|crates/soulseek/|tests/slskd-contract/)'
  # What reaches the image: the hub's code, the web UI it embeds, and how both are built.
  want image '^(crates/|web/|share/hosts\.json$|Cargo\.(toml|lock)$|rust-toolchain\.toml$|Dockerfile$|\.dockerignore$|scripts/vendor-web\.sh$|scripts/web-dist/)'
  want pages '^(site/|web/assets/app-icon\.svg$|web/fonts/nunito-latin\.woff2$)'
  # Not a check (YouTube being down must not hold a landing): its live run follows a change to the reader.
  want youtube_contract '^crates/discovery/src/search/youtube_music\.rs$'
  # The share Worker (ADR-0034): tested whenever it or a file it serves changes, deployed after landing.
  want share '^(share/|site/|web/assets/app-icon\.svg$|web/assets/fallback-art-512\.png$|web/fonts/nunito-latin\.woff2$|packaging/android/assetlinks\.json$|app/gen/android/app/src/main/AndroidManifest\.xml$)'
  # A feat or fix landed: the next release's draft is redrawn (ADR-0098).
  if git log --format=%s "$1..$2" | grep -Eq '^(feat|fix)\('; then out notes true; else out notes false; fi
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
  # A topic asked for first (queue.yml's `first`) goes alone, ahead of the rest: one that fixes what every batch trips on.
  first=$(printf '%s' "$eligible" | awk -v t="${FIRST:-}" '$1 == t' | head -n 1)
  if [ -n "$first" ]; then eligible=$first
  elif $split; then eligible=$(printf '%s' "$eligible" | head -n 1); else eligible=$(printf '%s' "$eligible" | head -n "$MAX_BATCH"); fi

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
  # A fast-forward only: if main moved (a push past the queue), nothing is lost and the next run retries. A push whose
  # answer was lost may still have landed: main says which.
  if ! git push -q origin "$CANDIDATE:refs/heads/main" \
      && [ "$(git ls-remote origin refs/heads/main | cut -f1)" != "$CANDIDATE" ]; then
    echo "main moved during the run; trying again"
    git push -q origin --delete "refs/heads/queue-candidate/$GITHUB_RUN_ID" || true
    out landed false
    retry
    return
  fi
  out landed true
  # Main has moved: from here no answer of GitHub's may end the run, which still has the image to publish and the next
  # run to start. What can't be done even after asking again is left for the next run, which finishes it.
  for b in $BRANCHES; do
    topic=${b%%:*} tip=${b#*:}
    # A tip that can't be told it landed keeps its branch: the next run finds it in main, says "already in main" and
    # deletes it, so `mise run ship` hears the verdict either way.
    if ! status "$tip" success "landed in main at $(short "$CANDIDATE")"; then
      echo "::warning::queue/$topic landed, but its status couldn't be set; the next run sets it"
      continue
    fi
    # Only the tip that was tested: a push meanwhile stays queued.
    git push -q origin --delete --force-with-lease="refs/heads/queue/$topic:$tip" "refs/heads/queue/$topic" \
      || echo "queue/$topic moved since it was tested; it stays queued"
  done
  git push -q origin --delete "refs/heads/queue-candidate/$GITHUB_RUN_ID" || true
  next || { echo "::warning::the next run couldn't be started; this run's last job asks again"; out chained false; }
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
    jobs=$(try_gh api "repos/$REPO/actions/runs/$GITHUB_RUN_ID/jobs?per_page=100" \
      --jq '.jobs[] | select(.name | test("^(plan|watch|land|reject)$") | not) | [.name, .status, (.conclusion // "")] | @tsv')
    failed=$(printf '%s\n' "$jobs" | awk -F'\t' '$3 == "failure" || $3 == "cancelled" { split($1, a, " / "); print a[1] }' | sort -u | tr '\n' ' ')
    if [ -n "$failed" ]; then
      FAILED=$failed reject
      try_gh run cancel "$GITHUB_RUN_ID" --repo "$REPO"
      return
    fi
    printf '%s\n' "$jobs" | awk -F'\t' '$2 != "completed"' | grep -q . || return 0
    sleep 20
  done
}

# A workflow on main, started by this run (its pushes start none themselves).
dispatch() { try_gh workflow run "$1" --repo "$REPO" --ref main; }

# Another run, after this one: the concurrency group holds it until this run ends.
retry() { dispatch queue.yml; }

# After a verdict, the branches still waiting (left from a split batch, or pushed during the run) get the next
# run at once; their own pushes' runs may have been dropped while this one held the group. When git can't say,
# a run with nothing to do costs a minute.
next() {
  waiting=$(git ls-remote origin 'refs/heads/queue/*') || waiting=unknown
  if [ -n "$waiting" ]; then retry; fi
}

# The run's last job, when a job broke off before its verdict or land couldn't start the next run: start it, unless
# the two runs before this one failed too, which is the queue's own fault and not a blip (the schedule tries again).
rescue() {
  waiting=$(try_gh api "repos/$REPO/git/matching-refs/heads/queue/" --jq length)
  [ "$waiting" -gt 0 ] || { echo "nothing waiting"; return; }
  # This run isn't finished yet: the two before it decide.
  before=$(try_gh api "repos/$REPO/actions/workflows/queue.yml/runs?status=completed&per_page=20" \
    --jq "[.workflow_runs[] | select(.id != ${GITHUB_RUN_ID:-0} and .conclusion != \"cancelled\" and .conclusion != \"skipped\")][0:2] | map(.conclusion) | join(\" \")")
  if [ "$before" = "failure failure" ]; then
    echo "::warning::three queue runs failed in a row: left to the schedule, look at the queue"; return
  fi
  retry
}

case "${1:-}" in
  plan) plan ;;
  land) land ;;
  reject) reject ;;
  watch) watch ;;
  rescue) rescue ;;
  dispatch) dispatch "$2" ;;
  *) die "usage: queue.sh plan|watch|land|reject|rescue|dispatch <workflow>" ;;
esac
