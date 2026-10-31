# API contract

Between `web/` and the server, and between crates (events). It's derived from `design/DESIGN.md` §8 and the shapes in `design/prototype/mock-data.js`, which is the canonical example of every object the UI reads.
- Backend owners keep their own section true and edit only that section.
- The web session builds against this page.
- A shape change is made here in the same commit as the code.

## Conventions

- **Paths and auth.** JSON over HTTPS, everything under `/api`, same origin as the UI. With `LULL_ACCESS_TOKEN` set, every `/api` call needs the token (see Server).
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

## Server (integrator)

| Method + path | Returns |
|---|---|
| `GET /health` (no token needed) | `{ version, services: { soulseek, jellyfin, lastfm, cache: { state: ok\|warn\|down\|off, detail, extra } } }`. Always 200 while serving. `extra`: cache `{ usedBytes, capBytes }`, lastfm `{ queued }` |
| `GET /api/ping` | 204, or 401 when the token is missing or wrong (the UI's token gate) |
| `POST /api/session { token }` | 204 + HttpOnly cookie `lull_token`, or 401 |
| `GET /api/events` | Server-sent events, one per bus event: `event: <topic>`, `data: <json>`. `resync` means the client missed events and refetches what it shows |

## Events

Topic → payload. Published on the bus and streamed to the browser. Owners add their topics here.

| Topic | Publisher | Payload |
|---|---|---|
| `service` | any (via `ctx.status`) | `{ name, service: { state, detail, extra } }` |
| `search.results` | soulseek | `{ searchId, results: [...], peers, done, entities }` (Soulseek section) |
| `transfer` | soulseek | `{ jobId, trackId, peer, state: queued\|transferring\|stalled\|done\|failed\|cancelled, queuePos?, received, size, speed, error? }` |
| `stream` | soulseek | `{ trackId, status: finding\|queued\|buffering\|streaming\|stalled\|fallback\|failed, peer?, peersFound?, queuePos?, received, size, failedPeers? }` drives the player's streaming states |
| `save` | library | `{ jobId, trackId, step: 1-5, status: running\|done\|failed\|pending, pct, error, kind: transfer\|match\|write\|null, nocover }` (steps: transfer, match, cover, tag+write, rescan). `kind` names the recovery for a failure; `pct` moves during step 1 only |
| `job` | any | `{ jobId, kind: transfer\|save\|import\|rescan, started, ... }`: one Activity row, the same shape `GET /api/jobs` lists. Library's rows: save `{ trackId, step, stepState, pct, error, failKind, nocover }`, rescan `{ state: running\|done\|failed, pct, detail }`. Other crates publish their own rows here to appear in `/api/jobs` |
| `play.started` | collection | `{ playId, trackId, at, duration, artist, title, album }`. lastfm now-playing, taste. The `plays` row is written first |
| `play.ended` | collection | `{ playId, trackId, at, playedS, completed, skipped, duration, artist, title, album }`, exactly once per play. `at` is the play's **start** (Last.fm's timestamp). Scrobbling, taste |
| `like` | collection | `{ trackId, liked }`. Taste, other devices |
| `follow` | collection | `{ kind: artist\|tag, id, followed }`. Discovery, other devices |
| `settings` | collection | The stored settings (the editable part of `GET /api/settings`), at start-up and on every change. Collection starts after every other crate, so a receiver made in `start()` gets the start-up copy |

## Soulseek (`crates/soulseek`)

| Method + path | Returns |
|---|---|
| `GET /api/search?q=` | `{ searchId, library: [trackId], entities }`. Library hits come back at once, via the `Library` seam. Soulseek results then stream as `search.results` events for about 10-20 s: songs merged across peers (one row, `sources` count) and folders grouped as albums, with `{ results, peers, done }` counters |
| `GET /api/tracks/{id}/sources` | `{ sources: [{ peer, path, format, sizeBytes, speed, freeSlot, queue, score, factors: { format, speed, slot, queue }, preferred, stalledBefore }], done }`, ranked, best first. Same as `MOCK.tracks[].sources` |
| `GET /api/stream/{trackId}[?peer=&path=]` | Audio with Range support. Starts the transfer if needed and serves bytes as they arrive. A range beyond what has arrived waits briefly, then 416. A stall falls back to the next peer (`stream` events). Saved tracks are streamed by library, not here |
| `DELETE /api/stream/{trackId}` | Stop: cancels the transfer (skip / next) |
| `GET /api/transfers` · `DELETE /api/transfers/{jobId}` · `POST /api/transfers/{jobId}/retry` | Transfer jobs for the Activity panel |

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

## Discovery (`crates/discovery`, `crates/lastfm`, `crates/lyrics`)

| Method + path | Returns |
|---|---|
| `GET /api/feed` | `MOCK.feed` shape (newReleases, picks with `reasons` + `via`, radioStarters, jumpBackIn, followedTags, coldStartTags, `updated` as unix time) + entities. A shelf that failed carries `errors: { shelf: message }` |
| `GET /api/radio?kind=track\|artist\|tag&id=&steer=0..1&exclude=` | `{ items: [trackId], entities }`: the next tracks to top up the queue |
| `GET /api/albums/{id}` · `GET /api/artists/{id}` · `GET /api/tags/{id}` | Album with tracks; artist with top tracks, discography, similar; tag with description, top artists, new releases, related. Saved state comes from the `Library` seam |
| `POST /api/feedback { kind: more-like\|less-like\|hide-artist, id }` | 204 |
| `POST /api/taste/reset` | 204 |
| `GET /api/lastfm` · `GET /api/lastfm/connect` (redirect to Last.fm) · `GET /api/lastfm/callback` · `DELETE /api/lastfm` · `POST /api/lastfm/retry` | Connection state `{ connected, user, scrobbling, queued }` and the auth flow |
| `GET /api/lyrics/{trackId}` | `{ kind: synced\|plain\|instrumental\|none, lines: [{ t, text }], plain?, source: "LRCLIB" }`, or 503 when rate-limited |

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
| `GET /api/settings` · `PATCH /api/settings` (partial object) | `MOCK.settings` minus appearance (per device) and Last.fm switches (`/api/lastfm`). Stored: `playback { gapless, crossfade, normalisation, loudnessTarget, volumeStep, radioWhenQueueEnds }`, `sources { formatPreset, minBitrate, maxPeerQueue, stallTimeout, autoPick }`, `library { duplicatePolicy }`, `sharing { uploadSlots, uploadLimitKBs }`, `discovery { adventurousness, on: { listenbrainz, musicbrainz, lastfm, discogs } }`. From config, read-only: `{ value, source: "config" }`, secrets `{ value: null, source: "config", secret: true, set }`. `cache.usedGB` is live (null until soulseek reports). A `PATCH` with an unknown key, a config key or a bad value is refused whole (400). Publishes `settings` |
| `GET /api/skins?q=&offset=&nsfw=` | `{ items: [{ md5, name, file, screenshotUrl, museumUrl, nsfw, favourite, downloaded }], more }`, 30 a page. Empty `q` = the museum's own order. NSFW hidden unless `nsfw=true` (the per-device setting). Screenshots load straight from the museum's CDN |
| `GET /api/skins/{md5}.wsz` | The skin file: downloaded once, validated (a zip with `main.bmp`), cached, same origin, `immutable` |
| `GET /api/skins/favourites` · `PUT/DELETE /api/skins/favourites/{md5}` | `{ items: [skin] }`, newest first · 204 (404 for a skin the museum doesn't have) |
