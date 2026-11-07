#!/bin/sh
# Writes lull-bin's PKGBUILD for a release from packaging/aur/PKGBUILD.in (docs/context/desktop.md, The AUR):
# the version, the checksums of the release's .deb files and their minisign
# signatures, the key they are signed with (tauri.conf.json's updater key: one source) and Arch's packages it needs.
#   sh scripts/aur-pkgbuild.sh <version> <folder with the release's lull_<v>_{amd64,arm64}.deb[.sig]> <out dir>
# The folder is the release job's bundles, or a download of a published release's assets.
set -eu
version=$1 bundles=$2 out=$3
repo=$(cd "$(dirname "$0")/.." && pwd)

sum() {
  [ -f "$bundles/$1" ] || { echo "aur-pkgbuild: $bundles/$1 is missing" >&2; exit 1; }
  sha256sum "$bundles/$1" | cut -d' ' -f1
}
# The second line of the base64 key file is the key itself.
key=$(node -e "const c = require(process.argv[1]); console.log(Buffer.from(c.plugins.updater.pubkey, 'base64').toString().split('\n')[1])" \
  "$repo/app/tauri.conf.json")
# What lull links on Arch, as namcap reads it from the binary (aur.yml fails on a difference); what it loads as it runs,
# which namcap can't see: the tray's AppIndicator, and the programs it runs (ADR-0064), Arch's yt-dlp and deno and the
# AUR's slskd-bin; and the icon theme its icons go into.
depends="'webkit2gtk-4.1' 'gtk3' 'glib2' 'pango' 'cairo' 'gdk-pixbuf2' 'libsoup3' 'dbus' 'alsa-lib' 'libgcc' 'glibc' 'libayatana-appindicator' 'yt-dlp' 'deno' 'slskd-bin' 'hicolor-icon-theme'"

mkdir -p "$out"
sed -e "s|@VERSION@|$version|" \
    -e "s|@DEPENDS@|$depends|" \
    -e "s|@MINISIGN_KEY@|$key|" \
    -e "s|@SHA256_AMD64_DEB@|$(sum "lull_${version}_amd64.deb")|" \
    -e "s|@SHA256_AMD64_SIG@|$(sum "lull_${version}_amd64.deb.sig")|" \
    -e "s|@SHA256_ARM64_DEB@|$(sum "lull_${version}_arm64.deb")|" \
    -e "s|@SHA256_ARM64_SIG@|$(sum "lull_${version}_arm64.deb.sig")|" \
    "$repo/packaging/aur/PKGBUILD.in" > "$out/PKGBUILD"
if grep -q '@[A-Z0-9_]*@' "$out/PKGBUILD"; then
  echo "aur-pkgbuild: a placeholder is left in $out/PKGBUILD" >&2
  exit 1
fi
