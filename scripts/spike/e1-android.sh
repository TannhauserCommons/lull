#!/bin/sh
# E1: is the unsigned clean APK signable afterwards? Runs as root in F-Droid's buildserver image, after
# `mise run android:apk clean` has left the unsigned APK in <temp>/apk.
#   sh scripts/spike/e1-android.sh sign    (a) sign a copy with a throwaway key by apksigner and `apksigcopier compare` it
#                                          with the unsigned bytes; (b) which apksigner this image has, can build-tools 34
#                                          be pinned. This decides PASS.
#   sh scripts/spike/e1-android.sh gradle  (c) the Gradle-signed APK with the SAME key, compared with the apksigner-signed
#                                          one: bytes, signature schemes, entries. Evidence only.
set -eu
cd "$(dirname "$0")/../.."
. scripts/ci/lib.sh
. scripts/spike/lib.sh
EXP=e1
spike_init

phase=${1:?usage: e1-android.sh sign|gradle}
unsigned=$(tmp_root)/apk/app-universal-release-unsigned.apk
ks=$OUT/spike.jks
signed=$OUT/signed-apksigner.apk
[ -f "$unsigned" ] || { note "no unsigned APK at $unsigned"; verdict FAIL; exit 1; }

sha() { sha256sum "$1" | cut -d' ' -f1; }
# The signature schemes and the certificate an APK verifies with.
schemes() { "$1" verify --verbose --print-certs "$2" 2>&1 | grep -E '^(Verifies|Verified using|Number of signers|Signer #1 certificate (DN|SHA-256))' || true; }
# Every entry of an APK with its size and CRC.
entries() {
  python3 -I -c 'import sys, zipfile
for i in sorted(zipfile.ZipFile(sys.argv[1]).infolist(), key=lambda i: i.filename):
    print(i.filename, i.file_size, i.CRC)' "$1"
}
sign() { "$1" sign --ks "$ks" --ks-pass pass:throwaway --key-pass pass:throwaway --ks-key-alias test --out "$2" "$unsigned"; }

case "$phase" in
  sign)
    # What scripts/ci/android-compare.sh installs: it brings Debian's apksigner.
    apt-get update -qq
    apt-get install -y -qq --no-install-recommends apksigcopier
    # The same throwaway key as the `signed` build makes (scripts/ci/android-apk.sh).
    keytool -genkeypair -keystore "$ks" -storepass throwaway -keypass throwaway \
      -alias test -dname CN=lull-test -keyalg RSA -keysize 2048 -validity 30

    note "unsigned APK: $(sha "$unsigned") $(wc -c < "$unsigned") bytes"

    # (b) which apksigner, from which package or build-tools.
    default=$(command -v apksigner || true)
    note "(b) apksigner on PATH: ${default:-none}"
    if [ -n "$default" ]; then
      note "(b) apksigner --version prints: $(apksigner --version 2>&1 | head -n 1) (the tool's own number, the same in many build-tools)"
      note "(b) its file belongs to: $(dpkg -S "$(readlink -f "$default")" 2>&1 | head -n 1)"
    fi
    # shellcheck disable=SC2016 # dpkg-query's own format, not the shell's
    note "(b) Debian packages: $(dpkg-query -W -f '${Package} ${Version}; ' apksigner apksigcopier 2>&1)"
    note "(b) build-tools in ANDROID_HOME before: $(find "$ANDROID_HOME/build-tools" -maxdepth 1 -mindepth 1 2>&1 | tr '\n' ' ')"
    note "(b) apksigner files under ANDROID_HOME: $(find "$ANDROID_HOME" -name 'apksigner*' 2>/dev/null | tr '\n' ' ')"
    cap sh -c "sdkmanager --list 2>&1 | grep -E 'build-tools;3[3-6]' | head -n 12" || true
    yes | sdkmanager --licenses > /dev/null 2>&1 || true
    bt34=
    if cap sdkmanager --install "build-tools;34.0.0"; then
      [ ! -x "$ANDROID_HOME/build-tools/34.0.0/apksigner" ] || bt34=$ANDROID_HOME/build-tools/34.0.0/apksigner
      note "(b) build-tools;34.0.0: sdkmanager installed it; apksigner $([ -n "$bt34" ] && echo present || echo missing)"
    else
      note "(b) build-tools;34.0.0: sdkmanager could not install it (raw.log)"
    fi
    [ -z "$bt34" ] || note "(b) build-tools 34.0.0 apksigner --version prints: $("$bt34" --version 2>&1 | head -n 1)"

    # (a) sign a copy, then F-Droid's check.
    ok=0
    if [ -n "$default" ] && cap sign "$default" "$signed" && cap "$default" verify --verbose --print-certs "$signed" \
       && cap apksigcopier compare "$signed" --unsigned "$unsigned"; then
      ok=1
    fi
    note "(a) apksigner (PATH) sign, verify, apksigcopier compare: $([ "$ok" = 1 ] && echo ok || echo FAILED)"
    [ "$ok" = 0 ] || note "(a) signed APK: $(sha "$signed") $(wc -c < "$signed") bytes; $(schemes "$default" "$signed" | tr '\n' ';')"

    # (a') the same with build-tools 34, the version F-Droid recommends.
    if [ -n "$bt34" ]; then
      signed34=$OUT/signed-apksigner-bt34.apk
      ok34=0
      if cap sign "$bt34" "$signed34" && cap "$bt34" verify --verbose --print-certs "$signed34" \
         && cap apksigcopier compare "$signed34" --unsigned "$unsigned"; then
        ok34=1
      fi
      note "(a') build-tools 34.0.0 apksigner sign, verify, apksigcopier compare: $([ "$ok34" = 1 ] && echo ok || echo FAILED)"
      if [ "$ok34" = 1 ] && [ "$ok" = 1 ]; then
        if cmp -s "$signed" "$signed34"; then note "(a') the two apksigners' output: identical bytes"
        else note "(a') the two apksigners' output: $(cmp -l "$signed" "$signed34" 2>/dev/null | wc -l) bytes differ"; fi
      fi
    fi
    if [ "$ok" = 1 ]; then verdict PASS; else verdict FAIL; fi
    ;;

  gradle)
    # (c) the build of the `signed` variant, with the key (a) made instead of a new one.
    unfinished() { status=$?; [ "$status" = 0 ] || note "(c) did not finish: exit $status (raw.log, the step log)"; }
    trap unfinished EXIT
    only_on_ci e1-android.sh
    [ -f "$ks" ] && [ -f "$signed" ] || { note "(c) needs the key and the apksigner-signed APK of the sign phase"; exit 1; }
    default=$(command -v apksigner)
    export NDK_HOME="${NDK_HOME:-$ANDROID_HOME/ndk/$(ndk_version)}"
    cat > app/gen/android/keystore.properties <<PROPERTIES
