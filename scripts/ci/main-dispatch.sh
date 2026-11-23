#!/bin/sh
# Starts a workflow on main: `sh scripts/ci/main-dispatch.sh <workflow file>` (`mise run main:dispatch <file>`). main.yml's
# after-landing jobs use it for the workflows that are not called (Pages, the share Worker's deploy, the YouTube Music
# contract, the release draft, the bundle caches' warm-up): a push made with the job's token starts nothing, a dispatch
# does. Needs GH_TOKEN with actions: write, and GITHUB_REPOSITORY.
set -eu

workflow=${1:-}
case "$workflow" in
  [a-z]*.yml) ;;
  *) echo "main-dispatch: name a workflow file in .github/workflows (pages.yml)" >&2; exit 2 ;;
esac
[ -f ".github/workflows/$workflow" ] || { echo "main-dispatch: no .github/workflows/$workflow" >&2; exit 2; }
gh workflow run "$workflow" --repo "${GITHUB_REPOSITORY:?GITHUB_REPOSITORY is not set}" --ref main
