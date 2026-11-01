# Desktop wrapper

A Tauri v2 app in `desktop/` (its own Cargo workspace) that loads the web UI from the user's lull server. It owns the windows, the tray, launch at login and the first-run connect page. Every player action runs in the web UI. Design spec: `design/desktop/DESKTOP.md`. Decisions: [ADR-0060](../adr/0060-desktop-wrapper-pages-token-ipc.md).

## Files

| Path | What |
|---|---|
| `desktop/src/main.rs` | Builder: plugins (single instance first, window state, autostart, global shortcut on Linux), commands, setup |
| `desktop/src/main_window.rs` | `main`: launch probe, web UI or bundled page, runtime IPC grant for the server origin, close-to-tray |
| `desktop/src/mini.rs` | Mini player windows `mini` (device) and `strip` |
| `desktop/src/tray.rs` | Tray icon and native menu; player items become `tray` events |
| `desktop/src/commands.rs` | The commands the pages call, checked at the boundary |
| `desktop/src/server.rs` | `/health`, `/api/ping`, `/api/session` from Rust (ureq) |
| `desktop/src/pages.rs` | URLs of the bundled page (`wrapper.html#view=…`) |
| `desktop/src/settings.rs` | `settings.json` in the app config dir |
| `desktop/src/platform/` | Windows taskbar theme (registry watch), Linux tray host, button layout, compositing, media keys |
| `desktop/build.rs` | Stages the web files the bundled page needs into `desktop/dist/`, declares command permissions |
| `desktop/capabilities/local.json` | IPC for the bundled page |
| `web/desktop/wrapper.html` | The bundled page: connect, unreachable, mini player (view in the hash) |
| `web/desktop/desktop.js`, `desktop.css` | The wrapper's UI; also loaded by the web UI (`index.html`), where it does nothing outside the wrapper |

## Run it

- `cd desktop && cargo run` (set `CARGO_TARGET_DIR` outside `Desktop` on Windows). Point it at `mise run dev` (`http://127.0.0.1:8080`) on the connect page.
- Pages in `web/desktop/` and the staged web files are embedded at compile time: rebuild after editing them. The web UI itself comes from the server, so its edits only need a reload.
- **Inspecting the webviews:** start with `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333` and open `http://127.0.0.1:9333/json`; each window is a DevTools target.
- Settings live in `%APPDATA%\io.github.tannhausercommons.lull\settings.json` (Linux: `~/.config/io.github.tannhausercommons.lull/`). WebView2 data, including the token cookie, is in `%LOCALAPPDATA%\io.github.tannhausercommons.lull\EBWebView`.
- Bundles: `npm ci && npm run bundle` in `desktop/`. CI does this (`.github/workflows/desktop.yml`): a `v*` tag attaches the Windows installer and the Linux deb, rpm and AppImage to that tag's Release; a manual run keeps them as workflow artifacts.

## Launch

1. No saved server: `main` opens the connect page.
2. A saved server: Rust probes `GET /health` (10 s). If it answers, `main` loads the server's origin; if not, it loads the unreachable page with the error. `main` shows once its page has loaded, unless **Start in the tray** is on and a tray exists.
3. The unreachable page retries through `window_action {retry}` with a 2, 4, 8, 16, 30 s backoff, then every 30 s; a success loads the web UI.
4. The wrapper switches between connect and unreachable by changing the page's hash, so `desktop.js` re-renders on `hashchange`.

The token: the connect page's **Connect** posts it to `/api/session` from Rust and puts the server's own `lull_token` cookie into the webview before loading the web UI. With "Remember on this device" off, the cookie loses its expiry and lasts until lull quits. Changing the server asks for the token again, because the wrapper's own check can't read the webview's cookie.

## The seam with the web UI

The web UI never calls Tauri directly. It uses `window.LullDesktop` from `desktop.js`, and `desktop.js` uses these parts of the web UI's `App`:
- `App.player`: `current`, `currentId`, `state` (`playing`, `pos`, `buffered`, `received`, `durationKnown`, `shuffle`, `repeat`, `volume`, `muted`), `duration`, `toggle`, `next`, `prev`, `pause`, `setShuffle`, `cycleRepeat`, `setVolume`, `seekFraction`;
- `App.isLiked`, `App.toggleLike`, `App.isSaved`, `App.saves.start`, `App.go`, `App.design`, `App.prefs`, `App.banner.list` (the `sys-unreachable` banner means "not connected");
- events `player`, `tick`, `like`, `save`, `ready`, `design`, `prefs`, `route`;
- cover art: the largest `navigator.mediaSession.metadata.artwork`, fetched by the web UI and passed on as a data URL.

