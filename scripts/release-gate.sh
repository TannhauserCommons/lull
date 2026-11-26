#!/bin/sh
# Whether a release may publish (ADR-0103): `sh scripts/release-gate.sh <tag or commit>` exits 0 when `check / gate`,
# from GitHub Actions, passed on the commit it names, and otherwise says what it found and exits 1. It never waits.
# app.yml and release-image.yml run it before they build a tag; before pushing a tag, a person runs it on the
# version commit (scripts/release.sh prints how).
set -eu
die() { echo "release-gate: $*" >&2; exit 1; }
ref=${1:-}
[ -n "$ref" ] || die "usage: release-gate.sh <tag or commit>"
repo=${GITHUB_REPOSITORY:-$(gh repo view --json nameWithOwner --jq .nameWithOwner)}
sha=$(gh api "repos/$repo/commits/$ref" --jq .sha) || die "GitHub has no commit $ref in $repo"
# The newest run of `gate` on the commit: the version's pull request runs it (check.yml), and a re-run or a second
# pull request adds another, so an older verdict never stands for a newer one.
run=$(gh api "repos/$repo/commits/$sha/check-runs?check_name=gate&filter=all&per_page=100" \
  --jq '[.check_runs[] | select(.app.slug == "github-actions")] | sort_by(.started_at) | last | select(.) | "\(.status) \(.conclusion) \(.html_url)"')
status=${run%% *} rest=${run#* } conclusion=${rest%% *} url=${rest#* }
[ -n "$run" ] || die "no check / gate has run on $sha: push it to its release branch and open its pull request first"
[ "$status" = completed ] || die "check / gate on $sha is $status: push the tag (or run this again) once it is green: $url"
[ "$conclusion" = success ] || die "check / gate on $sha ended $conclusion: $url"
echo "check / gate passed on $sha: $url"
