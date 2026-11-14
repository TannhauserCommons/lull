#!/bin/sh
# The app's checks (app/): the .js syntax of the desktop wrapper, fmt, clippy `-D warnings` and tests. The `app` job of
# .github/workflows/check.yml runs it on Linux x64 and Windows x64 (Git's sh, which the Windows runners have
# on PATH). On Linux it first installs Tauri's system packages; app/build.rs downloads the sidecars it
# bundles (network, sha256-pinned). Rust comes from `mise run toolchain` before it.
set -eu
cd "$(dirname "$0")/../.."
. scripts/ci/lib.sh

[ "$(uname -s)" != Linux ] || apt_install $LINUX_DEPS
node --check web/desktop/desktop.js
cd app
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
