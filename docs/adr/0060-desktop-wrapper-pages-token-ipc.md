# ADR-0060: Desktop wrapper: one bundled page, the token as the server's cookie, IPC granted at runtime

- Status: accepted
- Date: 2026-10-31

## Context

The desktop wrapper (`desktop/`, spec `design/desktop/DESKTOP.md`) loads the web UI from a server the user picks at runtime. Before that it shows its own connect and unreachable pages, and it opens mini-player windows. Four questions had more than one reasonable answer:
- where those pages and their assets come from;
- how a token typed into a bundled page reaches the web UI, which runs on another origin;
- how the web UI gets IPC when its origin isn't known at build time;
- what remembers the token.

## Decision

- **One bundled page, `web/desktop/wrapper.html`**, picks its view from the hash (`view=connect|offline|mini|strip`, plus the design and theme). `build.rs` copies only the web files it needs into `desktop/dist/`. Rejected:
  - pointing `frontendDist` at `web/`: that would embed the whole web UI and any untracked `node_modules` or `vendor`;
  - one HTML file per view: four copies of the same head.
- **Rust talks to the server** for the connect test, the launch probe and retries (ureq, using the OS trust store like the webview). The bundled page can't read `/health` itself: it's cross-origin and the server sends no CORS headers.
- **The token becomes the server's own cookie before the web UI loads.** Rust posts it to `/api/session` and puts the returned `lull_token` cookie into the webview (`set_cookie`), then loads the server. The web UI never sees a token gate, and the cookie format has one source, the server. Rejected:
  - asking the web UI to post it after loading: the token gate flashes, and the web UI would need a wrapper-only code path;
  - an initialization script carrying the token: it would sit in every page load of `main`.
- **The webview's cookie jar remembers the token, not the OS keychain** (DESKTOP.md said keychain). It's HttpOnly, scoped to the server, and kept for a year, exactly as in a browser. The keychain would hold a second copy that the webview can't use without the same cookie step. "Remember on this device" off drops the cookie's expiry, so it ends when lull quits.
- **IPC for the server origin is a runtime capability** (`add_capability`) for `main` only, holding the window permissions plus `window_action`, `set_now_playing`, `desktop_settings` and `set_desktop_settings`. The bundled page gets a static one (`capabilities/local.json`) with `test_server` and `window_action`. App commands have declared permissions (`AppManifest`), so nothing is granted by default.
- **One `window_action` command with a tagged argument** replaces DESKTOP.md's separate `quit` command: the close dialog sends `close-choice { keep, remember }`, so the "Don't ask again" answer is stored by the wrapper (it decides what Close does before the web UI loads).

## Consequences

- Editing the bundled page means rebuilding the wrapper; the web UI itself still reloads from the server.
- Changing to another server asks for its token again, because the wrapper's own check can't read the webview's cookie.
- An origin connected to earlier in a run keeps its IPC grant until restart; Tauri has no way to revoke a capability.
- The web UI's contract with the wrapper is `window.LullDesktop` plus the `App` members listed in `docs/context/desktop.md`. Renaming one of those breaks the tray, the mini player or the close dialog.
