#!/bin/sh
# The UI gates (`mise run check:ui`): performance first and alone, since its task-length budgets must not share the
# CPU with screenshots, then `fit` and every other gate, each in parallel (GATES in gates/playwright.config.mjs picks
# them, so a new gate needs no listing here or in CI).
# Extra arguments go to every run (e.g. `--grep lull`). docs/context/quality.md#ui-gates.
set -eu
cd "$(dirname "$0")/.."
npm ci --no-audit --no-fund --silent
npx playwright install chromium webkit >/dev/null
[ -f ../../web/vendor/webamp/webamp.bundle.min.js ] || sh ../../scripts/vendor-web.sh >/dev/null
status=0
node ../../scripts/loop-easings.mjs --check ../../web || status=1   # every stepped easing as the script makes it
GATES=perf npx playwright test -c gates/playwright.config.mjs --workers 1 "$@" || status=1
GATES=fit npx playwright test -c gates/playwright.config.mjs "$@" || status=1
GATES=rest npx playwright test -c gates/playwright.config.mjs "$@" || status=1
exit $status
