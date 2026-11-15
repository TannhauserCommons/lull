#!/bin/sh
# E1: is the unsigned clean APK signable afterwards? Runs as root in F-Droid's buildserver image, after
# `mise run android:apk clean` has left the unsigned APK in <temp>/apk.
#   sh scripts/spike/e1-android.sh sign    (b) pin build-tools 34.0.0 with sdkmanager and record what a later job needs to
#                                          pin and check it; (a') sign a copy of the unsigned APK with its apksigner and a
#                                          throwaway key, and `apksigcopier compare` it with the unsigned bytes: this decides
#                                          PASS. (a) the same with Debian's apksigner, evidence only, with its error output.
#   sh scripts/spike/e1-android.sh gradle  evidence only: (c) the Gradle-signed APK with the SAME key against the one signed
#                                          here; (d) the unsigned APK signed with exactly the schemes the Gradle-signed APK
#                                          has, against the Gradle-signed APK.
set -eu
cd "$(dirname "$0")/../.."
. scripts/ci/lib.sh
. scripts/spike/lib.sh
EXP=e1
spike_init

phase=${1:?usage: e1-android.sh sign|gradle}
unsigned=$(tmp_root)/apk/app-universal-release-unsigned.apk
ks=$OUT/spike.jks
bt=$ANDROID_HOME/build-tools/34.0.0/apksigner
signed34=$OUT/signed-bt34.apk
[ -f "$unsigned" ] || { note "no unsigned APK at $unsigned"; verdict FAIL; exit 1; }

sha() { sha256sum "$1" | cut -d' ' -f1; }
# Every entry of an APK with its size and CRC.
entries() {
  python3 -I -c 'import sys, zipfile
for i in sorted(zipfile.ZipFile(sys.argv[1]).infolist(), key=lambda i: i.filename):
    print(i.filename, i.file_size, i.CRC)' "$1"
}
# try <label> <command...>: ok, or FAILED with the first 15 lines the command printed.
try() {
  label=$1
  shift
  if "$@" > "$OUT/try.out" 2>&1; then
    note "$label: ok"
    cat "$OUT/try.out" >> "$RAW"
  else
    code=$?
    note "$label: FAILED (exit $code); first 15 lines of its output:"
    head -n 15 "$OUT/try.out" | sed 's/^/    /' | tee -a "$OUT/evidence.txt"
    cat "$OUT/try.out" >> "$RAW"
    return 1
  fi
}
# sign <apksigner> <out> [apksigner options...]: the unsigned APK signed with the throwaway key.
sign() {
  signer=$1 out=$2
  shift 2
  "$signer" sign --ks "$ks" --ks-pass pass:throwaway --key-pass pass:throwaway --ks-key-alias test "$@" --out "$out" "$unsigned"
}
# compare_apks <label> <a> <b>: bytes, where they differ, the verify output, F-Droid's check on b.
compare_apks() {
  label=$1 a=$2 b=$3
  note "$label: sha256 $(sha "$a") vs $(sha "$b")"
  if cmp -s "$a" "$b"; then
    note "$label: identical bytes"
  else
    note "$label: bytes differ; cmp -l (1-based position, octal bytes) first 10: $(cmp -l "$a" "$b" 2>/dev/null | head -n 10 | tr '\n' ';')"
    python3 -I scripts/spike/apk-diff.py "$a" "$b" | sed 's/^/    /' | tee -a "$OUT/evidence.txt"
  fi
  "$bt" verify --verbose --print-certs "$a" > "$OUT/verify-1.txt" 2>&1 || true
  "$bt" verify --verbose --print-certs "$b" > "$OUT/verify-2.txt" 2>&1 || true
  if cmp -s "$OUT/verify-1.txt" "$OUT/verify-2.txt"; then
    note "$label: apksigner verify --verbose --print-certs output matches line for line ($(grep -c . "$OUT/verify-1.txt") lines: $(grep '^Verified using' "$OUT/verify-1.txt" | sed 's/^Verified using //' | tr '\n' ';'))"
  else
    note "$label: apksigner verify --verbose --print-certs differs (< first, > second):"
    diff "$OUT/verify-1.txt" "$OUT/verify-2.txt" | head -n 20 | sed 's/^/    /' | tee -a "$OUT/evidence.txt" || true
  fi
  entries "$a" > "$OUT/entries-1.txt"
  entries "$b" > "$OUT/entries-2.txt"
  if cmp -s "$OUT/entries-1.txt" "$OUT/entries-2.txt"; then
    note "$label: zip entries (name, size, CRC) identical"
  else
    note "$label: zip entries that differ (< first, > second): $(diff "$OUT/entries-1.txt" "$OUT/entries-2.txt" | head -n 12 | tr '\n' ';')"
  fi
  try "$label: apksigcopier compare of the second against the unsigned APK" apksigcopier compare "$b" --unsigned "$unsigned" || true
}

