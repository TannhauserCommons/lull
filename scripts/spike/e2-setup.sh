#!/bin/sh
# E2 and E3: what the desktop build needs besides the Rust toolchain: Tauri's system packages (Linux) and the pinned
# Tauri CLI from app/package-lock.json.
set -eu
cd "$(dirname "$0")/../.."
. scripts/ci/lib.sh

# shellcheck disable=SC2086 # one word per package, on purpose
apt_install $LINUX_DEPS
(cd app && npm ci --no-audit --no-fund --ignore-scripts)
app/node_modules/.bin/tauri --version
