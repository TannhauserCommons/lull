# Web UI: structure, data layer, player engine

`web/` is the shipped UI: classic scripts and plain CSS, no build step, served by the binary (from disk in debug
builds, so a reload picks up edits). It was ported from `design/prototype`; `design/DESIGN.md` stays the reference for
the look, the components and the `App.*` seams (section 7). This page covers what the port added: where data comes
from, how the player plays, and the rules screens follow. The endpoint contract is [api.md](api.md).

## Files and load order

```
index.html   tokens.css base.css components.css screens/{discover,now,library,system}.css designs/<name>/design.css (last)
             designs/designs.js + the pre-paint script (design, theme, motion, text size)
             icons.js data.js align.js eq.js player-rules.js app.js layout.js connect.js offline.js screens/{discover,now,library,system,appearance}.js
             screens/setup.js
             the desktop app only (window.__TAURI__): screens/classic.css desktop/desktop.css
             desktop/webamp-session.js screens/{skins,classic}.js desktop/desktop.js, from the start, as its crash
             recovery hands the queue back through classic.js
             everything else loads on its first use: see "Code loads where it is used" below; the start's files and
             why each is there: tests/ui/gates/start-files.json
data.js      window.LullData = App.data: API client (the same read made twice at once is one request; a write
             forgets the reads on their way), entity cache, per-route reads, the event stream
player-rules.js  window.LullRules: the player's rules (what the queue moves to, plays, levels, preloading, radio,
             seeking, better copies), pure; app.js follows them and so does the desktop's native Classic player
             (tests/player-rules.json, tested on both sides)
align.js     window.LullAlign: how far apart two recordings of one moment are (switching to a better copy); pure,
             tested by tests/web/align.test.mjs (`node --test`, part of `mise run check`)
eq.js        window.LullEq: the equaliser's settings (per device), its curve and automatic level, its Web Audio chain;
             pure but for storage and chain(), tested by tests/web/eq.test.mjs. Loaded by desktop/winamp.html too
app.js       window.App: router, render helpers, overlays, save pipeline, player engine, shell, actions,
             keyboard, Media Session, drag and drop, server events + boot
layout.js    ADR-0007: size classes, App.layout (lull:layout), resize handles, the Now Playing column host
sw.js        service worker (app shell for offline start); manifest.webmanifest
hub.js       the hub's own settings: library source (folder / Jellyfin), Soulseek account, Settings > Devices (App.hubKit);
             with hub.css, loaded on use (App.needHub)
setup.js     the pairing gate, #/pair/<code>, first-run setup (/setup), library.source events
skins.js     Classic mode's skin and size, per device (App.classicSkin, App.classicSize), and Settings > Appearance >
             Classic: the size, the skin browser, .wsz files kept on this device, the NSFW switch (docs/context/skins.md)
stats.js     Stats and the taste profile (/stats), with stats.css; loaded on the first visit, built with system.js's kit
classic.js   Classic mode: Webamp alone, in an iframe on a stage in a browser, or in the desktop app's own window
vendor/      Webamp bundle and the QR encoder (qrcode-generator), copied by scripts/vendor-web.sh, gitignored
desktop/     the desktop app's wrapper pages and bridge (`LullDesktop`), owned by the Apps session
```

## Code loads where it is used

Code that belongs to one screen or panel loads on its first use, not from index.html. What a browser fetches at start
is what every visit needs before it can show anything: the shell, the player, Home, the pairing gate. Each file there
is listed with its reason in tests/ui/gates/start-files.json, and the on-use gate (gates/on-use.spec.mjs) fails on a
file at start that isn't, so a new screen can't join the start by default. Lighthouse's transfer budget
(tests/ui/budgets.json) is for the start; a feature that stays out of it doesn't spend it.

The way to do it (app.js, beside the router):

- `App.needScript(src, what, style)`: a screen's script and, if it has one, its stylesheet (in the screens layer),
  loaded once; the promise resolves when both are in, so the screen never shows unstyled; a failed fetch is tried
  again next time and reported with `what`. Named helpers wrap it: `App.needStats`, `App.needHub`; `App.needClassic`
  and `App.needEq` are the same pattern for Classic mode's several files and the equaliser's panel.
- A route's `need(params, query)`: a promise while there is code to load, nothing once it is here. The page showing
  stays until it resolves, then the route renders as if it had always been here. A route whose whole screen loads
  registers a stub with `need` up front (its nav, title and place work before it loads), and the screen's file
  registers the route again (Stats, Classic). A route that shows such code in some cases asks only then (Settings
  asks for hub.js for Library, Sources and Devices).
- Panels and dialogs call their helper before building (`App.needEq().then(...)`), and a section drawn inside an
  always-loaded screen shows a placeholder until its code arrives (Settings > Appearance > Classic).
