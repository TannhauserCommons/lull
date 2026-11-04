# lull desktop (Tauri v2 wrapper): design spec

The wrapper is thin: one webview that loads the web UI from the configured server URL, plus a tray, OS media controls, a mini-player window and a first-run connect screen. It holds no player logic. Everything below is a design decision for that wrapper.

Files: `preview.html` (design board, open from disk), `window.html` (one window's content, used by the board), `desktop.css` + `desktop.js` (the wrapper's own UI: title bars, connect and unreachable pages, close-to-tray dialog, mini player; `window.LullDesktop`), `preview.css` + `preview.js` (board only). Icons: `../assets/icons/`. Loaded after `prototype/app.js`, `desktop.js` adds a `/desktop` route and `desktop-*` states to the prototype's state switcher, and `?titlebar=win|gnome` on any route previews the title bar in the real shell.

## 1. Windows and their chrome

The wrapper follows the user's design (lull, skydeck, starlight, petal, glasshouse) and theme everywhere: the main window gets them from the web UI itself, the mini window and the bundled connect / unreachable pages get them from the wrapper (6.1). The board shows every window in each design (`#design=<name>`).

| Window (label) | Size | Resizable | Chrome |
|---|---|---|---|
| `main` | default 1280x800, min **480x600**, centred on first run | yes | Windows: custom (2.1). Linux: native by default (2.2) |
| `mini` (device) | 340x560 fixed (device 320 wide + stack) | no | frameless, transparent, always on top |
| `strip` | 540x90 fixed | no | frameless, transparent, always on top |

- Min 480 wide because the phone layout of the web UI plus the Windows caption buttons (138 px) needs it. Below 640 the web UI is in its phone layout; that is fine in a window.
- **Remember position and size** per window with the official `tauri-plugin-window-state`. On restore, clamp to a connected monitor (an unplugged screen must not hide the window). On Wayland the compositor places windows; only size is restored.
- **Single instance** with the official `tauri-plugin-single-instance`: a second launch shows, un-minimises and focuses `main` (or the open mini window). Needs D-Bus on Linux.
- `dragDropEnabled: false` on every window: on Windows the native file-drop handler blocks HTML5 drag and drop, and the web UI needs it (row drag, M3U and `.wsz` drops).
- Close button: **close to tray** when a tray exists (Windows always; Linux only when a StatusNotifier host is on D-Bus, see 4). The first time, the web UI shows a one-time dialog "Keep playing in the tray?" (Keep playing / Quit lull, "Don't ask again" on by default). Without a tray, close = quit.

## 2. Title bars

### 2.1 Windows 11: custom title bar integrated into the app header

Decision: `decorations: false`, `shadow: true` (keeps the 1 px OS border, the Windows 11 rounded corners and the drop shadow on an undecorated window). Reason: a 32 px native title bar on top of the app header wastes height and puts a grey OS strip over the night sky; integrating it is what Windows recommends for apps with their own header.

