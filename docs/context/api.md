# API contract

Between `web/` and the server, and between crates (events). It's derived from `design/DESIGN.md` §8 and the shapes in `design/prototype/mock-data.js`, which is the canonical example of every object the UI reads.
- Backend owners keep their own section true and edit only that section.
- The web session builds against this page.
- A shape change is made here in the same commit as the code.

## Conventions

- **Paths and auth.** JSON, everything under `/api` (Subsonic under `/rest`), same origin as the UI. Every call needs a paired device's token (ADR-0002, see Hub) except the few the Hub section marks open.
- **Field names.** camelCase in JSON. Ids are strings:
  - `t-…` track, `al-…` album, `ar-…` artist, from `lull_core::ids`: one id per song, whatever its source;
  - `pl-…` playlist, `job-…` job;
  - a tag's id is its name.
- **Errors** are `{ "error": kind, "message": text }`, where `message` is a sentence the UI can show as is:
  - 400 `bad_request`;
  - 401 `unauthorized`;
  - 404 `not_found`;
  - 409 `conflict`;
  - 502 `upstream` (a dependency answered badly);
  - 503 `unavailable` (a dependency is down or not configured);
  - 500 `internal`.
- **Entities.** Any response that names tracks, albums or artists also carries them as `"entities": { tracks: {id: Track}, albums: {id: Album}, artists: {id: Artist} }` (`lull_core::model::Entities`), so the UI's lookup cache (`M.track(id)`...) can resolve every id it's given.
  - A **Track** is the mock track without `sources`, `plays` and `liked`:
    - from the mock: `id, title, artistId, artist, albumId, album, year, no, disc, duration, tags, saved`;
    - file facts flattened in: `format, codec, lossless, khz, bits, kbps, sizeBytes`.
  - An **Album** or **Artist** is the mock shape (see `lull_core::model`).
- **Per-user state** comes from collection endpoints and is never inlined in entities: liked, followed, plays.
- **Lists** take `?limit=&offset=` and return `{ items, total, entities }`.
- **Time.** Durations are in seconds. Timestamps are unix seconds (`at`), and the UI formats them.

## Hub (`crates/server`, `lull_core::{devices, settings}`)

ADR-0002. A device token travels as the HttpOnly cookie `lull_device` (browsers, set by the hub) or `Authorization: Bearer <token>` (the app). Open without a token: `/health`, `GET /api/setup`, `POST /api/setup/claim`, `POST /api/pair`, the static UI.

| Method + path | Returns |
|---|---|
| `GET /health` (open) | `{ version, services: { soulseek, library, lastfm, cache: { state: ok\|warn\|down\|off, detail, extra } } }`. Always 200 while serving |
| `GET /api/ping` | 204, or 401 without a valid device token (the UI then shows pairing) |
| `GET /api/events` | Server-sent events, one per bus event: `event: <topic>`, `data: <json>`. `resync` means the client missed events and refetches what it shows. The connection also marks its device online (lull Connect) |
| `GET /api/setup` (open) | `{ claimed: bool, steps: { library: bool, soulseek: bool, lastfm: bool } }`: what first run still needs |
| `POST /api/setup/claim { code, name }` (open) | The first device: 200 `{ device }` + cookie when `code` is the setup code and no device is paired yet, else 401 |
| `POST /api/pair/codes` | `{ code, url, expiresAt }`: a single-use pair code (10 min); `url` is `<hub url>/#/pair/<code>` for the QR |
| `POST /api/pair { code, name, kind: browser\|app }` (open) | 200 `{ device, token? }`: browsers get the cookie, apps get `token` in the body; 401 for a wrong or used code |
| `GET /api/devices` | `{ items: [Device + { online, state? }], me }`. Device `{ id, name, kind: browser\|app\|subsonic, createdAt, lastSeen }` |
| `PATCH /api/devices/{id} { name }` / `DELETE /api/devices/{id}` | 204: rename / unpair (its token stops working at once) |
| `POST /api/devices/subsonic { name }` | `{ device, username: "lull", password }`: an app password for a Subsonic client, shown once |
| `POST /api/devices/me/state { trackId, position, playing, volume, queue: [trackId], queueIndex }` | 204; republished as `device.state` |
| `POST /api/devices/{id}/command { action: play\|pause\|seek\|next\|prev\|volume\|takeover, value? }` | 204; forwarded to that device as `device.command`. `takeover`: the target sends its state to the caller and pauses |
| `GET /api/config` | `{ items: [{ key, value, source: env\|stored\|default, secret: bool, restart: bool }] }`. Secrets report only whether they're set |
| `PATCH /api/config { LULL_X: value\|null }` | 200 `{ items }` after validating with `Config::from_vars`; 400 names the bad key. Keys with `restart: true` restart the hub's crates (`hub.restart`) |

