#!/bin/sh
# E2: one build of the Linux x64 bundles (deb, rpm, appimage), as app.yml's `bundle` job builds them, into $SPIKE/e2/<name>/.
#   sh scripts/spike/e2-build.sh <name> plain   as a build that is not a tag: no key, no config
#   sh scripts/spike/e2-build.sh <name> keyed   as the tag build: a throwaway key (empty password) and createUpdaterArtifacts
# Run after e2-setup.sh; every build uses the same app/target, one after the other.
# Left out on purpose: the tag build's "version" override (the new flow takes it from app/Cargo.toml) and Last.fm's keys
# (secrets): the same for both builds.
set -eu
cd "$(dirname "$0")/../.."
. scripts/spike/lib.sh
EXP=e2
spike_init

name=${1:?usage: e2-build.sh <name> plain|keyed}
mode=${2:?usage: e2-build.sh <name> plain|keyed}
dir=$OUT/$name
rm -rf "$dir" app/target/release/bundle
mkdir -p "$dir/files"
cd app

case "$mode" in
  plain)
    unset TAURI_SIGNING_PRIVATE_KEY TAURI_SIGNING_PRIVATE_KEY_PASSWORD
    config='{}'
    ;;
  keyed)
    [ -f "$OUT/key" ] || node_modules/.bin/tauri signer generate --ci --password "" --write-keys "$OUT/key"
    TAURI_SIGNING_PRIVATE_KEY=$(cat "$OUT/key")
    TAURI_SIGNING_PRIVATE_KEY_PASSWORD=
    export TAURI_SIGNING_PRIVATE_KEY TAURI_SIGNING_PRIVATE_KEY_PASSWORD
    config='{"bundle":{"createUpdaterArtifacts":true}}'
    ;;
  *) echo "e2-build: no mode '$mode' (plain or keyed)" >&2; exit 1 ;;
esac

start=$(date +%s)
{ set +e; node_modules/.bin/tauri build --ci --bundles deb,rpm,appimage --config "$config"; echo $? > "$dir/rc"; } 2>&1 | tee "$OUT/build-$name.log"
[ "$(cat "$dir/rc")" = 0 ] || { note "build $name ($mode) failed (build-$name.log)"; exit 1; }
seconds=$(( $(date +%s) - start ))

bundle=target/release/bundle
cp "$bundle"/deb/*.deb "$bundle"/rpm/*.rpm "$bundle"/appimage/*.AppImage "$dir/files/"
for sig in "$bundle"/*/*.sig; do
  [ ! -f "$sig" ] || cp "$sig" "$dir/files/"
done
# Everything the bundler left, but the AppDir it unpacked.
(cd "$bundle" && find . -type f -not -path '*.AppDir/*' | sort) > "$dir/listing.txt"
note "build $name ($mode, config $config): $seconds s; made: $(tr '\n' ' ' < "$dir/listing.txt")"
