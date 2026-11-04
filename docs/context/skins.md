# Skins: Winamp Skin Museum proxy, `.wsz` cache, favourites, your skins

`crates/skins`, for Classic mode (DESIGN.md §12). Endpoints: [api.md](api.md#collection-cratescollection-cratesskins).

## The museum API (checked 2026-10-31)

- **Endpoint.** `POST https://api.webamp.org/graphql`: public, no key, CORS open.
- **Browse.** `skins(first, offset, sort: MUSEUM)` returns `{ count, nodes }`.
  - **The design's `skins(sort: MUSEUM, filter: APPROVED)` is now refused** ("Combining `sort: MUSEUM` with `filter` is not supported").
  - lull sorts by `MUSEUM` without a filter and drops `nsfw: true` skins itself. In the museum order NSFW skins are rare (none in the first 200).
- **Search.** `search_classic_skins(query, first, offset, nsfw)` returns a list with no total, so lull asks for 31 rows to learn whether there's a next page.
- **Lookup.** `fetch_skin_by_md5(md5)` returns the skin with `download_url`, or `null`. lull uses it for downloads and favourites.
- **Fields used:** `md5`, `filename`, `screenshot_url` (275x348 PNG on `r2.webampskins.org`), `museum_url`, `nsfw`, `download_url`. There is no display name; lull shows the file name without `.wsz`, with `_` as spaces.
- **Files.** `.wsz` files are served from `r2.webampskins.org/skins/<md5>.wsz` with no CORS headers, which is why they are proxied. The md5 is the file's own md5; verified on a download.
- **Terms.** No terms of use, rate limits or API policy are published: not at skins.webamp.org/about, and not in the skin-database package. Webamp is MIT (Jordan Eldredge).
  - The about page credits the collection to the Internet Archive and asks nothing in return.
  - The skin-database README calls the GraphQL server "not currently used by anything", so the API comes with no stability promise.
  - lull behaves as below to stay polite, and keeps the design's attribution: "Skins from the Winamp Skin Museum" plus a per-skin `museumUrl`.

## Politeness

- **Debouncing** search keystrokes (300 ms) is the UI's job.
- **Pages are cached in memory for an hour,** up to 200 pages, oldest dropped first.
- **On a failure, the museum gets no calls for 30 s;** requests in that window get 503. If a cached page exists, even an expired one, it is served instead.
- **The shared HTTP client** adds timeouts and bounded retries with backoff (it honours `Retry-After`).
- **A `.wsz` is downloaded once,** then served from disk forever: it is named by its md5 and never changes.

## `.wsz` validation and cache

- **What counts as a skin:** the zip's central directory has an entry named `main.bmp`, in any folder and any case (Webamp searches the same way). Nothing is unpacked. Anything else (an HTML error page, a truncated zip, a modern `.wal`) is answered with 502 and is not cached.
- **Size cap: 16 MiB.** Classic skins are a few hundred KB.
- **Storage.** Files are cached at `<LULL_DATA_DIR>/skins/<md5>.wsz`, written to a `.part` file then renamed so a reader never sees half a file. The cache is not capped: it only grows by skins the listener opened.
- **Favourites** (`skins_favourites`) store the museum's metadata when added, so the favourites list renders offline. `downloaded` in any list means the file is in the cache, for the skin browser's offline view.

## Your skins

- **Where they live: the hub.** Skins the listener adds are kept in the hub's data, `<LULL_DATA_DIR>/skins/mine/` (made at start, beside the museum cache), so every device lists them, nobody needs the hub's file system (hubs are often remote), and they are backed up with the rest of its data. Nothing to set up.
- **`LULL_SKINS_DIR`** (optional; settable in the app too, Settings › Appearance › Classic › Skins folder) names a folder of `.wsz` files the listener already keeps on the hub. It is read **alongside** the hub's folder, never instead of it, and never written: added skins always go to the hub's folder, so setting or clearing it never hides them, and lull never changes a folder the listener manages (the deploy kit mounts it read-only: `SKINS_DIR` at `/skins`, with `LULL_SKINS_DIR=/skins`).
- **Adding** (`POST /api/skins/mine?name=`, the file as the body): checked as a download is (a zip naming `main.bmp`, at most 16 MiB; a bigger body is refused with 413 before it is read), named by its md5. The listener's skin already (the same bytes, in either folder) is answered with it, not kept twice. Otherwise it is written as the device's file name (folders and characters file systems refuse dropped, `.wsz` added; `Name (2).wsz` when taken), beside its final name first, then the hub's folder is read again. One add or removal at a time.
- **Removing** (`DELETE /api/skins/mine/{md5}`) deletes every file of that skin in the hub's folder. A skin only in `LULL_SKINS_DIR` is refused (409, with the folder named): lull only reads it.
- **Reading** (both folders, `folder.rs`): every `.wsz` under it (any case, any subfolder, links followed, hidden folders and `.part` files skipped) is checked as a download is and named by the md5 of its bytes, as the museum names skins. The same skin in two places is listed once: the hub's first, then the folder's, each at its first path in name order. A file that isn't a skin is listed under `skipped` with why, and logged once when it is first read; it never fails the list.
- **Reading again.** A watcher reads a folder again 2 s after changes settle; `POST /api/skins/mine/rescan` reads both at once (some network mounts send the watcher nothing: `watching: false`). Each read stats every file, but opens only those whose size or modification time changed: `skins_folder` keeps each file's md5 or problem by `(folder, path)`, so a restart doesn't read a large folder again; rows of a folder no longer used are dropped at start. A read that finds something else publishes `skins.mine`.
- **Serving.** `GET /api/skins/{md5}.wsz` looks in the hub's folder, then `LULL_SKINS_DIR`, then the museum cache. The bytes are checked against the md5 on every serve, because the answer is `immutable`: a file changed since the read is not served under its old md5, and the folder is read again.
- **No screenshots.** Your skins show their name (the file name, `_` as spaces) and path; Settings has no picture for them.

## Random skin

- **The rule** (`LullWebamp.pickSkin` in `web/desktop/webamp-session.js`, the only copy; cases in `tests/skin-pick.json`): uniformly one of the listener's own skins (Your skins: the hub's, plus `.wsz` files an older lull kept on this device); when they have none, one of the favourites. Never the skin in use; `null` when there is no other (one skin of their own, in use, says so instead of falling back to favourites).
- **Random skin** (Settings › Appearance › Classic) chooses the pick as choosing it by hand does (a hub skin is downloaded through the hub first, then `classic-skin` and the `storage` event), so an open Classic follows at once.
- **New random skin each time Classic opens** is per device: localStorage `lull:classicRandom` (`true`). Classic asks `App.classicSkin.opening()` before Webamp gets a skin, in the browser's stage and in `main` before it builds the app window's hand-off, so the app's Classic window (and a native one fed the same hand-off) opens in the pick without picking itself. The pick becomes the device's choice once its file can be had; otherwise the choice stays.

