# Web UI: structure, data layer, player engine

`web/` is the shipped UI: classic scripts and plain CSS, no build step, served by the binary (from disk in debug
builds, so a reload picks up edits). It was ported from `design/prototype`; `design/DESIGN.md` stays the reference for
the look, the components and the `App.*` seams (section 7). This page covers what the port added: where data comes
from, how the player plays, and the rules screens follow. The endpoint contract is [api.md](api.md).

## Files and load order

```
index.html   tokens.css base.css components.css screens/*.css desktop/desktop.css designs/<name>/design.css (last)
             designs/designs.js + the pre-paint script (design, theme, motion, text size)
             icons.js data.js align.js app.js layout.js connect.js offline.js screens/{discover,now,library,system,appearance}.js
             desktop/webamp-session.js screens/{skins,hub,setup,classic}.js desktop/desktop.js
data.js      window.LullData = App.data: API client, entity cache, per-route reads, the event stream
align.js     window.LullAlign: how far apart two recordings of one moment are (switching to a better copy); pure,
             tested by tests/web/align.test.mjs (`node --test`, part of `mise run check`)
app.js       window.App: router, render helpers, overlays, save pipeline, player engine, shell, actions,
             keyboard, Media Session, drag and drop, server events + boot
layout.js    ADR-0007: size classes, App.layout (lull:layout), resize handles, the Now Playing column host
sw.js        service worker (app shell for offline start); manifest.webmanifest
hub.js       the hub's own settings: library source (folder / Jellyfin), Soulseek account, Settings > Devices (App.hubKit)
setup.js     the pairing gate, #/pair/<code>, first-run setup (/setup)
skins.js     Classic mode's skin and size, per device (App.classicSkin, App.classicSize), and Settings > Appearance >
             Classic: the size, the skin browser, .wsz files kept on this device, the NSFW switch (docs/context/skins.md)
classic.js   Classic mode: Webamp alone, in an iframe on a stage in a browser, or in the desktop app's own window
vendor/      Webamp bundle and the QR encoder (qrcode-generator), copied by scripts/vendor-web.sh, gitignored
desktop/     the desktop app's wrapper pages and bridge (`LullDesktop`), owned by the Apps session
```

Designs: all five ship (`lull skydeck starlight petal glasshouse`), switchable live in Settings > Appearance; design,
theme (day/night/system), motion and text size are stored per device in `localStorage['lull:prefs']`.

## Data layer (`App.data`, screens call it `M`)

Screens read data **synchronously**, as they did from the prototype's mock. Three tools make that work against a
server:

- **Lookups** answer from the entity cache: `M.track(id)`, `M.album(id)`, `M.artist(id)`, `M.tag(id)` (an unknown tag
  resolves to its name), `M.playlist(id)`, `M.albumsBy(artistId)`, `M.tracksBy(artistId)`, `M.formats[label]`
  (quality-chip facts learned from tracks), `M.saveSteps`. The cache fills from every response's `entities`
  (`M.absorb` runs on each `M.api` answer). Tracks gain `sizeMB` (from `sizeBytes`); albums gain `sizeMB`, `format`
  and `duration` from their known tracks. A lookup can return `undefined`: render nothing for that id rather than
  crash.
- **Per-route reads**: `var r = M.get('/api/albums/' + id)` returns `{ loading, error, data }` at once and fetches the
  first time. When the fetch settles, the route that asked re-renders (`App.refresh`), so a render is:
  `if (r.loading) return skeleton; if (r.error) return App.errorCard(r.error, { path }); ...r.data...`.
  Reads are cached per path for the session: `M.reload(path)` refetches in place (old data stays visible),
  `M.invalidate(prefixes)` forgets, `M.peek(path)` returns data without fetching. A read made by a previous route never
  re-renders the current one.
- **Actions**: `M.api(path, { method, body, keepalive })` returns a promise; it rejects with `M.ApiError`
  `{ status, kind, message }` where `message` is the server's sentence (or "Can't reach the lull server"). Show it as is
  (toast or inline error). Never fake success: state changes that the server confirms either wait for the answer or
  change at once and roll back with an error toast (likes, follows).

Honest states: an endpoint that is not live yet answers 404 `not_found` or 503 `unavailable`; the screen shows the
design's error or empty state with that message and a retry (`data-act="reload-read" data-id="<path>"`). No sample
data, ever.

### What lives where