- An action on a start screen whose work is rare loads that work when asked, behind a one-line stub: importing or
  deleting a theme and the check of a theme's loops (screens/your-themes.js, from appearance.js; Theme Studio asks
  for the check through `App.themeLoops`), and drawing a theme's picture for its card (screens/theme-pictures.js,
  `App.themePictures`, ADR-0080: one off-screen frame of Home in the theme, drawn through an SVG foreignObject, each
  step that can't be split waiting until nothing was tapped, typed or scrolled for 1.5 s).
- A state most visits never reach loads with what it shows: the pairing gate opens from the start (setup.js) with its
  placeholder, and its form comes with hub.js (`App.hubKit.gateForm`), as first-run setup's page does
  (`App.hubKit.setupPage`); a paired device that is set up fetches neither.
- The way in fetches ahead: the `SOON` table in app.js lists, for each, where the pointer or focus goes and what it
  loads, so the first click doesn't wait (Classic, Stats, the hub's settings). Classic, which the W key opens
  without warning, also loads once the page is idle.
- What has to happen from the start stays in an always-loaded file, and the screen's file picks it up when it
  loads: setup.js hears `library.source` and keeps the latest (`App.librarySource`), hub.js shows it. Styles the
  start needs stay in a start stylesheet (the Connect picker and the pairing gate are system.css's).
- Offline: the service worker keeps every app file as it is fetched (sw.js), so a screen used once online opens
  offline too.

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
| queue, volume, shuffle, repeat, where it plays from | `App.player.state` (`context`), saved per device so a reload keeps the queue | the engine |
| where each page was (scroll, focused row) | the router, per history entry (`sessionStorage['lull:spots']`) | leaving a page; Back, Forward and a reload return there |

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
  `buffered` is `audio.buffered` cut to `received`. A seek may go anywhere for saved songs and up to what has
  arrived for Soulseek ones (`P.seekable(f)`, `LullRules.playable`/`seekTo`): `ps.arrived`, the hub's `ranges` (one
  part from 0 until a seek ahead, ADR-0014), with what the output holds. Past it, it goes there at once when the hub
  can start the transfer there (`ps.aheadS`, the event's `seekAheadS`) and the transfer running would take longer
  than that to get there: the output asks for that byte and the hub fetches from it ([streaming.md](streaming.md#legs)).
  Otherwise (sooner, or a format the hub can't start mid-file) the song plays on and the seek is kept (`ps.seekWait`):
  the seek bar and the mini line show a still notch where it lands (`.is-going`, `.seek__target` at `--target`) and
  the slider's value says "waiting for 2:40". Words come only for a long wait (`P.seekWaitLong`: estimated over 8 s or
  unknown, or lasting 3 s): then the state tab and the mini line say "Waiting for 2:40, about 8 s"
  (`LullRules.seekWaitS`, from the transfer's speed) and screen readers hear it once, and "Playing from 2:40" when it
  lands. It lands as soon as those bytes are here (`landKept`, on each `stream` event and tick). The next seek
  replaces it, another song drops it, and stepping (arrow keys, Media Session) goes on from it (`P.seekTarget`). A
  lyric line seeks the same way. The hub holds a range ahead of the transfer until its bytes arrive (never 416 for a
  range inside the file), so a start the browser maps past `received` (VBR, a big embedded cover) only buffers.
- **The seek bar's look** is one language in every design and view (the bar, Now Playing, the device waveform, the
  mini line, transfer progress in Activity): played is solid `--seek-played`; arrived but not played is one solid
  `--seek-buffered` (`--xfer` = the end of the part from 0, 1 for a saved song; the element's `buffered` ranges and
  the parts a seek ahead brought further on draw the same, as `.seek__buf`); not arrived is the plain track. While bytes arrive (`.is-transferring`) the arrived part's edge breathes;
  reduced motion stills it. No hatching: a design changes the tokens, not the language.
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
  crossfade, switching to better copies (`sources.switchToBetter`) or the equaliser is on, inside the first play gesture
  (the equaliser turned on mid-song builds it at once).
- **Equaliser**: [below](#equaliser), after both decks are mixed.
- **Better copies**: the playing song moves to a better copy mid-song without a jump; [below](#better-copies).
- **Plays**: `App.recordPlay(trackId, duration, recommended)` posts `POST /api/plays` when a song starts playing
  (`recommended` for radio songs) and its `end(how)` sends `PATCH /api/plays/{playId}` with `playedS` (time actually
  heard), `completed`, `skipped` when it ends, is skipped, or the page closes (`keepalive`). Classic mode uses it too.
- **Other players**: `App.hooks.media` (default `null` = the engine) is what media keys drive; Classic mode points it
  at Webamp while open. `P.jumpTo(index, { at, autoplay })`, `P.streamUrl(trackId, source)` serve the hand-back.
- **Radio**: `P.startRadio({ kind: track|artist|tag|album, id })` fetches `/api/radio`; while "keeps adding" is on
  the queue is topped up to three songs ahead (`exclude` = what is queued). With "radio when the queue ends", the
  last song seeds a radio as it starts. Changing the steer re-fills the radio songs not yet played.
- **Where it plays from** (`ps.context = { kind, id, name }`, persisted with the queue): `playList(ids, start,
  { context })` records the place a play starts from. Left out, it is the page showing, `App.here()`: each list route
  declares its `place(ctx)` (album, artist, playlist, tag, Liked, History, Saved songs, Downloads, search results by
  query, Home, Stats, a bookmarked or outside playlist); tile and button plays name theirs (`play-album`,
  `play-playlist`, a radio: `radio`, `<seed kind>:<seed id>`). Connect's Play here has none (the device state doesn't
  carry it); Classic's hand-back keeps it. `P.playingFrom()` is the playing song's place: the context, the radio that
  added it (radio when the queue ends), or `queue` (the listener's next up, a song played on its own). `PLACES` in
  app.js gives each kind its icon, its page (radio and the queue have none) and its live name (a renamed playlist).
  - **Shown** by `ui.playingFrom(cls, label)`: the player bar (`.pbar__from`; the title opens the same place, the
    album when it has no page), Now Playing (`.now-head__from`, and a theme's `playing-from` part), the Now column, the
    queue (its heading, and "next from <place>"), the phone mini player (`.mini__from`, text only).
  - **Opened** by `App.goPlaying()` (`data-act="go-playing"`): the place's page, the playing row scrolled to the
    middle, focused and lit (`.is-revealed`, 1.8 s), waiting up to 6 s for the page's read; radio and the queue open
    the queue.
  - **Marked**: a tile or link of a place carries `data-place="kind:id"` with `ui.placeCls(key)` and
    `ui.placeMark(key)` (the playing bars, a row's `.eq`); `App.syncRows` (`App.syncPlaces`) moves the mark when the
    place or play state changes, touching only the elements whose state differs.
- **Media Session**: metadata with `/api/art/{albumId}?size=96|256|512` once the 96 px cover is known to exist (else
  `assets/fallback-art-*.png`), `playbackState`, position state on track change and seek, all handlers; `seekto`
  is kept or clamped like the seek bar's.

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
| `seek(seconds)` | The engine only seeks inside what has arrived (`P.seekable`), keeping a seek past it until it arrives; the output doesn't need to know about Soulseek |
| `setVolume(v)` | `0..1`, the final level: the engine has already applied mute and its perceptual curve (`v = volume²`) |
| `setNext(item \| null)` | Preload `item` and, when the current one ends, continue into it with no gap. Continue only when the current one reached its end (within 2 s of its duration): an earlier end is a cut response, reported as `ended` with `next: null`, and the next item stays set. `null` cancels. Called again whenever the queue, shuffle or repeat changes |
| `crossfade(seconds)` | `0` = off. Otherwise, with a next item set, fade equal-power from the current into the next over the current's last `seconds` |
| `equalize(spec \| null)` | Optional (an Android app without it shows Settings' equaliser as needing an update). Play through `spec` = `LullEq.spec(settings)`: `{ levelDb, bands: [{ type: lowshelf \| peaking \| highshelf, hz, gainDb }] }`, a level and then the bands in order, as Web Audio's BiquadFilterNode computes them (peaks Q 1, shelves slope 1). `null` = off. Called at start and on every change |
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
  at once, pauses and says who took over; `enqueue` reads the songs it hasn't seen and adds them to next up without
  interrupting. `C.playingElsewhere()` and `C.enqueue(d, ids)` send one (a share page's Add to queue, discover.js).
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
  their `/api/art/…?size=96|256|512` URLs (sw.js answers covers from there when the hub can't: the size asked, else the nearest kept one, since the page may ask for up to 2048), the index (track and
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
  - It's `.nowcol`, in the main grid cell; `.main` gives it room with a right margin. Contents: art, where it plays
    from, title, artist and album, transfer state and synced lyrics (`App.nowColumn`, now.js).
  - It's offered (`.app[data-nowcol-ok]`, the player bar's button shows) when the main area stays `wide` beside it
    with the docked panel at its strip: the window's width alone decides, so the button never comes and goes as the
    panel opens or closes.
  - **It and the docked side panel:** both stay open when the main area stays `wide` beside both. Otherwise opening
    one closes the other: opening the column collapses the panel to its strip; opening the panel closes the column
    and forgets it, so closing the panel brings nothing back (`App.layout.makeRoom`). A window narrowed until both
    no longer fit hides the column until it is wide enough again. Below 1360 px the panel opens over the page and
    takes nothing from the column. Checked in all five designs at 1280, 1440 and 1920 px.
  - `.app[data-nowcol="open|strip"]` shows it; it's hidden on `/now`. Resizable from 300 to 560 px (`--now-w`) and
    collapsible to a strip.
- **One place shows the lyrics.** Now Playing's page when it is the page, else the Now column while it is open, else
  the side panel: the panel's Lyrics tab steps aside for the other two (its `hidden()`, now.js), and comes back by
  itself where it is the only one (the tab the listener chose is `ctxWant`). Asking for the lyrics (Y, the bar's
  button) then goes to where they are. A panel def may give `hiddenNote` as a function and a `reveal()`. A design
  never patches this, so every design behaves the same; the `now` gate checks it.
- **Now Playing says the same in every design:** title, artist, album, the quality badge and where it plays from,
  on the page at any size (on phones the Art tab's by-line carries the album) and in the Now column; a design may
  restyle them, never drop one (the `now` gate).
- **Player bar.** The song with its Like and Save, the transport, the listening tools. The transport is centred on the
  bar (its padding is even): both sides are as wide as the tools need, and no wider than leaves the middle
  `--fit-centre-min`, so the middle takes every pixel left and nothing stands empty beside the tools. They widen only
  as far as keeps the song's chips that the most room would show (layout.js sets `--fit-side` on `[data-fit-sides]`
  from its `[data-fit-side]` row; the song's side keeps `--fit-side-min`). Every control has a fixed slot (toggles
  show pressed or not, the crossfade tag floats above the tools); only the room the bar has moves tools into the "…"
  menu, which lists every one of them, and the volume slider into a popover over its button. The seek
  keeps one length: the time cells are as wide as the song's duration reads (`--time-ch`) and a design's status
  caption has one slot sized for its longest word.
- **Rows that fit themselves** (layout.js). The header, the player bar, its tools and the song's chips lay themselves
  out from the room they have, not from the window's width, so a docked panel, the Now column, the text size or a
  long title count as much as the window, and no design needs a width band to patch them.
  - A row is `[data-fit-row]`; a part that can give way is `[data-fit="<order>"]`, 1 first, parts of one order
    together. When a row's content needs more than its width, parts give way in order (`[data-fit-out]`) until it
    fits; with room again they come back. A row and the rows inside it (the bar, its tools, the song's chips) fit as
    one, from all shown, outer first and again until none moves, so however the room was reached they end as a fresh
    fit would: rows fitted one at a time, frames apart, settled differently by the path taken, or a frame late. A
    ResizeObserver re-fits a row when it resizes and when its children or parts do without it (a label that grew),
    every font that finishes loading re-fits them all (a part that gave way shows no box to tell the observer that
    narrower letters would let it back), so does every stylesheet that changes in `<head>` (a theme reloaded or
    edited, Theme Studio's draft: wider words inside boxes that keep their size, beside sides the fitter sized, resize
    nothing the observer watches), and `App.layout.refit()` runs after its content changes (renderBar,
    renderTray), watching what was drawn since. `[data-fit-out]` is the fitter's: `App.patch` leaves it on a part
    when it re-renders the bar with the same words, and `data-fit-gave` on its row (the orders that gave way, and
    `all` once every one did). A style that follows a part out reads the row's `[data-fit-gave~="2"]` or a sibling
    combinator, never `:has([data-fit-out])`: Chromium rechecks every `:has()` up the tree when an attribute in one
    changes, so in a design with a `.app:has(…) .view` rule each step of the fit restyled the whole page (5 ms a
    step in Starlight).
  - What giving way means is the component's (components.css, `!important`, so a design draws it but can't undo it):
    the header drops the "/" hint (1), the tray's long words (2), the full tray for the compact one (4), the nav
    pills' labels (5: Playlists, Stats; 6: Home, Library), lets the search narrow without its placeholder (7), and
    last the compact tray's word (8); its buttons never go, for nothing else opens what they open. The bar's tools
    leave for the "…" menu: Device view (2), Classic (3), Lyrics (4), Devices (5); the volume slider (1) becomes a
    popover over its button, shown while the pointer is on either or the slider has focus (the button opens it from
    the keyboard, Escape goes back; the wheel over the button turns it). The song's chips drop the source (1),
    "below minimum" (2), the format (3), and Private keeps its eye (4); what the source and format open is the "…"
    menu's Show source.
  - The phone's mini player: its text (`.mini__text`) is a row, and a design's state chip beside the song's line
    (`.mini__chip`, empty and hidden unless a design draws it: lull's "streaming") gives way (1) before it would cover a
    button or the line (`[data-fit-keep]`) would show less than its readable width.
  - The song side (`.pbar__now`) is a row of its own: "Playing from" (1), then Like and Save together (2), then, with
    nothing playing, the Private pill's word (4). What decides is the song's words: the title and artist
    (`[data-fit-keep]`) keep `--fit-keep` (10ch, components.css) or all of themselves when shorter, so the side's parts
    give way before either shows less. Like and Save are in the "…" menu, and the title opens where the song plays from.
  - What doesn't fit, by one rule for every row: content past the row's content box; content past the edge of a box
    around the row that clips (up to the row it fits with); a box in the row that clips content of its own; a part
    whose words are cut short (text-overflow: ellipsis, to the fraction of a pixel: a part shows whole or gives way
    whole); and words that keep a width shown narrower. A box that cuts its own text short on purpose (a long title)
    is not clipping anything that should give way.
  - A part a design takes out of the row's flow (glasshouse floats the tray into the rail) never gives way; a design
    that lays a row out with `display: contents` (lull's desk bar) hands its parts to the row around it.
  - Width rules are for genuine layout changes only: the shells (640, 1100, 1360) and content that rearranges by the
    room its container has (`@container`). The fit gate (quality.md#ui-gates) sweeps every width.
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
    dates and plays from the `songs` map of `/api/library/songs`. The number cell shows one thing at a time: the
    number, Play under the pointer or focus, the bars while the song plays. Rows drag from anywhere on them (no grip),
    the queue's by mouse past 5 px or by touch from the cover; Alt + arrows reorder from the keyboard. A row off screen skips layout and paint
    (`content-visibility: auto` on `.tlist > .trow`, keeping its last height), so a long list costs only its visible
    rows when the window or a column resizes.
  - **Long lists cost what short ones do.** In a list of more than 60 rows (a playlist, the queue, an album's discs and
    History's days, each counted as one list with `ui.lazyRows`, saved, liked and downloaded songs, search results, an
    import's lines, an outside playlist's songs), a row has its contents only within a screen of view (`ui.row`,
    `ui.lazyRow`); the first 24 are drawn at once, as the list opens, and then go the same way:
    - The `<li>` with its classes, data, label and tabindex is always there, so selection, the keyboard, screen
      readers, dragging, `App.syncRows` and scrolling to a song work on every row. Away from the screen it is an empty
      block (components.css `li[data-fill]:empty`) holding the height it had drawn, or, never drawn, a drawn row's of
      its list (`holdPlace`), so the list is as long as it will be and a jump ("Playing from", Back) lands where it
      stays. As the row's grid, with `content-visibility`, an empty row held ~10 KB of grid sizing and a paint layer of
      its own; a long list's rows have no `content-visibility`, which would report an estimated height.
    - Its contents are made from the state of the moment (likes, downloads, saved, playing) each time it comes near,
      and dropped once it is well away; a focused, dragged or menu-open row keeps them. Markup with lazy rows must
      reach the page in the task that made it (`pruneFills`). Reusing the contents of rows that left instead, updated
      in place, was measured: the same memory and main-thread time, since what scrolling costs is the drawn row's
      styles and layout, which any row coming into view needs anew.
    - The playing bars (`.eq`) are in the playing row only; `App.syncRows` moves them.
    - On the fixture, a 5,000-song playlist takes ~120 MB opened and ~240 MB scrolled through (it took 600 MB and
      1.5 GB); each song more costs its empty `<li>`, a few KB, which perf.spec's long-list gate holds.
  - **Album pages:** two columns from `wide`; the header is sticky in tall windows.
  - **Artist pages:** a hero across the page (photo, name, Last.fm listeners, Play, Shuffle, Follow, Radio; a real
    photo also glows behind it), then Popular (5 rows, See more for 10, with Last.fm plays at any width) beside About
    (from 960 px of the page), the discography with a filter pill per group (albums, singles and EPs, compilations),
    Appears on and Fans also like. A section the hub names in `pending` shows its skeleton in the same place, and
    `artist.updated` reads the page again, patched in place. The page doesn't hang on that one push
    (`readAgainWhileComing`, the album page's `partial` too): it reads again when it is seen again (the app back from
    the background) and once after the hub's 90 s bound, and a reconnect re-reads everything. A failed section shows
    its sentence and Try again.
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
  - Loops (components.css Keyframes, docs/themes.md "Drawing cheaply"): a loop that never ends is stepped where it is
    declared, `steps(n)` or a stepped easing (`--loop-<curve>-<n>`, a `linear()` staircase that samples the curve;
    scripts/loop-easings.mjs prints and checks them), so it changes as often as its motion needs from its first frame,
    whatever the engine or the screen's refresh rate, with no script watching the page. `App.still(reason, on)` sets
    `:root.is-still` while any reason holds, and that sets `--loop-play: paused`, which each decorative loop takes as
    its `animation-play-state`; progress loops leave it out. A theme's loop that names `--loop-fps` (the contract themes
    were written to) is stepped to it by app.js as it starts and held at playback rate 0 while still, unless it sets
    `--loop-holds: no`; as each loop starts, app.js also moves its delay by under a step so loops change on one beat.
    The one reason so far is the device pref `pauseInactive`
    ("Pause animations when lull isn't the active window", off by default): the window has no focus (`blur`, `focus`,
    `document.hasFocus()`), in the desktop app and a browser, never on Android. `.sky-specks` is absolute, not fixed,
    so it isn't a window-sized compositor layer forcing the app over it onto another.

### Equaliser

One per device, `localStorage['lull:eq']` = `{ on, bands: [10 dB], preamp: dB | null }` (web/eq.js). The bands are
Webamp's (a 60 Hz low shelf, peaks at 170 Hz to 14 kHz, a 16 kHz high shelf, ±12 dB), so Classic mode's equaliser
window is the same equaliser, not a second one.
- **Faces** (screens/eq.js, `App.eq`, loaded on first use by `App.needEq`): Settings > Playback (the switch, presets, Bass and Treble, "Every band" with
  the curve and the level) and the player's "…" > Equaliser (a popover, a sheet on phones: switch, presets, Bass and
  Treble). Built from the shared switch, chips, `.range--centred` and settings rows, so each design draws it as it
  draws those. A panel updates its values in place on every change (`LullEq.on`), from any panel, Webamp or window.
- **Bass and Treble** move a shelf shape over the bands (+6 dB of Bass peaks at +6 dB) and read back as the weighted
  mean of their bands, so a curve drawn band by band keeps its detail when one moves. A preset sets the bands; a
  curve matching none shows "Custom".
- **No clipping:** the level before the filters is automatic, the curve's highest point back to just under 0 dB
  (computed from the filters' formulas, not measured). Webamp's preamp shows it; moving Webamp's preamp sets it by
  hand until Bass, Treble, a preset or "Make automatic" makes it automatic again.
- **Where it runs:** the browser output (also the desktop app) runs `LullEq.chain` after the decks' master gain:
  native BiquadFilterNodes, no script on the samples, nothing at all while flat or off (the chain is bypassed). Gapless,
  crossfades, a better copy's handover and seeks happen before it and pass through unchanged. The Android app runs
  the same filters natively (`EqProcessor`, android.md). Classic: `LullWebamp.session` links Webamp's equaliser
  window (its sliders, on button and presets) to the settings both ways.

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

## What an import does, said plainly

- **Save the songs too** is one per-device choice (`libImportSaveSongs`), shown wherever it applies: the file import's
  preview, the link box, and an outside playlist's notice. A link import sends it (`saveSongs`), so the hub saves the songs
  whether or not this tab stays open; the file import saves its Soulseek songs from this tab once the playlist exists.
- **Every import ends in a playlist in Library, Playlists**, and its toast and Activity row say so ("Imported X into your
  playlists: n songs, m not found, saving k", with Open and Review).
- **A file imported again** under the name of an existing playlist updates that playlist's songs instead of making a
  second one; the name field says so before the import ("You have a playlist called X: importing updates its songs").
- **An outside playlist** opened from search is a preview and says so in a notice: playing it keeps nothing (playing
  reminds once, with "Add to library"). "Add to library" follows it (a bookmark); "Import as playlist" makes the user's
  own playlist (the link import); once imported, the page offers "Open your playlist" and "Update it" instead.

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
picker): its own `cover` (`/api/playlists/{id}/cover`, asked at the size it is drawn like album art) as one picture, else
`ui.mosaic` of its songs, the first four distinct albums 2×2, or one cover when the songs share an album. Both keep the
`.mosaic` frame every design styles; `.mosaic--one` holds a single cover (the lull design's playlist header shows it as
one larger tilted card). The create and edit dialog picks a picture (JPEG, PNG or WebP, 10 MB, checked before sending)
with a preview, or removes the cover; it is `PUT`/`DELETE` after the details save.

## Cover viewer

`App.viewCover({ kind: track|album|playlist, id })` shows a cover large without leaving the page: the design's dialog
(`.cover-view`) with the picture and its name, artist and album as links; Esc, the scrim and Close shut it and focus
returns to the opener. The picture is as large as the window allows, up to 1024 CSS px (the hub's 2048 at 2x scaling).
It opens at once with the size this page already decoded, and the size the dialog needs swaps in once decoded (covers
at the size they are drawn, below), so it never fades in twice. Opened from:
- the player bar's cover and the Now column's (`ui.coverZoom(trackId)`, an expand button inside `.cover-zoom`, shown
  on hover and keyboard focus; hidden on touch screens, which have no hover);
- "View cover" (`App.coverItem(kind, id)`) in the player's "…" (Now Playing's too) and every song, album and playlist
  menu, so also by right-click. A song row's cover keeps its click for the row (select, double-click to play).

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

Right-click, the Menu key (Shift+F10) and a touch long-press open lull's own "…" menu at the pointer (app.js section
11, `contextmenu`): everything with a "…" opens that same menu, built by the same code. `MENU_OF` names each item and
where its "…" is: song and queue rows, Home picks and the album and artist heroes hold theirs; a playlist's header
opens the playlist's, the player bar's song and Now Playing's song the player's. Items without one (tiles, a rail
playlist, the search's top result) open `App.itemMenus[track|album|artist|playlist](id, anchor)` (track in app.js,
the rest in library.js). The keyboard opens it by the item. Text fields and selected text keep the browser's menu (copy, paste),
and so does everything else; the desktop app's WebView2 shows its own menu only where the page leaves the event.

**Several songs at once.** A click on a song row selects it (`.is-selected`), Ctrl (Cmd) adds or drops one, Shift
reaches from the last one picked, and Shift with Up or Down does the same from the keyboard. With more than one
selected, the row's "…" and right-click open `App.itemMenus.songs(ids, anchor)` (play next, queue, add to playlist,
save, like, download) and dragging carries them all; `App.selectedIds(row)` is the list.

## Adding to playlists

Every way in opens one sheet, `App.hooks.addToPlaylist(ids, anchor)` (library.js section 9): a song's "…" and its
right-click, the player's and Now Playing's, several selected songs, an album's button and menu, and a playlist's
"Add to another playlist" (the playlist itself left out). It is a checklist: a playlist that already holds every song
is ticked, one holding some is part-ticked (`aria-checked="mixed"`), and nothing changes until Done (Ctrl+Enter).
- Ticking adds only the songs a playlist lacks; "Add again" on an already-ticked one adds them once more, the one way
  to make a duplicate. Unticking takes the songs out. A part-ticked one cycles ticked, unticked, as it was.
- "New playlist" names one in the sheet; it is made at Done, with the songs.
- Done saves each changed playlist (`App.savePlaylistOrder`, so offline it goes through the outbox) and makes the new
  ones; one toast names them all, with one Undo. Up and Down move between playlists; Enter in the search ticks the
  first match, or names a new playlist when none matches.
- Songs dropped on a playlist (`App.addToPlaylist`) add only what it lacks, too; when it has them all, the toast says
  so and offers "Add again".

## Links

One "Copy link" per item, and it works for anyone:
- **Songs, albums, artists** (`App.copyShareLink(kind, id)`: their menus, the player's More menu) copy `link` from
  `GET /api/share/{kind}/{id}`: a share host's page that chat apps show as a card (ADR-0034), or, until there is one,
  lull's open page (`site/`, ADR-0032) followed by `#lull?`; either names the item and the public ids lull knows, and
  leads to the public page (song.link, which opens in every music app, or Last.fm). Clicked, the page
  opens the listener's lull or offers the public page. Never this hub's address.
- **Playlists** imported from a link copy that link (`GET /api/import/links`); an outside preview's Copy link copies
  its page. One made in lull has no page to share and says so (Export as M3U sends it).
- **Opening** (`App.openLink`, `#/open?link=`, discover.js): a link pasted in search, or handed over by the apps'
  `lull://open?link=…`, opens this hub's own page for what it names (`GET /api/open`); playlist links open as their
  preview (`#/outside/`). A song opens on its album and plays when a click or key brought it there (sticky user
  activation), else a toast offers Play. Without lull, the link is just the public page.
- **Add to queue** (`#/open?link=…&do=queue`: a share page's Add to queue, through the apps' hand-off or the browser):
  a song, or an album's songs (`App.albumTracks`), join next up: of the listener's device playing now when this one
  isn't (Connect's `enqueue`, a toast naming it), else here (`P.addToQueue` with `paused`: with nothing loaded the
  first waits paused, never playing on its own), then lull goes back to the screen it showed, or Home when the page
  loaded on that route (`App.current.first`). An artist link just opens. `gates/open.spec.mjs`.

The clipboard write starts inside the click with the text still on its way (`ClipboardItem` with a promise), so it
keeps the click's permission while the hub answers (`App.copyLink`). No link (404, hub unreachable) is an error toast
with the reason; a browser that refuses the clipboard gets a toast showing the link.

## Covers at the size they are drawn

The hub serves every cover and photo at 96, 256, 512, 1024 and 2048 px, scaled down and never up, with `X-Image-Size`
(api.md `/api/art`). Each cover asks for the smallest size that covers its drawn box in device pixels (CSS size ×
`devicePixelRatio`), so it is as sharp as the screen shows and no larger, in every design and at any display scaling
(app.js, "covers at the size they are drawn"):
- **Measured, not guessed.** Designs resize covers in CSS (`--cover-size`, `100%` tiles, heroes sized by `--lib-art`),
  so the size a screen passes to `ui.cover` is only the first guess. A ResizeObserver measures every `img.cover__img`
  carrying `data-pic` (its picture without a size; a MutationObserver finds them) and again when it, the zoom or the
  screen's scaling changes. A size up to 10% short of the box counts (`ART_SLACK`). A photo that isn't square fills
  its box by its short side, so its long side asks more.
- **Never blank, never a flash.** The first `src` is the size a numeric CSS size needs, else a size of the same
  picture already decoded (so Now Playing, a hero or the cover viewer start with what the player bar has), else none
  until measured. A different size is decoded off the page and swapped in once ready, only when the cover is near the
  viewport, so nothing large decodes off screen; a big picture a small place borrowed is swapped for its own size the
  same way. `App.patch` leaves a cover `<img>` with the same `data-pic` alone: its `src` is the fitter's.
- **No more than the source has.** An answer smaller than asked (a 600 px original asked at 1024) marks the picture's
  full size, and no larger size of it is asked again.
- **The screen sets the hub's order** (ADR-0033). Two IntersectionObservers per scroll container (the page, or the box
  a list scrolls in) keep which pictures are visible and which are within a screen of view: every cover `<img>`, every
  cover waiting for its picture (`data-wait-pic`) and every artist drawn without a photo (`data-want-photo`).
  `PUT /api/pictures/screen` reports them top to bottom, at the size drawn (an artist without a photo at 0, so the hub
  looks for one), at most four times a second and once more when it settles; an unchanged screen isn't sent again, a
  hidden page sends nothing, and a reconnect sends it again. The hub fetches what plays, then what is visible, then
  what is near, newest screen first, and drops work for what left every screen: a long list scrolled fast fills the
  screen it stops on first.
- **Asked once, then told.** The hub fetches what screens show before they ask (library.md, discovery.md#pictures). A cover it
  answers 404 for ("not yet", or "none") makes every cover of that key wait without a src, the recipe showing, until
  `art.ready` names the key; then each fits again and asks once (a larger size that 404'd keeps the smaller picture
  meanwhile). Nothing retries on a timer, so a page asks for a picture once however many rows show it. A wait ends by
  itself after 2 minutes (a "none" is answered from the browser's cache then), and a reconnect ends them all. An
  artist's photo the hub vets after the page drew the artist without one comes in `art.ready`'s `images`: the entity
  takes it, and every cover drawn for the artist without a photo gets its `<img>`, with no page read again.
- **Heroes first.** `ui.cover(…, { hero: true })` (also `ui.mosaic`/`ui.playlistArt`'s last argument): the player
  bar's and the phone mini player's cover, Now Playing's, album, artist, tag and playlist headers and the cover viewer
  load at once with `fetchpriority="high"`; every other cover is `loading="lazy"`.
- **Blurred covers** (`ui.cover(…, { blurred: true })`, the artist hero's glow) take 96 px and aren't fitted.
- **Fixed sizes** where lull hands a picture outside the page: Media Session 96/256/512 (OS overlays; the desktop app
  passes the 512 to its mini player and SMTC), the Android player's and downloads' 512, Classic mode's 256 for the OS
  overlay, and Discord's 600 px public picture (discovery.md, Share links).

## Rules for screens

- No `PROTO`, no state switcher, no `ctx.state`, no simulated delays, no sample data.
- Render from `M.get` records and the store; refresh happens by itself when reads settle.
- Every control does something real; an endpoint that is down shows its error, not a pretend result.
- Cover art: `App.ui.cover` puts `<img class="cover__img">` (from `/api/art`, album keys only) inside the cover span,
  at the size it is drawn (Covers at the size they are drawn, below);
  `.has-art` is added once it loads and hides the recipe's `::before/::after`. Designs that draw on those
  pseudo-elements handle `.has-art` (starlight keeps its frame line, petal never mirrors real art). A picture this page
  has already loaded renders with `.has-art` from the start, and the player warms the next song's cover
  (`ui.warmArt`), so art never fades in twice.
- **Re-renders patch, they don't rebuild.** `App.refresh()` of the page already showing, the player bar, the side
  panel and the screens' own repaints go through `App.patch(el, markup)`: only what differs changes, children matched
  by `id`, `data-key`, `data-track-id` or `data-id`, else by position. Attributes are compared by name
  (`getAttributeNames`): reading `el.attributes` makes an Attr node for each, which the element keeps for life (eleven
  nodes on every row of a long list). An `<img>` with the same `src` is never touched,
  so nothing that stayed the same reloads or flashes. A new route still renders fresh. Because a refresh keeps the
  elements, a route's `mount` runs again on them: listen on elements with `ctx.on(target, type, fn)` (removed before
  the next render) and keep one-off effects (entrance animations, scrolling to a section) behind `!ctx.refresh`.
- **Segmented controls** (`.seg`): render `--n`, `--i` and the selected button's `aria-selected` / `aria-checked`.
  app.js places `.seg__ind` on that button by measuring it (a long label widens its column), whenever a `.seg` appears,
  its selection changes or a button resizes. It slides only when a patch keeps the `.seg`, so re-render it with
  `App.patch`, not `innerHTML`.
- **Back returns where the page was.** The router gives each history entry an id (`history.state.lullEntry`) and,
  when its page is left, records the scroll position, the element a third down the view with its offset (rows off
  screen have an estimated height, so the element places the page truly; the next frames correct it once the rows
  around it are drawn) and the focused control (by `data-track-id`, `data-place`, `data-album-id`, `data-fk` or `id`).
  Showing that entry again restores them and holds them for 4 s while a read adds content, until the listener
  scrolls, types or clicks. A new entry starts at the top. Screens get it for free: keep "Show all" and paging state
  outside the render (as library.js `limits` does), give list items one of those attributes, and rewrite the URL in
  place with `history.replaceState(history.state, ...)` so the entry keeps its id.
- **Live values write only on change** (`setText`, `setAttr`, `setVar` in app.js section 9): the clock ticks four
  times a second, and an identical write still costs a style and layout pass.
