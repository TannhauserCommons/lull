# ADR-0006: One Tauri v2 app for Windows, Linux and Android

- Status: accepted
- Date: 2026-11-01

## Context

The listener wants real apps, built on Rust, not only a website: Windows, Linux and Android, with no macOS or iOS (PROJECT.md). They must look exactly like the web UI, which is what the five designs and every theme style. The v1 desktop wrapper (`desktop/`, ADR-0060) only connected to a server somewhere else.

## Decision

**One Tauri v2 project, `app/`** (renamed from `desktop/`), for all three platforms. Every platform shows the same web UI, so designs and themes are identical everywhere.

**Shared Rust** (`app/src/`):
- pairing and hub discovery on the local network (mDNS);
- talking to the hub over HTTP from Rust. This avoids the browser's mixed-content and HTTPS rules, so a phone can use a hub at `http://192.168.x.x`.

**Desktop** (`app/src/desktop/`, `cfg(desktop)`): v1's tray, mini player, window state and media keys, plus:
- **The hub, in-process.** It runs `lull_server::run` and bundles slskd as a sidecar: a pinned, sha256-checked release binary that the app starts, stops and configures.
- **First run:** "this computer is my hub" or "connect to a hub".
- **Auto-update** through the Tauri updater, plus one-click `tailscale serve` (ADR-0002).

**Android** (`app/src/mobile/`, `cfg(mobile)`):
- **A native player plugin,** `app/plugins/player`: Media3 ExoPlayer with a MediaSession service. It gives background playback, lock-screen and Bluetooth controls, and gapless playback; crossfade and ReplayGain run on two native players.
- **Offline downloads** in app storage (ADR-0005).

**The web player's output is an interface.**
- In a browser and on desktop, it uses `<audio>`.
- In the Android app it uses `window.LullNative.player`. That contract, kept in `docs/context/web.md`, covers:
  - `load`, `play`, `pause`, `seek`, `setVolume`, `setNext` (gapless);
  - `crossfade`;
  - events for time, buffered, ended and error;
  - `downloads`.

**Distribution:**
- **Windows:** an NSIS installer.
- **Linux:** deb, rpm and AppImage.
- **Android:** an APK signed with a release key kept as a GitHub secret.
- All three ship from GitHub releases. Obtainium can keep the APK updated. There are no app stores (PROJECT.md non-goals).

## Alternatives

- **Separate desktop and mobile projects.** They'd drift apart and duplicate the pairing and discovery code.
- **Flutter or React Native for Android.** That means a second UI to keep pixel-identical to five designs and every user theme.
- **A WebView `<audio>` on Android.** Android pauses the WebView in the background, and it has no lock-screen or Bluetooth controls.

## Consequences

- **Hub releases ship with the app.** The app depends on `crates/server` as a path dependency, so a hub change is an app release too.
- **The native plugin is the only Kotlin in the repo.** It stays small: playback and downloads only.
- **ADR-0060's "wrapper has no logic" is narrowed:** no UI logic. The app does host the hub and the native player.
