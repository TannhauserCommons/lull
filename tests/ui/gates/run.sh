#!/bin/sh
# The UI gates (`mise run check:ui`): performance first and alone, since its task-length budgets must not share the
# CPU with screenshots, then `fit` and every other gate, each in parallel (GATES in gates/playwright.config.mjs picks
# them, so a new gate needs no listing here or in CI).
# GATES=perf|fit|rest runs that group only, and SHARD=2/3 one share of it: how CI splits the gates over seven machines
# (check.yml). Extra arguments go to every run (e.g. `--grep lull`). docs/context/quality.md#ui-gates.
set -eu
cd "$(dirname "$0")/.."
case "${GATES:-}" in
  '' | perf | fit | rest) ;;
  *) echo "GATES is perf, fit or rest (or unset for all three): $GATES" >&2; exit 2 ;;
esac
# `perf` and `fit` run in Chromium alone. (check.yml restores the Actions cache of exactly these browsers.)
case "${GATES:-}" in perf | fit) browsers=chromium ;; *) browsers="chromium webkit" ;; esac
# shellcheck disable=SC2086
sh setup.sh $browsers
[ -f ../../web/vendor/webamp/webamp.bundle.min.js ] || sh ../../scripts/vendor-web.sh >/dev/null
status=0
# Every stepped easing as the script makes it: belongs to no group, so it runs once, with the first performance shard.
case "${GATES:-perf}:${SHARD:-1/1}" in
  perf:1/*) node ../../scripts/loop-easings.mjs --check ../../web || status=1 ;;
esac
gates() { # <group> [Playwright arguments]: that group, when it is asked for or no group is
  group=$1; shift
  [ -z "${GATES:-}" ] || [ "$GATES" = "$group" ] || return 0
  GATES=$group npx playwright test -c gates/playwright.config.mjs ${SHARD:+--shard="$SHARD"} "$@" || status=1
}
gates perf --workers 1 "$@"
gates fit "$@"
gates rest "$@"
exit $status