**What the web UI calls (`LullDesktop`):**
- `inWrapper`: show Settings > App's desktop rows only when true.
- `settings()` resolves to `{ server, tray, closeToTray, launchAtLogin, startInTray, miniStyle, miniOnTop, ownTitlebar }`. `ownTitlebar` is null off Linux; without a tray, Close always quits.
- `setSettings(patch)` takes any of those keys except `server` and `tray`. Unknown keys are refused.
- `openMini()`: the player bar's Device view button.
- `windowAction('change-server')`: Settings > App, "Change server…".

`desktop.js` does the rest itself in the wrapper: title bar, Ctrl+Shift+M, the close dialog, tray and mini-player actions, `set_now_playing`, and design and theme reports.

**Commands** (`allow-<name>` permissions, from `build.rs`):

| Command | Args | Who may call |
|---|---|---|
| `window_action` | `{ request: { action, … } }`, where `action` is one of `full`, `mini-close`, `mini`, `connected {url, token, remember}`, `change-server`, `retry`, `close-choice {keep, remember}`, `media-keys` | bundled page, server origin |
| `test_server` | `{ url, token }` → `{ state: ok, health } \| { state: unreachable, error } \| { state: rejected }`, or an error for an invalid address | bundled page |
| `set_now_playing` | `{ now: { title, artist, playing, liked, saved, connected } }` | server origin |
| `desktop_settings`, `set_desktop_settings` | `{ patch }` | server origin |

The server origin gets its capability at runtime, for `main` only, when the wrapper first loads it. An origin used earlier in the same run keeps its grant until restart (Tauri can't revoke one).

**Events:**

| Event | From → to | Payload |
|---|---|---|
| `tray` | Rust → `main` | `{ action: play-pause \| next \| previous \| like \| save \| classic \| stop }` (`stop` comes from the Linux media-key fallback) |
| `mini` | mini window → `main` | `{ action: play-pause \| previous \| next \| like \| shuffle \| repeat \| vol-up \| vol-down \| seek \| hello, fraction? }` |
| `player` | `main` → all | snapshot `{ track, art, playing, liked, pos, duration, durationKnown, buffered, received, shuffle, repeat, volume }`, sent on change and in answer to `hello` |
| `appearance` | `main` → Rust and the mini player | `{ design, theme }`; stored and passed in the hash of every bundled page |
| `ask-close` | Rust → `main` | none: show "keep playing in the tray?" |

## Behaviour worth knowing

- **Close:**
  - on a bundled page, or without a tray, it quits;
  - otherwise it hides `main`, asking once first. "Don't ask again" stores the answer as `closeToTray`;
  - setting **Close to tray** in Settings also counts as answered.
- **Mini player:**
  - opening it hides `main`; closing it (button, Alt+F4, tray) shows `main` again;
  - **Show lull** and **Classic mode** in the tray close it first;
  - a second launch focuses the mini player if one is open.
- **Tray glyph (Windows):** follows the taskbar theme (`SystemUsesLightTheme`), not the app theme. A thread waits on the registry key, because Tauri's ThemeChanged only tracks the app theme. Its size follows the primary monitor's scale.
- **Linux:**
  - **Tray:** only with a StatusNotifier host on D-Bus; without one there's no tray and Close quits. The tray always gets `tray-dark-32.png`: Tauri hands AppIndicator a file, so a themed icon name can't be used.
  - **Title bar:** native by default. With **Use lull title bar** (`ownTitlebar`, restart), the window is undecorated and draws GNOME header-bar buttons in the order of `button-layout`.
  - **Mini player:** opaque `--sky` fallback without a compositor. On Wayland it shows a one-time note about Always on Top.
  - **Media keys:** grabbed with global shortcuts only when the web UI reports no `navigator.mediaSession` (X11 only). Windows never grabs them: SMTC handles them through Media Session. That path works only while a media element plays: Chromium gives a page a system media session only then. Verified on Windows with a Jellyfin song: Play/Pause, Next, Previous and Stop all reach the web UI's own handlers.
- **Window state:** the plugin restores size, position and maximised for `main`, `mini` and `strip`. Visibility and decorations are always the wrapper's choice.

## Known limits

- The Snap Layouts flyout on the custom Maximise button doesn't appear (DESKTOP.md 2.1).
- Links to other sites (`target="_blank"`, other origins) aren't sent to the system browser yet, and nothing here checked what WebView2 does with them. Pages from other origins get no IPC.
