#!/bin/sh
# Builds lull's Android release APK for arm64 phones, the same way everywhere: GitHub's release job,
# F-Droid's build server (packaging/fdroid/) and a contributor's machine (docs/context/android.md).
# Signed when app/gen/android/keystore.properties exists (the release job writes it), unsigned
# otherwise, which is what F-Droid builds and then compares with the signed one.
#
# Needs: what scripts/android-toolchains.sh installs (the pinned Rust toolchain and Tauri CLI); the
# NDK of app/gen/android/app/build.gradle.kts in NDK_HOME; the Android SDK in ANDROID_HOME; a JDK.
# No Node: the build never runs npm.
set -eu

die() { echo "android-build: $*" >&2; exit 1; }

cd "$(dirname "$0")/../app"
root=$(cd .. && pwd)

wanted=$(sed -n 's/.*"@tauri-apps\/cli": "\(.*\)".*/\1/p' package.json)
have=$(cargo tauri --version 2>/dev/null | sed -n 's/^tauri-cli //p') || true
[ "$have" = "$wanted" ] || die "needs tauri-cli $wanted built from source (scripts/android-toolchains.sh), found '${have:-none}'"
ndk=$(sed -n 's/.*ndkVersion = "\(.*\)".*/\1/p' gen/android/app/build.gradle.kts)
[ "$(sed -n 's/^Pkg.Revision *= *//p' "${NDK_HOME:-.}/source.properties" 2>/dev/null)" = "$ndk" ] \
  || die "set NDK_HOME to NDK $ndk (app/gen/android/app/build.gradle.kts)"

# The same bytes from any checkout: no build machine's paths in the native library.
cargo_home=${CARGO_HOME:-$HOME/.cargo}
export CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS="--remap-path-prefix=$root=/lull --remap-path-prefix=$cargo_home=/cargo"
# Gradle's Rust task calls the CLI back (buildSrc's BuildTask): the same cargo-built one.
export LULL_TAURI_CLI=cargo

# The CLI gives Gradle a versionName and versionCode only when the config names a version: the app's
# (scripts/release.sh sets it). The versionCode is tauri.android.conf.json's.
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
# F-Droid's scanner deletes the Gradle wrapper (gradlew and its jar, a prebuilt binary). Its own
# `gradle` (gradlew-fdroid) runs the same Gradle, the version in gradle/wrapper/gradle-wrapper.properties.
if [ ! -f gen/android/gradle/wrapper/gradle-wrapper.jar ]; then
  command -v gradle >/dev/null || die "no Gradle wrapper jar and no gradle on PATH"
  printf '#!/bin/sh\nexec gradle "$@"\n' > gen/android/gradlew
  chmod +x gen/android/gradlew
fi
# tauri.conf.json's beforeBuildCommand only makes dist/ for a clean clone (with npm); build.rs fills it.
mkdir -p dist
cargo tauri android build --ci --apk --target aarch64 \
  --config "{\"version\":\"$version\",\"build\":{\"beforeBuildCommand\":null}}"
code=$(sed -n 's/.*"versionCode": \([0-9]*\).*/\1/p' tauri.android.conf.json)
grep -qx "tauri.android.versionCode=$code" gen/android/app/tauri.properties || die "the APK didn't get versionCode $code"
