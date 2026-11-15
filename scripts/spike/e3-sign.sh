#!/bin/sh
# E3: signatures made after the build verify. Takes the files of E2's artifact (spike-2-files): the plain build's AppImage and
# .deb, the keyed build's AppImage with the .sig its bundler made, and the throwaway key. Signs each afterwards with the pinned
# Tauri CLI (app.yml's `signer sign` over a .deb) and verifies every .sig with
#   - minisign -V (the C tool Arch's PKGBUILD uses): the .sig and the key decoded from Tauri's base64 wrapping;
#   - scripts/spike/verify, a Rust binary on minisign-verify 0.3.0, the crate crates/server/src/kit.rs and
#     tauri-plugin-updater 2.14.1 use: it runs kit.rs's `signed_by` steps (allow_legacy false) and the plugin's
#     `verify_signature` steps (allow_legacy true) over the file.
# PASS = every after-the-fact signature verifies with minisign and the updater's mode, and so does the bundler's.
set -eu
cd "$(dirname "$0")/../.."
. scripts/ci/lib.sh
. scripts/spike/lib.sh
EXP=e3
spike_init

e2=$SPIKE/e2
tauri=$PWD/app/node_modules/.bin/tauri
pub=$e2/key.pub
apt_install minisign
cap cargo build --release --manifest-path scripts/spike/verify/Cargo.toml --target-dir "$SPIKE/verify-target"
verify=$SPIKE/verify-target/release/spike-verify
# The second line of the decoded public key file is the key itself (scripts/aur-pkgbuild.sh).
line=$(base64 -d "$pub" | sed -n 2p)
note "throwaway public key: $line"

TAURI_SIGNING_PRIVATE_KEY=$(cat "$e2/key")
TAURI_SIGNING_PRIVATE_KEY_PASSWORD=
export TAURI_SIGNING_PRIVATE_KEY TAURI_SIGNING_PRIVATE_KEY_PASSWORD

app_a=$(ls "$e2"/A/files/*.AppImage)
deb_a=$(ls "$e2"/A/files/*.deb)
app_b=$(ls "$e2"/B/files/*.AppImage)
bundler=$OUT/bundler-made.AppImage.sig
[ -f "$app_b.sig" ] || { note "the keyed build's AppImage has no .sig from its bundler"; verdict FAIL; exit 1; }
mv "$app_b.sig" "$bundler"

for f in "$app_a" "$deb_a" "$app_b"; do
  cap "$tauri" signer sign --password "" "$f"
done

fails=0
# check <label> <file> <sig> [reject]: every verifier must accept (or, for a negative control, reject).
check() {
  label=$1 file=$2 sig=$3 want=${4:-accept}
  base64 -d < "$sig" > "$sig.minisig"
  if cap minisign -V -m "$file" -x "$sig.minisig" -P "$line"; then m=accept; else m=reject; fi
  out=$("$verify" "$pub" "$file" "$sig") || out=
  hub=reject
  printf '%s\n' "$out" | grep -qx 'hub=ok' && hub=accept
  upd=reject
  printf '%s\n' "$out" | grep -qx 'updater=ok' && upd=accept
  printf '%s\n' "$out" >> "$RAW"
  ok=no
  if [ "$want" = accept ] && [ "$m" = accept ] && [ "$upd" = accept ]; then ok=yes; fi
  if [ "$want" = reject ] && [ "$m" = reject ] && [ "$upd" = reject ] && [ "$hub" = reject ]; then ok=yes; fi
  note "$label: minisign -V $m; minisign-verify as the hub (kit.rs) $hub, as the updater plugin $upd; expected $want: $ok"
  [ "$ok" = yes ] || fails=$((fails + 1))
}

check "plain build's AppImage, signed after the build" "$app_a" "$app_a.sig"
check "plain build's .deb, signed after the build" "$deb_a" "$deb_a.sig"
check "keyed build's AppImage, signed after the build" "$app_b" "$app_b.sig"
check "keyed build's AppImage, signature made by the bundler" "$app_b" "$bundler"
check "negative control, the .deb's signature over the AppImage" "$app_a" "$deb_a.sig" reject

note "after-the-fact signature of the keyed AppImage vs the bundler's: $(wc -c < "$app_b.sig") vs $(wc -c < "$bundler") bytes; trusted comments:"
note "    after the fact: $(base64 -d < "$app_b.sig" | sed -n 3p | cut -c 1-200)"
note "    bundler:        $(base64 -d < "$bundler" | sed -n 3p | cut -c 1-200)"
if cmp -s "$app_a" "$app_b"; then
  note "the plain and the keyed build's AppImage are the same bytes"
else
  note "the plain and the keyed build's AppImage differ in bytes (E2 says whether that is only packaging)"
fi
if [ "$fails" = 0 ]; then verdict PASS; else verdict FAIL; fi