- **Caption buttons** top-right, flush with the window's top and right edges: Minimise, Maximise/Restore, Close; each **46x32**, total 138x32. Glyphs 10 px, 1 px stroke, `--fg` of the sky surface. Hover `--bg-2`, pressed `--bg-3`. **Close hover = `--down` fill with `--lilac` glyph** (6.2:1). Inactive window: glyphs go to `--fg-2`. Buttons are real `<button>`s with `aria-label` and the shared tooltip.
- **Drag region**: `data-tauri-drag-region` on `.header`, its `.spacer`, `.header__brand`, `.rail__head` and a 16 px transparent strip across the top of the window (the shell's outer margin). The attribute only applies to the element that carries it, so each of those needs it. Double-click toggles maximise (Tauri does this). Interactive children (search, tray, buttons) keep working.
- The header reserves `padding-right: 138px` (topnav / dock strips: `margin-right: 130px` + 8 px padding) and its children may shrink, so its tools never sit under the caption buttons (`:root[data-titlebar="win"]`, in `desktop.css`); below 1440 px wide the tray drops its long labels.
- Hook-up: the web UI ships `desktop.css`/`desktop.js` too; the wrapper's initialization script calls `LullDesktop.attachTitlebar('win')` on Windows (`attachTitlebar('gnome', buttonLayout)` when the Linux option is on). In a browser nothing is attached.
- Capabilities for the main window: `core:window:default`, `core:window:allow-minimize`, `core:window:allow-toggle-maximize`, `core:window:allow-close`, `core:window:allow-hide` (close to tray), `core:window:allow-start-dragging`, `core:event:default` (the `tray` listener), plus the app commands in section 4 (Wiring).
- **Snap**: drag-to-edge snapping, Win+arrows and Win+Z work, because dragging uses the native move loop. The **Snap Layouts flyout on hover over Maximise does not** appear in core Tauri v2: the webview covers the hit-test. Accept this for v1. Third-party plugins add a native overlay for it; vet one before adopting it.
- Maximised: the window loses its rounded corners; nothing in the layout changes.

### 2.2 Linux: native decorations by default

Decision: `decorations: true` by default. Reasons: GNOME users expect the window controls their `button-layout` setting chooses (often close only); KDE users expect KWin's server-side Breeze title bar with their own button set, and GTK3 apps get server-side decorations on KWin. A web-drawn title bar can't follow either reliably, and Linux has no shadow for undecorated windows.

- GNOME: GTK draws its own header bar (title "lull", controls per `button-layout`).
- KDE Plasma: KWin draws the Breeze title bar (icon, title, buttons per the user's theme).
- **Optional** setting "Use lull title bar" (Settings > App, Linux only, off by default) follows GNOME header-bar conventions: `decorations: false`; the app header becomes the header bar; round 24 px window buttons (34 px hit) in the order the wrapper reads from `org.gnome.desktop.wm.preferences button-layout` (left or right side, any of minimise/maximise/close); 1 px `--edge` border around the window since there is no shadow; the same drag regions as Windows. Turning it on or off needs a restart, and the setting says so.

## 3. Mini player (device window and strip)

Opened from the tray ("Mini player"), the player bar's Device view button (inside the wrapper; in a browser it opens Now Playing) or Ctrl+Shift+M. `main` hides while a mini window is shown; "Open full window" brings it back. Style (device or strip) is a setting.

- **Device** (the default markup; each design restyles it, and a design.js may replace it through the `miniPlayer` slot: lull draws it as a small lull window with a transport row, skydeck as its handheld): the handheld from the Now Playing spec at 320 px wide: screen with art, title, artist, solid quality chip, waveform strip and pixel-face times; D-pad (left/right = previous/next, up/down = volume), A = play/pause (gloss 56), B = like (gloss 48), shuffle and repeat pills. Top edge of the body: LED + "lull" label, then "Open full window" and "Close mini player" (32 px visual, 44 px hit). Drag region: the device body and the bezel (`data-tauri-drag-region`), never the screen (the waveform seeks).
- **Strip**: 520x64 `.s-panel` bar, `--r-panel`, `--stack-1l`, 2 px seek line on its top edge: bezel thumbnail, title/artist (drag region), previous, gloss play 40, next, like, open full window, close.
- **Transparency**: the window is transparent (`transparent: true`, `decorations: false`, `shadow: false`, page background `transparent`) so the device silhouette, with its bottom-right swoop and its stack, is the window shape. Windows 10/11: works; use `noRedirectionBitmap` to avoid a white flash. Linux: needs a compositor. Fallback when the wrapper can't get an alpha visual (X11 without compositing): opaque window, `--sky` background with the device inset 10 px (`.dk-mini--opaque`). Don't use the OS shadow on transparent windows; the stack is the shadow.
- **Always on top**: on by default. Windows and X11 honour it. Wayland compositors ignore app-set keep-above and position; the first time the mini opens on Wayland, show a toast in it: "Your desktop decides what stays on top: use the window menu (Alt+Space) > Always on Top."

## 4. Tray

- Icon: monochrome mark (the app icon's rounded square, solid, with the lull sparkle knocked out). Files in `../assets/icons/`:
  - Windows: `tray-light-{16,20,24,32}.png` (dark glyph, for a light taskbar) and `tray-dark-{16,20,24,32}.png` (light glyph, for a dark taskbar); 16/20/24/32 = 100/125/150/200 % scaling. Pick by the **taskbar** theme (`SystemUsesLightTheme` under `HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize`), not the app theme, and swap on `WM_SETTINGCHANGE` "ImmersiveColorSet".
  - Linux: install `tray-symbolic.svg` as `lull-tray-symbolic` in the hicolor theme so the panel recolours it; when only a file path can be passed (AppImage), use `tray-dark-32.png` (panels are dark by default).
- Tooltip (Windows): "lull: Paper Moon Static, Velvet Orchard" or "lull: nothing playing".
- Click: Windows left click shows/hides `main`; right click opens the menu. Linux: tray clicks are not delivered (AppIndicator), so the menu opens on any click and holds every action.
- GNOME shows tray icons only with the AppIndicator extension (Ubuntu ships it). Detect a StatusNotifier host (`org.kde.StatusNotifierWatcher` on the session bus); without one, don't create the tray and make Close quit.
- **Native menu** (drawn by the OS; labels and order are ours, sentence case):

| # | Item | Kind | Notes |
|---|---|---|---|
| 1 | `Paper Moon Static, Velvet Orchard` | disabled text | "Nothing playing" when idle, "Not connected" when the server is unreachable |
| - | separator | | |
| 2 | `Pause` / `Play` | normal | label follows state |
| 3 | `Next` | normal | |
| 4 | `Previous` | normal | |
| - | separator | | |
| 5 | `Like` | check item | checked when liked |
| 6 | `Save to library` | normal | `Saved` + disabled once saved |
| - | separator | | |
| 7 | `Show lull` | normal | |
| 8 | `Mini player` | check item | checked while a mini window is open |
| 9 | `Classic mode` | normal | shows `main` and opens Classic mode |
| - | separator | | |
| 10 | `Quit lull` | normal | |

  Items 2-6 are disabled while not connected or idle.
- Wiring (no player logic in the wrapper). `desktop.js` section 8 already does the web UI side when `window.__TAURI__` is present:
  - Tray to web UI: the wrapper handles **Show lull** and **Mini player** itself (they are window operations) and shows `main` before forwarding **Classic mode**. Every other item emits a `tray` event to `main` with `{ action: "play-pause" | "next" | "previous" | "like" | "save" | "classic" }`; the web UI runs its own actions.
  - Web UI to wrapper, three commands the wrapper implements:

| Command | Args | Called by |
|---|---|---|
| `set_now_playing` | `{ title, artist, playing, liked, saved, connected }` | `main`, whenever any of them changes: tray labels, check items and tooltip |
| `window_action` | `{ action: "full" }`, `{ action: "mini-close" }` | mini windows: open the full window / close the mini window |
| | `{ action: "mini" }` | `main`: Ctrl+Shift+M or the player bar's Device view button (not wired in `desktop.js` yet) |
| | `{ action: "connected", url }`, `{ action: "change-server" }` | local connect / unreachable pages: save the server and load the web UI / open the connect page |
| `quit` | none | "Quit lull" in the close-to-tray dialog |

  - Mini windows mirror `main`'s player and send their buttons back; `desktop.js` drives them with a mock player today. Wire it the same way as the tray: `main` emits the current track, play state, like state, position, duration and buffered end to the mini windows on its `player` / `tick` / `like` events, and the mini windows emit `{ action }` events to `main` for their `data-dk` buttons (`play`, `prev`, `next`, `like`, `shuffle`, `repeat`, `vol-up`, `vol-down`, `seek` with a fraction), which `main` runs as its own actions.

  Minimise, maximise, hide and dragging use the window API directly (capabilities in 2.1). Grant IPC only to the configured server origin and the bundled local pages (runtime capability with that remote URL) and only these commands.

## 5. OS media controls

- **Primary path: the Media Session API** that the web UI already sets (metadata, artwork, playbackState, action handlers, position state). Nothing extra in the wrapper.
  - Windows (WebView2): Media Session works and reaches the system media controls (SMTC: volume flyout, lock screen, hardware media keys).
  - Linux (WebKitGTK): Media Session maps to **MPRIS** (GNOME/KDE media widgets, media keys) in WebKitGTK builds that enable it (default again since 2024). Older or distro builds may ship without it, so feature-detect `'mediaSession' in navigator`, never sniff versions.
- **Fallback** (Linux only, when `navigator.mediaSession` is missing): the web UI tells the wrapper, which registers global shortcuts `MediaPlayPause`, `MediaTrackNext`, `MediaTrackPrevious`, `MediaStop` with the official `tauri-plugin-global-shortcut` and forwards them as `tray` events. X11 only: Wayland has no global key grabs, so there the fallback is the tray menu. Do not register these on Windows (they would fight SMTC).
- Artwork: the server's cover URL, or `../assets/fallback-art-512.png` when there is no art.

## 6. First-run connect screen and unreachable state

Both are **local pages bundled in the wrapper** (the server isn't known or isn't answering, so the web UI can't load). They use the same `tokens.css`, `base.css`, `components.css`, `icons.js`, fonts and `desktop.css`, plus the design (6.1), so they match the app. On Windows they include the caption buttons (no native title bar).

### 6.1 Design and theme in wrapper pages

Every page the wrapper owns (connect, unreachable, the mini window) loads the design like `prototype/index.html` does: `<link rel="stylesheet" id="design-css" href="designs/lull/design.css">` after `desktop.css`, then `designs/designs.js`, and a pre-paint script that calls `Designs.boot()` and `Designs.applyTheme(theme)` (see the head of `window.html`). These pages can't read the web UI's own prefs (a bundled page has another origin than the server, so `localStorage` is not shared), so the wrapper passes them in:

- The main window reports `{ design, theme }` to the wrapper (a Tauri event) on load and whenever the web UI emits `design` or `prefs` (`App.on('design', ...)`, `App.on('prefs', ...)`; read the values from `App.prefs()`). The wrapper keeps the last value in its store.
- It opens the mini window and the connect / unreachable pages with `#design=<name>&theme=<day|night|system>` in the URL (`Designs.wanted()` already reads `design=` from the hash; the pre-paint script reads `theme=`), using the stored value, or `lull` / `system` before the first report.
- A change while the mini window is open: send it the same event and let it call `Designs.set(name)` and `Designs.applyTheme(theme)`.

- **Connect** (= the token gate look with a URL field above): wordmark on the sky, one `.s-card` dialog: "Server address" (`https://music.home.arpa`, help "The address you open lull at in a browser."), "Access token" (password field with Show/Hide, help "Only if your server sets one."), "Remember on this device" toggle (stores the token in the OS credential store), **Test connection** (secondary) and **Connect** (primary, tests first). A fixed-height status line (no layout shift, `role="status"`):
  - idle: "lull checks the address and token before saving."
  - testing: dashed spinner + "Testing music.home.arpa…" + Cancel.
  - ok: status-ok glyph + "Connected to lull 0.1. Soulseek, Jellyfin and Last.fm answered."
  - unreachable: status-down glyph + "Can't reach music.home.arpa. Check the address and that this computer is on your home network or VPN." URL field gets the error.
  - token rejected: token field error "Token rejected" + status "The server answered but didn't accept this token."
  - invalid address (client-side): "Enter a full address, like https://music.home.arpa".
  Test: `GET <url>/health` with a 10 s timeout, then an authenticated `GET <url>/api/ping`.
- **Server unreachable at launch**: same window, the cracked-star empty state: "Can't reach your server", what to check, a countdown "retrying in 8s" (backoff 2, 4, 8, 16, 30 s, then every 30 s, and at once when the network changes; the app retries, hidden or not, and the page shows each attempt), **Retry now** (primary), **Change server** (secondary, opens Connect), and a Details disclosure (address, error, last connected, attempt). The tray shows "Not connected".
- Once the web UI is loaded, a dropped server is the web UI's own "Can't reach your server" banner; the wrapper does nothing extra.

## 7. Settings > App (inside the wrapper only)

Server address (read-only + "Change server…"), Close to tray, Launch at login (official `tauri-plugin-autostart`), Start in the tray, Mini player style (device / strip), Mini player always on top, Use lull title bar (Linux only, restart), Forget token. Hidden in a browser.

## 8. Packaging icons

The mark is the lull design's language: a glossy lavender rounded square (the `--l-gloss` ramp, lighter at the top, a white highlight band, a soft bottom glow, indigo `#43389c` edge) holding the white lull sparkle, with a small pink sparkle and a dot of light. The wordmark (`../assets/wordmark.svg`, `wordmark-night.svg`) is the glossy "lull" pill from the lull design's header, outlined Audiowide (OFL), so it needs no font.

All generated by `mise run icons` (`scripts/icons/`: resvg for the sizes and `app.ico`, the pinned Tauri CLI for Android's launcher icons; it also copies them to `web/assets` and the fastlane store icon) into `../assets/icons/` from three masters: `../assets/app-icon.svg` (everything except the two below), `../assets/icons/app-icon-small.svg` (hand-tuned 16/24/32 on the pixel grid: heavier edge, one sparkle, pink accent only from 32 up) and `../assets/app-icon-maskable.svg` (full bleed, mark inside the 80 % safe circle, source of `icon-maskable-512.png`). Media Session fallback art: `../assets/fallback-art-{96,256,512}.png`, rendered from `../assets/fallback-art.svg`.

| Target | Files |
|---|---|
| Windows exe + installer | `app.ico` (16, 24, 32, 48, 64, 256; 16-32 hand-tuned) |
| Tauri `bundle.icon` | `icon-32.png`, `icon-128.png`, `icon-256.png` (as 128@2x), `app.ico` |
| Linux hicolor (deb, rpm, AppImage) | `icon-{16,24,32,48,64,128,256,512}.png` + `../assets/app-icon.svg` as scalable + `tray-symbolic.svg` |
| PWA manifest | `icon-192.png`, `icon-512.png`, `icon-maskable-512.png` (`purpose: maskable`, content inside the 80 % safe circle) |
| Tray | `tray-light-*.png`, `tray-dark-*.png`, `tray-symbolic.svg` |

Regenerate the large sizes from the master; never upscale the small ones. The `tauri icon` CLI output can be used for Store tiles, but replace its 16/24/32 and `.ico` with these.

## Sources

Tauri v2 docs: window customization (custom title bar, `data-tauri-drag-region`, permissions), system tray (Linux click events not emitted), config reference (`shadow`, `transparent`, `dragDropEnabled` on Windows), plugins (single-instance, window-state, global-shortcut, autostart). tauri-apps/tauri issue 6913 (Media Session: works in WebView2; WebKitGTK re-enabled it by default in 2024). WebKitGTK 2.38 release notes (Media Session over MPRIS). Microsoft Windows 11 title bar guidance (caption buttons, drag regions). GNOME HIG (header bars, window controls follow `button-layout`). KDE HIG (server-side decorations).
