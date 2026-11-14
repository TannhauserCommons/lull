#!/bin/sh
# The listener's existing Jellyfin (tests/hub-contract/jellyfin.sh): a real one, set up through its startup API, and the
# package's hub pointed at it by `lull setup`'s questions. Needs docker, node, ffmpeg (the fixture library; installed
# here on Debian) and the packages of package-build.sh in $PACKAGES.
set -eu
cd "$(dirname "$0")/../.."
. scripts/ci/lib.sh

apt_install ffmpeg
packages=${PACKAGES:-$(tmp_root)/packages}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
tar -C "$work" -xzf "$packages"/lull-server_*_linux-x64.tar.gz
sh tests/hub-contract/jellyfin.sh "$work/lull-server/lib/lull"
