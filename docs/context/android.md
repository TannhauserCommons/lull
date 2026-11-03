# Android app

The same Tauri v2 project as the desktop app (`app/`, ADR-0006), built for Android. A phone is never a hub (ADR-0002): it pairs with one, shows the hub's web UI in its WebView, and adds what a browser can't do there. That means playback with the screen off, lock-screen, notification, Bluetooth and headset controls, gapless playback, crossfade and ReplayGain on native players, and downloads to app storage. Desktop: [desktop.md](desktop.md). The web side of the contract: [web.md](web.md#output-interface-adr-0006) and [offline downloads](web.md#offline-downloads-and-outbox).

## Files

| Path | What |
|---|---|
| `app/src/mobile/mod.rs` | Builder (player plugin, opener), the phone's settings (hub and token), launch, pairing, the runtime IPC grant for the hub origin |
| `app/src/mobile/native.js` | `window.LullNative.player`, `.downloads` and `.updates`, injected before the page's scripts; the player's and downloads' calls are `plugin:lull-player|<command>` |
| `app/gen/android/.../MainActivity.kt` | Shared links: "Share" to lull from any app (`SEND text/plain`, the first web link in the text) and `lull://open?link=…` (`VIEW`), both intent filters in the manifest. `SharedLinks` holds the link until the hub's web UI takes it through the `lullLinks` JavaScript interface (`App.takeSharedLink`, at boot and when a running page is nudged), which opens it as `#/open`; the bundled pages never ask |
| `app/src/mobile/update.rs` | Updates: the newest release against this build, and its APK opened in the browser (`tauri-plugin-opener`) |
| `app/src/hub.rs`, `discovery.rs`, `pairing.rs`, `pages.rs`, `store.rs` | Shared with desktop: hub client, mDNS, `discover_hubs`/`find_hub`, bundled page URLs, `settings.json` |
| `app/plugins/player/` | The native player plugin: `src/lib.rs` registers the Kotlin plugin and offers `configure` and `multicast` to Rust; `build.rs` lists the commands (one permission each; `default` has the player's and downloads', not `scan`) |
| `app/plugins/player/android/.../Engine.kt` | The output contract on two Media3 ExoPlayer decks |
| `app/plugins/player/android/.../Equaliser.kt` | The equaliser's spec from the web UI and the `AudioProcessor` each deck plays through |
| `app/plugins/player/android/.../PlaybackService.kt` | The MediaSession service: foreground notification, lock screen, Bluetooth, headset |
| `app/plugins/player/android/.../Downloads.kt` | Downloads through Android's DownloadManager, covers and `index.json` beside them |
| `app/plugins/player/android/.../PlayerPlugin.kt` | The commands, run on the main thread, and `scan`: the connect page's QR scanner (ZXing's camera screen) |
| `app/tauri.android.conf.json` | Minimum SDK 24 and the versionCode. The asset protocol for downloaded covers is in the shared `tauri.conf.json` (scope `**/lull-art/*`), because tauri-build checks the `protocol-asset` feature against every platform's config |
| `app/capabilities/mobile.json` | IPC for the bundled connect page (pairing, `lull-player:allow-scan`) |
| `app/gen/android/` | The Gradle project `tauri android init` generated (in CI: there's no NDK on the workstation), tracked with lull's changes: plain http allowed in release builds too (LAN hubs), release signing from `keystore.properties`, the pinned NDK (`ndkVersion`), no dependency-info block in the APK, `buildSrc`'s Rust task calling `cargo tauri` when `LULL_TAURI_CLI=cargo`, lull's launcher icons (`tauri icon` from `web/assets/icons/icon-512.png`, background `#F7F4FF`). Its own `.gitignore` keeps Tauri's per-build files out |
| `app/tauri.android.conf.json` `versionCode` | Android's version number, set by `scripts/release.sh` (below) |
| `scripts/android-toolchains.sh`, `scripts/android-build.sh` | The one Android build: the pinned Rust toolchain and the Tauri CLI from source, then the release APK. CI and F-Droid run both |
| `packaging/fdroid/`, `fastlane/metadata/android/en-US/` | The fdroiddata recipe draft and how to submit it (and to IzzyOnDroid); the store listing, screenshots and per-versionCode changelogs |

## Build it

One recipe, run the same way by CI, F-Droid and anyone else (ADR-0061): `sh scripts/android-toolchains.sh`, then `sh scripts/android-build.sh`. It builds the release APK for arm64 (`app/gen/android/app/build/outputs/apk/universal/release/`): signed when `app/gen/android/keystore.properties` exists, `…-unsigned.apk` otherwise. No secrets and no Node.
- **Pinned:** Rust by `rust-toolchain.toml` (patch release included), the Tauri CLI by `app/package.json` (built from source with `cargo install --locked`), the NDK by `ndkVersion` in `app/gen/android/app/build.gradle.kts` (28.2.13676358; the script checks `NDK_HOME`), Gradle by its wrapper's properties (F-Droid's scanner deletes the wrapper's jar and `gradlew`; the script then points `gradlew` at F-Droid's own `gradle`, gradlew-fdroid, which runs that same version), the JDK, SDK and OS by F-Droid's buildserver image (Debian trixie, JDK 21), which CI pins by digest. Changing any of them changes the APK's bytes: bump the NDK in `build.gradle.kts`, `app.yml` and the fdroiddata recipe together.
- **Reproducible:** the script remaps the checkout and `CARGO_HOME` out of the Rust library's paths, and the APK has no dependency-info signing block. An unsigned build from any folder equals the signed release APK apart from its signature, which is how F-Droid publishes lull's own signed APK (packaging/fdroid/README.md).
- **CI** (`.github/workflows/app.yml`, job `android`, in F-Droid's buildserver image): every change to `app/`, `crates/` or `web/desktop/` builds the APK, signed with a throwaway key (artifact `lull-android-arm64-test.apk`, installable for testing; uninstall a release first), and checks it. It builds without the Gradle wrapper, as F-Droid does, and then:
  - the Gradle dependency tree (printed in the log) has no Google Play Services, Firebase or ML Kit, and `fdroid scanner` finds no non-free library in the APK;
  - a clean clone in another folder, wrapper included, builds an unsigned APK that `apksigcopier compare` matches with the signed one.
  
  A `v*` tag signs with the release keystore and attaches `lull-<version>-android-arm64.apk` to the tag's Release; `latest.json` points at it (`android.url`). Obtainium can follow those releases too.
- **Locally:** the Android SDK, NDK 28.2.13676358 (`NDK_HOME`) and a JDK 17 or newer, then the two scripts; or `npx tauri android dev` in `app/` with a phone attached (npm's CLI: Gradle's Rust task calls `npm run tauri` unless `LULL_TAURI_CLI=cargo`). This workstation has no NDK on purpose (disk): build in F-Droid's image with Docker instead, or in CI.
- **Release signing** reads `app/gen/android/keystore.properties` (`storeFile`, `storePassword`, `keyAlias`, `keyPassword`). CI writes it from the secrets `ANDROID_KEYSTORE_BASE64`, `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS` and `ANDROID_KEY_PASSWORD`. Never commit it.
- **versionCode:** version X.Y.Z is `X*1000000 + Y*1000 + Z` (Tauri's formula; v0.2.0 is 2000), so it rises with every release while Y and Z stay below 1000. `scripts/release.sh` writes it into `app/tauri.android.conf.json` (F-Droid's update check reads it there at each tag) and refuses one that doesn't rise. It also writes `fastlane/metadata/android/en-US/changelogs/<versionCode>.txt` from the feat and fix commits since the last release, unless that file was written by hand first.
- **No secrets inside:** the APK has no Last.fm key: Last.fm is the hub's (its key is compiled into the hub, and a hub without one says in Settings › Last.fm that it needs the listener's own).