## In the web UI and the app (`web/screens/skins.js`, `classic.js`, `desktop/webamp-session.js`, `desktop/winamp.js`)

- **Classic mode is only Webamp.** In a browser it sits alone on a stage with a small Back to lull button (Esc, W and Webamp's close button also leave); in the desktop app it has its own window. Everything about skins is in **Settings › Appearance › Classic**: the skin in use, Random skin, Add skins, and the lists Your skins (on the hub, and any files an older lull kept on this device), Browse (the museum), Favourites and Downloaded.
- **The choice is per device:** localStorage `lull:classicSkin` = `{ md5, name, screenshotUrl }`, or null for Webamp's base skin. It replaced `lull:defaultSkin` and the skin last applied in Classic (`lull:classic`), which the first load migrates and deletes.
- **Live changes.** Choosing in Settings reaches an open Classic at once: `classic-skin` in the same page, and the `storage` event in other tabs and in the app's Classic window (the app's windows share one WebView2 profile, so one localStorage).
- **Choosing a museum skin downloads it through the hub first** (`GET /api/skins/{md5}.wsz`), so a skin that can't be had says why in Settings, and Classic later reads it from the HTTP cache (the file is `immutable`).
- **Adding skins** ("Add skins" in Settings, Add skins in Your skins' empty state, or `.wsz` files dropped anywhere on lull or on Webamp, several at once too) uploads each to the hub in turn (checked here first: a `.wsz` under 16 MB). One file is chosen at once; several join Your skins and the skin in use stays; one toast says how many were added, were there already, or couldn't be (with the hub's sentence). A skin on the hub has Delete (confirmed; it goes for every device, and a skin in use here falls back to Webamp's base skin); a skin in `LULL_SKINS_DIR` has a lock in Delete's place that says it is only read there. The folder is named under Your skins while it holds skins or the hub can't see it change, so an empty mounted volume shows nothing.
- **Skins an older lull kept on this device** (IndexedDB `lull-skins`, store `files`, key `md5` = `local-<hash of name, size, date>`) are still listed (Only on this device) and usable, and Your skins offers once to **Move to your hub**: each is uploaded, then dropped here, and a skin in use stays in use under its hub md5. "Keep them here" stores `lull:skinsMoveOffer = declined` and the offer doesn't come back on this device. Classic names a device skin as `<web root>local-skins/<md5>.wsz`: a URL on the hub's origin that nothing serves, so the app's hand-off check (every URL on the hub) passes and `LullWebamp.skinFile` reads it from IndexedDB instead.
- **How Webamp gets a skin** (`LullWebamp.loadSkin`): lull fetches the file (the hub's sentence on a failure), hands Webamp the bytes as a `blob:` URL and waits up to 10 s for it to show them. Webamp reads every skin with `fetch`, so the hub's CSP allows `blob:` in `connect-src`; without it every skin failed with Webamp's "Failed to download skin from blob:…" alert.
- **Webamp never shows a browser dialog.** It reports failures with `window.alert()`; `LullWebamp.quietAlerts` takes them: during a skin load they fail that load with a sentence (the stage says it with a link to Settings; the window shows a note over Webamp for 8 s), any other becomes a toast or that note. A skin that fails leaves the one before it on.
- **Webamp's own Skins menu** lists the favourites; what is picked there lasts until Classic closes.
- **Size** (1×, 1.5×, 2×, 2.5×, 3×) is per device too: localStorage `lull:classicSize`, followed live the same way (`classic-size`, the `storage` event). Each player scales the whole page Webamp lives in, so CSS pixels scale and Webamp's pointer math stays exact: the app's Classic window zooms its webview (`classic_fit`, docs/context/desktop.md); the browser stage puts Webamp in a same-origin iframe, 1/size of the stage, drawn at the size with a transform, and moves the windows so the page's centre stays put. A sub-tree `zoom` or `transform` around Webamp in lull's own page was ruled out: Chromium reports pointer positions on screen while layout stays unscaled, so Webamp's window moves, playlist resize and drag and marquee (raw deltas) would run 1.5–3× too fast. Skins stay sharp at every size: Webamp's `image-rendering: pixelated` is nearest-neighbour, measured blur-free at 100/125/150% display scale for zoom, transform and both iframe and page zoom; at a fractional total scale (1.5× at 100%, any size at 125%) skin pixels alternate between two widths, which no technique avoids without changing the size. Webamp's own double size (a `scale(2)` of the main window and equalizer only) is not used: its Double Size (Ctrl+D, Options, the clutter bar's D) is turned back at once and switches lull's size between 1× and 2× instead (`LullWebamp.doubleSize`). A size the screen or stage can't hold gives way to the largest that fits, said once.
