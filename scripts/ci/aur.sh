#!/bin/sh
# lull-bin, lull in the AUR (docs/context/desktop.md, The AUR): the PKGBUILD for a release (scripts/aur-pkgbuild.sh), built
# with makepkg in Arch's own container on Arch's WebKitGTK, checked by namcap, installed with pacman, started under Xvfb
# until its window draws, and run as its own hub with its slskd (packaging/aur/check-installed.sh); the aarch64 sources
# are downloaded and checked too (checksums, minisign). Nothing is pushed to the AUR.
#
# Runs as root in an Arch container (the `aur` job of .github/workflows/check.yml). The release is $VERSION (1.2.3 or
# v1.2.3), else the newest one of $GITHUB_REPOSITORY; $GH_TOKEN, when set, signs the lookup of it.
set -eu
cd "$(dirname "$0")/../.."
root=$PWD

pacman -Syu --noconfirm --needed git nodejs minisign namcap xorg-server-xvfb xorg-xauth imagemagick dbus procps-ng

: "${GITHUB_REPOSITORY:?set GITHUB_REPOSITORY (owner/name) to find the release}"
fetch() { # <url> [output file]
  if [ -n "${GH_TOKEN:-}" ]; then
    curl -fsSL -H "Authorization: Bearer $GH_TOKEN" ${2:+-o "$2"} "$1"
  else
    curl -fsSL ${2:+-o "$2"} "$1"
  fi
}

# The release
version=${VERSION:-}
version=${version#v}
if [ -z "$version" ]; then
  version=$(fetch "https://api.github.com/repos/$GITHUB_REPOSITORY/releases/latest" \
    | node -e "let s=''; process.stdin.on('data', d => s += d).on('end', () => console.log(JSON.parse(s).tag_name.replace(/^v/, '')))")
fi
mkdir -p bundles
for f in "lull_${version}_amd64.deb" "lull_${version}_amd64.deb.sig" "lull_${version}_arm64.deb" "lull_${version}_arm64.deb.sig"; do
  fetch "https://github.com/$GITHUB_REPOSITORY/releases/download/v$version/$f" "bundles/$f"
done
sh scripts/aur-pkgbuild.sh "$version" bundles aur

# makepkg won't run as root, and its user can't install what the package needs: root does, from .SRCINFO.
useradd -m builder
chown -R builder aur
cd aur
su builder -c 'makepkg --printsrcinfo > .SRCINFO'
pacman -S --noconfirm --needed $(sed -n 's/^\t\(make\)\?depends = //p' .SRCINFO)
su builder -c 'makepkg -f --noconfirm'
# aarch64: its sources downloaded and checked (checksums, then prepare's minisign check), on this x86_64 machine
sed 's/^CARCH=.*/CARCH="aarch64"/' /etc/makepkg.conf > /tmp/aarch64.conf
su builder -c 'makepkg -f --nobuild --nodeps --config /tmp/aarch64.conf'

# namcap: the depends are exactly what the binaries link: one missing, or one not needed, fails it. AppIndicator is
# loaded at run time (the tray), which namcap can't see.
namcap PKGBUILD
namcap -i lull-bin-*.pkg.tar.zst > namcap.txt || { cat namcap.txt; exit 1; }
cat namcap.txt
if grep -E ' E: | W: .*(Referenced library|detected and not included|implicitly satisfied|may not be needed)' namcap.txt \
  | grep -v "may not be needed ('libayatana-appindicator')"; then
  exit 1
fi
cd "$root"

pacman -U --noconfirm aur/lull-bin-*.pkg.tar.zst
# lull starts, draws, and runs its own hub and slskd, on Arch
test "$(readlink -f /usr/bin/lull)" = /usr/lib/lull/lull
for tool in slskd yt-dlp deno; do test -x /usr/lib/lull/$tool; done
useradd -m listener
su listener -c 'xvfb-run -a -s "-screen 0 1440x900x24" dbus-run-session -- sh packaging/aur/check-installed.sh'