| Fact | Source of truth | Kept current by |
|---|---|---|
| tracks, albums, artists | server entities | every response (`M.absorb`) |
| liked, followed | `GET /api/likes`, `/api/follows` -> `App.store` `liked`, `followed` | `App.toggleLike`, `App.setLiked`, `App.toggleFollow` (PUT/DELETE); `like` / `follow` events from other devices |
| playlists | `GET /api/playlists` -> store `playlists` (= `M.playlists`) | `App.createPlaylist`, `App.savePlaylistOrder`, `App.addToPlaylist`; how each is shown (`sort`) by `App.setPlaylistSort` (shown at once, kept by `PUT /api/playlists/{id}/sort` for every device; a failure keeps it for this visit and says so) |
| saves | store `saves` `{ trackId: { jobId, step, status, pct, error, kind, nocover } }`, seeded from unfinished save rows of `GET /api/jobs` | `App.saves.start(ids, { mode, source })` (`mode` = the duplicate dialog's separate/replace), `retry`, `save` events |
| activity | `GET /api/jobs` -> store `activity` (rows keyed `id` = jobId) | `job`, `transfer` events |
| services | `GET /health` -> store `services` (`state ok|warn|down|off`, `label`, `detail`, `queued`, `usedGB`, `capGB`) | `service` events |
| server reachability, auth, version | `App.server { reachable, authorized, version }`, event `server` | `/api/ping`, `/health`, event stream errors |
| private listening | `GET /api/devices` (`me`'s `private`) -> store `privateListening` | Settings > Privacy (`App.setPrivateListening` = `PATCH /api/devices/me`); `POST /api/plays` answering `private: true`. Shown as a "Private" tag with the track in the player bar and in the phone mini player; History hides "Send to Last.fm" while it is on |
| playback settings | `GET /api/settings` playback -> `App.setPlayback(patch)` | Settings > Playback calls `App.setPlayback` after a PATCH; `settings` events |
| recent searches | store `recentSearches`, this device only (`localStorage`) | discover.js |
| queue, volume, shuffle, repeat | `App.player.state`, saved per device so a reload keeps the queue | the engine |

### Server events

`data.js` opens one `EventSource('/api/events')`. Any payload's `entities` are absorbed first. app.js handles
`service`, `stream`, `save`, `job`, `transfer`, `like`, `follow`, `settings` and `resync`, then re-emits every topic as `App.emit('server:<topic>', payload)` (discover.js listens to
`server:search.results`). On every reconnect after the first, and on `resync`, app.js invalidates all reads and
refetches health, collections and the current route. A closed stream (401) checks `/api/ping`.

### Boot

`DOMContentLoaded` -> design ready -> shell + first route render (empty data shows skeletons) -> `GET /health`
(services, version, then the service worker) and `GET /api/ping` (`App.checkPairing`): 204 starts events and loads
collections; 401 sets `App.server.authorized = false`, and setup.js opens the pairing gate (behind it, service
banners stay hidden: only "unreachable" and "offline" show); a network failure still
starts, so the installed app opens offline with the cached shell and an "unreachable" banner. `App.server.checked`
says the ping has answered.

### Pairing and first run (ADR-0002)

- **The gate** (setup.js) asks `GET /api/setup`: an unclaimed hub takes its setup code (`POST /api/setup/claim`, then
  `/setup`); a claimed one takes a pair code from a paired device (`POST /api/pair`, `kind: browser`). The QR code's
  link `#/pair/<code>` fills the code in. Both set the device cookie, then `App.start()`. Any 401 later (this device
  unpaired) opens the gate again.
- **`/setup`**: library, Soulseek, Last.fm; each step is done when `GET /api/setup` `steps.<name>` is true. Last.fm's
  callback lands on `#/settings/lastfm`, which returns to `/setup` while a setup connect is under way.
- **Hub config in Settings**: any `config.LULL_*` path (system.js) reads `GET /api/config` and saves with
  `PATCH /api/config` (values as strings); keys set by the environment and every key until the hub answers show
  read-only. Secrets only say whether they are set (`set`; their `value` is always null). An unset key (`value`
  null) shows the default it runs on, read from `/api/settings` (`CFG_SETTING`): the selected choice, or a text
  field's placeholder, so leaving the field alone stores nothing.
- **Soulseek account**: only when `GET /api/setup` says `soulseekManaged` (the desktop app runs its own slskd) do
  setup and Settings > Sources show name and password fields (`LULL_SOULSEEK_USERNAME`, `LULL_SOULSEEK_PASSWORD`);
  otherwise they show slskd's state and say the login is set where slskd runs.
- **Not on HTTPS** (`!isSecureContext`): the gate, Settings > Devices and the pair dialog say the app can't install or
  keep downloads here, and point to `tailscale serve`.

## Player engine (`App.player`)

The state shape and methods are DESIGN.md 7.8; the insides:

- **Sound comes from an output** (below): two `<audio>` decks in browsers and the desktop app, the native player in
  the Android app. Saved songs play from `/api/library/stream/{id}`, downloaded ones from this device; Soulseek
  songs from `/api/stream/{id}` (`?peer=&path=` after a manual source pick, `P.useSource(trackId, source)` with a
  source row from `/api/tracks/{id}/sources`). Leaving a Soulseek song whose transfer hasn't finished sends
  `DELETE /api/stream/{id}`.
- **Streaming state** comes from `stream` events (`finding queued buffering streaming stalled fallback failed`,
  `received/size`, `peer`, `peersFound`, `queuePos`, `failedPeers`), merged with the element's own `playing`/`waiting`.
  `buffered` is `audio.buffered` cut to `received`. A seek may go anywhere for saved songs and only up to what has
  arrived for Soulseek ones (`P.seekable(f)`); beyond it snaps back, shakes and announces.
- **Errors**: when the element fails, the engine fetches the same URL (`Range: bytes=0-0`) to read the server's
  sentence and shows it (`failed` / `jellyfin-error`).
- **Peer fallback**: the server ends the old response and raises `attempt` in `stream` events; the engine reopens the
  stream URL and seeks back to the same time. A Soulseek response that ends before the song does (and before the
  transfer completed) is treated as that fallback in progress, never as the song's end.
- **Gapless**: the next song is preloaded 30 s before the end (90 s when the current transfer is done; at once for the
  native output, whose page may sleep) and handed
  over on `ended`. Off when Settings > Playback turns gapless off. A queue edit that leaves it next keeps it loaded.
- **Preloading** (`playback.prefetchCount`, 0-3, default 2): after the playing song has its source, the engine sends
  the next that many unsaved songs (in queue order; with shuffle, the one picked next) to `PUT /api/stream/prefetch`,
  the whole list each time, debounced 800 ms after a queue, track, status or mode change. The hub fetches them from
  Soulseek and cancels the ones that drop off; a skip then opens a stream that is already on disk.
- **Queue reorder** (`now.js`): pointer events, not native drag and drop. The lifted row follows the pointer and the
  rows it passes slide by transforms, painted once per frame from positions measured at the start; mouse from
  anywhere on a row after 5 px, touch from the cover (the list keeps scrolling); the list scrolls near its edges;
  Escape cancels. Keyboard: Alt + arrows.
- **Crossfade** (`playback.crossfade` seconds): the next deck starts under an equal-power fade.
- **Normalisation** (`off | replaygain-track | replaygain-album | loudness`): Web Audio gain per deck. ReplayGain uses
  `track.replayGain { track, album }` (dB) when the library supplies it; otherwise the measured loudness of what has
  played (unweighted RMS) is levelled toward `playback.loudnessTarget` (-18 / -14 / -11) within -12..+6 dB. The Web Audio graph is built only when normalisation,
  crossfade or switching to better copies (`sources.switchToBetter`) is on, inside the first play gesture.
- **Better copies**: the playing song moves to a better copy mid-song without a jump; [below](#better-copies).
- **Plays**: `App.recordPlay(trackId, duration, recommended)` posts `POST /api/plays` when a song starts playing
  (`recommended` for radio songs) and its `end(how)` sends `PATCH /api/plays/{playId}` with `playedS` (time actually
  heard), `completed`, `skipped` when it ends, is skipped, or the page closes (`keepalive`). Classic mode uses it too.
- **Other players**: `App.hooks.media` (default `null` = the engine) is what media keys drive; Classic mode points it
  at Webamp while open. `P.jumpTo(index, { at, autoplay })`, `P.streamUrl(trackId, source)` serve the hand-back.
- **Radio**: `P.startRadio({ kind: track|artist|tag|album, id })` fetches `/api/radio`; while "keeps adding" is on
  the queue is topped up to three songs ahead (`exclude` = what is queued). With "radio when the queue ends", the
  last song seeds a radio as it starts. Changing the steer re-fills the radio songs not yet played.
- **Media Session**: metadata with `/api/art/{albumId}?size=96|256|512` once the 96 px cover is known to exist (else
  `assets/fallback-art-*.png`), `playbackState`, position state on track change and seek, all handlers; `seekto`
  clamps like the seek bar.

### Output interface (ADR-0006)

The engine above decides *what* plays (queue, radio, plays, streaming states, seek limits). *Sound* comes from an
output with one contract and two implementations:

- **browser and desktop:** the two `<audio>` decks (gapless, crossfade and normalisation as described above);
- **Android app:** `window.LullNative.player`, a Media3 player with a MediaSession service (Mobile session). The app
  defines `window.LullNative` before the page's scripts run; the engine uses it whenever `LullNative.player` exists.

**Item**, what the engine hands the output:

| Field | Meaning |
|---|---|
| `key` | string, unique per queue entry (the same song queued twice has two keys); every event names it |
| `url` | absolute. A hub stream (`/api/library/stream/{id}`, `/api/stream/{id}[?peer=&path=]`) or, for a download, the `fileUrl` from `downloads.list()`. The native side adds the device's `Authorization: Bearer` itself; the web never sees the token |
| `trackId`, `title`, `artist`, `album`, `duration` | for the lock screen and notification; `duration` in seconds, may be `null` (Soulseek, not known yet) |
| `artwork` | absolute `/api/art/{albumId}?size=512` URL or `null` (same auth rule as `url`) |
| `gainDb` | normalisation to apply, in dB (ReplayGain track or album, per Settings > Playback); `null` = none |

**Methods.** Each returns a Promise that rejects with an `Error` whose `message` the UI can show.

| Method | Does |
|---|---|
| `load(item, { at = 0, autoplay = true })` | Replaces the current item (and drops any next one), starting at `at` seconds. Also used to reopen the same item after a peer fallback cut its response |
| `play()` · `pause()` | |
| `seek(seconds)` | The engine only seeks inside what has arrived (`P.seekable`); the output doesn't need to know about Soulseek |
| `setVolume(v)` | `0..1`, the final level: the engine has already applied mute and its perceptual curve (`v = volume²`) |
| `setNext(item \| null)` | Preload `item` and, when the current one ends, continue into it with no gap. Continue only when the current one reached its end (within 2 s of its duration): an earlier end is a cut response, reported as `ended` with `next: null`, and the next item stays set. `null` cancels. Called again whenever the queue, shuffle or repeat changes |
| `crossfade(seconds)` | `0` = off. Otherwise, with a next item set, fade equal-power from the current into the next over the current's last `seconds` |
| `switchTo(item, { commit })` | Optional (the browser output has it; without it a better copy waits for the next play). `item` is a better copy of the playing song (same `key`): line it up and crossfade into it with no jump, calling `commit()` (a Promise) right before the fade. Resolves `{ switched: true, residualMs, offsetS }` or `{ switched: false, why: mismatch \| quiet \| abandoned \| refused \| slow \| busy \| error }`; `time` positions stay the song's ([Better copies](#better-copies)) |

**Events.** `LullNative.player.on(type, fn)` returns a function that unsubscribes. `fn` gets one object.

| Event | Payload | When |
|---|---|---|
| `time` | `{ key, position, duration, playing, waiting }` | At least 4 times a second while playing, and on every change of `playing` (true while sound is meant to come out) or `waiting` (true while it stalls for data). `duration` is `null` until known |
| `buffered` | `{ key, ranges: [[from, to]] }` | Seconds held by the player, when they change |
| `ended` | `{ key, next }` | The item ended. `next` = the key it continued into (gapless or crossfade), else `null` and playback stopped. A response that ends before the song does is reported as `ended` too: the engine knows when that is a Soulseek fallback in progress and reopens with `load` |
| `error` | `{ key, message, status }` | The item can't play. `status` = the HTTP status when the hub answered one (the engine fetches the hub's sentence), else `null` |
| `crossfade` | `{ key, next }` | Optional: a fade into `next` started (the player bar shows it), or was abandoned (`next: null`, on pause) |
| `remote` | `{ action: next \| prev }` | Lock screen, notification, Bluetooth or headset. The output itself applies play, pause and seek from those controls and reports them through `time`; next and prev need the queue, so the engine does them |

Rules for the native side: playback must continue with the screen off (the WebView may be throttled, so `setNext` is
always set one song ahead); one item plays at a time except during a crossfade; volume is the app's own stream
level, not the system's.

`window.LullNative.downloads` (offline downloads, ADR-0005) is in [Offline downloads](#offline-downloads-and-outbox). `window.LullNative.updates` (`status`, `check`, `install`: the app's own version and the newest release's APK) is in [android.md](android.md#updates); Settings > App shows it as the app's version row, as it does `LullDesktop.updates` in the desktop app.

## lull Connect (ADR-0005)

`connect.js` (`App.connect`) owns the device list: store `devices` `{ items, me, loading, error }` from
`GET /api/devices`, kept current by `device.online` / `device.state` (an unknown device id rereads the list).
Settings > Devices renders from it.

- **Reporting**: `POST /api/devices/me/state` at once on a new song, play, pause or end; at most once a second for
  seeks, volume and queue changes; every 30 s while playing. A state identical to the last one sent is skipped
  (except the heartbeat, a takeover and the first report after a reconnect). One start runs at a time, however
  often `server` fires while the device list loads. A 404 (a hub without Connect) stops it until the next resync or
  hub restart.
- **Commands**: `device.command` for this device runs `play pause next prev seek volume`; `takeover` reports
  at once, pauses and says who took over.
- **Picker** (`data-act="connect"`: the player bar, Now's action row): a popover, a sheet on phones. Picking a
  device opens its controls (play/pause, skip, seek, volume; positions move by the time since its report).
  **Play here** sends `takeover`, waits up to 3 s for the fresh `device.state`, reads tracks this device hasn't
  seen (`GET /api/tracks/{id}`), and plays that queue here from the same position.

## Offline downloads and outbox

ADR-0005, `offline.js`. One IndexedDB database `lull` (stores `outbox`, `downloads`, `kept`).

**Outbox** (`App.outbox`): likes, follows, plays and playlist changes go through `App.outbox.call(path, opts, op)`:
sent at once when the hub answers, queued with a client uuid and `at` when it can't (`App.server.reachable` false,
the browser offline, or a request that got no answer). The queue goes to `POST /api/sync` in order, 200 ops a batch,
when the hub is reachable again (the `server` event, `resync`, the browser's `online`); applied and refused ops leave
the queue, refusals are named in one toast, and collections reload. A play started offline is queued whole when it
ends, with its real `startedAt`. A playlist made offline gets a `pl-tmp-…` id (`tempId` in its ops) until the hub
answers with its real one (`playlists` in the sync answer).

**Downloads** (`App.downloads`): `keep({ kind: track|album|playlist|liked, id }, ids)`, `unkeep(key)`,
`remove(ids)`, `retry(ids)`, `has(id)`, `state(id)`, `coverage(ids)`, `localUrl(id)` (the engine's source for a
downloaded song), `estimate()`. Two at a time; progress emits `downloads`. Kept collections follow their source
while online (store `playlists` / `liked` changes): new songs download, dropped ones go unless another kept
collection holds them. `ui.downloadButton(collection, ids)` is the icon button on albums, playlists and Liked;
song menus have Download / Remove download; rows show a downloaded mark. The Downloads screen is the Library
segment `/downloads`. Downloads need a secure context (HTTPS or localhost); elsewhere the button says why.

- **Browser backend**: audio in Cache Storage `lull-downloads` under `downloads/audio/<trackId>`, covers under
  their `/api/art/…?size=96|256|512` URLs (sw.js answers covers from there when the hub can't), the index (track and
  album JSON, size, state) in IndexedDB; `navigator.storage.persist()` on the first download. A downloaded song
  plays from a Blob URL.
- **App backend**: `window.LullNative.downloads`, whose contract (Mobile session implements it) is:

| Method | Does |
|---|---|
| `add({ trackId, url, artwork, meta })` | Queues a download of `url` (absolute hub stream URL, Bearer added natively) and `artwork` (`/api/art/…?size=512` or `null`) into app storage. `meta` is the Track JSON, kept with the file and given back by `list`. Resolves once queued; a track already kept resolves at once |
| `remove(trackIds)` | Deletes those files; resolves when gone |
| `list()` | Resolves to `[{ trackId, state: downloading \| done \| failed, fileUrl, artUrl, sizeBytes, meta, error }]`. `fileUrl` is what `player.load` takes; `artUrl` is a URL the WebView can show in an `<img>` (or `null`) |
| `usage()` | Resolves to `{ usedBytes, freeBytes }` |

Events (`LullNative.downloads.on(type, fn)`, returns an unsubscribe function): `progress { trackId, received, size }`,
`done { trackId, fileUrl, artUrl, sizeBytes }`, `error { trackId, message }`. Downloads continue while the app is in
the background.

## Layout (ADR-0007)

A bigger window reveals more; nothing is stretched. Layout is CSS: container queries plus custom properties.

- **Size classes.** `.main` is the `main` container. layout.js mirrors its width class onto `.app[data-size]`:
  `compact` below 640, `regular` to 1099, `wide` to 1599, `xwide` to 2199, `ultra` above. It also sets
  `.app[data-height]` (`short` under 760 px). A ResizeObserver feeds both; nothing is measured in a resize loop.
  `App.layout.size()` and `App.layout.at(name)` read the class, and `layout:size` fires when it changes.
- **Containers.** `main` (`.main`), `panel` (`.ctx`), `nowcol` (`.nowcol`), `section` (a Home module) and `tlist`
  (a song list; the playlist table is `libpl`). A container is the containing block of `position: fixed`
  descendants, so a design's fixed widget inside `.main` is placed against `.main`, not the window.
- **Stored choices.** `App.layout.get()` / `set(patch)` (emits `layout`) / `reset()`. They're kept in
  `localStorage['lull:layout']` as `{ panelWidth, panelCollapsed, nowColumn, nowWidth, searchSplit, panelSplit }`,
  per device. Unknown or out-of-range values fall back to defaults. `reset()` is the hook
  for "Reset layout" in Settings > Appearance.
- **Side panel.**
  - At 1360 px and wider it's docked. Closing it there collapses it to an icon strip (`.app[data-panel="strip"]`),
    one button per tab. Below 1360 px it opens over the page.
  - Its left edge is a resize handle (`[data-resize="panel"]`, 280 to 560 px, on `--ctx`).
  - From `xwide`, a switch shows Queue and Lyrics as two panes (`.app[data-panel-split]`, `.ctx[data-two]`): stacked
    in a narrow panel, side by side once the panel is 600 px wide.
- **Now Playing column.**
  - It's `.nowcol`, in the main grid cell; `.main` gives it room with a right margin. Contents: art, transfer state
    and synced lyrics (`App.nowColumn`, now.js).
  - A player bar button opens it from `wide` (`.app[data-nowcol-ok]`). It counts as offered when the main area
    would stay `wide` with it open, so opening it can't take the button away.
  - `.app[data-nowcol="open|strip"]` shows it; it's hidden on `/now`. Resizable from 300 to 560 px (`--now-w`) and
    collapsible to a strip.
- **Resize handles.**
  - Pointer: a drag, throttled to animation frames, that the column follows live and that is saved when it ends;
    double-click restores the default. While it runs, the width is `--ctx-drag` / `--now-drag` (registered, not
    inherited) on the few elements that read it, not `--ctx` / `--now-w` on `.app`: an inherited property restyles the
    whole page every frame. A grid that places the panel reads `var(--ctx-drag, var(--ctx))`.
  - Keyboard: arrows (Shift for bigger steps), Home and End for the limits.
- **Screens.**
  - **Home:** modules in 1, 2 or 3 CSS columns (`main` at 1100 and 1600), in the order Picked for you, New releases,
    Radio starters, Jump back in, Tags (`MODULE_IDS`). Each module is a `section` container.
    "Picked for you" shows 6, 9 or 12 picks as its module widens. An empty module is one line.
  - **Shelves:** fixed tile widths, so a wider shelf shows more tiles.
  - **Song lists:** album at 760 px, and date added where the list has dates. From 1100 px, play counts (lists that
    have them); from 1240 px, the bitrate and sample rate. Both go in the badge cell, which gets a fixed width so a
    design's grid can't misplace them; the playlist table shows the bitrate from 1300 px. The saved songs list gets
    dates and plays from the `songs` map of `/api/library/songs`. A row off screen skips layout and paint
    (`content-visibility: auto` on `.tlist > .trow`, keeping its last height), so a long list costs only its visible
    rows when the window or a column resizes.
  - **Album pages:** two columns from `wide`; the header is sticky in tall windows.
  - **Artist pages:** a hero across the page (photo, name, Last.fm listeners, Play, Shuffle, Follow, Radio; a real
    photo also glows behind it), then Popular (5 rows, See more for 10, with Last.fm plays at any width) beside About
    (from 960 px of the page), the discography with a filter pill per group (albums, singles and EPs, compilations),
    Appears on and Fans also like. A section the hub names in `pending` shows its skeleton in the same place, and
    `artist.updated` reads the page again, patched in place.
  - **Search:** "Side by side" from `wide`: catalogs on the left, Soulseek on the right.
  - **Short windows:** tighter chrome.
- **Designs.**
  - Layout states that must hold whatever a design draws (the strips, the column's margin) are `!important` in the
    components layer.
  - Scrollbars are base.css's: a thin rounded thumb in `--scroll-thumb` (each surface's quiet colour) on a clear track,
    no arrow buttons. A design retints the token; `scrollbar-width`/`scrollbar-color` on an element would make Chromium
    and WebView2 draw their native bar there, so those are only set for Firefox.
  - Each design adds its own wide-screen touches in its `design` layer, using the same `data-size` and container
    names.

### Better copies

ADR-0013, [streaming.md](streaming.md#better-copies) for the hub's half. While the playing Soulseek song is a lesser copy, its `stream` events offer `upgrade`; the engine (`trySwitch`) switches when:
- the listener didn't pick the copy (`e.source`), and the output has `switchTo`;
- the song plays steadily (not paused, waiting or crossfading), at least 45 s from its end;
- `ready`, and what arrived covers 20 s past where the song is.

The browser output then:
1. **Plays the copy silently** on its second deck (`/api/stream/{id}/copy?peer=&path=`), from where the song is in its own time (the next song's preload waits).
2. **Measures** both decks over 2.5 s, recorded in the same render quanta (one ScriptProcessor), with `LullAlign.measure`: low-band correlation for the lag (±1 s, then the length difference as a second guess for an intro one copy has), full rate for the sample. `locked()` needs confidence 0.6 and a 0.1 margin over any other lag.
3. **Lines it up**: a seek puts the copy 150 ms ahead, its DelayNode holds it back by what is measured, and a narrow measurement checks what is left (under 0.25 ms; measured live at 0.0005-0.014 ms).
4. **Waits for a quiet moment** (a level under 0.6 of the recent median, at most 3 s), calls `commit()` (`POST /api/stream/{id}/switch`), and fades equal-power over 200 ms, the copy's level matched to the song's and gliding to its own over a few seconds.
5. **Keeps the timeline**: the deck keeps its offset and delay, so positions, seeks and buffered ranges stay the song's.

Copies that don't line up (other masters, edits, loops) never switch mid-song: Source details says the better copy plays from the next play. So does a song within 45 s of its end, and the Android app's native output. A seek, pause or skip interrupts an attempt (retried, 3 tries); after the hub switched, it finishes the handover at once. The engine reads the stream events by `jobId`, so the switched-to copy's own `attempt` count is adopted, never reopened. Now Playing's Source details shows a "Better copy" row (on its way, ready, lining up, switched from what at which time, or why it waits).

## Search and source rules (ADR-0008)

- **Search** (discover.js section 2).
  - The catalog answer comes first from `GET /api/search`. It holds the top result card, songs (beside it once the
    column passes 900 px), artists (round photos from the entity's `image`) and releases (with their `type`: album,
    EP, single).
  - While `pending` names catalogs still searching, a quiet line with a spinner says so in the songs heading (one line,
    so it leaves without moving the list). `search.more` appends its ids to each list, never reordering.
  - Soulseek is asked for 3+ characters once the query is submitted: Enter in either field, "Ask now", or 1.5 s
    without typing. The call is `GET /api/soulseek/search`, with rows as `search.results`. Its `onlySoulseek` songs
    are "Only on Soulseek". The same moment asks `/api/search?q=…&submitted=1`, which adds MusicBrainz release groups
    by title to the catalog run; its answer merges like `search.more`.
  - The Files tab lists the raw songs and folders. It appears only when `settings.search.showFiles` is on.
  - Older hubs: an old-shape answer from `/api/search` shows "Search needs a newer hub". So does a 404 from
    `/api/soulseek/search`.
- **Settings > Sources.**
  - Two profiles, `sources.stream` and `sources.save`. Each has: lossless only or a minimum bitrate, format chips
    (none means any), which copy, and what to do if none is good enough.
  - Saving adds `autoUpgrade` and `upgradeEveryH`. Then come the peer limits and "Switch to a better copy while a song plays" (`switchToBetter`, shown when the hub has it).
  - Each control PATCHes only its own key, and the hub merges objects.
  - A hub without the profiles shows a sentence instead.
  - Settings > Search has "Show file results".
- **Switching source mid-song.** Now Playing's "Source" opens the source picker. For the playing song,
  `P.useSource` reopens the chosen peer's stream at the current time as the same play: `load()` gets `keepPlay`,
  so no skip is recorded.
- **Below the minimum.**
  - **Badge:** `stream.belowMinimum` puts a "below minimum" badge (`ui.belowBadge`) in the player bar and Now Playing.
  - **Choose:** `status: "choose"` (streaming `below: ask`) sets the player's `choose` status and opens the source
    picker once per load, with a note and "Skip this song". Picking a copy plays it through `P.useSource`.
  - **Picker rows:** each copy says when it is below the minimum or has a long queue (`meets`, `queueTooLong`). For
    a save, the picker asks `/api/tracks/{id}/sources?for=save`, the saving profile's order.
  - **Saves:** `waiting` (saving `below: wait`) shows the hub's sentence in the save pipeline and Activity. Saving
    `ask` fails at step 1, and the existing recovery picks the copy. Activity marks `upgrade` jobs "Upgrade" and
    saves of a copy below the minimum (`belowMinimum`) with the badge.
  - **Stopping a save:** `App.saves.cancel(id)` sends `DELETE /api/saves/{jobId}`; a `cancelled` event drops the
    record. It's reached from "Stop waiting" (Activity, and the waiting save button's toast) and "Remove this save" on
    a failed row. Skip in the choose prompt moves on, and the engine's `letGo` stops the transfer.

## Bare songs

`ui.cover` (and `ui.mosaic`) ask about every song they draw that isn't saved and has no pictured album: once a
session, batched (`POST /api/catalog/resolve`, 200 at most). The answer's entities, and `catalog.tracks` events for
songs the hub resolves later, are absorbed; covers then repaint (route and player bar, at most twice a second). So a
song from any path, an import's line or a Last.fm-only result, shows the catalog's spelling and cover without a reload.

## Outside playlists

Search's Playlists section (`discover.js`: `playlists` rows in the answer and `search.more`, a shelf after albums; a
`playlist` top result) opens `#/outside/{link key}` (`library.js` section 7b): the album-page layout over
`GET /api/outside?link=`, every row playable at once (`play-ids`), unmatched rows badged "As named", pending ones
"Matching" until an `outside` event swaps in the catalog's song. Save bookmarks it (`PUT`/`DELETE /api/bookmarks?link=`;
a `bookmark` event updates other devices), Make a copy is the link import, Open on YouTube/Deezer the source. Library's
Playlists lists bookmarks after lull's own ("saved from youtube and deezer"), each tile with its site.

## Playlist covers

`ui.playlistArt(p, size, label)` draws a playlist wherever it shows (rail, tiles, the playlist page, the add-to-playlist
picker): its own `cover` (`/api/playlists/{id}/cover`, asked at 96, 256 or 512 px like album art) as one picture, else
`ui.mosaic` of its songs, the first four distinct albums 2×2, or one cover when the songs share an album. Both keep the
`.mosaic` frame every design styles; `.mosaic--one` holds a single cover (the lull design's playlist header shows it as
one larger tilted card). The create and edit dialog picks a picture (JPEG, PNG or WebP, 10 MB, checked before sending)
with a preview, or removes the cover; it is `PUT`/`DELETE` after the details save.

## Imports

Library's Import dialog (`lib-import`, library.js section 10): a playlist file dropped anywhere or chosen there,
`.m3u`/`.m3u8` to `POST /api/playlists/import`, an Exportify `.csv` to `POST /api/import/exportify`; both give the
same preview and the playlist is `POST`ed with the ids kept. Listening history: Last.fm by user name
(`POST /api/import/history { source: lastfm, user }`) or a ListenBrainz export file sent as the body; the job shows
in Activity ("Listening history", plays added and duplicates).

**Import from a link** (`linkForm`, in the Import dialog and under the Playlists grid): `POST /api/import/link { url }`,
then the `job` row shows in Activity ("Playlist link", reading, "Matching songs, n of m", then matched and not found,
with Review). When an import this tab started ends, a toast offers Review (or Open when all matched). The review dialog
(`openLinkReview`, `GET /api/import/links/{id}`) filters Not found, In the playlist, Skipped and All; a song is found
in place with catalog search (`/api/search`, first 6 songs), kept as named, skipped, or undone, each a
`PUT /api/import/links/{id}/songs/{pos}` whose answer repaints the list. "Update from the link" imports it again.

## Playlist sorting

The playlist page shows its songs by the playlist's `sort` (`{ key: custom|added|title|artist|album|duration, dir }`,
library.js section 8): a column header sorts by it (`role=columnheader` with `aria-sort` and an arrow), clicking it
again turns it round, and the sort menu also has Reversed (the custom order, descending). A new column starts the way
people expect: dates newest first, the rest A to Z or shortest first; ties keep the playlist's order. Dragging, Alt +
arrows, Move up/down and Remove act on the playlist's own order, so they work only while it shows custom order;
sorted, a note says so with a "Show custom order" button. Copies and imports send their songs' dates (`added`), so
"Date added" means when a song came into the playlist (an Exportify file's `Added At`).

## Context menus

Right-click, the Menu key (Shift+F10) and a touch long-press on a song row, a Home pick, or an album, artist or
playlist tile (and a rail playlist) open lull's own "…" menu at the pointer (app.js section 11, `contextmenu`): a row
or card with a "…" button opens that button's menu; a tile opens `App.itemMenus[album|artist|playlist](id, anchor)`
(library.js). The keyboard opens it by the item. Text fields and selected text keep the browser's menu (copy, paste),
and so does everything else; the desktop app's WebView2 shows its own menu only where the page leaves the event.

## Links

`App.copyShareLink(kind, id, lullPath)` is "Copy link" (album and artist menus, the player's More menu): it copies the
public page from `GET /api/share/{kind}/{id}` (song.link, or Last.fm) and says which. `App.copyLullLink(path)` is "Copy
lull link", this hub's own address for the screen, which opens only on the listener's devices; playlists have only
that. The clipboard write starts inside the click with the text still on its way (`ClipboardItem` with a promise), so
it keeps the click's permission while the hub answers. No public link (404, 503, hub unreachable) is an error toast
with the reason and a "Copy lull link" button; a browser that refuses the clipboard gets a toast showing the link.

## Rules for screens

- No `PROTO`, no state switcher, no `ctx.state`, no simulated delays, no sample data.
- Render from `M.get` records and the store; refresh happens by itself when reads settle.
- Every control does something real; an endpoint that is down shows its error, not a pretend result.
- Cover art: `App.ui.cover` puts `<img class="cover__img">` (from `/api/art`, album keys only) inside the cover span;
  `.has-art` is added once it loads and hides the recipe's `::before/::after`. Designs that draw on those
  pseudo-elements handle `.has-art` (starlight keeps its frame line, petal never mirrors real art). A picture this page
  has already loaded renders with `.has-art` from the start, and the player warms the next song's cover
  (`ui.warmArt`), so art never fades in twice.
- **Re-renders patch, they don't rebuild.** `App.refresh()` of the page already showing, the player bar, the side
  panel and the screens' own repaints go through `App.patch(el, markup)`: only what differs changes, children matched
  by `id`, `data-key`, `data-track-id` or `data-id`, else by position. An `<img>` with the same `src` is never touched,
  so nothing that stayed the same reloads or flashes. A new route still renders fresh. Because a refresh keeps the
  elements, a route's `mount` runs again on them: listen on elements with `ctx.on(target, type, fn)` (removed before
  the next render) and keep one-off effects (entrance animations, scrolling to a section) behind `!ctx.refresh`.
- **Segmented controls** (`.seg`): render `--n`, `--i` and the selected button's `aria-selected` / `aria-checked`.
  app.js places `.seg__ind` on that button by measuring it (a long label widens its column), whenever a `.seg` appears,
  its selection changes or a button resizes. It slides only when a patch keeps the `.seg`, so re-render it with
  `App.patch`, not `innerHTML`.
- **Live values write only on change** (`setText`, `setAttr`, `setVar` in app.js section 9): the clock ticks four
  times a second, and an identical write still costs a style and layout pass.
