# Android app

The same Tauri v2 project as the desktop app (`app/`, ADR-0006), built for Android. A phone is never a hub (ADR-0002): it pairs with one, shows the hub's web UI in its WebView, and adds what a browser can't do there. That means playback with the screen off, lock-screen, notification, Bluetooth and headset controls, gapless playback, crossfade and ReplayGain on native players, and downloads to app storage. Desktop: [desktop.md](desktop.md). The web side of the contract: [web.md](web.md#output-interface-adr-0006) and [offline downloads](web.md#offline-downloads-and-outbox).

## Files

| Path | What |
|---|---|
| `app/src/mobile/mod.rs` | Builder (player plugin, opener), the phone's settings (hub and token), launch, pairing, the runtime IPC grant for the hub origin |
| `app/src/mobile/native.js` | `window.LullNative.player`, `.downloads` and `.updates`, injected before the page's scripts; the player's and downloads' calls are `plugin:lull-player|<command>` |
| `app/src/mobile/update.rs` | Updates: the newest release against this build, and its APK opened in the browser (`tauri-plugin-opener`) |
| `app/src/hub.rs`, `discovery.rs`, `pairing.rs`, `pages.rs`, `store.rs` | Shared with desktop: hub client, mDNS, `discover_hubs`/`find_hub`, bundled page URLs, `settings.json` |
| `app/plugins/player/` | The native player plugin: `src/lib.rs` registers the Kotlin plugin and offers `configure` and `multicast` to Rust; `build.rs` lists the commands (one permission each; `default` has the player's and downloads', not `scan`) |
| `app/plugins/player/android/.../Engine.kt` | The output contract on two Media3 ExoPlayer decks |
| `app/plugins/player/android/.../PlaybackService.kt` | The MediaSession service: foreground notification, lock screen, Bluetooth, headset |
| `app/plugins/player/android/.../Downloads.kt` | Downloads through Android's DownloadManager, covers and `index.json` beside them |
| `app/plugins/player/android/.../PlayerPlugin.kt` | The commands, run on the main thread, and `scan`: the connect page's QR scanner (ZXing's camera screen) |
| `app/tauri.android.conf.json` | Minimum SDK 24. The asset protocol for downloaded covers is in the shared `tauri.conf.json` (scope `**/lull-art/*`), because tauri-build checks the `protocol-asset` feature against every platform's config |
| `app/capabilities/mobile.json` | IPC for the bundled connect page (pairing, `lull-player:allow-scan`) |
| `app/gen/android/` | The Gradle project `tauri android init` generated (in CI: there's no NDK on the workstation), tracked with lull's changes: plain http allowed in release builds too (LAN hubs), release signing from `keystore.properties`, lull's launcher icons (`tauri icon` from `web/assets/icons/icon-512.png`, background `#F7F4FF`). Its own `.gitignore` keeps Tauri's per-build files out |

## Build it

- **CI** (`.github/workflows/app.yml`, job `android`): every change to `app/`, `crates/` or `web/desktop/` builds a debug APK for arm64 (a workflow artifact). A `v*` tag builds the release APK, signed with the release keystore, and attaches `lull-<version>-android-arm64.apk` to the tag's Release; `latest.json` points at it (`android.url`). Obtainium can follow those releases too.
- **Locally:** the Android SDK, NDK (`NDK_HOME`) and JDK 17, then `npx tauri android build --apk --target aarch64` in `app/` (or `npx tauri android dev` with a phone attached). Gradle's Rust task calls `npm run tauri`, hence that script in `app/package.json`. This workstation has no NDK on purpose (disk): Android builds in CI only.
- **Release signing** reads `app/gen/android/keystore.properties` (`storeFile`, `storePassword`, `keyAlias`, `keyPassword`). CI writes it from the secrets `ANDROID_KEYSTORE_BASE64`, `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS` and `ANDROID_KEY_PASSWORD`. Never commit it.

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
