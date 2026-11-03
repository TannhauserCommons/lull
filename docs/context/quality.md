# Quality: web assets, visual matrix, budgets

Owned by the Quality session. Files: `scripts/web-dist/`, `crates/server/src/web.rs`, the web stage of the `Dockerfile`, `tests/ui/`, `.github/workflows/ui.yml` and `ui-gates.yml`.

## Production web assets

`web/` is buildless in development and served from disk in debug builds. A release build embeds what `scripts/web-dist/build.mjs` makes of it:
- **Minified file by file, never bundled.** `index.html` loads scripts in a fixed order and designs swap `designs/<name>/design.css` by name, so every file keeps its path. CSS goes through lightningcss and JS through esbuild (both pinned in `scripts/web-dist/package-lock.json`). JS stays classic: no `format`, no `target`, so top-level names that other scripts read survive. Files already named `*.min.js` (Webamp) are left alone.
- **Pre-compressed.** Next to each text file (`.html .css .js .svg .json .webmanifest .txt .ico`) it writes `<file>.br` (brotli 11) and `<file>.gz` (gzip 9), kept only when they save 5% or more. Fonts and images are compressed already and get no variants.
- **Where it runs.** The Dockerfile's `web` stage vendors Webamp, then runs `node scripts/web-dist/build.mjs web dist`; the build stage copies `dist` over `web/`. For the app build (Tauri's `frontendDist`), `mise run web:dist [out dir]` does the same; the out dir may be `web/` itself in a throwaway CI checkout. Run the script from the repo root.

`web.rs` serves from the embedded files:
- It picks the first of `br`, `gzip` that `Accept-Encoding` allows (a `q=0` refuses it) and has a variant file, and answers with `Content-Encoding`. Without a variant, or in a checkout, it serves the plain file.
- Every answer carries `Vary: Accept-Encoding`, `Cache-Control: no-cache` and an ETag of the bytes sent, so each encoding has its own tag; `If-None-Match` gets a bodyless 304 with the same headers.
- `rust-embed` reads `web/` from disk in debug builds, so the variants don't exist there; the hub's compression layer (`compress.rs`, hub.md) then compresses those text files per request, as it does every JSON answer.

**Sizes** (`web/` at `a324215`, 144 files; the hub sent everything uncompressed before):

| | files | before (raw) | minified raw | on the wire, brotli |
|---|---|---|---|---|
| JS | 18 | 1604 KiB | 1366 KiB | 377 KiB |
| CSS | 14 | 719 KiB | 540 KiB | 96 KiB |
| everything | 144 | 3369 KiB | 2951 KiB | 1399 KiB |

The rest is the Webamp bundle, fonts, SVG and images. Brotli over the old raw transfer is the real gain: JS 1604 → 377 KiB, CSS 719 → 96 KiB.

## Visual matrix

