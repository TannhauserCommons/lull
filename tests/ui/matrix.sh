#!/bin/sh
# The visual matrix on the hub `hub.sh up` started: every route x five designs x day/night x phone/desktop
# (docs/context/quality.md, Visual matrix). SHARD=2/4 runs one quarter of it, as CI splits it over four machines; extra
# arguments go to Playwright. With no goldens committed it takes the pictures instead of comparing (playwright.config.mjs).
set -eu
cd "$(dirname "$0")"
exec npx playwright test ${SHARD:+--shard="$SHARD"} "$@"
