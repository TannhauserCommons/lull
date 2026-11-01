# Web UI: structure, data layer, player engine

`web/` is the shipped UI: classic scripts and plain CSS, no build step, served by the binary (from disk in debug
builds, so a reload picks up edits). It was ported from `design/prototype`; `design/DESIGN.md` stays the reference for
the look, the components and the `App.*` seams (section 7). This page covers what the port added: where data comes
from, how the player plays, and the rules screens follow. The endpoint contract is [api.md](api.md).

## Files and load order

```
index.html   tokens.css base.css components.css screens/*.css desktop/desktop.css designs/<name>/design.css (last)
             designs/designs.js + the pre-paint script (design, theme, motion, text size)
             icons.js data.js app.js connect.js offline.js screens/{discover,now,library,system,appearance,hub,setup,classic}.js desktop/desktop.js
data.js      window.LullData = App.data: API client, entity cache, per-route reads, the event stream
app.js       window.App: router, render helpers, overlays, save pipeline, player engine, shell, actions,
             keyboard, Media Session, drag and drop, server events + boot
sw.js        service worker (app shell for offline start); manifest.webmanifest
hub.js       the hub's own settings: library source (folder / Jellyfin), Soulseek account, Settings > Devices (App.hubKit)
setup.js     the pairing gate, #/pair/<code>, first-run setup (/setup)
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
| playlists | `GET /api/playlists` -> store `playlists` (= `M.playlists`) | `App.createPlaylist`, `App.savePlaylistOrder`, `App.addToPlaylist` |
| saves | store `saves` `{ trackId: { jobId, step, status, pct, error, kind, nocover } }`, seeded from unfinished save rows of `GET /api/jobs` | `App.saves.start(ids, { mode, source })` (`mode` = the duplicate dialog's separate/replace), `retry`, `save` events |
| activity | `GET /api/jobs` -> store `activity` (rows keyed `id` = jobId) | `job`, `transfer` events |
| services | `GET /health` -> store `services` (`state ok|warn|down|off`, `label`, `detail`, `queued`, `usedGB`, `capGB`) | `service` events |
| server reachability, auth, version | `App.server { reachable, authorized, version }`, event `server` | `/api/ping`, `/health`, event stream errors |
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
  over on `ended`. Off when Settings > Playback turns gapless off.
- **Crossfade** (`playback.crossfade` seconds): the next deck starts under an equal-power fade.
- **Normalisation** (`off | replaygain-track | replaygain-album | loudness`): Web Audio gain per deck. ReplayGain uses
  `track.replayGain { track, album }` (dB) when the library supplies it; otherwise the measured loudness of what has
  played (unweighted RMS) is levelled toward `playback.loudnessTarget` (-18 / -14 / -11) within -12..+6 dB. The Web Audio graph is built only when normalisation or
  crossfade is on, inside the first play gesture.
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

`window.LullNative.downloads` (offline downloads, ADR-0005) is in [Offline downloads](#offline-downloads-and-outbox).

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

## Imports

Library's Import dialog (`lib-import`, library.js section 10): a playlist file dropped anywhere or chosen there,
`.m3u`/`.m3u8` to `POST /api/playlists/import`, an Exportify `.csv` to `POST /api/import/exportify`; both give the
same preview and the playlist is `POST`ed with the ids kept. Listening history: Last.fm by user name
(`POST /api/import/history { source: lastfm, user }`) or a ListenBrainz export file sent as the body; the job shows
in Activity ("Listening history", plays added and duplicates).

## Rules for screens

- No `PROTO`, no state switcher, no `ctx.state`, no simulated delays, no sample data.
- Render from `M.get` records and the store; refresh happens by itself when reads settle.
- Every control does something real; an endpoint that is down shows its error, not a pretend result.
- Cover art: `App.ui.cover` puts `<img class="cover__img">` (from `/api/art`, album keys only) inside the cover span;
  `.has-art` is added once it loads and hides the recipe's `::before/::after`. Designs that draw on those
  pseudo-elements handle `.has-art` (starlight keeps its frame line, petal never mirrors real art).
