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
| `save` | library | `{ jobId, trackId, step: 1-5, status: running\|done\|failed\|pending, pct, error?, kind?, nocover? }` (steps: transfer, match, cover, tag+write, rescan) |
| `job` | any | `{ jobId, kind: transfer\|save\|import\|rescan, ... }` mirrors `MOCK.activity` rows |
| `play.started` | collection | `{ playId, trackId, at, duration }` (internal: lastfm now-playing, taste) |
| `play.ended` | collection | `{ playId, trackId, at, playedS, completed, skipped }` (internal: scrobbling, taste) |

## Soulseek (`crates/soulseek`)

| Method + path | Returns |
|---|---|
| `GET /api/search?q=` | `{ searchId, library: [trackId], entities }`. Library hits come back at once, via the `Library` seam. Soulseek results then stream as `search.results` events for about 10-20 s: songs merged across peers (one row, `sources` count) and folders grouped as albums, with `{ results, peers, done }` counters |
| `GET /api/tracks/{id}/sources` | `{ sources: [{ peer, path, format, sizeBytes, speed, freeSlot, queue, score, factors: { format, speed, slot, queue }, preferred, stalledBefore }], done }`, ranked, best first. Same as `MOCK.tracks[].sources` |
| `GET /api/stream/{trackId}[?peer=&path=]` | Audio with Range support. Starts the transfer if needed and serves bytes as they arrive. A range beyond what has arrived waits briefly, then 416. A stall falls back to the next peer (`stream` events). Saved tracks are streamed by library, not here |
| `DELETE /api/stream/{trackId}` | Stop: cancels the transfer (skip / next) |
| `GET /api/transfers` · `DELETE /api/transfers/{jobId}` · `POST /api/transfers/{jobId}/retry` | Transfer jobs for the Activity panel |

## Library (`crates/library`)

| Method + path | Returns |
|---|---|
| `GET /api/tracks/{id}` | Track (catalog + saved state) + entities |
| `GET /api/library/{albums\|artists\|songs\|genres}?sort=&filter=` | Jellyfin-backed lists. When Jellyfin is down, the last cached read-only list with `stale: true` |
| `GET /api/library/stream/{trackId}` | Saved-track audio, proxied from Jellyfin with Range support |
| `GET /api/art/{albumId}?size=96\|256\|512` | Cover image (Jellyfin, else Cover Art Archive), same origin, cached |
| `POST /api/saves { ids: [trackId], source?: { trackId: { peer, path } } }` | `{ jobs: [jobId] }`. Progress comes as `save` events |
| `POST /api/library/duplicates { ids }` | `{ items: [{ trackId, existing: Track, better: bool }] }`, for the duplicate dialog |
| `GET /api/saves/{jobId}/releases` · `POST /api/saves/{jobId}/release { mbid \| keepTags: true }` · `POST /api/saves/{jobId}/retry` | Release picker and retry |
| `GET /api/jobs` | All jobs, transfer + save + import + rescan (`MOCK.activity` shape) |
| `POST /api/library/rescan` | Asks Jellyfin to rescan |

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

| Method + path | Returns |
|---|---|
| `GET /api/likes` · `PUT /api/likes/{trackId}` · `DELETE /api/likes/{trackId}` | `{ items: [trackId], entities }` |
| `GET /api/follows` · `PUT/DELETE /api/follows/{artist\|tag}/{id}` | `{ artists: [id], tags: [id] }` |
| `POST /api/plays { trackId, duration }` → `{ playId }` · `PATCH /api/plays/{playId} { playedS, completed, skipped }` | Records listening. Publishes `play.started` / `play.ended` |
| `GET /api/history?before=` | `MOCK.history` shape, grouped by day, with scrobble status |
| `GET /api/playlists` · `POST` · `GET/PATCH/DELETE /api/playlists/{id}` · `PUT /api/playlists/{id}/tracks [trackId]` (order) | `MOCK.playlists` shape |
| `POST /api/playlists/import` (M3U body) → per-line match preview: `library \| will-search \| not-found` · `GET /api/playlists/{id}.m3u8` | M3U |
| `GET /api/stats?period=7d\|30d\|12m\|all` | `MOCK.stats.byPeriod[p]` shape + taste summary |
| `GET /api/settings` · `PATCH /api/settings` | `MOCK.settings` shape. Values from server config come back read-only (`{ value, source: "config" }`) and are never revealed if secret |
| `GET /api/skins?q=&offset=` | Museum skins: `{ items: [{ md5, name, screenshotUrl, museumUrl, nsfw }], more }` |
| `GET /api/skins/{md5}.wsz` | The skin file: validated, cached, same origin |
| `GET /api/skins/favourites` · `PUT/DELETE /api/skins/favourites/{md5}` | Favourites |
