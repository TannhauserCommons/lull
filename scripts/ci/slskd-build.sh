#!/bin/sh
# lull's slskd for Windows (ADR-0062): app/slskd/build.sh, run twice in different folders, must give the same bytes,
# and the bytes app/build.rs pins once it pins this build. Builds from upstream's pinned release (network; needs git,
# curl and python3, on Linux). The `slskd-build` job of .github/workflows/check.yml runs it.
set -eu
cd "$(dirname "$0")/../.."
root=$PWD

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
sh app/slskd/build.sh "$work/first"
sh app/slskd/build.sh "$work/second"
cd "$work/first"
for zip in slskd-*.zip; do
  a=$(sha256sum "$zip" | cut -d' ' -f1)
  b=$(sha256sum "../second/$zip" | cut -d' ' -f1)
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
