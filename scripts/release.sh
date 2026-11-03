#!/bin/sh
# Cuts a release: `mise run release 0.2.0`. Sets lull's version (the hub's workspace and the app; the
# app embeds the hub, ADR-0006), commits it on main and tags v0.2.0. Nothing leaves this clone:
# pushing the tag publishes, because .github/workflows/app.yml then builds the installers, the signed
# updater files, latest.json and the Android APK, and attaches them to the tag's GitHub Release.
# F-Droid and IzzyOnDroid pick the tag up from there (packaging/fdroid/).
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

# Android's versionCode: X.Y.Z is X*1000000 + Y*1000 + Z (Tauri's own formula, so v0.2.0 was 2000),
# rising with every release while Y and Z stay below 1000. F-Droid reads it from this file at the tag.
echo "$version" | grep -Eq '^[0-9]+\.[0-9]{1,3}\.[0-9]{1,3}$' || die "minor and patch must stay below 1000 (Android's versionCode)"
code=$(echo "$version" | awk -F. '{ print $1 * 1000000 + $2 * 1000 + $3 }')
old_code=$(sed -n 's/.*"versionCode": \([0-9]*\).*/\1/p' app/tauri.android.conf.json)
[ "$code" -gt "$old_code" ] || die "versionCode $code isn't above $old_code"

# The first `version = ` line of each manifest is the package's own ([workspace.package] at the root).
for manifest in Cargo.toml app/Cargo.toml; do
  sed -i "0,/^version = \".*\"$/s//version = \"$version\"/" "$manifest"
done
sed -i "s/\"versionCode\": [0-9]*/\"versionCode\": $code/" app/tauri.android.conf.json
cargo update --workspace --offline --quiet
cargo update --workspace --offline --quiet --manifest-path app/Cargo.toml

# What's new, for F-Droid and IzzyOnDroid (500 characters at most): written by hand beforehand, or
# drafted here from the feat and fix commits since the last release.
changelog=fastlane/metadata/android/en-US/changelogs/$code.txt
if [ ! -f "$changelog" ]; then
  since=$(git describe --tags --abbrev=0 --match 'v*' 2>/dev/null || true)
  git log --format=%s ${since:+"$since"..HEAD} \
    | sed -n 's/^\(feat\|fix\)([^)]*): //p' \
    | awk '{ line = "* " toupper(substr($0, 1, 1)) substr($0, 2); if (n + length(line) + 1 > 500) exit; print line; n += length(line) + 1 }' \
    > "$changelog"
  [ -s "$changelog" ] || echo "Smaller fixes." > "$changelog"
fi

git add Cargo.toml Cargo.lock app/Cargo.toml app/Cargo.lock app/tauri.android.conf.json "$changelog"
git commit -q -m "build(release): lull $version" -m "Sets the hub's and the app's version for the v$version release (scripts/release.sh)."
git tag -a "v$version" -m "lull $version"
echo "Tagged v$version (Android versionCode $code; what's new: $changelog, or write that file first to word it yourself). Run \`mise run check\`, then publish it with: git push origin main v$version"
