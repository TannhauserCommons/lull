#!/bin/sh
# The listener's logins across an update (tests/hub-contract/secrets-update.sh, ADR-0091): the last release before the
# credentials' vault, built from its tag, updated to this commit with the `secrets` switch on, through a kit that fails
# its health window and then one that starts. Fetches that tag from origin and the Tauri CLI from npm (network). Rust
# comes from `mise run toolchain` before it.
set -eu
cd "$(dirname "$0")/../.."

RELEASE=0.2.6
work=$(mktemp -d)
trap 'git worktree remove --force "$work/release" 2>/dev/null || true; rm -rf "$work"' EXIT
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$PWD/target}
bin=$CARGO_TARGET_DIR/debug/lull
cli=$(node -p "require('./app/package.json').devDependencies['@tauri-apps/cli']")
npx --yes "@tauri-apps/cli@$cli" signer generate --ci -p "" -w "$work/kit.key" -f

# The release and this commit, installed as the package.
v2=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1 | awk -F. '{ print $1 "." $2 "." $3 + 1 }')
v3=$(echo "$v2" | awk -F. '{ print $1 "." $2 "." $3 + 1 }')
kit=$(cat "$work/kit.key.pub")
# The build settings reach only the build: the hub refuses a LULL_* setting it doesn't know when it starts, and the
# contract starts the hub it builds. Extra arguments are more settings for one build.
build() {
  env LULL_DEV_KIT_KEY="$kit" LULL_DEV_RELEASES=http://127.0.0.1:9999/latest.json LULL_BUILD_INSTALL=package "$@" \
    cargo build --locked --bin lull
}
git fetch -q --depth 1 origin tag "v$RELEASE"
git worktree add -q "$work/release" "v$RELEASE"
(cd "$work/release" && build)
cp "$bin" "$work/lull-release"
build LULL_BUILD_RELEASE="$v2" LULL_BUILD_CHANGES=0
cp "$bin" "$work/lull-v2"

# The logins survive a failed update and move, with nothing asked, on one that starts.
sh tests/hub-contract/secrets-update.sh "$work/lull-release" "$RELEASE" "$work/lull-v2" "$v2" "$v3" "$work/kit.key"
