#!/bin/sh
# Copies the pinned Webamp bundle into web/vendor/ (gitignored, embedded in the binary).
# Used by `mise run setup` and the Dockerfile, so both ship the same file.
set -eu
cd "$(dirname "$0")/../web"
npm ci --ignore-scripts --no-audit --no-fund
mkdir -p vendor/webamp
cp node_modules/webamp/built/webamp.bundle.min.js vendor/webamp/