`tests/ui/` (Playwright, pinned; `mise` is not needed, `npm ci` in that folder). `matrix.spec.mjs` makes one test per picture:
- **Routes** (`routes.mjs`): every fixed route, every Settings section, plus an album, artist, tag and playlist page found in the hub's library by `global-setup.mjs` once there is one. Add a route there when a screen registers one.
- **Designs and themes:** `lull skydeck starlight petal glasshouse` × `day night`, set through the `lull:prefs` the UI reads before first paint.
- **Viewports:** `phone` 390×844 (mobile, touch) and `desktop` 1440×900 for every route; `laptop` 1366×768, `xwide` 1920×1080 and `ultra` 2560×1440 (ADR-0007's sizes) only for the screens whose layout follows the window (`layoutRoutes` in `routes.mjs`: Home, Search, Library albums and songs, Now Playing, album, artist, playlist), so the run stays within its hour.
- **Per picture it asserts:** no horizontal overflow (names the widest element), no title squeezed under 48 px (a grid counting its columns from the window instead of its own width), no console error or uncaught exception, no failed or 4xx/5xx request, then compares the viewport screenshot (animations disabled, caret hidden, fixed clock and timezone, 0.2% of pixels tolerated).
- **Tolerated requests** are listed with their reason in `expected-failures.json`. Delete an entry when its cause lands; don't add one to get green.

**The CI hub.** `run-hub.sh` runs the built image (`lull:ui`) on host networking next to:
- `stub-slskd.mjs`: an slskd that is logged in, shares a library and gets the same one-peer, one-song answer to every search. Unanswered searches are how lull spots a wedged slskd, so a stub that found nothing would raise the Soulseek banner after a few searches and shift every page under Lighthouse. It answers only what `crates/soulseek/src/slskd.rs` calls and 404s anything else loudly.
- a fixture library (`fixtures/make-library.mjs`, ffmpeg): 4 albums, 14 tracks of 6 s sine tones, invented artists, real tags, embedded and folder covers, in `Artist/Album (Year)/NN - Title.ext`. Never commit generated media. It is mounted at `/music`.
- a dead proxy for everything outside localhost, so MusicBrainz, Cover Art Archive and the rest fail identically on every run rather than changing the pictures.
- `LULL_SETUP_CODE` and `LULL_LIBRARY=folder` are passed only once `crates/core` reads those keys (the hub refuses unknown keys), and `global-setup.mjs` claims the hub and keeps the `lull_device` cookie only once `GET /api/setup` exists (ADR-0002). Before that the hub is open.

**Goldens are not committed by the Quality session.** With no `tests/ui/goldens/`, or with `UI_CAPTURE=1`, a run writes the pictures instead of comparing, and the `ui` workflow uploads them as the `goldens` artifact. `mise run ui:update` triggers a capture run on the current branch and downloads them into `tests/ui/goldens/`; review and commit them. Pictures from any other machine differ by fonts and anti-aliasing, so only CI's image captures them. Failing runs upload `diffs` (expected, actual, diff images and the HTML report).

## Budgets

Lighthouse CI on the mobile profile (simulated slow 4G, 4x CPU), three runs per route, the median asserted. `lighthouse.sh` runs `lighthouserc.cjs` once per route in `budgets.json` (`home`, `search`, `library/albums`, `now`, `settings/playback`), because lhci drops the `#fragment` and would merge the routes into one page. Per route: `total-byte-weight` (transfer size), largest contentful paint, total blocking time, cumulative layout shift.

**Measured** on the CI hub with the fixture library (run 41315307272, `main` with theme packages, minified and brotli-compressed, median of three):

| route | transfer | LCP | TBT | CLS |
|---|---|---|---|---|
| home | 369 KB | 4.3 s | 87 ms | 0 |
| search | 368 KB | 4.3 s | 40 ms | 0 |
| library/albums | 385 KB | 4.1 s | 27 ms | 0 |
| now | 367 KB | 4.0 s | 91 ms | 0 |
| settings/playback | 369 KB | 4.0 s | 52 ms | 0 |

How the baseline moved, so the next raise has a precedent to compare with:
- **302 KB, LCP 3.5 s** (run 41308967643): empty library, `main` at `a324215`.
- **326 KB, LCP 3.7 s** (run 41312904377): web changes landed on `main`; library/albums adds about 17 KB of covers.
- **369 KB, LCP 4.3 s** (this table): the themes work. A design is now a theme package: `theme.css`, `theme.json`, one HTML file per slot, `purify.min.js` (DOMPurify, 11 KB) and `/api/themes`, and `appearance.js` grew by 7 KB. LCP rose because the stylesheet, the package and its slots now load one after another. CLS fell to 0.

**Budgets move only with a stated reason the user would accept**, never because `main` got heavier: a budget that follows every regression catches nothing. The numbers below, set from the 369 KB / 4.3 s run, are **temporary**: they are raised only until the themes work stops loading the theme stylesheet, package and slots one after another. When that lands, set them back to the pre-themes figures (run 41312904377: transfer about 359 KB, library/albums 377 KB; LCP 4.35-4.65 s; CLS 0.025) and delete this note.

**Budgets** = median plus a margin:
- **Transfer +10%** (about 405 KB, library/albums 424 KB): it moves only when we change files, so the margin is small. A change that adds a script or a font fails it.
- **LCP +25%** (4.0-4.3 s -> 5.05-5.4 s): the simulated network makes it stable, but runners differ.
- **CLS +0.02** over a median of 0.
- **TBT 150 ms,** not median +50%: lab TBT of 0-90 ms is noise, and a floor keeps the check from flaking while still catching a real long task.

Raise a budget only with a measured reason a listener would accept, in the commit (a feature they asked for, not drift): run the workflow, read the `lighthouse` artifact's reports, and edit `budgets.json` the same way.

**A feature's code loads where it is used** (web.md, Code loads where it is used): a screen or panel's script and styles load on its first use, so a feature doesn't spend the start's transfer budget, and a budget isn't raised to fit code that could load on use. The on-use gate (below) fails on a new file at start that `gates/start-files.json` doesn't list with its reason. A file already at start can still grow: when Lighthouse's transfer goes over, find what in it only one screen uses and move that out (Stats left system.js this way) before reaching for the budget.

## UI gates

`mise run check:ui` (CI: `.github/workflows/ui-gates.yml` on every change to `web/` or `tests/ui/`). Node and Chromium only, about ten minutes: no image, no hub, no outside service. Run it before pushing a `web/` change.

**The fixture hub** (`tests/ui/fixture-hub/`): `server.mjs` serves this checkout's `web/` and answers `/api` from `library.mjs`, an invented library made by a seeded generator, the same on every run: 64 artists, 150 albums (the last ten the catalog's, unsaved; streaming one of them fails), 1,672 songs, a 1,200-song playlist (`pl-1`) and a 60-song one (`pl-2`), 300 likes, a week of history, synced lyrics. Covers are generated PNGs at the size asked (the 512 px one weighs what a photo does); audio is a silent WAV as long as the song. It models the contract in api.md, not the hub: anything it doesn't answer is a 404 the specs count as a failure, so add the endpoint there when a screen starts reading a new one. `node tests/ui/fixture-hub/server.mjs 8790` runs it by hand; `FIXTURE_TRACK_SECONDS=20` makes every song 20 s and `FIXTURE_AUDIO_DIR=<dir with song.mp3, song.flac>` serves real files (a listening soak in minutes); `LULL_WEB=<dir>` serves another `web/` (an A/B against main).