case "$phase" in
  sign)
    # What scripts/ci/android-compare.sh installs: it brings Debian's apksigner (a).
    apt-get update -qq
    apt-get install -y -qq --no-install-recommends apksigcopier
    # The same throwaway key as the `signed` build makes (scripts/ci/android-apk.sh).
    keytool -genkeypair -keystore "$ks" -storepass throwaway -keypass throwaway \
      -alias test -dname CN=lull-test -keyalg RSA -keysize 2048 -validity 30
    note "unsigned APK: $(sha "$unsigned") $(wc -c < "$unsigned") bytes"

    # (b) the pin.
    default=$(command -v apksigner || true)
    note "(b) Debian apksigner on PATH: ${default:-none}; $(dpkg-query -W -f '${Package} ${Version}' apksigner 2>&1)"
    cap sh -c "sdkmanager --list 2>&1 | grep -E 'build-tools;3[3-6]' | head -n 12" || true
    yes | sdkmanager --licenses > /dev/null 2>&1 || true
    if cap sdkmanager --install "build-tools;34.0.0" && [ -x "$bt" ]; then
      note "(b) pinned with: sdkmanager --install \"build-tools;34.0.0\""
      note "(b) sdkmanager --list_installed: $(sdkmanager --list_installed 2>&1 | grep 'build-tools;34' | tr -s ' ' | tr '\n' ';')"
      note "(b) build-tools 34.0.0 lib/apksigner.jar sha256: $(sha "$ANDROID_HOME/build-tools/34.0.0/lib/apksigner.jar")"
      note "(b) build-tools 34.0.0 apksigner --version prints: $("$bt" --version 2>&1 | head -n 1)"
      note "(b) $(grep -h 'revision\|major\|minor\|micro' "$ANDROID_HOME/build-tools/34.0.0/package.xml" 2>/dev/null | tr -d ' \t' | tr '\n' ' ')"
    else
      note "(b) build-tools;34.0.0 could not be installed or has no apksigner (raw.log)"
      verdict FAIL
      exit 1
    fi

    # (a) Debian's apksigner: evidence only, with the output of whichever step fails.
    signed_deb=$OUT/signed-debian.apk
    if [ -n "$default" ]; then
      note "(a) Debian apksigner --version prints: $(apksigner --version 2>&1 | head -n 1)"
      if try "(a) Debian apksigner sign" sign "$default" "$signed_deb" \
         && try "(a) Debian apksigner verify" "$default" verify --verbose --print-certs "$signed_deb"; then
        try "(a) apksigcopier compare of the Debian-signed APK" apksigcopier compare "$signed_deb" --unsigned "$unsigned" || true
      fi
    fi

    # (a') build-tools 34.0.0's apksigner: the verdict.
    if try "(a') build-tools 34.0.0 apksigner sign (default schemes)" sign "$bt" "$signed34" \
       && try "(a') build-tools 34.0.0 apksigner verify" "$bt" verify --verbose --print-certs "$signed34" \
       && try "(a') apksigcopier compare of the build-tools 34.0.0-signed APK with the unsigned APK" apksigcopier compare "$signed34" --unsigned "$unsigned"; then
      verdict PASS
    else
      verdict FAIL
    fi
    ;;

  gradle)
    # The build of the `signed` variant, with the key the sign phase made instead of a new one.
    unfinished() { status=$?; [ "$status" = 0 ] || note "did not finish: exit $status (raw.log, the step log)"; }
    trap unfinished EXIT
    only_on_ci e1-android.sh
    [ -f "$ks" ] && [ -f "$signed34" ] || { note "needs the key and the signed APK of the sign phase"; exit 1; }
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
    note "Gradle-signed APK: $(sha "$gradle") $(wc -c < "$gradle") bytes"

    # What the Gradle-signed APK was signed with, as the build-tools 34.0.0 apksigner reports it.
    "$bt" verify --verbose --print-certs "$gradle" > "$OUT/verify-gradle.txt" 2>&1 || true
    note "Gradle-signed APK, apksigner verify says: $(grep -E '^(Verifies|Verified using|Number of signers)' "$OUT/verify-gradle.txt" | tr '\n' ';')"
    scheme() { if grep -q "^Verified using v$1 scheme.*: true" "$OUT/verify-gradle.txt"; then echo true; else echo false; fi; }
    v1=$(scheme 1) v2=$(scheme 2) v3=$(scheme 3) v4=$(scheme 4)

    note "(c) Gradle-signed vs build-tools 34.0.0 apksigner with its default schemes:"
    compare_apks "(c)" "$gradle" "$signed34"

    # (d) only the schemes the Gradle-signed APK has.
    flags="--v1-signing-enabled $v1 --v2-signing-enabled $v2 --v3-signing-enabled $v3 --v4-signing-enabled $v4"
    note "(d) signing with: $flags"
    signed_d=$OUT/signed-bt34-schemes.apk
    # shellcheck disable=SC2086 # one option per word, on purpose
    try "(d) build-tools 34.0.0 apksigner sign with those schemes" sign "$bt" "$signed_d" $flags
    compare_apks "(d) Gradle-signed vs (d)" "$gradle" "$signed_d"
    # Does --min-sdk-version (the manifest's minSdkVersion, 24) change anything?
    signed_d24=$OUT/signed-bt34-schemes-min24.apk
    # shellcheck disable=SC2086
    if try "(d) the same with --min-sdk-version 24" sign "$bt" "$signed_d24" $flags --min-sdk-version 24; then
      if cmp -s "$signed_d" "$signed_d24"; then note "(d) --min-sdk-version 24 changes nothing: same bytes"
      else note "(d) --min-sdk-version 24 changes the output: $(cmp -l "$signed_d" "$signed_d24" 2>/dev/null | wc -l) byte positions"; fi
    fi
    # Does signing twice give the same bytes (is the signature deterministic)?
    signed_d2=$OUT/signed-bt34-schemes-again.apk
    # shellcheck disable=SC2086
    if try "(d) the same signed a second time" sign "$bt" "$signed_d2" $flags; then
      if cmp -s "$signed_d" "$signed_d2"; then note "(d) signing twice: same bytes"
      else note "(d) signing twice: $(cmp -l "$signed_d" "$signed_d2" 2>/dev/null | wc -l) byte positions differ"; fi
    fi
    ;;

  *) echo "e1-android: no phase '$phase' (sign or gradle)" >&2; exit 1 ;;
esac
