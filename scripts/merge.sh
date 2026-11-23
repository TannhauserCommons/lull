#!/bin/sh
# Merges a green pull request onto main: `mise run merge <pr>`. Only the commit `check / gate` passed is merged:
# the merge names that head, so a push after the check (or after the owner's approval) makes it refuse. It rebases,
# so every commit keeps its own subject, which the release notes read. Whether a change may merge without the
# owner's yes is merge-policy's call, made before this runs; this checks only that the head is green.
set -eu
die() { echo "merge: $*" >&2; exit 1; }
pr=${1:-}
echo "$pr" | grep -Eq '^[0-9]+$' || die "give the pull request's number: mise run merge <pr>"
repo=$(gh repo view --json nameWithOwner --jq .nameWithOwner)
head=$(gh pr view "$pr" --repo "$repo" --json headRefOid,state --jq 'select(.state == "OPEN") | .headRefOid')
[ -n "$head" ] || die "#$pr isn't open"
gate=$(gh api "repos/$repo/commits/$head/check-runs?check_name=gate&filter=latest" \
  --jq '[.check_runs[] | select(.app.slug == "github-actions")][0] | select(.) | "\(.status) \(.conclusion)"')
[ "$gate" = "completed success" ] || die "check / gate isn't green on #$pr's head $(echo "$head" | cut -c1-7) (${gate:-no run}): mise run ship first"
gh pr merge "$pr" --repo "$repo" --rebase --delete-branch --match-head-commit "$head"
echo "merged #$pr ($(echo "$head" | cut -c1-7)); main runs main.yml on it"
