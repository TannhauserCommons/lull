#!/bin/sh
# A hub updating itself (tests/hub-contract/self-update.sh): two debug builds, a kit signed with a key made here and
# thrown away, a stand-in for GitHub's newest release. Only a debug build reads that key and that stand-in. The Tauri
# CLI that makes the key comes from npm (network). Rust comes from `mise run toolchain` before it.
set -eu
cd "$(dirname "$0")/../.."

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
bin=${CARGO_TARGET_DIR:-target}/debug/lull
cli=$(node -p "require('./app/package.json').devDependencies['@tauri-apps/cli']")
npx --yes "@tauri-apps/cli@$cli" signer generate --ci -p "" -w "$work/kit.key" -f

# Two versions of lull, installed as the package.
v1=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
v2=$(echo "$v1" | awk -F. '{ print $1 "." $2 "." $3 + 1 }')
v3=$(echo "$v1" | awk -F. '{ print $1 "." $2 "." $3 + 2 }')
kit=$(cat "$work/kit.key.pub")
# The build settings reach only the build: the hub refuses a LULL_* setting it doesn't know when it starts, and the
# contract starts the hub it builds. Extra arguments are more settings for one build.
build() {
  env LULL_DEV_KIT_KEY="$kit" LULL_DEV_RELEASES=http://127.0.0.1:9999/latest.json LULL_BUILD_INSTALL=package "$@" \
    cargo build --locked --bin lull
}
build
cp "$bin" "$work/lull-v1"
build LULL_BUILD_RELEASE="$v2" LULL_BUILD_CHANGES=0
cp "$bin" "$work/lull-v2"

# It updates, goes back from a kit that doesn't start, and refuses a changed one.
sh tests/hub-contract/self-update.sh "$work/lull-v1" "$v1" "$work/lull-v2" "$v2" "$v3" "$work/kit.key"