storeFile=$ks
storePassword=throwaway
keyAlias=test
keyPassword=throwaway
PROPERTIES
    # As the `signed` variant does: no prebuilt Gradle wrapper.
    rm app/gen/android/gradlew app/gen/android/gradlew.bat app/gen/android/gradle/wrapper/gradle-wrapper.jar
    sh scripts/android-build.sh
    gradle=$OUT/signed-gradle.apk
    cp app/gen/android/app/build/outputs/apk/universal/release/app-universal-release.apk "$gradle"

    note "(c) Gradle-signed: $(sha "$gradle") $(wc -c < "$gradle") bytes; apksigner-signed: $(sha "$signed") $(wc -c < "$signed") bytes"
    if cmp -s "$gradle" "$signed"; then
      note "(c) bytes equal: yes"
    else
      note "(c) bytes equal: no, $(cmp -l "$gradle" "$signed" 2>/dev/null | wc -l) byte positions differ"
    fi
    schemes "$default" "$gradle" > "$OUT/schemes-gradle.txt"
    schemes "$default" "$signed" > "$OUT/schemes-apksigner.txt"
    if cmp -s "$OUT/schemes-gradle.txt" "$OUT/schemes-apksigner.txt"; then
      note "(c) apksigner verify, same schemes, signers and certificate digest: yes ($(tr '\n' ';' < "$OUT/schemes-gradle.txt"))"
    else
      note "(c) apksigner verify differs (< Gradle, > apksigner): $(diff "$OUT/schemes-gradle.txt" "$OUT/schemes-apksigner.txt" | tr '\n' ';')"
    fi
    entries "$gradle" > "$OUT/entries-gradle.txt"
    entries "$signed" > "$OUT/entries-apksigner.txt"
    if cmp -s "$OUT/entries-gradle.txt" "$OUT/entries-apksigner.txt"; then
      note "(c) zip entries (name, size, CRC): identical"
    else
      note "(c) zip entries that differ (< Gradle, > apksigner): $(diff "$OUT/entries-gradle.txt" "$OUT/entries-apksigner.txt" | head -n 20 | tr '\n' ';')"
    fi
    if cap apksigcopier compare "$gradle" --unsigned "$unsigned"; then
      note "(c) apksigcopier compare Gradle-signed with the unsigned clean APK: ok"
    else
      note "(c) apksigcopier compare Gradle-signed with the unsigned clean APK: FAILED"
    fi
    ;;

  *) echo "e1-android: no phase '$phase' (sign or gradle)" >&2; exit 1 ;;
esac
