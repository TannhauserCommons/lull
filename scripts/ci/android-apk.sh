#!/bin/sh
# One of the two Android builds, the way F-Droid builds it (packaging/fdroid/), in F-Droid's buildserver image with
# scripts/android-toolchains.sh run; the APK lands in <temp>/apk for the compare job.
#
#   sh scripts/ci/android-apk.sh signed   in this checkout, signed with a throwaway key made here (a release tag's key
#                                         stays in the release workflow): the APK to try. Also checks that no non-free
#                                         library got in: the Gradle dependency tree and F-Droid's scanner.
#   sh scripts/ci/android-apk.sh clean    unsigned, from a clean clone in another folder, as F-Droid builds the tag.
#
# android-compare then requires the two to match except for the signature, which is what lets F-Droid ship the release APK.
# Needs ANDROID_HOME and the toolchains of scripts/android-toolchains.sh; the NDK is the project's own (ndk_version).
set -eu
cd "$(dirname "$0")/../.."
. scripts/ci/lib.sh

build=${1:?usage: android-apk.sh signed|clean}
only_on_ci android-apk.sh
: "${ANDROID_HOME:?set ANDROID_HOME to the Android SDK}"
export NDK_HOME=${NDK_HOME:-$ANDROID_HOME/ndk/$(ndk_version)}
out=$(tmp_root)/apk
work=$(mktemp -d)
mkdir -p "$out"
apk=universal/release/app-universal-release

case "$build" in
  signed)
    keytool -genkeypair -keystore "$work/release.jks" -storepass throwaway -keypass throwaway \
      -alias test -dname CN=lull-test -keyalg RSA -keysize 2048 -validity 30
    cat > app/gen/android/keystore.properties <<PROPERTIES
storeFile=$work/release.jks
storePassword=throwaway
keyAlias=test
keyPassword=throwaway
PROPERTIES
    # As F-Droid's scanner does before its build: no prebuilt Gradle wrapper. The clean clone keeps it, so the two
    # builds also cover both ways Gradle starts.
    rm app/gen/android/gradlew app/gen/android/gradlew.bat app/gen/android/gradle/wrapper/gradle-wrapper.jar
    sh scripts/android-build.sh
    cp "app/gen/android/app/build/outputs/apk/$apk.apk" "$out/"

    (cd app/gen/android && ./gradlew -q :app:dependencies --configuration universalReleaseRuntimeClasspath) > "$work/deps.txt"
    cat "$work/deps.txt"
    if grep -Ei 'com\.google\.(android\.gms|firebase|mlkit)|play-services|crashlytics' "$work/deps.txt"; then
      echo "::error::A non-free Google library is in the APK's dependencies (F-Droid can't ship it)"
      exit 1
    fi
    fdroid scanner --exit-code "app/gen/android/app/build/outputs/apk/$apk.apk"
    ;;
  clean)
    git config --global --add safe.directory "$PWD"
    git clone -q "$PWD" "$work/again"
    git -C "$work/again" checkout -q "$(git rev-parse HEAD)"
    sh "$work/again/scripts/android-build.sh"
    cp "$work/again/app/gen/android/app/build/outputs/apk/$apk-unsigned.apk" "$out/"
    ;;
  *) echo "android-apk: no build '$build' (signed or clean)" >&2; exit 1 ;;
esac
