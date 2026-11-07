#!/bin/sh
# Builds the Linux server package (ADR-0090) from a lull binary built for this machine's architecture:
# lull-server_<version>_<arch>.deb, lull-server-<version>.<arch>.rpm and lull-server_<version>_linux-<arch>.tar.gz in
# <out> (the desktop app's own packages are lull_…), and the hub kit a hub updates itself with (ADR-0090, kit.rs):
# lull-hub-<version>-linux-<arch>-<program>.gz for lull, slskd, yt-dlp and Deno, and the manifest naming their
# SHA-256s, lull-hub-<version>-linux-<arch>.json, which a release signs.
#
#   sh packaging/linux/build.sh <lull binary> <version> <out>
# Needs node, unzip and curl; fetches the nfpm it builds the .deb and .rpm with, pinned below.
set -eu
lull=$1 version=$2 out=$3
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
# nfpm (https://github.com/goreleaser/nfpm): one config for both formats. Its release's own sha256s.
nfpm_version=2.48.0
case "$(uname -m)" in
  x86_64)
    target=x86_64-unknown-linux-gnu arch=amd64 rpm_arch=x86_64 tar_arch=x64
    nfpm_asset=nfpm_${nfpm_version}_Linux_x86_64.tar.gz
    nfpm_sha256=6d7f9a28b2f397934403950fcbba83f598b08d02edbe79a0815b3b4897341750 ;;
  aarch64)
    target=aarch64-unknown-linux-gnu arch=arm64 rpm_arch=aarch64 tar_arch=arm64
    nfpm_asset=nfpm_${nfpm_version}_Linux_arm64.tar.gz
    nfpm_sha256=a9a75213763783b7669719e505474c748c5ad7dc66591a195619deb50f85c2f4 ;;
  *) echo "no lull package for $(uname -m)" >&2; exit 1 ;;
esac
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
curl -fsSL -o "$stage/nfpm.tar.gz" "https://github.com/goreleaser/nfpm/releases/download/v$nfpm_version/$nfpm_asset"
echo "$nfpm_sha256  $stage/nfpm.tar.gz" | sha256sum -c --quiet
tar -C "$stage" -xzf "$stage/nfpm.tar.gz" nfpm
nfpm=$stage/nfpm
mkdir -p "$stage/lib" "$stage/licenses" "$out"
node "$here/sidecars.mjs" "$target" "$stage/lib"
install -m 0755 "$lull" "$stage/lib/lull"
mv "$stage/lib/slskd-LICENSE.txt" "$stage/licenses/slskd-LICENSE.txt"
cp "$root/LICENSE" "$stage/licenses/LICENSE.txt"
cp "$root/app/licenses/THIRD-PARTY.md" "$stage/licenses/THIRD-PARTY.md"
cp "$here/lull.service" "$here/postinstall.sh" "$here/preremove.sh" "$here/postremove.sh" "$here/install.sh" "$stage/"
# The packages' version: a release as it is; a build on top of one with its changes and commit (1.2.3+4.abcdef0,
# which both dpkg and rpm order after 1.2.3).
# nfpm expands no variables in paths, so the config is written out for each format.
config() { # <arch>
  sed -e "s|\${ARCH}|$1|g" -e "s|\${VERSION}|$version|g" -e "s|\${STAGE}|$stage|g" "$here/nfpm.yaml" >"$stage/nfpm-$1.yaml"
  echo "$stage/nfpm-$1.yaml"
}
"$nfpm" package --config "$(config "$arch")" --packager deb --target "$out/lull-server_${version}_${arch}.deb"
"$nfpm" package --config "$(config "$rpm_arch")" --packager rpm --target "$out/lull-server-${version}.${rpm_arch}.rpm"
tar -C "$stage" -czf "$out/lull-server_${version}_linux-${tar_arch}.tar.gz" --transform 's,^\.,lull-server,' \
  ./lib ./licenses ./lull.service ./postinstall.sh ./preremove.sh ./postremove.sh ./install.sh
# The hub kit: the same programs, gzipped one by one (an image's hub takes only lull), and the manifest.
files=""
for program in lull slskd yt-dlp deno; do
  gzip -9 -n -c "$stage/lib/$program" >"$out/lull-hub-${version}-linux-${tar_arch}-${program}.gz"
  sha=$(sha256sum "$stage/lib/$program" | cut -d' ' -f1)
  files="$files${files:+,}\"$program\":\"$sha\""
done
printf '{"version":"%s","format":1,"arch":"%s","files":{%s}}' "$version" "$tar_arch" "$files" \
  >"$out/lull-hub-${version}-linux-${tar_arch}.json"
ls -l "$out"