**Performance** (`gates/perf.spec.mjs`, budgets in `gates/perf-budgets.json`, scenarios in `gates/scenarios.mjs`). The CPU is slowed 4x and each scenario is read from a Chrome trace: counts that don't move between runs (elements restyled, layouts, main-thread frames, bytes) wherever possible, and task lengths with about twice the measured time as room for a slower runner. Per design (all five): one class flip, text change or insertion restyles at most a hundred elements on Home, a playlist and Now Playing; the hover highlight is complete in the first frame, and a sweep over the long playlist causes no layout (but a tooltip opening) and spends under 100 ms in event handlers (the bug that made the highlight trail spent 320); no main-thread frames at rest; playing costs under 220 ms a second on Now Playing (about 60 measured). In lull: startup, scrolling and moving between screens, typing a search, opening a menu, dragging in the queue, resizing the panel, playing beside a 1,200-song list, and a compressed listening session (60 songs skipped through, in a playlist of mixed albums, while the screens change, the desktop app's bridge stubbed by `gates/desktop-bridge.js`): heap, nodes and listeners left behind, and what crosses to the mini player (a snapshot under 2 KB, the cover once per song).

**Key screens** (`gates/screens.spec.mjs`): Home, a search, a playlist, an album, Now Playing and Settings, in every design and theme, at a phone (390x844), a laptop (1366x768) and a wide window (1920x1080). Each must load without console errors or failed requests, keep inside the window, give every title room, and match its golden in `gates/goldens/linux/`. Goldens are CI's only (fonts and anti-aliasing differ elsewhere); on another system a first run writes its own, gitignored, so later runs compare against them. **Approving a change in how a screen looks:** push, then `mise run ui:approve` (it runs the workflow in capture mode and downloads the pictures), review `git diff --stat tests/ui/gates/goldens` and the pictures, and commit them with the change. A failing run uploads `gates-diffs` (expected, actual and diff images).

**Covers** (`gates/covers.spec.mjs`, web.md "Covers at the size they are drawn"): counts the fixture hub keeps (`/__fixture/art-requests`, so the browser's cache hits don't count), each test in its own scope there (its context sends `X-Fixture-Client`): the gates beside it on the hub never add to its counts, see its covers pending or get its `art.ready`. On Home, the 1,200-song playlist and an album: every cover on screen loaded within 2 s of the page settling, no cover URL asked for twice, and no more cover requests than cover images in the page. A cover the hub is still fetching (`/__fixture/art-pending`: 404 "not yet", `no-store`) is asked once per size and not again for 6 s; `art.ready` (`/__fixture/art-ready`) makes the page ask once more and show it. At 1x and 2x, every cover asks for the smallest served size that covers its box in device pixels (10% slack), never past the 512 px the fixture's pictures have (it serves the hub's five sizes, capped there, with `X-Image-Size`). Against `web/` before this work it fails the retry and the 2x sizes.

