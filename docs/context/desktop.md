# Desktop app (Windows and Linux)

The Tauri v2 project in `app/` (its own Cargo workspace; ADR-0006) shows the hub's web UI in a window, with a tray, a mini player and media keys. On first run the listener picks **This computer is my hub** (the app runs the hub in-process with a bundled slskd) or **Connect to a hub** (pair with one elsewhere). Every player action runs in the web UI. Design spec: `design/desktop/DESKTOP.md`. Decisions: [ADR-0006](../adr/0006-one-tauri-app-for-windows-linux-android.md), [ADR-0060](../adr/0060-desktop-wrapper-pages-token-ipc.md). Android: [android.md](android.md).

## Files

| Path | What |
|---|---|
| `app/src/main.rs`, `app/src/lib.rs` | Entry points: `lull_app::run()` (Tauri's mobile entry point needs a library) |
| `app/src/hub.rs` | Shared: talking to a hub from Rust (ureq): `/health`, `/api/ping`, `POST /api/pair`, `PATCH /api/config`, `/api/events`, the `lull_device` cookie |
| `app/src/discovery.rs` | Shared: mDNS `_lull._tcp`, browsing for hubs and advertising this computer's |
| `app/src/pairing.rs` | Shared: the connect page's `discover_hubs` and `find_hub` |
| `app/src/pages.rs` | Shared: URLs of the bundled page (`wrapper.html#view=…`), with the design and theme they open in |
| `app/src/store.rs` | Shared: `settings.json` in the app config dir, written atomically |
| `app/src/desktop/mod.rs` | Builder: plugins (single instance first, window state, autostart, updater, global shortcut on Linux), commands, setup, exit |
| `app/src/desktop/local_hub.rs` | This computer is the hub: `lull_server::run` in-process, its env, the app's own device token, following `hub.restarting` |
| `app/src/desktop/slskd.rs` | The bundled slskd: one supervisor thread, its folders, port and API key, restart when its share changes |
| `app/src/desktop/main_window.rs` | `main`: welcome page, local hub or remote probe, device cookie, runtime IPC grant for the hub origin, close-to-tray |
| `app/src/desktop/commands.rs` | The commands the pages call, checked at the boundary |
| `app/src/desktop/tailscale.rs` | `tailscale serve --bg <port>` for the hub, and its status |
| `app/src/desktop/updater.rs` | Update check at launch, download, "Restart to update" in the tray |
| `app/src/desktop/mini.rs` | Mini player windows `mini` (device) and `strip` |
| `app/src/desktop/tray.rs` | Tray icon and native menu; player items become `tray` events |
| `app/src/desktop/settings.rs` | The desktop's settings: the hub choice and this app's token, window options |
| `app/src/desktop/platform/` | Windows taskbar theme (registry watch) and slskd's job object; Linux tray host, button layout, compositing, media keys |
| `app/build.rs` | Stages the web files the bundled page needs into `app/dist/`; fetches slskd (sha256-pinned) into `app/binaries/`; declares command permissions |
| `app/tauri.conf.json`, `tauri.windows.conf.json`, `tauri.linux.conf.json` | Shared config (updater key and endpoint, the asset protocol Android's download covers use, scoped to `**/lull-art/*`); the desktop configs add the slskd sidecar and its licence |
| `app/licenses/THIRD-PARTY.md` | slskd's AGPL notice, shipped next to the app with slskd's `LICENSE` |
| `app/capabilities/local.json` | IPC for the bundled page |
| `web/desktop/wrapper.html` | The bundled page: welcome, connect, hub unreachable, hub didn't start, mini player (view in the hash) |
| `web/desktop/desktop.js`, `desktop.css` | The app's UI; also loaded by the web UI (`index.html`), where it does nothing outside the app |

## Run it

- `cd app && cargo run` (set `CARGO_TARGET_DIR` outside `Desktop` on Windows). The first build downloads slskd (55 MB) into `app/binaries/`.
- Pages in `web/desktop/` and the staged web files are embedded at compile time: rebuild after editing them. The web UI itself comes from the hub; the in-process hub embeds it at compile time too (release builds), so web edits need a rebuild here.
- **Inspecting the webviews:** start with `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333` and open `http://127.0.0.1:9333/json`; each window is a DevTools target.
- **Where things live** (Windows; Linux uses `~/.config/` and `~/.local/share/` with the same identifier):
  - `%APPDATA%\io.github.tannhausercommons.lull\settings.json`: the app's settings, including its device token;
  - `%APPDATA%\io.github.tannhausercommons.lull\hub\`: the hub's database (`LULL_DATA_DIR`);
  - `%APPDATA%\io.github.tannhausercommons.lull\slskd\`: slskd's app dir (its data, logs and `slskd.yml`);
  - `%APPDATA%\io.github.tannhausercommons.lull\transfers\`: `complete/` and `incomplete/` (`LULL_TRANSFERS_DIR`);
  - `%LOCALAPPDATA%\io.github.tannhausercommons.lull\logs\hub.log`: the hub's log, truncated at each start;
  - `%LOCALAPPDATA%\io.github.tannhausercommons.lull\EBWebView`: WebView2 data.
- Bundles: `npm ci && npm run bundle` in `app/`. CI does this (`.github/workflows/app.yml`): it compiles lull's own Last.fm key into the hub (secrets `LULL_LASTFM_API_KEY`/`_SECRET`, as the image does); a `v*` tag sets the version from the tag, signs the updater files (secret `TAURI_SIGNING_PRIVATE_KEY`, no password) and attaches the Windows installer, the Linux deb, rpm and AppImage, their `.sig` files and `latest.json` to that tag's Release; a manual run keeps unsigned bundles as workflow artifacts.

## Launch

1. **No hub chosen:** `main` opens the welcome page: **This computer is my hub** or **Connect to a hub**.
2. **This computer:** `local_hub::start` (off the main thread), then `main` loads `http://127.0.0.1:8484` with the device cookie. A failure loads the hub-error page with the reason ("Port 8484 is already used…"); **Try again** starts it again, **Use another hub** goes back to the welcome page.
3. **A hub elsewhere:** Rust probes `GET /health` (10 s), then `GET /api/ping` with the token. Both fine: `main` loads the hub's origin with the device cookie. Unpaired (401): the connect page, saying so. No answer: the unreachable page with the error, which retries through `window_action {retry}` with a 2, 4, 8, 16, 30 s backoff, then every 30 s.
4. `main` shows once its page has loaded, unless **Start in the tray** is on and a tray exists. It starts at `about:blank` when the cookie has to go in first; that page never shows.
5. The app switches between its pages by changing the page's hash, so `desktop.js` re-renders on `hashchange`.

**Pairing** (connect page): the page lists hubs found by mDNS for 3 s (**Search again**), each with its most reachable IPv4 address (home LAN ranges first, Tailscale's 100.64/10 next, link-local last), or takes an address (`192.168.1.20:8484` means http, a dotted name like `hub.example.ts.net` means https). With a 6-character pair code from a paired device, `find_hub` checks the address, then `window_action {pair}` posts `/api/pair { code, name: "lull on <computer>", kind: app }` and keeps the token. Choosing a remote hub while this computer's hub runs restarts the app, because the hub can't stop under its open connections.

**The device token** (ADR-0002): the app keeps one token for the hub it uses, in `settings.json`. At each launch it goes into the webview as the session cookie `lull_device` for the hub's host (HttpOnly, SameSite Lax, Secure on https). Rust sends it as `Authorization: Bearer`. Changing the hub forgets it.

## This computer is the hub

`local_hub::start`:
1. **Paths** under the app's data dir (above). The hub's log goes to `hub.log` through `tracing`.
2. **Env** for `lull_server::run` (shown as locked in Settings, ADR-0002): `LULL_DATA_DIR`, `LULL_LISTEN=0.0.0.0:8484` (phones reach it on the LAN; Windows asks to allow it through the firewall on first run), `LULL_SLSKD_URL` (loopback, a free port chosen per run), `LULL_SLSKD_API_KEY` (random per run), `LULL_TRANSFERS_DIR`, `LULL_SLSKD_CONFIG_FILE`.
3. **First run seeds** `LULL_MUSIC_DIR` with the OS Music folder as a *stored* setting, so the hub's setup screen can change it.
   **Each start stores** `LULL_PUBLIC_URL` = `http://<LAN address>:8484` (the address of the default route), because pair codes carry it and the webview itself talks to `127.0.0.1`. It does so only while the stored value is empty or plain http; an https address (Tailscale serve, or the listener's own) is never touched.
4. **The app's own device:** the stored token if `devices::verify` still knows it, else `devices::create(name, Kind::App)`.
5. **slskd starts**, then the hub; the app waits until port 8484 accepts connections (30 s at most) or the hub's task fails.
6. **mDNS** advertises `_lull._tcp` with the computer's name and port 8484. A network that refuses multicast only loses discovery.
7. **Following settings:** a thread follows `/api/events` with the app's token. On `hub.restarting` it reads the settings again (env over stored) and restarts slskd if what it shares changed (`LULL_MUSIC_DIR`, `LULL_SHARE`): slskd takes shares only at start.

**slskd** (`slskd.rs`), 0.26.2 like the deploy kit:
- **Fetched at build time** by `build.rs`: the release zip for the target, checked against the pinned sha256, kept in `app/binaries/`; the binary becomes `binaries/slskd-<target-triple>[.exe]`, Tauri's sidecar name, and ships next to `lull.exe`. To bump it, change the version and both hashes in `build.rs` (`gh release view <v> --repo slskd/slskd --json assets` lists them).
- **Command line** (fixed for the run): `--app-dir`, `--http-ip-address 127.0.0.1`, `--http-port`, `--no-https`, `--headless` (lull is the UI; `--content-path` points at the app dir because slskd checks that its static folder exists even then), `--downloads <transfers>/complete`, `--incomplete <transfers>/incomplete`, and `--shared <music dir>` unless `LULL_SHARE=none`.
- **Env** (not in the process list): `SLSKD_API_KEY`, and slskd's own web login (`lull` and a random password, never the default).
- **`slskd.yml`** in its app dir (created empty before slskd's first start, so its watcher follows it) belongs to the hub: the app passes it as `LULL_SLSKD_CONFIG_FILE`, and the soulseek crate writes the account from `LULL_SOULSEEK_USERNAME`/`PASSWORD` into it and logs slskd in again (docs/context/soulseek.md). slskd reads env < yml < command line and reloads the file by itself.
- **One supervisor thread** owns the process and does every spawn. slskd never outlives the app: on Windows it sits in a kill-on-close job object, on Linux it gets `PR_SET_PDEATHSIG` (tied to the spawning thread, hence the one thread). Quitting stops it; so does an update.
- **Licence:** AGPL-3.0, unmodified. `licenses/THIRD-PARTY.md` and slskd's `LICENSE` ship next to the app.

**Tailscale** (`tailscale.rs`): when this computer is the hub, Settings > App can show `LullDesktop.tailscale()`: `{ installed, running, url, serving }` from `tailscale status --json` and `tailscale serve status --json` (read-only). The listener's click on **Use Tailscale** calls `LullDesktop.tailscaleServe()`, which runs `tailscale serve --bg 8484` (20 s at most: it waits when HTTPS is off in the tailnet) and then stores the https address as `LULL_PUBLIC_URL` through `PATCH /api/config`, so pair codes carry it.

**Updates** (`updater.rs`): at launch the app reads `latest.json` from the newest GitHub Release, checks the build's signature against `plugins.updater.pubkey` and downloads it in the background. The tray then offers **Restart to update to lull X** first in its menu. Windows runs the NSIS installer passively and restarts lull; Linux replaces the AppImage and restarts. While the repo is private, the check fails quietly (GitHub answers 404 to anonymous requests).

## The seam with the web UI

The web UI never calls Tauri directly. It uses `window.LullDesktop` from `desktop.js`, and `desktop.js` uses these parts of the web UI's `App`:
- `App.player`: `current`, `currentId`, `state` (`playing`, `pos`, `buffered`, `received`, `durationKnown`, `shuffle`, `repeat`, `volume`, `muted`), `duration`, `toggle`, `next`, `prev`, `pause`, `setShuffle`, `cycleRepeat`, `setVolume`, `seekFraction`;
- `App.isLiked`, `App.toggleLike`, `App.isSaved`, `App.saves.start`, `App.go`, `App.design`, `App.prefs`, `App.banner.list` (the `sys-unreachable` banner means "not connected");
- events `player`, `tick`, `like`, `save`, `ready`, `design`, `prefs`, `route`;
- cover art: the largest `navigator.mediaSession.metadata.artwork`, fetched by the web UI and passed on as a data URL.

**What the web UI calls (`LullDesktop`):**
- `inWrapper`: show Settings > App's desktop rows only when true.
- `settings()` resolves to `{ server, hub, tray, closeToTray, launchAtLogin, startInTray, miniStyle, miniOnTop, ownTitlebar }`. `server` is the hub's address (`http://127.0.0.1:8484/` when this computer is the hub); `hub` is `local`, `remote` or null. `ownTitlebar` is null off Linux; without a tray, Close always quits.
- `setSettings(patch)` takes any of those keys except `server`, `hub` and `tray`. Unknown keys are refused.
- `openMini()`: the player bar's Device view button.
- `windowAction('change-server')`: Settings > App, "Change hub…" (the welcome page).
- `tailscale()` and `tailscaleServe()`, only when `hub` is `local`: Settings > App's "Reach this hub from anywhere" row (see Tailscale above). Both resolve to `{ installed, running, url, serving }`; `tailscaleServe()` rejects with Tailscale's own message. Call `tailscaleServe()` only from the listener's click: it changes their Tailscale.

`desktop.js` does the rest itself in the wrapper: title bar, Ctrl+Shift+M, the close dialog, tray and mini-player actions, `set_now_playing`, and design and theme reports.

**Commands** (`allow-<name>` permissions, from `build.rs`):

| Command | Args | Who may call |
|---|---|---|
| `window_action` | `{ request: { action, … } }`, where `action` is one of `full`, `mini-close`, `mini`, `this-computer`, `connect`, `pair {url, code}`, `change-server`, `retry`, `close-choice {keep, remember}`, `media-keys` | bundled page, hub origin |
| `discover_hubs` | → `[{ name, url }]`, the hubs that answered mDNS within 3 s | bundled page |
| `find_hub` | `{ url }` → `{ state: ok, url, health } \| { state: unreachable, error }`, or an error for an invalid address | bundled page |
| `set_now_playing` | `{ now: { title, artist, playing, liked, saved, connected } }` | hub origin |
| `desktop_settings`, `set_desktop_settings` | `{ patch }` | hub origin |
| `tailscale_status`, `tailscale_serve` | → `{ installed, running, url, serving }`; an error when this computer isn't the hub | hub origin |

The hub origin gets its capability at runtime, for `main` only, when the app first loads it. An origin used earlier in the same run keeps its grant until restart (Tauri can't revoke one).

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
