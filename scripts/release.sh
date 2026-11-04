#!/bin/sh
# Cuts a release: `mise run release 0.2.0` (ADR-0097). Finds the newest commit on origin/main that every
# required check passed, makes the release commit on it (lull's version in the hub's workspace and the
# app, which embeds the hub, ADR-0006; Android's versionCode; what's new) and tags it v0.2.0. Nothing
# leaves this clone: pushing the tag publishes, because .github/workflows/app.yml then builds the
# installers, the signed updater files, latest.json and the Android APK, and attaches them to the tag's
# GitHub Release. F-Droid and IzzyOnDroid pick the tag up from there (packaging/fdroid/). Pushing the
# commit to the queue lands the version on main.
set -eu

die() { echo "release: $*" >&2; exit 1; }

version=${1:-}
version=${version#v}
echo "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$' || die "give the new version as X.Y.Z"
# Android's versionCode: X.Y.Z is X*1000000 + Y*1000 + Z (Tauri's own formula, so v0.2.0 was 2000),
# rising with every release while Y and Z stay below 1000. F-Droid reads it from this file at the tag.
echo "$version" | grep -Eq '^[0-9]+\.[0-9]{1,3}\.[0-9]{1,3}$' || die "minor and patch must stay below 1000 (Android's versionCode)"
code=$(echo "$version" | awk -F. '{ print $1 * 1000000 + $2 * 1000 + $3 }')
changelog=fastlane/metadata/android/en-US/changelogs/$code.txt

git fetch -q --tags origin main
! git rev-parse -q --verify "refs/tags/v$version" >/dev/null || die "v$version exists already"
repo=$(gh repo view --json nameWithOwner --jq .nameWithOwner)

# The newest green commit: the queue's status (ADR-0095), or before the queue, every check run passed.
green() {
  for sha in $(git rev-list --first-parent -n 200 origin/main); do
    # Only GitHub Actions can set it: main's ruleset requires it from there (ADR-0095).
    queue=$(gh api "repos/$repo/commits/$sha/statuses" \
      --jq '[.[] | select(.context == "queue" and .creator.login == "github-actions[bot]")][0].state // ""')
    [ "$queue" = success ] && { echo "$sha"; return; }
    [ -z "$queue" ] || continue
    runs=$(gh api "repos/$repo/commits/$sha/check-runs?per_page=100" \
      --jq '[.check_runs[].conclusion] | if length > 0 and all(. == "success" or . == "skipped" or . == "neutral") then "green" else "" end')
    [ "$runs" = green ] && { echo "$sha"; return; }
  done
  die "no green commit in main's last 200"
}
base=$(green)
echo "releasing from $(git log -1 --format='%h %s' "$base")"
[ "$base" = "$(git rev-parse origin/main)" ] || echo "  (main's tip $(git rev-parse --short origin/main) isn't green yet; the newer commits wait for the next release)"

# What's new, for F-Droid and IzzyOnDroid (500 characters at most): the commit's own, written by hand
# beforehand, or a draft from the feat and fix commits since the last release. A draft stops here for
# a person to read and edit; run again to tag with it. The previous release is the highest v* tag: it
# may sit a commit off main's line, where `git describe` wouldn't find it.
draft=""
if ! git cat-file -e "$base:$changelog" 2>/dev/null; then
  if [ ! -f "$changelog" ]; then
    since=$(git tag -l 'v*' --sort=-v:refname | head -n 1)
    mkdir -p "$(dirname "$changelog")"
    git log --format=%s ${since:+"$since"..}"$base" \
      | sed -n 's/^\(feat\|fix\)([^)]*): //p' \
      | awk '{ line = "* " toupper(substr($0, 1, 1)) substr($0, 2); if (n + length(line) + 1 > 500) exit; print line; n += length(line) + 1 }' \
      > "$changelog"
    [ -s "$changelog" ] || echo "Smaller fixes." > "$changelog"
    echo "Drafted what's new in $changelog ($(wc -c < "$changelog") of 500 characters):"
    sed 's/^/  /' "$changelog"
    die "read and edit it, then run \`mise run release $version\` again"
  fi
  [ "$(wc -c < "$changelog")" -le 500 ] || die "$changelog is over 500 characters"
  draft=$changelog
fi

# The release commit, made on the green commit in a worktree of its own, so this checkout stays as it is.
# Inside the repository's .git: a path every git understands, Git for Windows' included.
work=$(mktemp -d "$(git rev-parse --git-common-dir)/release.XXXXXX")
trap 'git worktree remove --force "$work" 2>/dev/null || true' EXIT
git worktree add -q --detach "$work" "$base"
[ -z "$draft" ] || cp "$draft" "$work/$changelog"
(
  cd "$work"
  current=$(sed -n 's/^version = "\(.*\)"$/\1/p' app/Cargo.toml | head -n 1)
  newest=$(printf '%s\n%s\n' "$current" "$version" | sort -V | tail -n 1)
  [ "$version" != "$current" ] && [ "$newest" = "$version" ] || die "$version isn't newer than $current"
  old_code=$(sed -n 's/.*"versionCode": \([0-9]*\).*/\1/p' app/tauri.android.conf.json)
  [ "$code" -gt "$old_code" ] || die "versionCode $code isn't above $old_code"

  # The first `version = ` line of each manifest is the package's own ([workspace.package] at the root).
  for manifest in Cargo.toml app/Cargo.toml; do
    sed -i "0,/^version = \".*\"$/s//version = \"$version\"/" "$manifest"
  done
  sed -i "s/\"versionCode\": [0-9]*/\"versionCode\": $code/" app/tauri.android.conf.json
  cargo update --workspace --offline --quiet
  cargo update --workspace --offline --quiet --manifest-path app/Cargo.toml

  git add Cargo.toml Cargo.lock app/Cargo.toml app/Cargo.lock app/tauri.android.conf.json "$changelog"
  git commit -q -m "build(release): lull $version" -m "Sets the hub's and the app's version for the v$version release, on $(git rev-parse --short "$base"), main's newest green commit (scripts/release.sh)."
  git tag -a "v$version" -m "lull $version"
)
[ -z "$draft" ] || rm "$draft"
echo "Tagged v$version on $(git rev-parse --short "v$version^{commit}") (Android versionCode $code). Publish the release and land the version on main with:"
echo "  git push origin v$version v$version^{commit}:refs/heads/queue/release-$version"
