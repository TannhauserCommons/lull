#!/bin/sh
# E2's verdict. PASS = the keyed build (B) differs from the plain one (A) only by added .sig files, plus differences that two
# plain builds (A and A2, built only when A and B differ) also show.
set -eu
cd "$(dirname "$0")/../.."
. scripts/spike/lib.sh
EXP=e2
spike_init

ab=$OUT/A-B.paths
base=$OUT/A-A2.paths
[ -f "$ab" ] || { note "no comparison of A and B"; verdict FAIL; exit 0; }
[ -f "$base" ] || : > "$base"

# Files the keyed build made that the plain one did not, and files it lacks.
diff "$OUT/A/listing.txt" "$OUT/B/listing.txt" | grep '^[<>]' > "$OUT/listing.diff" || true
bad=$(grep -v '^> .*\.sig$' "$OUT/listing.diff" || true)
sigs=$(sed -n 's/^> .*\///p' "$OUT/listing.diff" | tr '\n' ' ')
note "files only the keyed build made: ${sigs:-none}"
[ -z "$bad" ] || note "other differences in what the bundler made: $(echo "$bad" | tr '\n' ';')"
appimage_sig=$(grep -c '\.AppImage\.sig$' "$OUT/listing.diff" || true)

unexplained=$(comm -23 "$ab" "$base")
note "files inside the .deb and AppImage that differ between plain and keyed: $(wc -l < "$ab"); between two plain builds: $(wc -l < "$base"); only between plain and keyed: $(printf '%s' "$unexplained" | grep -c . || true)"
[ -z "$unexplained" ] || echo "$unexplained" | head -n 20 | sed 's/^/    only plain vs keyed: /' | tee -a "$OUT/evidence.txt"
if [ -f "$OUT/A2/listing.txt" ]; then
  note "baseline: A2 was built (A and B differed), so plain-vs-plain differences count as not caused by signing"
else
  note "baseline: A and B had the same files inside, so A2 was not built"
fi

if [ -z "$bad" ] && [ -z "$unexplained" ] && [ "$appimage_sig" -ge 1 ]; then
  verdict PASS
else
  [ "$appimage_sig" -ge 1 ] || note "the keyed build made no AppImage .sig: createUpdaterArtifacts did not do what the hypothesis assumes (or the spike's key setup is wrong)"
  verdict FAIL
fi
