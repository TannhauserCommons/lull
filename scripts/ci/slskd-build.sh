#!/bin/sh
# lull's slskd for Windows (ADR-0062): app/slskd/build.sh, run twice in different folders, must give the same bytes,
# and the bytes app/build.rs pins once it pins this build. Builds from upstream's pinned release (network; needs git,
# curl and python3, on Linux). The `slskd-build` job of .github/workflows/check.yml runs it, and slskd-windows.yml before
# a publish: with $SLSKD_OUT set, the first build's zips are kept there.
set -eu
cd "$(dirname "$0")/../.."
root=$PWD

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
sh app/slskd/build.sh "$work/first"
sh app/slskd/build.sh "$work/second"
cd "$work/first"
# A build that made nothing must not pass: with no zip the pattern stays literal and both hashes would be empty.
set -- slskd-*.zip
[ -e "$1" ] || { echo "::error::app/slskd/build.sh made no slskd-*.zip"; exit 1; }
for zip in "$@"; do
  a=$(sha256sum "$zip" | cut -d' ' -f1)
  b=$(sha256sum "../second/$zip" | cut -d' ' -f1)
  [ -n "$a" ] && [ -n "$b" ] || { echo "::error::$zip has no hash in one of the two builds ('$a', '$b')"; exit 1; }
  [ "$a" = "$b" ] || { echo "::error::$zip differs between two builds ($a, $b): the recipe isn't reproducible"; exit 1; }
  # A pin names its zip in the URL and its sha256 beside it (app/build.rs, SLSKD).
  pinned=$(grep -A3 "$zip\"" "$root/app/build.rs" | grep -o '[0-9a-f]\{64\}' | head -n 1 || true)
  if [ -z "$pinned" ]; then
    echo "$zip: $a (not pinned in app/build.rs yet)"
  elif [ "$pinned" != "$a" ]; then
    echo "::error::$zip builds to $a, but app/build.rs pins $pinned"
    exit 1
  else
    echo "$zip: $a, as pinned"
  fi
done
if [ -n "${SLSKD_OUT:-}" ]; then
  mkdir -p "$SLSKD_OUT"
  cp "$@" "$SLSKD_OUT/"
fi
