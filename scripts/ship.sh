#!/bin/sh
# Opens a pull request for HEAD and waits for its check: `mise run ship <topic>`. Pushes HEAD to the branch
# <topic>, opens the pull request (or updates the open one), and waits for `check / gate` on that commit: green
# (exit 0, with the command that merges it) or red with the run to open (exit 1). It never merges: who may merge
# is merge-policy's, and `mise run merge <pr>` does it pinned to the commit that was checked. The topic says what
# the change holds (`loops-in-css`); shipping the same topic again replaces what its pull request holds.
set -eu
die() { echo "ship: $*" >&2; exit 1; }
topic=${1:-}
echo "$topic" | grep -Eq '^[a-z0-9][a-z0-9.-]*$' || die "name the change: mise run ship <topic> (lower case, digits, dashes)"
[ "$topic" != main ] || die "main is not a topic name"
git diff --quiet && git diff --cached --quiet || die "commit or set aside your changes first"
git fetch -q origin main
[ -n "$(git rev-list origin/main..HEAD)" ] || die "nothing on HEAD that main doesn't have"
# The merge rebases; a conflict is cheaper to find here.
git merge-tree --write-tree origin/main HEAD >/dev/null 2>&1 || die "HEAD conflicts with origin/main: rebase on it first"
# Only the App's token opens pull requests: a person's account stamps its time zone on the test-merge commit
# GitHub makes for every open pull request, and the repository is public. An App's installation token can list
# its installation's repositories; a person's token can't.
gh api installation/repositories --jq .total_count >/dev/null 2>&1 \
  || die "open pull requests with the App's token (GH_TOKEN), never a person's: see docs/context/quality.md, Landing a change"
tip=$(git rev-parse HEAD)
repo=$(gh repo view --json nameWithOwner --jq .nameWithOwner)
git push -q --force origin "HEAD:refs/heads/$topic"
pr=$(gh pr list --repo "$repo" --head "$topic" --state open --json number --jq '.[0].number // empty')
if [ -z "$pr" ]; then
  # The oldest commit's subject names the change; the body lists every commit, each with its own body.
  title=$(git log --reverse --format=%s origin/main..HEAD | head -n 1)
  body=$(git log --reverse --format='### %s%n%n%b' origin/main..HEAD)
  pr=$(gh pr create --repo "$repo" --base main --head "$topic" --title "$title" --body "$body" | sed -n 's#.*/pull/\([0-9]*\)$#\1#p')
  [ -n "$pr" ] || die "the pull request didn't open"
  echo "opened #$pr for $(git rev-parse --short HEAD) as $topic; waiting for check / gate"
else
  echo "updated #$pr to $(git rev-parse --short HEAD); waiting for check / gate"
fi
# check.yml's `gate` is the one status main's ruleset requires. Polled on the pushed commit: nothing else reaches
# this shell.
last=""
deadline=$(( $(date +%s) + 2 * 3600 ))
while [ "$(date +%s)" -lt "$deadline" ]; do
  run=$(gh api "repos/$repo/commits/$tip/check-runs?check_name=gate&filter=latest" \
    --jq '[.check_runs[] | select(.app.slug == "github-actions")][0] | select(.) | "\(.status)\t\(.conclusion)\t\(.html_url)"' 2>/dev/null || true)
  status=$(printf '%s' "$run" | cut -f1) conclusion=$(printf '%s' "$run" | cut -f2) url=$(printf '%s' "$run" | cut -f3)
  [ "$status" = "$last" ] || { [ -z "$status" ] || echo "  $status"; last=$status; }
  if [ "$status" = completed ]; then
    case "$conclusion" in
      success) echo "green: #$pr. Merge it, where merge-policy allows, with: mise run merge $pr"; exit 0 ;;
      *) echo "red: #$pr ($conclusion)" >&2; echo "  $url" >&2; exit 1 ;;
    esac
  fi
  sleep 20
done
die "no verdict after two hours: see https://github.com/$repo/pull/$pr/checks"