**The chrome at every width** (`gates/fit.spec.mjs`, the check is `chrome` in `checks.mjs`): in every design and theme, with a song loaded, the window is narrowed from 2560 to 360 px and widened back, 20 px at a time, and at each step the header, the player bar, the mini player and the tab bar must keep every control inside its bar and the window, never put one control on another, never cut text without an ellipsis, and keep play, previous, next, the player's "…" and search. The desktop app's mini player is checked at its own sizes (the device 340 px wide, the strip 540 x 90). The header and the bar fit themselves (web.md, Rows that fit themselves), so a failure here means a part needs a place in the give-way order, not a width band. Text that grows in place (wider letter-spacing on the header and the bar, as a web font with wider letters than the fallback's gives; no row resizes) must fit within two frames at 1440, 1140 and 960 px, and the rows come back as they were when it narrows again. A width that fails says what the rows looked like then (each part's box and whether it gave way, the fonts' state) and whether the next frame and the settled page still overflow.

**Code loaded on use** (`gates/on-use.spec.mjs`, web.md "Code loads where it is used"): code that belongs to one screen or panel loads on its first use. A browser must start with exactly the scripts and stylesheets `gates/start-files.json` lists, each with the reason the start needs it: a new one fails until it loads on use (the documented way: a route's `need`, `App.needScript`) or is listed with its reason; one listed but gone fails too, so the list stays true. What loads on use must still work as before, each pinned: a .wsz dropped anywhere becomes the Classic skin, Classic opens styled and leaving it returns to the page it was opened from; Stats shows styled on its first visit; Settings > Appearance shows Classic's group; Settings > Library, Sources and Devices open on the first click with hub.css; first-run setup opens with its steps; an unpaired device lands on the pairing gate with only its placeholder before the form; a `library.source` event before hub.js loads fetches the library source and setup reads again and shows in Settings > Library once it opens; the Connect picker is styled without hub.css. In the desktop app (the bridge stubbed in) Classic and the bridge load with the page and a crash report from Webamp's window reaches classic.js without Classic having been opened. Each check fails with its part taken out.

**Now Playing in every design** (`gates/now.spec.mjs`): with a song playing and its lyrics asked for in the side panel, Now Playing at a phone, a laptop, a short (1440x700) and a wide window, and the Now column, must show the title, artist, album, quality and "playing from", and the lyrics in one place only (web.md, One place shows the lyrics). Pages from different designs can then be mixed.

**Running the gates beside another checkout:** the fixture hub listens on 8790 and a local run reuses a hub already there, which may serve another checkout's `web/`. Give yours its own port: `FIXTURE_PORT=8797 mise run check:ui`.

**Accessibility** (`gates/a11y.spec.mjs`): axe-core, WCAG 2.2 A and AA, on the same screens in every design and theme. `gates/a11y-known.json` lists the violations main had when the gate arrived, each with its rule and where; a new one fails. The list only shrinks: fix one, delete its entry.

**Budgets move only with a measured reason a listener would accept** (a feature, not drift), stated in the commit with the run's numbers, as above for Lighthouse.

**Finding a cause** when a gate fails: the Chrome trace of the scenario (DevTools Performance panel, or the `summarise` numbers), and for slow style recalculation Chrome's selector stats (trace category `disabled-by-default-blink.debug`, event `SelectorStats`), which name the selector. The pattern behind most of the fixes so far: a `:has()` on a page-wide element whose argument can be anywhere walks the whole page each time it is checked; limit it to where the element sits (`.main:has(> :where(.view) > .now)`; `:where()` keeps the specificity) or use a state lull already sets (docs/themes.md, Staying fast).


## Theme hooks

`tests/web/theme-hooks.test.mjs` (part of `mise run check`) keeps docs/themes.md section 11, the hook reference, complete. It fails when:
- a class block lull's CSS or a built-in design styles is listed neither as stable nor as internal;
- a `data-*` attribute web/ sets or styles is listed in neither;
- a built-in design styles a class or attribute web/ no longer has, so the rule never matches;
- the reference lists a hook web/ no longer has, or lists one as both.

A new hook is added to section 11 in the same commit: as stable when a theme may style it, else as internal.