## Events

Topic → payload. Published on the bus and streamed to the browser. Owners add their topics here.

| Topic | Publisher | Payload |
|---|---|---|
| `service` | any (via `ctx.status`) | `{ name, service: { state, detail, extra } }` |
| `search.results` | soulseek | `{ searchId, query, files, peers, done, results?, entities?, error? }`. While the search runs only the counters move (`done: false`); the last event (`done: true`) carries `results` and `entities`, or `error`. Rows in the Soulseek section |
| `transfer` | soulseek | `{ jobId, trackId, peer, state: queued\|transferring\|stalled\|done\|failed\|cancelled, queuePos, received, size, speed, error }`. Bytes, bytes/s; `peer` changes on fallback. Absent values are `null` |
| `stream` | soulseek | `{ trackId, jobId, status: finding\|queued\|buffering\|streaming\|stalled\|fallback\|failed, peer, format, peersFound, queuePos, received, size, attempt, failedPeers: [{ peer, at }], error }` drives the player's streaming states, while the track streams. `at` is the percent reached. `attempt` goes up on each fallback: reopen the stream URL (the old response ends) and seek back by time |
| `save` | library | `{ jobId, trackId, step: 1-5, status: running\|done\|failed\|pending, pct, error, kind: transfer\|match\|write\|null, nocover }` (steps: transfer, match, cover, tag+write, rescan). `kind` names the recovery for a failure; `pct` moves during step 1 only |
| `job` | any | `{ jobId, kind: transfer\|save\|import\|rescan, started, ... }`: one Activity row, the same shape `GET /api/jobs` lists. Library's rows: save `{ trackId, step, stepState, pct, error, failKind, nocover }`, rescan `{ state: running\|done\|failed, pct, detail }`. Other crates publish their own rows here to appear in `/api/jobs` |
| `play.started` | collection | `{ playId, trackId, at, duration, artist, title, album }`. lastfm now-playing, taste. The `plays` row is written first |
| `play.ended` | collection | `{ playId, trackId, at, playedS, completed, skipped, duration, artist, title, album }`, exactly once per play. `at` is the play's **start** (Last.fm's timestamp). Scrobbling, taste |
| `like` | collection | `{ trackId, liked }`. Taste, other devices |
| `follow` | collection | `{ kind: artist\|tag, id, followed }`. Discovery, other devices |
| `settings` | collection | The stored settings (the editable part of `GET /api/settings`), at start-up and on every change. Collection starts after every other crate, so a receiver made in `start()` gets the start-up copy |
| `scrobble` | lastfm | `{ playId (string), status: pending\|done\|short }`: queued, sent to Last.fm, or short of Last.fm's rule (History's scrobble column). Nothing while Last.fm is off, disconnected or scrobbling is switched off |
| `lastfm.history` | lastfm | `{ user, artists: [{ name, plays }] }`: the listener's Last.fm top artists, a taste signal for discovery; empty when off or disconnected |
| `hub.restart` | any | `{ reason }`: asks the hub to restart its crates with the current settings (a setting changed). The hub publishes `hub.restarting` and the UI reconnects |
| `hub.restarting` | hub | `{ reason }`: the hub's crates are restarting; SSE drops for about a second |
| `device.state` | hub | `{ deviceId, trackId, position, playing, volume, queue, queueIndex, at }` |
| `device.command` | hub | `{ deviceId, action, value, from }`: only the device named acts on it |
| `device.online` | hub | `{ deviceId, online }` |
| `theme.changed` | themes | `{ id }`: a stored theme's files changed; devices using it reload its CSS |
| `library.source` | library | `{ mode: folder\|jellyfin, state: switching\|done\|failed, detail }` |

