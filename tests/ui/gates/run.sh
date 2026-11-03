#!/bin/sh
# The UI gates (`mise run check:ui`): performance first and alone, since its task-length budgets must not share the
# CPU with screenshots, then the key screens, accessibility, covers and the chrome's fit at every width in parallel.
# Extra arguments go to every run (e.g. `--grep lull`). docs/context/quality.md#ui-gates.
set -eu
cd "$(dirname "$0")/.."
npm ci --no-audit --no-fund --silent
npx playwright install chromium >/dev/null
[ -f ../../web/vendor/webamp/webamp.bundle.min.js ] || sh ../../scripts/vendor-web.sh >/dev/null
status=0
npx playwright test -c gates/playwright.config.mjs --project perf --workers 1 "$@" || status=1
npx playwright test -c gates/playwright.config.mjs --project screens --project a11y --project covers --project fit --project classic "$@" || status=1
exit $status
