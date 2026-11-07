# ADR-0063: the app chooses how its window is drawn, notices the processor, and never locks the listener out

- Status: accepted (owner, 2026-11-07)
- Date: 2026-11-07
- Landed: 34f941a3 (the choice and the lighter look), 9935dd04 and 86b9789f (the check), 3d015217 (the AUR package)

## Context

A listener on an NVIDIA hybrid laptop ran lull 0.2.5's AppImage on Arch at about 2 frames a second, with no video
memory in use; `prime-run` changed nothing, nor did `WEBKIT_DISABLE_DMABUF_RENDERER=1` with it. A listener with no
graphics card at all said it runs fine. lull set nothing about WebKitGTK's hardware acceleration and noticed nothing.

WebKitGTK falls back to the processor quietly: no usable EGL, a software GL (Mesa's llvmpipe), or
`WEBKIT_DISABLE_DMABUF_RENDERER`, which since 2.44 turns its GPU path off altogether. Its page scripts can't tell:
WebGL's renderer always reads "Apple GPU". WebView2 reports the true renderer.

## Decision

1. **A choice, read before anything starts.** Settings › App › Graphics: Automatic (the default), Use the graphics
   card, Processor only. WebKitGTK takes its switches from the environment, so lull reads `settings.json` first thing,
   before Tauri or any thread, and sets them; WebView2 gets browser arguments when the app makes its environment. A
   change takes a restart (Settings offers it).
2. **What each mode sets.**
   - Linux, the graphics card: clears the switches that turn the GPU off (`WEBKIT_DISABLE_COMPOSITING_MODE`,
     `WEBKIT_DISABLE_DMABUF_RENDERER`, `WEBKIT_SKIA_ENABLE_CPU_RENDERING`, `LIBGL_ALWAYS_SOFTWARE`) and, on a laptop
     with two cards, renders on the one that didn't show the boot screen: NVIDIA's render offload switches (as
     `prime-run` and switcheroo-control set them), Mesa's `DRI_PRIME=1`, and WebKit's `WEBKIT_WEB_RENDER_DEVICE_FILE`
     for that card's render node.
   - Linux, the processor only: compositing, the DMA-BUF renderer and Skia's GPU painting off, Mesa's software GL, and
     Mesa's EGL alone where installed, so a vendor driver that crashes is never loaded.
   - Windows: `--ignore-gpu-blocklist --force-high-performance-gpu`, or `--disable-gpu`.
   - The browser lull opens links in gets the environment lull was started with.
3. **lull finds out what draws.** Linux: WebKitGTK's own report, `webkit://gpu`, read once a run in a hidden webview.
   Windows: WebGL's renderer, as the page sees it. A software renderer (llvmpipe, softpipe, swrast, SwiftShader,
   Microsoft Basic Render Driver), no accelerated renderer, or the policy "never" means the processor. Each run's
   finding goes in `logs/webview.log`.
4. **The lighter look where the processor draws:** no blur behind glass and fewer animations (every animation and
   decorative loop stops; short colour and opacity fades stay), as the root attribute `:root[data-graphics="light"]`,
   a theme hook like `data-motion`. Settings says so in one line, with "Use the full look". Its CSS and code are the
   desktop app's own (`desktop.css`, `desktop.js`), which a browser never loads.
5. **Never locked out.** A start in a chosen mode is on trial from when the window shows (or its page dies) until the
   listener's first click or key. One still on trial at the next start goes back to Automatic, and Settings says why.
   Automatic is never on trial: it is what lull did before the setting.
6. **The test machines hold every design to it.** GitHub's runners have no graphics card: the real Linux app runs
   there in every mode (switches, a drawn window, the processor found, the lighter look, the fallback), and in the
   lighter look every design must scroll long pages at 2+ frames a second and keep Home at rest under 10% of a core.
7. **Arch listeners get lull on Arch's own WebKitGTK** (`lull-bin` in the AUR, from the release's .deb), as most Tauri
   apps there are, instead of the AppImage's bundled engine.

## Alternatives considered

- **Switches in a desktop file or launcher for everyone** (as many AUR packages set `WEBKIT_DISABLE_DMABUF_RENDERER`):
  every listener gets a workaround whether they need it or not, and since 2.44 it turns the GPU off. Rejected: lull
  decides per computer and says so.
- **Detecting with WebGL on Linux:** WebKit masks the renderer. Rejected.
- **An EGL probe of lull's own in Rust:** duplicates WebKit's logic and can disagree with it. Rejected for the engine's
  own report.
- **Measuring the frame rate as lull runs:** noisy, and needs something moving. Kept for CI only.
- **Automatic switching itself to "Processor only" when the processor draws:** measured slower than WebKitGTK's
  default on llvmpipe in almost every design and page. Rejected.
- **Windows' per-app GPU preference (registry):** keyed by the program's path, which for WebView2 is the shared
  runtime's, changing with each runtime update and affecting every app that uses it. Rejected.
- **Falling back from Automatic too** after a start that never came up: could keep a working GPU off for a listener
  who started lull in the tray and quit without opening it. Left out.

## Consequences

- One more short-lived webview at each start on Linux, after the window's first page.
- A new theme hook, `data-graphics`.
- A new check (`graphics.yml`) in the queue for `app/`, `web/` and `tests/desktop/`, about 8.5 minutes, beside the UI
  checks; `tests/desktop/graphics-known.json` lists what misses its budget on main (Petal's long playlist).
- The start-up budgets went up 1,005 B on every route: Settings' Graphics row is 61 B in `system.js`, the AUR update
  state 9 B, and earlier landings had used the room.