## Soulseek (`crates/soulseek`)

| Method + path | Returns |
|---|---|
| `GET /api/search?q=` | `{ searchId, library: [trackId], libraryError?, soulseek, entities }`. Library hits come back at once, via the `Library` seam (`libraryError` says why there are none when it failed). Soulseek then answers as `search.results` events with that `searchId`: live `files` / `peers` counters for up to 15 s, then the results (slskd reveals answers only when a search ends). The same words again within 10 minutes reuse the search, and `soulseek` is its latest event (else `null`). Queries under 3 characters search the library only |
| `search.results` rows | `{ kind: "track", trackId, sources, best: Source, dup? }`: one song merged across peers; `sources` = how many peers; `dup` is `note` (already saved) or `better` (this file beats the saved one). `{ kind: "album", albumId, trackIds, folders: [{ peer, path, format, tracks, sizeBytes, speed, freeSlot, queue, score, cover }] }`: a peer folder holding 3+ songs, the same album on several peers listed best first. Songs first by peers sharing them, then by score; at most 200 songs and 40 albums |
| `GET /api/tracks/{id}/sources` | `{ sources: [Source], done }`, best first. A Source is `{ peer, path, format, sizeBytes, speed (MB/s), freeSlot, queue, score 0-100, factors: { format, speed, slot, queue, stalled }, preferred, stalledBefore }`: the factors add up to the score, `stalled` is 0 or a negative penalty. When nothing is known about the song yet this starts a search for it and answers `done: false`: ask again every couple of seconds until `done`. 404 for a track id lull never handed out |
| `GET /api/stream/{trackId}[?peer=&path=]` | Audio with Range support (200 whole, 206 one range). Starts the transfer if needed (searching for the song first when no source is known) and serves bytes as they arrive; `peer` + `path` (a Source's) plays that source. The answer waits for the first bytes; a range beyond what has arrived waits 8 s, then 416. A stall falls back to the next peer and ends the response (`stream` events). 502 when no peer delivers. Saved tracks are streamed by library, not here |
| `DELETE /api/stream/{trackId}` | 204. Stop: cancels the transfer (skip / next) unless a save needs it; slskd keeps the partial file, so playing it again resumes |
| `GET /api/soulseek/diagnostics` | What slskd itself sees, since its UI and logs are out of reach in the deploy kit: `{ slskd: { version, server, distributedNetwork, connectionWatchdog, pendingReconnect }, searches: [{ query, state, peers, files, at }], log: [{ timestamp, level, message }] }`. `searches` is how the latest network searches ended, newest last; `log` is slskd's latest lines (empty without an administrator key) |
| `GET /api/transfers` · `DELETE /api/transfers/{jobId}` · `POST /api/transfers/{jobId}/retry` | `{ items, total }`: running then recently ended transfers, each the `job` row `{ jobId, kind: "transfer", trackId, peer, speed (MB/s), pct, state, queuePos, started, error }` (also published as `job` events). Cancel: 204. Retry (a failed or cancelled one): `{ jobId }` of the new transfer |

## Library (`crates/library`)

All of these answer 503 `unavailable` when Jellyfin isn't configured. Lists and lookups come from lull's copy of the library, so they keep working while Jellyfin is down.

| Method + path | Returns |
|---|---|
| `GET /api/tracks/{id}` | `{ track, entities }`. A saved track (also under the id it was saved as, see save-flow.md#ids), else the catalog's with `saved: false`; 404 if neither knows it |
| `GET /api/library/songs?sort=album\|title\|artist\|recent` | `{ items: [trackId], total, entities: { tracks }, stale }` |
| `GET /api/library/albums?sort=recent\|title\|artist\|year` | `{ items: [albumId], total, entities: { albums }, stale }`. Library albums are `saved: "all"` |
| `GET /api/library/artists?sort=name` | `{ items: [artistId], total, entities: { artists }, stale }` |
| `GET /api/library/genres?sort=songs\|name` | `{ items: [{ id, name, artists, songs, trackIds }], total, entities: { tracks }, stale }` (`trackIds`: a few, for the mosaic) |
| ↳ every list | Also `filter=` (words in title, artist or album), `limit=` (default 200, max 5000), `offset=`. An unknown `sort` is 400. `stale: true` while Jellyfin is down and the list is the last one read |
| `GET /api/library/stream/{trackId}` | The saved file from Jellyfin. `Range` is passed through (206 + `Content-Range`), the body streams through without buffering. 404 if not saved |
| `GET /api/art/{albumId}?size=96\|256\|512` | The cover image (`Cache-Control: private, max-age=86400`). Jellyfin's for saved albums, else the Cover Art Archive's (found through the catalog and MusicBrainz for albums that aren't saved). Cached on disk; 404 when there is none |
| `POST /api/library/duplicates { ids, source?: { trackId: { peer, path, size } } }` | `{ items: [{ trackId, existing: Track, better }] }` for the ids that are saved already. `better`: the offered file is clearly better (lossless over lossy, or a fifth more bitrate) |
| `POST /api/saves { ids, source?: { trackId: { peer, path, size } }, duplicate?: { trackId: separate\|replace } }` | `{ jobs: [jobId], skipped: [trackId] }`. One job per id; progress as `save` events. `source` is a row of `/api/tracks/{id}/sources` (`sizeBytes` is accepted for `size`); without one, Soulseek's `source_for` picks (the peer it streamed from, else the best known), and with none known the job fails at step 1 (`kind: transfer`). An id already saved is skipped unless `duplicate` says what to do (the duplicate dialog's answer; "keep" = don't send it). An id already saving returns its running job |
| `GET /api/saves/{jobId}/releases` | `{ items: [{ mbid, recording, releaseGroup, title, artist, kind, date, country, tracks, score }], track }`: the release picker after a `match` failure, best first (`kind`: "album, CD"); `track` is what lull knows of the file |
| `POST /api/saves/{jobId}/release { mbid } \| { keepTags: true }` | `{ jobId }`. Continues the save with that release, or with the file's own tags. 409 if the job isn't waiting for a release |
| `POST /api/saves/{jobId}/retry [{ source }]` | `{ jobId, step, status }`. Restarts a failed save from its failed step (a new `source` for a transfer failure), or asks Jellyfin again for a pending one. 409 otherwise |
| `GET /api/jobs` · `DELETE /api/jobs` | `{ items: [row] }`, newest first: library's save and rescan jobs plus the latest `job` event of every other job. `DELETE` clears finished rows (failed saves stay) → 204 |
| `POST /api/library/rescan` | 202 `{ jobId }`: runs Jellyfin's library scan; the rescan job follows its progress and ends with "Jellyfin found N new songs" |

**lull 2 additions** (ADR-0003; the owner turns each row into fact and drops this note):

| Method + path | Returns |
|---|---|
| `GET /api/library/source` | `{ mode: folder\|jellyfin, musicDir, jellyfin: { url, user, connected } \| null, files?, lastScan? }` |
| `PUT /api/library/source { mode, jellyfin?: { url } }` | 202. Stores `LULL_LIBRARY` (and the Jellyfin URL) through `core::settings`, reconciles by path, asks for `hub.restart`; progress as `library.source` events |
| `GET /api/jellyfin/discover` | `{ items: [{ name, url, version }] }`: Jellyfin servers answering on the local network (UDP 7359) |
| `POST /api/jellyfin/quickconnect { url }` | `{ code, secret }`: show `code`; the listener approves it in Jellyfin � Quick Connect |
| `GET /api/jellyfin/quickconnect/{secret}` | `{ state: waiting\|approved\|expired }`. On `approved` the token is stored as `LULL_JELLYFIN_API_KEY` and the hub restarts into Jellyfin mode |
| `POST /api/jellyfin/signin { url, username, password }` | 204: the fallback when Quick Connect is off on that server; the password is used once and never stored |

In folder mode every list above works from the folder index, `stream` serves the file, and `rescan` rescans the folder. Saved-only routes answer 503 only when neither source is usable.

## Discovery (`crates/discovery`, `crates/lastfm`, `crates/lyrics`)

Sources and limits: [discovery.md](discovery.md); taste: [taste.md](taste.md); Last.fm: [scrobbling.md](scrobbling.md).

| Method + path | Returns |
|---|---|
| `GET /api/feed` | `{ updated, summary, newReleases: [albumId], picks: [{ trackId, reasons: [{ kind: like\|tag\|listeners\|fresh, name, ref? }], via }], radioStarters: [{ id, title, sub, seed: { kind: track\|artist\|tag, id } }], jumpBackIn: [{ kind: album\|artist, id }], followedTags, coldStartTags, errors?: { newReleases\|picks: message }, entities }`. `ref` is an artist id or a tag (absent for "listeners like you"); `via` names the source. The first call may take tens of seconds (the UI's loading state); afterwards it is instant |
| `GET /api/radio?kind=track\|artist\|tag&id=&steer=0..1&exclude=id,id` | `{ items: [trackId], entities }`: 12 tracks to top up the queue. `steer` defaults to 0.4; 404 when the seed is unknown |
| `GET /api/albums/{id}` | `{ album: Album (trackIds, duration, saved), entities }` (tracks with saved state) |
| `GET /api/artists/{id}` | `{ artist: Artist (tags, listeners, bio), topTracks: [trackId], albums: [albumId] (newest first), similar: [artistId], errors?: { section: message }, entities }` |
| `GET /api/tags/{id}` | `{ tag: { id, name, desc, related: [tag] }, artists: [artistId], newReleases: [albumId], errors?, entities }`. `id` is the tag name (URL-encoded); `newReleases` are the last 90 days |
| `POST /api/feedback { kind: more-like\|less-like\|hide-artist, id }` | 204. `id` is a track or artist id; hiding applies to the feed at once |
| `GET /api/taste` | `{ artists: [[artistId, weight]], tags: [[tag, weight]], entities }`: the Stats taste block, weights 0..1 |
| `POST /api/taste/reset` | 204. Forgets learned signals; follows and hidden artists stay |
| `GET /api/lastfm` | `{ configured, connected, user, scrobbling, useHistory, queued, error? }` |
| `PATCH /api/lastfm { scrobbling?, useHistory? }` | The same object. The two Settings > Last.fm switches live here (409 when not connected) |
| `GET /api/lastfm/connect` | A browser navigation: 303 to Last.fm's sign-in; Last.fm returns the browser to `/api/lastfm/callback`, which lands on `/#/settings/lastfm` |
| `DELETE /api/lastfm` | 204. Disconnects and drops queued scrobbles |
| `POST /api/lastfm/retry` | 204. Sends every waiting scrobble now |
| `GET /api/lyrics/{trackId}` | `{ kind: synced\|plain\|instrumental\|none, lines: [{ t, text }], plain?, source: "LRCLIB" }` (`t` in seconds), 404 for an unknown track, or 503 while LRCLIB is failing or rate-limiting |

## Collection (`crates/collection`, `crates/skins`)

Details and rules: [collection.md](collection.md), [skins.md](skins.md).

| Method + path | Returns |
|---|---|
| `GET /api/likes?limit=&offset=` | `{ items: [trackId], total, entities }`, newest first |
| `PUT /api/likes/{trackId}` · `DELETE /api/likes/{trackId}` | 204 (idempotent). 404 for a track the catalog doesn't know. Publishes `like` |
| `GET /api/follows` | `{ artists: [id], tags: [id], entities }`, newest first. Artist entities carry only `name` |
| `PUT /api/follows/artist/{id} { name }` · `PUT /api/follows/tag/{name}` · `DELETE /api/follows/{artist\|tag}/{id}` | 204. The artist name must give back the id (`ArtistId::of(name)`), else 400. Publishes `follow` |
| `POST /api/plays { trackId, duration?, recommended? }` | `{ playId }` (a string). `duration` defaults to the catalog's; `recommended`: started from a feed pick or radio (Stats hit rate). Publishes `play.started` |
| `PATCH /api/plays/{playId} { playedS, completed, skipped }` | `{ playId }`. Once per play: a second end is 409. Publishes `play.ended`. Use `fetch(..., { keepalive: true })` so a closing tab still ends its play |
| `GET /api/history?before=&limit=&tz=` | `{ days: [{ day: "YYYY-MM-DD", items: [{ playId, trackId, at, playedS, completed, skipped, scrobble: done\|pending\|short\|null }] }], next, entities }`, newest first. `tz` = minutes east of UTC (`-new Date().getTimezoneOffset()`), so days split at the listener's midnight. `next` is the `before` of the next page; a day can continue there (merge equal `day`s). `scrobble` comes from lastfm's `scrobble` event; null = never reported |
| `GET /api/playlists?limit=&offset=` | `{ items: [Playlist], total, entities }`, most recently changed first. Playlist = `{ id: "pl-…", name, desc, created, updated, items: [{ trackId, added }] }` (unix times) |
| `POST /api/playlists { name, desc?, trackIds?, imported? }` | 201 + Playlist + entities. Every id must be in the catalog (else 400). `imported: { file, library, willSearch, notFound }` (the preview's counts) publishes an import `job` for Activity |
| `GET /api/playlists/{id}` · `PATCH { name?, desc? }` · `DELETE` | Playlist + entities · Playlist + entities · 204 |
| `PUT /api/playlists/{id}/tracks [trackId]` | The whole new order (adds, removes, moves). Songs that stay keep their `added`. Playlist + entities |
| `POST /api/playlists/import` (body: the .m3u/.m3u8 bytes) | `{ name, lines: [{ n, location, artist, title, status: library\|will-search\|not-found, trackId }], counts: { library, willSearch, notFound }, libraryChecked, entities }`. Creates nothing; the UI then `POST`s the playlist with the ids it keeps. `libraryChecked: false` = the library couldn't be asked, so saved songs may show as `will-search` |
| `GET /api/playlists/{id}.m3u8` | M3U8 attachment: `#EXTINF` + `#EXTALB` + an absolute stream link per song (`/api/library/stream/…` saved, `/api/stream/…` otherwise), based on `LULL_PUBLIC_URL`, else the request's host. Importing it back gives the same ids |
| `GET /api/stats?period=7d\|30d\|12m\|all&tz=` | `MOCK.stats.byPeriod[p]`: `{ minutes, plays, artists, saves, skipRate, newArtists, hitRate: { recommended, liked, saved }, topArtists: [[id, n]], topTracks: [[id, n]], topTags: [[tag, percent]], clock: [24 plays per local hour] }` + entities. The taste block is discovery's `GET /api/taste` |
| `GET /api/settings` · `PATCH /api/settings` (partial object) | `MOCK.settings` minus appearance (per device) and Last.fm switches (`/api/lastfm`). Stored: `playback { gapless, crossfade, normalisation, loudnessTarget, volumeStep, radioWhenQueueEnds }`, `sources { formatPreset, minBitrate, maxPeerQueue, stallTimeout, autoPick }`, `library { duplicatePolicy }`, `sharing { uploadSlots, uploadLimitKBs }`, `discovery { adventurousness, on: { listenbrainz, musicbrainz, lastfm } }`. From config, read-only: `{ value, source: "config" }`, secrets `{ value: null, source: "config", secret: true, set }`. `cache.usedGB` is live (null until soulseek reports). A `PATCH` with an unknown key, a config key or a bad value is refused whole (400). Publishes `settings` |
| `GET /api/skins?q=&offset=&nsfw=` | `{ items: [{ md5, name, file, screenshotUrl, museumUrl, nsfw, favourite, downloaded }], more }`, 30 a page. Empty `q` = the museum's own order. NSFW hidden unless `nsfw=true` (the per-device setting). Screenshots load straight from the museum's CDN |
| `GET /api/skins/{md5}.wsz` | The skin file: downloaded once, validated (a zip with `main.bmp`), cached, same origin, `immutable` |
| `GET /api/skins/favourites` · `PUT/DELETE /api/skins/favourites/{md5}` | `{ items: [skin] }`, newest first · 204 (404 for a skin the museum doesn't have) |
| `POST /api/sync { ops: [{ id, at, op, ... }] }` | `{ applied: [id], rejected: [{ id, message }], playlists: { tempId: playlistId } }`. Ops apply in order, each in its own transaction, and are idempotent by `id` (a client-made uuid, 1-100 chars): a resent op is acknowledged, not applied again, and a resent `playlist.create` still reports its playlist. `at` (unix seconds) dates a like, follow or playlist change; a play uses its `startedAt`. A fault in one op (unknown track, bad shape, unknown `tempId`) rejects only that op; a missing `id` or more than 1000 ops is a 400 for the whole request. `op`: `play { trackId, startedAt, playedS, completed, skipped }`, `like { trackId, liked }`, `follow { kind, id, name?, followed }` (`name` required to follow an artist), `playlist.create { tempId, name, desc?, trackIds? }`, `playlist.edit { playlistId \| tempId, name?, desc?, trackIds? }`, `playlist.delete { playlistId \| tempId }`. A `tempId` resolves to the playlist an earlier `playlist.create` in any batch made. Publishes the same events as the matching routes (a synced play publishes `play.ended` only) |
| `POST /api/import/exportify` (body: Exportify CSV, up to 16 MB) | The M3U import preview shape (`name: null`, `lines`, `counts`, `libraryChecked`, `entities`; `location` is the Spotify URI); the UI then `POST`s the playlist with the ids it keeps. 400 when the file lacks the `Track Name` and `Artist Name(s)` columns |

**lull 2 additions** (ADR-0005; the owner turns each row into fact and drops this note):

| Method + path | Returns |
|---|---|
| `POST /api/import/history { source: lastfm, user }` � `POST /api/import/history` (body: a ListenBrainz export JSON) | 202 `{ jobId }`: plays are added as imported (they never scrobble), progress as `job` rows |

## Themes (`crates/themes`)

ADR-0004; format and hooks: [themes.md](../themes.md) *(owner writes)*.

| Method + path | Returns |
|---|---|
| `GET /api/themes` | `{ items: [{ id, name, author, api, extends, shell, swatches, modes, builtIn }] }`: the five designs plus the listener's themes |
| `GET /api/themes/{id}/{path}` | A file of a stored theme (`theme.css`, `slots/*.html`, `assets/*`), ETag'd |
| `PUT /api/themes/{id}` (body: a `.lulltheme` zip) � `PUT /api/themes/{id}/{path}` (one file, from Theme Studio) | 200 manifest; 400 names what's wrong (missing `theme.json`, a script, an outside URL) |
| `DELETE /api/themes/{id}` | 204 |
| `GET /api/themes/{id}.lulltheme` | The package as a zip, for sharing |

## Subsonic (`crates/subsonic`)

OpenSubsonic (API 1.16.1) at `/rest/<endpoint>`, `.view` suffix accepted, by GET or form POST. XML by default, JSON with `f=json`. Auth on every call but `getOpenSubsonicExtensions`: `u=lull` with `p=` (plain or `enc:`) or `t=md5(password + salt)`+`s=`, against an app password from `POST /api/devices/subsonic`; or `apiKey=` with any paired device's token. Ids are lull's (`ar-`, `al-`, `t-`, `pl-`); cover art ids are album ids. Rules and spec gaps: [subsonic.md](subsonic.md).

| Group | Endpoints |
|---|---|
| System | `ping`, `getLicense`, `getOpenSubsonicExtensions` (`apiKeyAuthentication`, `formPost`), `getMusicFolders` |
| Browse | `getIndexes`, `getArtists`, `getArtist`, `getAlbum`, `getSong`, `getAlbumList2`, `getRandomSongs`, `getGenres` |
| Search, media | `search3` (empty query lists all), `stream` and `download` (the saved file, Range honoured), `getCoverArt` (`size`) |
| Likes, plays | `star`, `unstar` (songs only), `getStarred2`, `scrobble` (a submission becomes `Collection::record_play`) |
| Playlists | `getPlaylists`, `getPlaylist`, `createPlaylist`, `updatePlaylist`, `deletePlaylist` (saved songs only; indexes count those) |
