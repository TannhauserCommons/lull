#!/bin/sh
# The package installed as a service in a fresh Ubuntu, Debian or Fedora running systemd, and held to the hub contract
# (tests/hub-contract/in-systemd.sh). Needs docker, and the packages of package-build.sh in $PACKAGES.
#
#   sh scripts/ci/package-contract.sh ubuntu|debian|fedora
set -eu
cd "$(dirname "$0")/../.."
. scripts/ci/lib.sh

distro=${1:?usage: package-contract.sh ubuntu|debian|fedora}
packages=${PACKAGES:-$(tmp_root)/packages}
case "$distro" in
  fedora) package=$(ls "$packages"/*.rpm) ;;
  *) package=$(ls "$packages"/*.deb) ;;
esac
sh tests/hub-contract/in-systemd.sh "$distro" "$package"
