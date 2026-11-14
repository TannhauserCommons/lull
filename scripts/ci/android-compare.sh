#!/bin/sh
# F-Droid publishes the release APK only when its own unsigned build matches it except for the signature
# (packaging/fdroid/, Binaries). So does this: the two APKs of android-apk.sh, downloaded into apk/ (or $APK_DIR).
# Runs as root in F-Droid's buildserver image.
set -eu
cd "$(dirname "$0")/../.."
apk=${APK_DIR:-apk}

apt-get update -qq
apt-get install -y -qq --no-install-recommends apksigcopier
apksigcopier compare "$apk/app-universal-release.apk" --unsigned "$apk/app-universal-release-unsigned.apk"
