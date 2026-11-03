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
| `app/src/updates.rs` | Shared: the update check's `Status` that Settings > App shows, and Android's reading of `latest.json` |
| `app/src/desktop/mod.rs` | Builder: plugins (single instance first, window state, autostart, updater, global shortcut on Linux), commands, setup, exit |
| `app/src/desktop/local_hub.rs` | This computer is the hub: `lull_server::run` in-process, its env, the app's own device token, following `hub.restarting` |
| `app/src/desktop/slskd.rs` | The bundled slskd: one supervisor thread, its folders, port and API key, restart when its share changes, when it stops by itself and when the hub finds it wedged |
| `app/src/desktop/main_window.rs` | `main`: welcome page, local hub or remote probe, device cookie, runtime IPC grant for the hub origin, close-to-tray |
| `app/src/desktop/glass.rs` | A see-through `main` (below): which material the theme gets, the listener's and the system's opt-outs |
| `app/src/desktop/commands.rs` | The commands the pages call, checked at the boundary |
| `app/src/desktop/open_links.rs` | `lull://open?link=<link>` (a launch's argument, or a second launch's, which hands over): the link opens in `main` as the web UI's `#/open`, once `main` shows the hub. The installed app (the release identifier only, so a dev build never takes its links) registers the scheme at each start: `HKCU\Software\Classes\lull` on Windows, a `lull-lull-links.desktop` entry and `xdg-mime` on Linux (the AppImage's own path when run as one) |
| `app/src/desktop/tailscale.rs` | `tailscale serve --bg <port>` for the hub, and its status |
| `app/src/desktop/recovery.rs` | A window whose page's process died: `logs/webview.log`, reload and resume, lull's own page for a crash loop; every window is built here on the shared WebView2 environment (Crashes, below) |
| `app/src/desktop/updater.rs` | Update checks (at launch, every 6 hours, on request), the download with its progress, the install; the tray's update item |
| `app/src/desktop/mini.rs` | Mini player windows `mini` (device) and `strip` |
| `app/src/desktop/tray.rs` | Tray icon and native menu; player items become `tray` events |
| `app/src/desktop/player.rs` | Which window plays (`main`, or the Classic window), and showing `set_now_playing` in the tray, the taskbar buttons and Discord |
| `app/src/desktop/classic.rs` | Classic mode's own window: Webamp alone (`web/desktop/winamp.html`), `main` asleep meanwhile |
| `app/src/desktop/links.rs`, `signin.rs` | Other sites open in the browser; Last.fm's sign-in goes there and comes back through a one-time listener |
| `app/src/desktop/discord.rs` | Discord Rich Presence over Discord's IPC pipe; `APPLICATION_ID` is lull's Discord application |
| `app/src/desktop/settings.rs` | The desktop's settings: the hub choice and this app's token, window options |
| `app/src/desktop/platform/` | Windows taskbar theme and transparency effects (registry watch), the window's glass (DWM, window-vibrancy; a KWin property on Linux), slskd's job object, the taskbar's thumbnail buttons (`taskbar.rs`, glyphs drawn in `glyphs.rs`), a sleeping webview, the browser; Linux tray host, button layout, compositing, media keys |
| `app/build.rs` | Stages the web files the bundled page needs into `app/dist/`; fetches slskd, yt-dlp and Deno (sha256-pinned) into `app/binaries/`; declares command permissions |
| `app/tauri.conf.json`, `tauri.windows.conf.json`, `tauri.linux.conf.json` | Shared config (updater key and endpoint, the asset protocol Android's download covers use, scoped to `**/lull-art/*`, and the bundled pages' CSP: Tauri leaves its `style-src` alone, since a nonce it adds would make browsers ignore `'unsafe-inline'` and drop every `style="…"` the pages render); the desktop configs add the slskd, yt-dlp and Deno sidecars and slskd's licence |
| `app/licenses/THIRD-PARTY.md` | slskd's AGPL notice (shipped next to the app with slskd's `LICENSE`), yt-dlp's and Deno's |
| `app/capabilities/local.json` | IPC for the bundled page |
| `web/desktop/wrapper.html` | The bundled page: welcome, connect, hub unreachable, hub didn't start, a window that kept crashing, mini player (view in the hash) |
| `web/desktop/desktop.js`, `desktop.css` | The app's UI; also loaded by the web UI (`index.html`), where it does nothing outside the app |
| `web/desktop/winamp.html`, `winamp.js` | Served by the hub: the Classic window's page (Webamp alone) |
| `web/desktop/webamp-session.js` | One Webamp playing lull's queue (hand-in, hand-back, listening), and the listener's Classic skin (loading it, Webamp's alerts, `.wsz` files kept on this device), shared by the Classic window, the browser's Classic stage (`web/screens/classic.js`) and Settings (`web/screens/skins.js`) |

## Run it

- **Every dev build gets its own identifier.** This is the default way to build or run `app/`:
  `cd app && TAURI_CONFIG='{"identifier":"io.github.tannhausercommons.lull.dev"}' cargo run` (PowerShell: `$env:TAURI_CONFIG='{"identifier":"io.github.tannhausercommons.lull.dev"}'` first). Set `CARGO_TARGET_DIR` outside `Desktop` on Windows. The first build downloads slskd (55 MB), yt-dlp (24 MB on Windows, 41 MB on Linux) and Deno (a 43 MB zip) into `app/binaries/`.
  - **Why:** without `TAURI_CONFIG` a dev build *is* the installed app as far as the OS can tell: same identifier, so the same settings (device token included), WebView2 profile and single-instance lock. Beside an installed lull it hands over to it and exits, and anything written into "its" settings is the installed app's.
  - Use one test identifier per purpose (`….lull.dev`, `….lull.<what you test>`); seed, inspect and delete only that identifier's folders (the list below, with its name), never `io.github.tannhausercommons.lull`'s.
  - Only `npm run bundle` and CI's release builds use the real identifier.
- Pages in `web/desktop/` and the staged web files are embedded at compile time: rebuild after editing them. The web UI itself comes from the hub; the in-process hub embeds it at compile time too (release builds), so web edits need a rebuild here.
- **Inspecting the webviews:** start with `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333` and open `http://127.0.0.1:9333/json`; each window is a DevTools target.
- **Where things live** (Windows; Linux uses `~/.config/` and `~/.local/share/` with the same identifier). These are the installed app's; a dev build's are the same paths under its own identifier:
  - `%APPDATA%\io.github.tannhausercommons.lull\settings.json`: the app's settings, including its device token;
  - `%APPDATA%\io.github.tannhausercommons.lull\hub\`: the hub's database (`LULL_DATA_DIR`);
  - `%APPDATA%\io.github.tannhausercommons.lull\slskd\`: slskd's app dir (its data, logs and `slskd.yml`);
  - `%APPDATA%\io.github.tannhausercommons.lull\transfers\`: `complete/` and `incomplete/` (`LULL_TRANSFERS_DIR`);
  - `%LOCALAPPDATA%\io.github.tannhausercommons.lull\logs\hub.log`: the hub's log, truncated at each start;
  - `%LOCALAPPDATA%\io.github.tannhausercommons.lull\logs\webview.log` (and `webview.log.1`): which WebView2 runs, and every crash of a window's page (Crashes, below);
  - `%LOCALAPPDATA%\io.github.tannhausercommons.lull\EBWebView\Crashpad\reports\*.dmp`: WebView2's crash dumps, kept here and never sent to Microsoft;
  - `%LOCALAPPDATA%\io.github.tannhausercommons.lull\logs\lull.dmp`: a minidump of lull's own process if it crashes, overwritten by the next one;
  - `%LOCALAPPDATA%\io.github.tannhausercommons.lull\EBWebView`: WebView2 data.
- Bundles: `npm ci && npm run bundle` in `app/`. CI does this (`.github/workflows/app.yml`): it compiles lull's own Last.fm key into the hub (secrets `LULL_LASTFM_API_KEY`/`_SECRET`, as the image does); a `v*` tag sets the version from the tag, signs the updater files (secret `TAURI_SIGNING_PRIVATE_KEY`, no password; the .deb and .rpm with `tauri signer sign`, since the bundler signs only the installer and the AppImage) and attaches the Windows installers, the Linux deb, rpm and AppImage packages, their `.sig` files, the Android APK and `latest.json` (api.md, App releases) to that tag's Release; a manual run keeps unsigned bundles as workflow artifacts. Four desktop builds, each native on GitHub's runner for its architecture: Windows x64 (`windows-latest`) and ARM64 (`windows-11-arm`), Linux x64 (`ubuntu-22.04`) and ARM64 (`ubuntu-22.04-arm`), the oldest glibc of each.
- **Cutting a release:** `mise run release 0.2.0` on an up-to-date, clean `main` (`scripts/release.sh`) sets the version in `Cargo.toml` (the hub's workspace) and `app/Cargo.toml` and Android's versionCode, updates both lock files, drafts the F-Droid changelog (android.md, Build it), commits `build(release): lull 0.2.0` and tags `v0.2.0`. Nothing is pushed: run `mise run check`, then `git push origin main v0.2.0` publishes it. The tag's run builds for about half an hour; the Release appears when every bundle is in.

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
2. **Env** for `lull_server::run` (shown as locked in Settings, ADR-0002): `LULL_DATA_DIR`, `LULL_LISTEN=0.0.0.0:8484` (phones reach it on the LAN; Windows asks to allow it through the firewall on first run), `LULL_SLSKD_URL` (loopback, a free port chosen per run), `LULL_SLSKD_API_KEY` (random per run), `LULL_TRANSFERS_DIR`, `LULL_SLSKD_CONFIG_FILE`, and `LULL_YTDLP`/`LULL_DENO` (the sidecars next to `lull.exe`; when one is missing the hub looks on PATH).
3. **First run seeds** `LULL_MUSIC_DIR` with the OS Music folder as a *stored* setting, so the hub's setup screen can change it.
   **Each start stores** `LULL_PUBLIC_URL` = `http://<LAN address>:8484` (the address of the default route), because pair codes carry it and the webview itself talks to `127.0.0.1`. It does so only while the stored value is empty or plain http; an https address (Tailscale serve, or the listener's own) is never touched.
4. **The app's own device:** the stored token if `devices::verify` still knows it, else `devices::create(name, Kind::App)`.
5. **slskd starts**, then the hub; the app waits until port 8484 accepts connections (30 s at most) or the hub's task fails.
6. **mDNS** advertises `_lull._tcp` with the computer's name and port 8484. A network that refuses multicast only loses discovery.
7. **Following settings:** a thread follows `/api/events` with the app's token. On `hub.restarting` it reads the settings again (env over stored) and restarts slskd if what it shares changed (`LULL_MUSIC_DIR`, `LULL_SHARE`): slskd takes shares only at start.

**yt-dlp and Deno** (YouTube, docs/context/youtube.md), the versions the server image pins: fetched by `build.rs` like slskd (yt-dlp's standalone `yt-dlp.exe`, `yt-dlp_arm64.exe`, `yt-dlp_linux` or `yt-dlp_linux_aarch64`; Deno's release zip for the target), checked against pinned sha256s and shipped as sidecars (`binaries/yt-dlp-<target>`, `binaries/deno-<target>`). The hub runs them itself, without a console window; Keep yt-dlp up to date may put a newer yt-dlp in the hub's data folder. To bump either, change the version and hashes in `build.rs` and the `Dockerfile` together.

**slskd** (`slskd.rs`), 0.26.2 like the deploy kit:
- **Fetched at build time** by `build.rs`: the release zip for the target (win-x64, win-arm64, linux-x64, linux-arm64: each architecture runs its own build, none under emulation), checked against the pinned sha256, kept in `app/binaries/`; the binary becomes `binaries/slskd-<target-triple>[.exe]`, Tauri's sidecar name, and ships next to `lull.exe`. To bump it, change the version and every hash in `build.rs` (`gh release view <v> --repo slskd/slskd --json assets` lists them).
- **Command line** (fixed for the run): `--app-dir`, `--http-ip-address 127.0.0.1`, `--http-port`, `--no-https`, `--headless` (lull is the UI; `--content-path` points at the app dir because slskd checks that its static folder exists even then), `--downloads <transfers>/complete`, `--incomplete <transfers>/incomplete`, and `--shared <music dir>` unless `LULL_SHARE=none`.
- **Env** (not in the process list): `SLSKD_API_KEY`, and slskd's own web login (`lull` and a random password, never the default).
- **`slskd.yml`** in its app dir (created empty before slskd's first start, so its watcher follows it) belongs to the hub: the app passes it as `LULL_SLSKD_CONFIG_FILE`, and the soulseek crate writes the account from `LULL_SOULSEEK_USERNAME`/`PASSWORD` into it and logs slskd in again (docs/context/soulseek.md). slskd reads env < yml < command line and reloads the file by itself.
- **One supervisor thread** owns the process and does every spawn. slskd never outlives the app: on Windows it sits in a kill-on-close job object, on Linux it gets `PR_SET_PDEATHSIG` (tied to the spawning thread, hence the one thread). Quitting stops it; so does an update.
- **Restarts:** slskd that stops by itself starts again after 5 s, then 10, 20 and so on up to 5 minutes while it keeps stopping; after 10 minutes of running the wait starts over at 5 s. A failed start is retried the same way. When the hub finds slskd wedged and fresh logins didn't help, `/health` says `services.soulseek.extra.restartSlskd` (docs/context/soulseek.md). Headless slskd refuses the web login the hub would restart it with, so a thread reads `/health` every minute and restarts the process, at most once per 30 minutes. Same port and API key each time, so the hub keeps its URL.
- **Licence:** AGPL-3.0, unmodified. `licenses/THIRD-PARTY.md` and slskd's `LICENSE` ship next to the app.

**Tailscale** (`tailscale.rs`): when this computer is the hub, Settings > App can show `LullDesktop.tailscale()`: `{ installed, running, url, serving }` from `tailscale status --json` and `tailscale serve status --json` (read-only). The listener's click on **Use Tailscale** calls `LullDesktop.tailscaleServe()`, which runs `tailscale serve --bg 8484` (20 s at most: it waits when HTTPS is off in the tailnet) and then stores the https address as `LULL_PUBLIC_URL` through `PATCH /api/config`, so pair codes carry it.

**Updates** (`updater.rs`): the app reads `latest.json` from the newest GitHub Release (`plugins.updater.endpoints`) at launch, every 6 hours while it runs, and when the listener asks. Background checks are silent: they only change Settings and the tray. A newer build downloads in the background and its signature is checked against `plugins.updater.pubkey`; nothing installs before the listener's **Restart to update**. Windows runs the NSIS installer passively and restarts lull; Linux replaces the AppImage, or installs the .deb or .rpm the app came from (after the system's password prompt), and restarts. GitHub gets 30 s to connect and to send each part of an answer.
- **States** (`updates::Status`, the same in Settings > App and the tray's update item above **Quit lull**): not checked yet (**Check for updates**); checking; up to date (**lull 0.1.0 is up to date**, a click checks again); downloading, with its percentage; ready (**Restart to update to lull X**); failed, in plain words where the cause is plain ("Couldn't reach GitHub. Are you offline?", "No lull release has been published yet"), else the updater's own message.
- A check while one runs, while an update downloads or while one waits returns the current state instead of starting another.

## Crashes

When a process behind a window's page dies, `recovery.rs` hears it (WebView2's `ProcessFailed` on Windows, WebKitGTK's `web-process-terminated` on Linux), writes one line to `logs/webview.log` and brings the window back, so the engine's own "This page is having a problem" never stays up.
- **lull's own process** crashing (not a page's) writes `logs\lull.dmp` (an unhandled-exception filter calling dbghelp's `MiniDumpWriteDump`, with the path made at start), then Windows ends it as usual.
- **One WebView2 environment** for every window (`recovery::builder`), made on the UI thread before the first: wry's own options plus `IsCustomCrashReportingEnabled`, so dumps stay in `EBWebView\Crashpad\reports` on this computer. The log's first line each run names the WebView2 version and that folder.
- **The log line:** the window, the process (renderer, unresponsive renderer, browser, GPU, a helper), the reason, the exit code (`0xC0000005` is an access violation), WebView2's process description and the module it crashed in, then what the app was doing (Classic open, `main` asleep, the mini player, the song and second) and what recovery did. At 256 KB it moves to `webview.log.1`.
- **A renderer hung:** WebView2 says so when the page ignores input for a while (`RENDER_PROCESS_UNRESPONSIVE`). A reload can't run in a busy renderer (tried: the page stayed frozen), so recovery ends that process: the one hosting the window's main frame (`GetProcessExtendedInfos`, matched on the webview's `FrameId`). Its exit then goes on as below, counted once.
- **A renderer exited:** the window reloads. `main` then resumes the song, second and play state it last reported (`resume_after_crash`, once; not while Classic mode plays, since Webamp plays then). The Classic window takes its handoff again, kept current as Webamp reports and moved on by the time since, so Webamp starts where it was. The mini player asks `main` for the player again.
- **A loop** (one window's page dying 3 times in 2 minutes): `main` shows lull's own page (`crashed`) with **Reload**, which loads the web UI again with the same resume; Classic mode hands the queue back to `main` as Webamp would have; the mini player closes.
- **The browser process** (every window's) exited: lull restarts, at most once in 10 minutes (a stamp beside the log).
- **GPU and helper processes:** they come back by themselves; logged only.
- **If it happens** (the listener's steps): open `%LOCALAPPDATA%\io.github.tannhausercommons.lull` in Explorer (Win+R, paste it, Enter). Zip `logs\webview.log` (and `webview.log.1` if it's there), `logs\lull.dmp` if it's there, and the newest `.dmp` in `EBWebView\Crashpad\reports`, and send the zip with the time it happened and what lull was doing. A dump is a few MB and holds the page's memory at the crash: share it only with lull's maintainers. On Linux the log is in `~/.local/share/io.github.tannhausercommons.lull/logs/`; WebKitGTK keeps no dumps.

Verified in a debug build, crashing pages through DevTools (`Page.crash`, `Browser.crashGpuProcess`): `main` while playing (reloaded and played on from the same second), three in a row (lull's page, then Reload), the mini player, the GPU process (logged with `0xC0000005`, nothing else needed), the Classic window once (Webamp started again on the same song) and three times (the queue back in `main`, playing), and `main` while Classic played (it took the queue back when Webamp closed). Opening and closing Classic mode and the mini player 15 times with a song playing crashed nothing. A page hung in a loop and clicked (WebView2 reported it unresponsive after about 10 s) came back playing three times out of three, the third as lull's page (a loop). A 40-minute soak (songs of 20 s, MP3 and FLAC, Discord on) kept lull's own process flat (about 570 to 730 handles, 30 GDI and 44 USER objects, 95 MB), so nothing native grows per song or per event; `main`'s renderer went up and down between about 150 and 580 MB, collected about every 10 minutes.

## The seam with the web UI

The web UI never calls Tauri directly. It uses `window.LullDesktop` from `desktop.js`, and `desktop.js` uses these parts of the web UI's `App`:
- `App.player`: `current`, `currentId`, `state` (`playing`, `pos`, `buffered`, `received`, `durationKnown`, `shuffle`, `repeat`, `volume`, `muted`), `duration`, `toggle`, `next`, `prev`, `pause`, `setShuffle`, `cycleRepeat`, `setVolume`, `seekFraction`;
- `App.isLiked`, `App.toggleLike`, `App.isSaved`, `App.saves.start`, `App.go`, `App.design`, `App.prefs`, `App.banner.list` (the `sys-unreachable` banner means "not connected");
- events `player`, `tick`, `like`, `save`, `ready`, `design`, `prefs`, `route`;
- cover art: the largest `navigator.mediaSession.metadata.artwork`, fetched by the web UI and passed on as a data URL.

**What the web UI calls (`LullDesktop`):**
- `inWrapper`: show Settings > App's desktop rows only when true.
- `settings()` resolves to `{ server, hub, tray, closeToTray, launchAtLogin, startInTray, miniStyle, miniOnTop, ownTitlebar, glass, window, discordPresence }`. `glass` is Settings › Appearance's **See-through window**; `window` is what `main` shows now (`glass`, below). `server` is the hub's address (`http://127.0.0.1:8484/` when this computer is the hub); `hub` is `local`, `remote` or null. `ownTitlebar` is null off Linux; `discordPresence` is null while the build has no Discord application; without a tray, Close always quits.
- `setSettings(patch)` takes any of those keys except `server`, `hub`, `tray` and `window`. Unknown keys are refused.
- `glass`: the see-through window. `state` is `{ material, held }` (null until the app answered), and `on(fn)` hears every change. `desktop.js` asks for it itself.
- `openMini()`: the player bar's Device view button.
- `openClassic(handoff)` and `onClassic(fn)`: Classic mode (`web/screens/classic.js`), see below. An app older than the page rejects `openClassic`, and the page shows Webamp on its own stage instead.
- `windowAction('change-server')`: Settings > App, "Change hub…" (the welcome page).
- `updates`: this app's own version and updates (Settings > App, "App version"). `status()`, `check()` and `install()` resolve to `{ current, state: idle | checking | current | downloading | ready | failed, version?, received?, total?, error? }` (`install()` resolves to nothing: the app restarts; it rejects when the install failed); `on(fn)` hears every change. `check()` resolves once GitHub has answered; a download goes on, reported through `on`. Android's `window.LullNative.updates` has the same calls without `on` (android.md).
- `tailscale()` and `tailscaleServe()`, only when `hub` is `local`: Settings > App's "Reach this hub from anywhere" row (see Tailscale above). Both resolve to `{ installed, running, url, serving }`; `tailscaleServe()` rejects with Tailscale's own message. Call `tailscaleServe()` only from the listener's click: it changes their Tailscale.

`desktop.js` does the rest itself in the wrapper: title bar, Ctrl+Shift+M, the close dialog, tray and mini-player actions, `set_now_playing`, and design and theme reports. It sends `set_now_playing` on player events, not as the song plays: a new position goes out only when it jumped.

**Commands** (`allow-<name>` permissions, from `build.rs`):

| Command | Args | Who may call |
|---|---|---|
| `window_action` | `{ request: { action, … } }`, where `action` is one of `full`, `mini-close`, `mini`, `this-computer`, `connect`, `pair {url, code}`, `change-server`, `retry`, `close-choice {keep, remember}`, `media-keys`, `classic-done` | bundled page, hub origin |
| `discover_hubs` | → `[{ name, url }]`, the hubs that answered mDNS within 3 s | bundled page |
| `find_hub` | `{ url }` → `{ state: ok, url, health } \| { state: unreachable, error }`, or an error for an invalid address | bundled page |
| `set_now_playing` | `{ now: { title, artist, playing, liked, saved, connected, id, album, position, duration } }` (the last four optional; seconds) | hub origin |
| `classic_open` | `{ handoff: { tracks: [{ id, url, title, artist, album, duration, art }], index, pos, playing, shuffle, repeat, volume, skin, skins } }`; every URL on the hub `main` shows | hub origin (`main`) |
| `classic_handoff` | → the handoff, once; null after | hub origin (Classic window) |
| `classic_fit` | `{ width, height, zooms }`: Webamp's size at 1×, the sizes it may take (the listener's first) → the zoom taken: the first that fits the work area of the window's monitor (else the last); the webview zooms to it, the window fits Webamp and moves back inside that work area | hub origin (Classic window) |
| `desktop_settings`, `set_desktop_settings` | `{ patch }` | hub origin |
| `tailscale_status`, `tailscale_serve` | → `{ installed, running, url, serving }`; an error when this computer isn't the hub | hub origin |
| `resume_after_crash` | → `{ id, position, playing }` once after `main`'s page reloaded from a crash, else null | hub origin (`main`) |
| `window_glass` | `{ request: { materials: [mica \| acrylic \| blur \| transparent], dark, reduced } \| null }` → `{ material, held: off \| system \| unsupported \| restart \| null }` | hub origin (`main`) |
| `app_update`, `app_update_check`, `app_update_install` | → the update report above; install → nothing (restarts) or an error | hub origin (`main`) |

The hub origin gets its capabilities at runtime when the app first loads it: one for `main`, a smaller one for the Classic window (`core:window` show, size, focus, minimise, drag; events; `window_action`, `classic_handoff`, `classic_fit`). An origin used earlier in the same run keeps its grant until restart (Tauri can't revoke one).

**Events:**

| Event | From → to | Payload |
|---|---|---|
| `tray` | Rust → the playing window (`main`, or the Classic window while open) | `{ action: play-pause \| next \| previous \| like \| save \| classic \| stop }` (`stop` comes from the Linux media-key fallback); also the taskbar buttons |
| `classic` | Classic window → `main` | `{ type: now, id, playing, pos, duration }`, `{ type: listen, id, duration }`, `{ type: listened, how, seconds }`, `{ type: action, action: like \| save, id }`, `{ type: back, back: { ids, index, pos, playing, shuffle, repeat, local } \| null }`, `{ type: failed, message }` |
| `classic-close` | Rust → Classic window | none: hand the queue back, then `classic-done` |
| `mini` | mini window → `main` | `{ action: play-pause \| previous \| next \| like \| shuffle \| repeat \| vol-up \| vol-down \| seek \| hello, fraction? }` |
| `player` | `main` → all | snapshot `{ track, playing, liked, pos, duration, durationKnown, buffered, received, shuffle, repeat, volume }`, sent on change and in answer to `hello` |
| `player-art` | `main` → all | `{ art }`: the cover as a data URL (or `null`), once per song and in answer to `hello`. Never inside `player`: that goes out with every position change, and a cover in it grew the page's memory by tens of MB a minute |
| `appearance` | `main` → Rust and the mini player | `{ design, theme }`; stored and passed in the hash of every bundled page |
| `ask-close` | Rust → `main` | none: show "keep playing in the tray?" |
| `app-update` | Rust → all | the update report, on every change of the check (progress per whole percent) |
| `glass` | Rust → `main` | `{ material, held }`, when what `main` shows changed without the page asking (the listener's switch, Windows' transparency effects) |

## Behaviour worth knowing

- **Close:**
  - on a bundled page, or without a tray, it quits;
  - otherwise it hides `main`, asking once first. "Don't ask again" stores the answer as `closeToTray`;
  - setting **Close to tray** in Settings also counts as answered.
- **Mini player:**
  - opening it hides `main`; closing it (button, Alt+F4, tray) shows `main` again;
  - **Show lull** and **Classic mode** in the tray close it first;
  - a second launch focuses the mini player if one is open;
  - **its window is the device:** frameless and transparent, the page without any wallpaper (an unlayered, important rule in `wrapper.html`, since a design's wallpaper is unlayered CSS and beats `desktop.css`'s layer). It opens at 340×560 (device) or 540×90 (strip), then `desktop.js` sets its height to its design's player (`[data-dk-player]`), following it as it changes; the width is the design's;
  - the strip's wrapper keeps 18 px below it for the deepest design's shadow (glasshouse).
- **Taskbar buttons (Windows):** Previous, Play/Pause and Next under the thumbnail of `main` and of the Classic window (`ITaskbarList3`). They are added when Windows reports the window's taskbar button (`TaskbarButtonCreated`), show the latest `set_now_playing` (Play or Pause; greyed while idle or not connected), and click through `player::forward`. The glyphs are drawn at the display's small-icon size, dark on a light taskbar, white on a dark one.
- **System media overlay (Windows):** WebView2 gives the playing page a system media session from its `navigator.mediaSession`: title, artist, cover and Previous/Play/Pause/Next all work (checked through `GlobalSystemMediaTransportControlsSessionManager`). Windows names the session after `msedgewebview2.exe`, not lull, and Chromium passes no album. In Classic mode, `winamp.js` sets the session from the hand-off's titles and covers.
- **Classic mode:** the web UI's `/classic` hands the queue to `classic_open` and goes back to the page it came from. The Classic window (borderless, transparent, not resizable) loads `<hub>/desktop/winamp.html`, takes the handoff, starts Webamp where the app was, fits itself to Webamp's three windows, and moves by their title bars (`startDragging`). `main` hides and sleeps: WebView2's `IsVisible` off and its low memory target. Measured with a song paused: `main`'s renderer went from 2.4 s of CPU per 20 s (visible, its design's animations) to 0.18 s, its page tasks from 1.2 s to 0.005 s. Its page keeps running, so the listening history, Like and Save stay the web UI's. Closing Webamp, Alt+F4 or the tray's **Show lull** hands the queue, song, second, shuffle and repeat back; the tray's **Classic mode** brings the window forward.
- **Classic size** (1× to 3×, Settings › Appearance › Classic) is the Classic webview's page zoom (`classic_fit`): CSS pixels scale with it, so Webamp's own pointer math, title-bar drags and sliders stay exact, and its `image-rendering: pixelated` keeps skins sharp. A size the monitor's work area can't hold gives way to the largest that fits, with a note. An app older than `classic_fit` (0.2.0) stays at 1× and says so; the web page falls back to `setSize`.
- **Links:** every window's navigations and new-window requests go through `links.rs`: anything http(s) that isn't a bundled page or a hub this run opened opens in the default browser, redirects included. Last.fm's sign-in (`last.fm/api/auth`, reached by the hub's redirect) opens in the browser with its `cb` pointed at `http://127.0.0.1:<port>/lastfm`; when Last.fm sends the browser there, `main` loads the hub's own callback with that token and its cookie. A sign-in never finished keeps that listener's thread waiting until lull quits.
- **Discord:** with **Show what I'm playing on Discord** on, a thread speaks Discord's IPC (`discord-ipc-0..9`) as lull's public Discord application (`APPLICATION_ID` in `discord.rs`; no OAuth, no secret: each listener's own Discord client shows their presence). A "Listening to" activity (type 2): `details` the song (linked to its song.link), `state` the artist (linked to the artist's share page), the cover as the large picture with the album on hover (linked to the album's page), lull's glass badge as the small picture, start and end times (the progress bar), and the buttons "Open on song.link" and "Get lull" (other people see them; Discord hides them from the listener). Links and covers come from the hub's `GET /api/share/{track,artist,album}/{id}`, remembered per id for the run. Settings > App sets its look (`Look`, stored as `discord` in `settings.json`): what follows "Listening to" in the member list (`status_display_type`: song, the default; artist; or lull), the album cover (on; off shows lull's sky picture), the buttons (on), and a pause (hide it, the default; or show it with the paused badge and no progress bar). The thread waits 0.8 s for a burst of skips or seeks to settle and sends only what Discord would show differently (a start moved by under 2 s doesn't count): Discord allows 5 changes per 20 s. Without Discord it looks again every 20 s while a song plays. Nothing in lull reacts to games: Discord keeps one activity per application, so a game's own activity sits beside lull's. The pictures are `app/discord/*.png`, made from the SVGs there by `uv run app/discord/render.py` and fetched by Discord from GitHub (`raw.githubusercontent.com`, through Discord's media proxy, which caches them).
- **Tray glyph (Windows):** follows the taskbar theme (`SystemUsesLightTheme`), not the app theme. A thread waits on the registry key, because Tauri's ThemeChanged only tracks the app theme. Its size follows the primary monitor's scale.
- **Linux:**
  - **Tray:** only with a StatusNotifier host on D-Bus; without one there's no tray and Close quits. The tray always gets `tray-dark-32.png`: Tauri hands AppIndicator a file, so a themed icon name can't be used.
  - **Title bar:** native by default. With **Use lull title bar** (`ownTitlebar`, restart), the window is undecorated and draws GNOME header-bar buttons in the order of `button-layout`.
  - **Mini player:** opaque `--sky` fallback without a compositor. On Wayland it shows a one-time note about Always on Top.
  - **Media keys:** grabbed with global shortcuts only when the web UI reports no `navigator.mediaSession` (X11 only). Windows never grabs them: SMTC handles them through Media Session. That path works only while a media element plays: Chromium gives a page a system media session only then. Verified on Windows with a Jellyfin song: Play/Pause, Next, Previous and Stop all reach the web UI's own handlers.
- **A see-through `main`** (`glass.rs`, docs/themes.md section 12):
  - **The request.** The theme in use asks through `window_glass`: its materials in its order, for the mode shown. `designs.js` resolves theme.json's `window`, and `desktop.js` asks again whenever the theme or its mode changes.
  - **The grant.** The app grants the first material this computer draws without lag, unless **See-through window** is off (`held: off`), Windows' Transparency effects are off or the page reports `prefers-reduced-transparency` (`system`), none is drawable (`unsupported`), or Linux made `main` solid (`restart`). The page sets `:root[data-window]` from the answer, and from the `glass` event when the listener's switch or the system changes it.
  - **Kept for the next launch.** The request is stored as `window` in `settings.json`, so the next launch opens `main` as the theme wants it.
  - **Windows.** `main` changes live: blur-behind with an empty region lets the page's alpha through, the webview's background turns clear (solid white again when off), and window-vibrancy draws the material. Mica and Acrylic also extend the frame over the whole window, because the system draws them in the frame.
    - What each Windows draws without lag: Mica from build 22000; Acrylic from 22523, as a system backdrop; plain blur only on Windows 10; clear everywhere.
    - Acrylic before 22523 and plain blur on Windows 11 go through the old accent policy, whose window trails the pointer when dragged.
    - Measured on Windows 10 22H2 (165 Hz). A script moved `main` across 400 px, then resized it by 220×160 px, 60 times a second for 5 s each, timing how long DWM took to show each step (`DwmFlush`):

      | `main` | Moving: average, 95th percentile, steps over 1.5 refreshes | Resizing: the same |
      |---|---|---|
      | made opaque (as 0.2.1), solid theme | 4.2 ms, 6.9 ms, 0% | 6.1 ms, 8.9 ms, 5% |
      | made transparent, solid theme | 4.2 ms, 6.8 ms, 0% | 6.5 ms, 9.3 ms, 7% |
      | clear | 4.7 ms, 7.4 ms, 0% | 6.7 ms, 9.3 ms, 7% |
      | blur (two runs) | 4.7–4.8 ms, 7.4–7.5 ms, 0% | 6.9–7.1 ms, 9.3–9.6 ms, 9–12% |
      | acrylic (accent policy) | 10.3 ms, 13.0 ms, 71% | 17.7 ms, 18.5 ms, 97% |

    - Idle, with nothing moving, lull's own processes used the same few tens of ms of CPU per 20 s with glass or without, and no GPU. DWM redraws the blur only when something behind the window changes.
    - The Personalize registry watch (`follow_personalize`) also follows Transparency effects.
  - **Linux.** A window gets an alpha channel only when it is made. `main` is made with one when the stored request would be granted (a compositor is needed). Blur is KWin's `_KDE_NET_WM_BLUR_BEHIND_REGION` on X11; GNOME and Wayland get clear glass only.
  - **The title bar.** On Windows it is always lull's own, so nothing changes. On Linux the native frame or lull's own (`ownTitlebar`) stays as set.
  - **Bundled pages** in `main` (welcome, unreachable) don't take part: they paint their wallpaper, so the window looks solid.
- **Window state:** the plugin restores size, position and maximised for `main`, `mini`, `strip` and `classic` (the mini player's and Classic's pages then fit their size). Visibility and decorations are always the wrapper's choice.

## Known limits

- The Snap Layouts flyout on the custom Maximise button doesn't appear (DESKTOP.md 2.1).
- Pages from other origins get no IPC.
- In the Classic window Webamp's windows stay docked together (the window moves instead), and its double-click on a title bar (window shade) is lost to the drag. Skins are chosen in Settings › Appearance › Classic, in `main`; the window follows a new choice at once (the `storage` event, docs/context/skins.md), shows Webamp's own Skins menu (favourites), and takes dropped `.wsz` files for that session only. Webamp's alerts show as a note over its windows, never as a dialog.
