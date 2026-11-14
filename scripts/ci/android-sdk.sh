#!/bin/sh
# What F-Droid's recipe installs before the pinned Rust toolchain (packaging/fdroid/): Debian's rustup, the SDK manager
# and fdroidserver, then the NDK of the Android project into ANDROID_HOME. Runs as root in F-Droid's buildserver image
# (the `android` jobs of .github/workflows/check.yml), which has no sudo.
set -eu
cd "$(dirname "$0")/../.."
. scripts/ci/lib.sh

apt-get update -qq
apt-get install -y -qq --no-install-recommends build-essential rustup sdkmanager fdroidserver
sdkmanager --install "ndk;$(ndk_version)"
