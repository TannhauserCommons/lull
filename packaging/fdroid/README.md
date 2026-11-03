# F-Droid and IzzyOnDroid

How lull's Android app gets into F-Droid's main repository and IzzyOnDroid. Why it's built this way: [ADR-0061](../../docs/adr/0061-fdroid-reproducible-builds.md). The build itself: [android.md](../../docs/context/android.md#build-it).

| File | What |
|---|---|
| `io.github.tannhausercommons.lull.yml` | The draft for fdroiddata's `metadata/` folder |
| `../../fastlane/metadata/android/en-US/` | The store listing both repositories read from the tag: title, descriptions, icon, screenshots, `changelogs/<versionCode>.txt` |
| `../../scripts/android-toolchains.sh`, `android-build.sh` | What F-Droid's recipe runs, and CI's `android` job too |

**Reproducible:** F-Droid builds the tag unsigned and publishes lull's own release APK from GitHub only when the two are identical apart from the signature (`Binaries`, `AllowedAPKSigningKeys`). So F-Droid users and GitHub users have the same signed APK and can move between them. CI's `android` job makes the same comparison on every change.

## Before submitting

1. **A release with the FOSS scanner.** v0.2.0 still has Google's ML Kit inside, so the first build F-Droid can take is the next release: `mise run release 0.2.1`, `mise run check`, `git push origin main v0.2.1`.
2. **Fill in the draft:**
   - `commit:` the full hash of the tag's commit: `git rev-parse v0.2.1^{commit}`.
   - `AllowedAPKSigningKeys` is filled in already: the release certificate of the v0.2.0 APK. `apksigner verify --print-certs lull-0.2.1-android-arm64.apk` must print the same "certificate SHA-256 digest".
   - If the release is not 0.2.1, its `versionName` and `versionCode` (`app/tauri.android.conf.json`).

## F-Droid: the merge request

1. Make a GitLab account at gitlab.com/users/sign_up, as the project (TannhauserCommons), with the project's own email address.
2. Fork https://gitlab.com/fdroid/fdroiddata (the Fork button, into your namespace).
3. In your fork, on a new branch `io.github.tannhausercommons.lull`, add the filled-in draft as `metadata/io.github.tannhausercommons.lull.yml` (Web IDE, or Code › Add file).
4. Open a merge request into `fdroid/fdroiddata` `master` with the title `New app: lull` and the text below. Tick "Allow commits from members who can merge".
5. Wait for its pipeline. A red `fdroid build` or `lint` job is usually a formatting or version detail; reviewers often fix those themselves.

The merge request text:

```
New app: lull, a client for a self-hosted music server

lull is a music app you host yourself (AGPL-3.0-only). The Android app pairs with the listener's own
lull hub (by QR code or pair code) and plays from it: background playback with a MediaSession,
gapless playback and crossfade on Media3, and downloads that play offline.

* [x] The app complies with the inclusion criteria.
* [x] The original app author has been notified, and supports the inclusion: this request is from the author.
* [x] All related fdroiddata and RFP issues have been referenced in this merge request: none.
* [x] Builds with `fdroid build` and all pipelines pass.
* [x] Fastlane metadata in the repo (fastlane/metadata/android/en-US/).
* [x] Releases are tagged (v*) and AutoUpdateMode/UpdateCheckMode follow the tags; versionCode is in
      app/tauri.android.conf.json, set by the release script.
* [x] Reproducible build: Binaries points at the signed APK of each GitHub release, and upstream CI
      builds that APK with the same scripts in fdroidserver's buildserver image, then checks that an
      unsigned build from a clean clone matches it (apksigcopier compare).

Notes for review:
* The QR scanner is ZXing (com.journeyapps:zxing-android-embedded, Apache-2.0); there are no Google
  Play Services, Firebase or ML Kit libraries.
* The build needs no secrets and no Node: the Tauri CLI is built from source by
  scripts/android-toolchains.sh at the version the repo pins, Rust follows rust-toolchain.toml.
* Only arm64-v8a is built for now.
* NonFreeNet: the phone talks only to the listener's own hub, but the hub uses Soulseek, YouTube and
  Last.fm. "Check for updates" (asks GitHub) runs only when pressed.
```

## IzzyOnDroid: the request

IzzyOnDroid takes the APK of each GitHub release as it is, so it's the quicker way to be listed. Open an issue at https://codeberg.org/IzzyOnDroid/repodata/issues with the "App Inclusion Request" template (it needs a Codeberg account), and fill it in:

- **Source code:** https://github.com/TannhauserCommons/lull
- **Licence:** AGPL-3.0-only
- **Categories:** Multimedia, Online Media Player
- **Summary:** Play music from your own lull hub, with the screen off and offline
- **Description:** the fastlane `full_description.txt`.
- **APK:** `lull-<version>-android-arm64.apk` on each GitHub release; arm64 only; signed with the release key; about the size of the release APK.
- **Build instructions (reproducible):** `sh scripts/android-toolchains.sh`, then `NDK_HOME=<NDK 28.2.13676358> sh scripts/android-build.sh`, on Debian trixie with JDK 21 (fdroidserver's buildserver image); the unsigned output is `app/gen/android/app/build/outputs/apk/universal/release/app-universal-release-unsigned.apk`.
- **AI Tools Usage:** answer it truthfully. IzzyOnDroid's policy turns down apps written fully or partly by generative AI, so read https://izzyondroid.org/docs/general/AppInclusionPolicy/ before sending the request.
