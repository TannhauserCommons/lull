#!/bin/sh
# E2: compares two builds of e2-build.sh: the files inside their .deb and AppImage (packaging dates differ between any two
# builds, so whole-file hashes prove nothing), the main binary byte by byte, and which files the bundler made.
#   sh scripts/spike/e2-compare.sh <x> <y>      writes $SPIKE/e2/<x>-<y>.paths, the files whose bytes differ, and
#                                               differs=true|false to GITHUB_OUTPUT
set -eu
cd "$(dirname "$0")/../.."
. scripts/spike/lib.sh
EXP=e2
spike_init

x=${1:?usage: e2-compare.sh <x> <y>}
y=${2:?usage: e2-compare.sh <x> <y>}

# A build's .deb and AppImage unpacked, and the sha256 of every file in them.
unpack() {
  b=$OUT/$1
  t=$b/tree
  [ ! -f "$t/hashes.txt" ] || return 0
  mkdir -p "$t/deb" "$t/appimage"
  dpkg-deb -x "$b"/files/*.deb "$t/deb"
  image=$(ls "$b"/files/*.AppImage)
  chmod +x "$image"
  (cd "$t/appimage" && "$image" --appimage-extract > /dev/null)
  (cd "$t" && {
    find deb appimage -type f -exec sha256sum {} +
    find deb appimage -type l -exec sh -c 'for l; do printf "link  %s -> %s\n" "$l" "$(readlink "$l")"; done' _ {} +
  } | sort -k2 > hashes.txt)
}
unpack "$x"
unpack "$y"

paths=$OUT/$x-$y.paths
diff "$OUT/$x/tree/hashes.txt" "$OUT/$y/tree/hashes.txt" \
  | sed -n -e 's/^[<>] [0-9a-f]*  //p' -e 's/^[<>] link  //p' | sed 's/ -> .*//' | sort -u > "$paths" || true
note "$x vs $y: $(wc -l < "$paths") file(s) differ, are added or are removed among those inside the .deb and the AppImage"
head -n 20 "$paths" | sed 's/^/    /' | tee -a "$OUT/evidence.txt"

# The main binary: the .deb's usr/bin/lull.
bx=$OUT/$x/tree/deb/usr/bin/lull
by=$OUT/$y/tree/deb/usr/bin/lull
note "$x vs $y usr/bin/lull: $(sha256sum < "$bx" | cut -d' ' -f1) ($(wc -c < "$bx") bytes) vs $(sha256sum < "$by" | cut -d' ' -f1) ($(wc -c < "$by") bytes)"
if ! cmp -s "$bx" "$by"; then
  note "$x vs $y usr/bin/lull: $(cmp -l "$bx" "$by" 2>/dev/null | wc -l) byte positions differ (and the sizes above)"
  strings -a -n 8 "$bx" | sort -u > "$OUT/$x.strings"
  strings -a -n 8 "$by" | sort -u > "$OUT/$y.strings"
  note "$x vs $y strings only in $x ($(comm -23 "$OUT/$x.strings" "$OUT/$y.strings" | wc -l)) / only in $y ($(comm -13 "$OUT/$x.strings" "$OUT/$y.strings" | wc -l)); the first of each:"
  comm -23 "$OUT/$x.strings" "$OUT/$y.strings" | head -n 15 | cut -c 1-200 | sed 's/^/    only in x: /' | tee -a "$OUT/evidence.txt"
  comm -13 "$OUT/$x.strings" "$OUT/$y.strings" | head -n 15 | cut -c 1-200 | sed 's/^/    only in y: /' | tee -a "$OUT/evidence.txt"
  note "$x, $y: occurrences of createUpdaterArtifacts / create_updater: $(grep -ci 'createUpdaterArtifacts\|create_updater' "$OUT/$x.strings" || true) / $(grep -ci 'createUpdaterArtifacts\|create_updater' "$OUT/$y.strings" || true)"
fi

for n in "$x" "$y"; do
  note "$n files: $(cd "$OUT/$n/files" && for f in *; do printf '%s %s %s; ' "$f" "$(wc -c < "$f")" "$(sha256sum "$f" | cut -c 1-16)"; done)"
done
note "$x vs $y files the bundler made (< only $x, > only $y): $(diff "$OUT/$x/listing.txt" "$OUT/$y/listing.txt" | grep '^[<>]' | tr '\n' ';' || true)"

if [ -n "${GITHUB_OUTPUT:-}" ]; then
  [ -s "$paths" ] && echo "differs=true" >> "$GITHUB_OUTPUT" || echo "differs=false" >> "$GITHUB_OUTPUT"
fi