## Launch and pairing

1. **Not paired:** the connect page (`wrapper.html#view=connect&phone=1`). It shows hubs found by mDNS (the plugin holds Android's multicast lock while it looks), an address field, a pair code field and **Scan the QR code**. The QR code is `<hub url>/#/pair/<code>` from a paired device's Settings > Devices (`plugin:lull-player|scan`: ZXing's camera screen, which asks for the camera itself; backing out resolves `{ content: null }` and the page stays as it was, a refused camera rejects with what to do instead).
2. **Pairing:** `find_hub` checks the address, then `window_action {pair}` posts `/api/pair { code, name: "lull on Android", kind: app }` from Rust. The token goes into `settings.json` in the app's config dir.
3. **Paired:** Rust probes `/health` and `/api/ping` with the token.
   - **Both fine:** the plugin's `configure` gives Kotlin the token, which puts it in the WebView's `lull_device` cookie and on its own stream, cover and download requests. Then `main` loads the hub's origin.
   - **Unpaired (401):** the connect page says so.
   - **No answer:** an https hub (Tailscale) opens anyway: its web UI comes from the service worker, and downloads play. A plain-http hub can't have a service worker, so the app shows the unreachable page.

**Why Kotlin sets the cookie:** Tauri's `set_cookie` does nothing on Android (wry), so `configure` uses Android's `CookieManager` before the first page request.

**Why the page origin is the hub's:** the web UI, its API calls and its event stream stay same-origin, as in a browser. Talking to the hub from Rust covers pairing and probes, so a LAN `http://` hub pairs fine. The cost is offline start on plain-http hubs (above). Remote access is the listener's Tailscale anyway (ADR-0002), and that is https.

The app uses the https scheme for its own pages (`https://tauri.localhost`), so downloaded covers (`https://asset.localhost/…`) aren't mixed content on an https hub's page.

## The native player

`window.LullNative.player` implements web.md's output interface exactly; the web UI never sees the token.
- **Two decks** (`Engine.kt`), main thread only. One plays.
  - Crossfade off: `setNext` puts the next item in the playing deck's playlist, and ExoPlayer plays into it gaplessly.
  - Crossfade on: the next item waits paused in the other deck. When the current one has `crossfade` seconds left, the other deck starts and both fade equal-power (cos/sin) over those seconds. `crossfade { key, next }` goes out when the fade starts, and `ended { key, next }` when it's done. A pause or seek during the fade abandons it (`crossfade { next: null }`).
- **Cut responses:** an automatic move into the next item that comes more than 2 s before the current item's duration is a cut, not an end. The deck stops and reports `ended { next: null }`, so the web UI reopens the song with `load`. A network error after playback started is reported the same way. An HTTP status from the hub is an `error` with that `status`.
- **Level:** `setVolume(v)` × ReplayGain `10^(gainDb/20)`, capped at full scale (a player can't amplify). Only the playing deck holds audio focus.
- **Equaliser** (`Equaliser.kt`): `equalize(spec)` stores the web UI's spec; each deck's audio sink runs an `EqProcessor` (a Media3 `AudioProcessor`: the level, then the biquads with Web Audio's formulas, on 16-bit or float PCM), which picks a new spec up at its next buffer and passes sound through untouched while it is null. The decks are audio-only renderers (`MediaCodecAudioRenderer` with that sink).
- **Events:** `time` every 250 ms while playing and on every change; `buffered` as `[[0, bufferedPosition]]` when it moves by 0.5 s or more; `remote { next | prev }` from the MediaSession.
- **MediaSession** (`PlaybackService.kt`): the session's player is the active deck behind a `ForwardingPlayer`, which sends next/prev to the web UI (`remote`) and play/pause through the engine. The engine swaps it when a crossfade finishes. Covers load with the token. Media3 runs the foreground notification while something plays; binding a `MediaController` on the first `load` starts the service. Swiping lull away while paused ends the service.

## Downloads

`window.LullNative.downloads` implements web.md's backend contract.
- **Audio:** Android's DownloadManager writes it to the app's external files dir (`Android/data/<id>/files/downloads/`, removed with the app). The download carries `Authorization: Bearer`, keeps going in the background and after lull closes, and shows the system's progress notification.
- **Cover and JSON:** the cover (`/api/art/…?size=512`) goes to `files/lull-art/`, and the track JSON to `downloads/index.json` with the download's id and state.
- **Progress:** polled from DownloadManager once a second while anything downloads, sent as `progress`, then `done` or `error`. Kotlin names them `download-progress`/`-done`/`-error`; `native.js` maps them back.
- **`list()`:** reads DownloadManager again first, so downloads that finished while lull was closed show as done. `fileUrl` is a `file://` URL for `player.load`. `artUrl` is the cover through Tauri's asset protocol (scope `**/lull-art/*`).

## Updates

Tauri's updater doesn't run on Android, so Settings > App's **App version** row (`window.LullNative.updates`, the desktop's calls without `on`; desktop.md) does it by hand, only on the listener's click:
1. **Check for updates** (`app_update_check`) reads `latest.json` from `plugins.updater.endpoints[0]` in `tauri.conf.json`, the desktop's address (20 s). A 404 means no release yet; no answer means offline.
2. Its `version` against this build's (semver): not newer means **lull 0.1.0 is the newest version**. Newer means **lull X is available**, with the APK from the manifest's `android.url` (https only; a newer release without one is an error).
3. **Download** (`app_update_install`) opens that URL in the browser, which downloads the APK; opening it installs it over this version, since both are signed with the release key. Android asks once to allow installs from the browser. The page never passes a URL: Rust opens the one it found.

## Known limits

- Plain-http hubs don't open offline (no service worker); use the hub's Tailscale address for away-from-home use.
- Positive ReplayGain can't raise the level above full scale.
- Only arm64 APKs are built.
