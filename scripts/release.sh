#!/bin/sh
# Cuts a release: `mise run release 0.2.0`. Sets lull's version (the hub's workspace and the app; the
# app embeds the hub, ADR-0006), commits it on main and tags v0.2.0. Nothing leaves this clone:
# pushing the tag publishes, because .github/workflows/app.yml then builds the installers, the signed
# updater files, latest.json and the Android APK, and attaches them to the tag's GitHub Release.
set -eu

die() { echo "release: $*" >&2; exit 1; }

version=${1:-}
version=${version#v}
echo "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$' || die "give the new version as X.Y.Z"
[ "$(git rev-parse --abbrev-ref HEAD)" = main ] || die "release from main"
git diff --quiet && git diff --cached --quiet || die "commit or set aside your changes first"
git fetch -q origin
[ "$(git rev-parse HEAD)" = "$(git rev-parse origin/main)" ] || die "main isn't origin/main: pull or push first"
! git rev-parse -q --verify "refs/tags/v$version" >/dev/null || die "v$version exists already"

current=$(sed -n 's/^version = "\(.*\)"$/\1/p' app/Cargo.toml | head -n 1)
newest=$(printf '%s\n%s\n' "$current" "$version" | sort -V | tail -n 1)
[ "$version" != "$current" ] && [ "$newest" = "$version" ] || die "$version isn't newer than $current"

# The first `version = ` line of each manifest is the package's own ([workspace.package] at the root).
for manifest in Cargo.toml app/Cargo.toml; do
  sed -i "0,/^version = \".*\"$/s//version = \"$version\"/" "$manifest"
done
cargo update --workspace --offline --quiet
cargo update --workspace --offline --quiet --manifest-path app/Cargo.toml

git add Cargo.toml Cargo.lock app/Cargo.toml app/Cargo.lock
git commit -q -m "build(release): lull $version" -m "Sets the hub's and the app's version for the v$version release (scripts/release.sh)."
git tag -a "v$version" -m "lull $version"
echo "Tagged v$version. Run \`mise run check\`, then publish it with: git push origin main v$version"
