# ADR-0061: Android on F-Droid: a FOSS QR scanner, and one reproducible build signed by lull

- Status: accepted
- Date: 2026-11-03

## Context

The listener asked for lull's Android app on F-Droid. F-Droid's main repository builds every app from source on its own servers and ships nothing non-free. Three things stood in the way:
- the pairing QR scanner, `tauri-plugin-barcode-scanner`, pulls in Google Play Services (ML Kit);
- the APK was built only on GitHub's runners, with the newest NDK there, Temurin 17 and npm's prebuilt Tauri CLI;
- an F-Droid build is signed with F-Droid's key unless it matches the upstream APK byte for byte. Android refuses to update an app signed with a different key, so listeners couldn't move between the GitHub (or in-app update) APK and F-Droid's.

## Decision

- **The QR scanner is ZXing** (`com.journeyapps:zxing-android-embedded`, Apache-2.0, all Java), as a `scan` command in lull's own player plugin, granted only to the bundled connect page. Rejected:
  - CameraX with a FOSS decoder: more of our own code for the same camera screen;
  - zxing-cpp's Android wrapper: a native library in an AAR, for no gain at QR codes this size.
- **One build recipe everywhere:** `scripts/android-toolchains.sh` (the Rust toolchain of `rust-toolchain.toml`, now pinned to the patch release, and the Tauri CLI built from source at `app/package.json`'s version), then `scripts/android-build.sh` (the release APK, signed only when `keystore.properties` exists). F-Droid's recipe and CI's `android` job both run them, and CI runs them in F-Droid's buildserver image (pinned by digest), so the JDK, Gradle and SDK match F-Droid's. The NDK is pinned in `build.gradle.kts`.
- **Reproducible, signed by lull:** the fdroiddata recipe names the release APK (`Binaries`) and lull's certificate. Rust paths are remapped, and the APK carries no Google dependency-info block. CI builds a second, unsigned APK from a clean clone in another folder and requires `apksigcopier compare` to pass, as F-Droid's publish step will. Rejected: letting F-Droid sign. It's simpler, but listeners couldn't switch, and F-Droid can't change that later.
- **versionCode** is `X*1000000 + Y*1000 + Z`, Tauri's own formula, so v0.2.0's 2000 stands. `scripts/release.sh` writes it to `app/tauri.android.conf.json`, where F-Droid's update check reads it.
- **The in-app update check stays.** It's opt-in (only on the listener's click) and installs nothing itself, which F-Droid's inclusion policy allows. Because F-Droid ships the same signed APK, its update installs over F-Droid's.

## Consequences

- No Node in the Android build. Gradle's Rust task calls `cargo tauri` when `LULL_TAURI_CLI=cargo` (a lull change to the generated `buildSrc`); `npx tauri android build` still works for contributors.
- CI's Android job builds twice (about twice as long). That's the price of hearing about a non-reproducible change before a release does.
- Bumping the NDK, the Rust toolchain or the Tauri CLI changes the bytes: change the NDK in `build.gradle.kts`, CI and the fdroiddata recipe's `ndk:` together.
