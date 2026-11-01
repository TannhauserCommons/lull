# Web UI: structure, data layer, player engine

`web/` is the shipped UI: classic scripts and plain CSS, no build step, served by the binary (from disk in debug
builds, so a reload picks up edits). It was ported from `design/prototype`; `design/DESIGN.md` stays the reference for
the look, the components and the `App.*` seams (section 7). This page covers what the port added: where data comes
from, how the player plays, and the rules screens follow. The endpoint contract is [api.md](api.md).

## Files and load order

```
index.html   tokens.css base.css components.css screens/*.css desktop/desktop.css designs/<name>/design.css (last)
             designs/designs.js + the pre-paint script (design, theme, motion, text size)
             icons.js data.js app.js screens/{discover,now,library,system,appearance,classic}.js desktop/desktop.js
data.js      window.LullData = App.data: API client, entity cache, per-route reads, the event stream
app.js       window.App: router, render helpers, overlays, save pipeline, player engine, shell, actions,
             keyboard, Media Session, drag and drop, server events + boot
sw.js        service worker (app shell for offline start); manifest.webmanifest
vendor/      Webamp bundle, copied by scripts/vendor-web.sh (`mise run setup`), gitignored
desktop/     owned by the Desktop session (web/desktop/ and the Tauri app)
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
(services, version, then the service worker) and `GET /api/ping`: 204 starts events and loads collections; 401 sets
`App.server.authorized = false` (system.js shows the token gate, `POST /api/session`, then calls `App.start()`); a
network failure still starts, so the installed app opens offline with the cached shell and an "unreachable" banner.

## Player engine (`App.player`)

The state shape and methods are DESIGN.md 7.8; the insides:

- **Two `<audio>` decks**: the playing one and the next. Saved songs play from `/api/library/stream/{id}`; Soulseek
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
- **Gapless**: the next song is preloaded 30 s before the end (90 s when the current transfer is done) and handed
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

## Rules for screens

- No `PROTO`, no state switcher, no `ctx.state`, no simulated delays, no sample data.
- Render from `M.get` records and the store; refresh happens by itself when reads settle.
- Every control does something real; an endpoint that is down shows its error, not a pretend result.
- Cover art: `App.ui.cover` puts `<img class="cover__img">` (from `/api/art`, album keys only) inside the cover span;
  `.has-art` is added once it loads and hides the recipe's `::before/::after`. Designs that draw on those
  pseudo-elements handle `.has-art` (starlight keeps its frame line, petal never mirrors real art).
