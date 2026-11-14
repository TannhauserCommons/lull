#!/bin/sh
# What the UI tests need installed: `sh tests/ui/setup.sh <browser>...` (chromium, webkit). `npm ci` in tests/ui, then
# Playwright's browsers. On CI also their system libraries and $PACKAGES (more Ubuntu packages, e.g. ffmpeg), through an apt
# folder the Actions cache can restore, so a slow Ubuntu mirror doesn't cost the job its time (on 2026-11-04 one fetched
# these 114 MB at 78 kB/s for 24 minutes): check.yml restores that cache before this runs, and only main saves it.
set -eu
[ $# -gt 0 ] || { echo "usage: setup.sh <browser>... (chromium, webkit)" >&2; exit 2; }
cd "$(dirname "$0")"

if [ -n "${CI:-}" ]; then
  # apt keeps what it downloads in a folder the cache can read and restore (its own is root's).
  mkdir -p "$HOME/apt-cache/partial"
  sudo chown _apt "$HOME/apt-cache/partial"
  printf 'Dir::Cache::Archives "%s/apt-cache";\nAPT::Keep-Downloaded-Packages "true";\n' "$HOME" \
    | sudo tee /etc/apt/apt.conf.d/99-ui-browsers-cache >/dev/null
fi

npm ci --no-audit --no-fund --silent
if [ -n "${CI:-}" ]; then
  npx playwright install --with-deps "$@"
else
  npx playwright install "$@" >/dev/null
fi
# shellcheck disable=SC2086
if [ -n "${PACKAGES:-}" ]; then sudo apt-get install -y $PACKAGES; fi
