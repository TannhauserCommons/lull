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

ADR-0002; how it works: [hub.md](hub.md). A device token travels as the HttpOnly cookie `lull_device` (browsers, set by the hub; the app's webview may carry it too) or `Authorization: Bearer <token>` (the app). Everything under `/api` needs it except `GET /api/setup`, `POST /api/setup/claim` and `POST /api/pair`; everything outside `/api` is open (the static UI, `/health`, and `/rest`, which checks Subsonic app passwords itself). Loopback gets no exemption. A Subsonic app password is refused under `/api`.

**Codes** are typed by people: case, spaces and dashes don't matter. After 10 wrong codes in a minute, every code is refused (401) for the rest of that minute.

| Method + path | Returns |
|---|---|
| `GET /health` (open) | `{ version, services: { soulseek, library, lastfm, cache: { state: ok\|warn\|down\|off, detail, extra } } }`. Always 200 while serving |
| `GET /api/ping` | 204, or 401 without a valid device token (the UI then shows pairing). A cookie-carried token gets its cookie renewed (browsers cap a cookie at 400 days) |
| `GET /api/events` | Server-sent events, one per bus event: `event: <topic>`, `data: <json>`. `resync` means the client missed events and refetches what it shows. The connection marks its device online (lull Connect). It ends when the hub restarts its crates (after `hub.restarting`; reconnect) or when its device is unpaired |
| `GET /api/setup` (open) | `{ claimed: bool, soulseekManaged: bool, steps: { library: bool, soulseek: bool, lastfm: bool } }`. `claimed`: a browser or app is paired. A step is `true` when done: that service reports `ok` or `warn` in `/health`; `false` means first run still needs it. `soulseekManaged`: lull runs this slskd (`LULL_SLSKD_CONFIG_FILE`, the desktop app), so the Soulseek account is set in lull with `LULL_SOULSEEK_USERNAME` and `LULL_SOULSEEK_PASSWORD`; otherwise it is configured where slskd runs, and setup shows slskd's connection state |
| `POST /api/setup/claim { code, name, kind?: browser\|app }` (open) | The first device, with the setup code (printed in the hub's log while no browser or app is paired, or `LULL_SETUP_CODE`). Answers like `POST /api/pair`; 401 for a wrong code or once a device is paired |
| `POST /api/pair/codes` | `{ code, url, expiresAt }`: a single-use 6-character pair code, valid 10 minutes; `url` is `<hub url>/#/pair/<code>` for the QR, from `LULL_PUBLIC_URL` or else the address the asking device used |
| `POST /api/pair { code, name, kind?: browser\|app }` (open) | 200 `{ device }` plus the cookie for a browser (the default kind), `{ device, token }` for an app; 401 for a wrong, used or expired code; 400 without a name |
| `GET /api/devices` | `{ items: [Device + { online, state? }], me }`. Device `{ id, name, kind: browser\|app\|subsonic, createdAt, lastSeen }`; `state` is the last `device.state` of an online device |
| `PATCH /api/devices/{id} { name }` / `DELETE /api/devices/{id}` | 204: rename / unpair (its token stops working and its event streams end at once; unpairing the last browser or app arms a new setup code). 404 for an unknown id |
| `POST /api/devices/subsonic { name }` | `{ device, username: "lull", password }`: an app password for a Subsonic client, shown once |
| `POST /api/devices/me/state { trackId: string\|null, position, playing, volume, queue: [trackId], queueIndex: int\|null }` | 204; republished as `device.state`. `position` in seconds, `volume` 0-1 |
| `POST /api/devices/{id}/command { action: play\|pause\|seek\|next\|prev\|volume\|takeover, value? }` | 204; forwarded as `device.command`. `seek` (seconds) and `volume` need `value`. 409 when that device has no open event stream. `takeover`: the target sends its state to the caller and pauses |
| `GET /api/config` | `{ items: [{ key, value, set, source: env\|stored\|default, secret, restart }] }`, one per key the app can set (`lull_core::settings::KEYS`). `value` is `null` when unset and always for a secret; `set` says whether it has a value. `env` keys are locked. `restart` is `true` for every key: the crates read them all at start |
| `PATCH /api/config { LULL_X: value\|null }` | 200 `{ items }`. `null` or `""` goes back to the default. The whole merged configuration is checked with `Config::from_vars` before anything is stored; a 400 names the key (unknown, environment-only, set in the environment, or invalid). `LULL_LIBRARY` is refused: switch with `PUT /api/library/source`, which reconciles the library first. `LULL_SOULSEEK_USERNAME` and `LULL_SOULSEEK_PASSWORD` (set together) are refused unless `soulseekManaged`: "Soulseek is configured where slskd runs". After the restart, lull writes the account into slskd's config file, waits for slskd to reload it and logs in again (`DELETE`, then `PUT /api/v0/server`); the `soulseek` service shows the result. Then the hub restarts its crates (`hub.restart`) |

## Events

Topic → payload. Published on the bus and streamed to the browser. Owners add their topics here.

| Topic | Publisher | Payload |
|---|---|---|
| `service` | any (via `ctx.status`) | `{ name, service: { state, detail, extra } }` |
| `search.results` | soulseek | `{ searchId, query, files, peers, done, results?, entities?, onlySoulseek?, error? }`. While the search runs only the counters move (`done: false`); the last event (`done: true`) carries `results` and `entities`, or `error`. `onlySoulseek` is `null` until discovery's `search.catalog` for the query has arrived, then the result songs no catalog listed; a catalog arriving after the results publishes the last event again with it. Rows in the Soulseek section |
| `transfer` | soulseek | `{ jobId, trackId, peer, state: queued\|transferring\|stalled\|done\|failed\|cancelled, queuePos, received, size, speed, error }`. Bytes, bytes/s; `peer` changes on fallback. Absent values are `null` |
| `stream` | soulseek | `{ trackId, jobId, status: finding\|choose\|queued\|buffering\|streaming\|stalled\|fallback\|failed, peer, format, peersFound, queuePos, received, size, attempt, failedPeers: [{ peer, at }], error, belowMinimum, choices? }` drives the player's streaming states, while the track streams. `at` is the percent reached. `attempt` goes up on each fallback: reopen the stream URL (the old response ends) and seek back by time. `belowMinimum`: the copy playing doesn't pass the streaming minimum (`playBest`, or the listener's pick): show a badge. `choose` (streaming `below: ask`): no copy passes; `choices` lists up to 8 Sources found; answer by opening the stream with one's `peer` + `path`, or skip |
| `save` | library | `{ jobId, trackId, step: 1-5, status: running\|done\|failed\|pending\|waiting\|cancelled, pct, error, kind: transfer\|match\|write\|null, nocover, upgrade, belowMinimum }` (steps: transfer, match, cover, tag+write, rescan). `kind` names the recovery for a failure; `pct` moves during step 1 only. `waiting` (saving `below: wait`): no copy passes the saving minimum yet; `error` says so and the job searches again by itself. `upgrade`: started by auto-upgrade. `belowMinimum`: the copy saved doesn't pass the saving minimum (auto-upgrade watches it). `cancelled`: `DELETE /api/saves/{jobId}`; the job is gone after this event |
| `job` | any | `{ jobId, kind: transfer\|save\|import\|rescan, started, ... }`: one Activity row, the same shape `GET /api/jobs` lists. Library's rows: save `{ trackId, step, stepState, pct, error, failKind, nocover }`, rescan `{ state: running\|done\|failed, pct, detail }`. Other crates publish their own rows here to appear in `/api/jobs` |
| `play.started` | collection | `{ playId, trackId, at, duration, artist, title, album }`. lastfm now-playing, taste. The `plays` row is written first |
| `play.ended` | collection | `{ playId, trackId, at, playedS, completed, skipped, duration, artist, title, album }`, exactly once per play. `at` is the play's **start** (Last.fm's timestamp). Scrobbling, taste |
| `playlist` | collection | `{ playlistId, deleted }`: once per playlist create, rename or description change, track change and delete, from the routes, the `Collection` seam and `/api/sync` alike (a deleted playlist has `deleted: true`). Library mirrors playlists into Jellyfin from it; other devices refetch |
| `like` | collection | `{ trackId, liked }`. Taste, other devices |
| `follow` | collection | `{ kind: artist\|tag, id, followed }`. Discovery, other devices |
| `settings` | collection | The stored settings (the editable part of `GET /api/settings`), at start-up and on every change. Collection starts after every other crate, so a receiver made in `start()` gets the start-up copy |
| `scrobble` | lastfm | `{ playId (string), status: pending\|done\|short }`: queued, sent to Last.fm, or short of Last.fm's rule (History's scrobble column). Nothing while Last.fm is off, disconnected or scrobbling is switched off |
| `lastfm.history` | lastfm | `{ user, artists: [{ name, plays }] }`: the listener's Last.fm top artists, a taste signal for discovery; empty when off or disconnected |
| `hub.restart` | any | `{ reason }`: asks the hub to restart its crates with the current settings (a setting changed). The hub publishes `hub.restarting` and the UI reconnects. Requests made while one waits are covered by it |
| `hub.restarting` | hub | `{ reason }`: the hub's crates are restarting. Every SSE stream ends right after it; requests meanwhile wait for the new crates (about a second) |
| `device.state` | hub | `{ deviceId, trackId, position, playing, volume, queue, queueIndex, at }` |
| `device.command` | hub | `{ deviceId, action, value, from }`: only the device named acts on it |
| `device.online` | hub | `{ deviceId, online }`: the device's first event stream opened, or its last one closed (or it was unpaired) |
| `theme.changed` | themes | `{ id }`: a stored theme's files changed; devices using it reload its CSS |
| `library.source` | library | `{ mode: folder\|jellyfin, state: switching\|done\|failed, detail }`: a switch of the library source (`mode` is the target). `done` is followed by `hub.restart` |
| `search.more` | discovery | `{ query, artists, albums, tracks, errors?, entities, done }`: catalog additions for a running `/api/search` query (only ids not shown yet, best first), appended by the UI without reordering shown rows. `entities` also refreshes shown rows that gained facts (art, length); `errors` is every failed source so far; `done: true` on the last |
| `search.catalog` | discovery | `{ query, trackIds }`: the query's 50 best catalog tracks, once every source is in (again when the same words are searched while the run is kept). Soulseek pre-warms their sources (paced) and uses them to tell which Soulseek songs no catalog knew |

## Soulseek (`crates/soulseek`)

| Method + path | Returns |
|---|---|
| `GET /api/soulseek/search?q=` (ADR-0008: moves here from `/api/search`, which becomes discovery's catalog search) | `{ searchId, soulseek, entities }`. Answers as `search.results` events with that `searchId`, as before: live `files` / `peers` counters for up to 15 s, then the results, or `error` when slskd lost them. The same words again within 10 minutes reuse a search that is running or ended with results; an empty or failed one runs again. Queries under 3 characters don't search. Results also carry `onlySoulseek: [trackId]`, the songs no catalog knew (built from file tags), once discovery's `search.catalog` for the same query has arrived |
| `search.results` rows | `{ kind: "track", trackId, sources, best: Source, dup? }`: one song merged across peers; `sources` = how many peers; `dup` is `note` (already saved) or `better` (this file beats the saved one by `Quality::beats`). `{ kind: "album", albumId, trackIds, folders: [{ peer, path, format, tracks, sizeBytes, speed, freeSlot, queue, score, cover }] }`: a peer folder holding 3+ songs, the same album on several peers listed best first. Songs first by peers sharing them, then by score; at most 200 songs and 40 albums |
| `GET /api/tracks/{id}/sources[?for=save]` | `{ sources: [Source], done }` in the order the streaming profile (or with `for=save`, the saving one) takes them: copies that pass its minimum by its `pick`, then the rest, then peers with too long a queue. A Source is `{ peer, path, format, sizeBytes, speed (MB/s), freeSlot, queue, score 0-100, factors: { format, speed, slot, queue, stalled }, preferred, stalledBefore, meets, queueTooLong, aboveMax }`: the factors add up to the score, `stalled` is 0 or a negative penalty; `meets` = passes the profile's minimum; `queueTooLong` = busy with more people waiting than `maxPeerQueue` (never picked automatically); `aboveMax` = above the streaming cap `sources.stream.maxKbps` (lossless counts as above any cap), taken only after the copies within it (always false with `for=save`). When nothing is known about the song yet this starts a search for it and answers `done: false`: ask again every couple of seconds until `done`. 404 for a track id lull never handed out |
| `GET /api/stream/{trackId}[?peer=&path=]` | Audio with Range support (200 whole, 206 one range). Starts the transfer if needed (searching for the song first when no source is known) and serves bytes as they arrive, from copies in the streaming profile's order; `peer` + `path` (a Source's) plays that source. With another copy than the one playing, it replaces the transfer (switching source mid-song, or answering `choose`): the old one is cancelled unless a save needs it, and its response ends; seek back by time. The answer waits for the first bytes; a range beyond what has arrived waits 8 s, then 416. A stall falls back to the next copy and ends the response (`stream` events). 502 when no peer delivers. Saved tracks are streamed by library, not here |
| `DELETE /api/stream/{trackId}` | 204. Stop: cancels the transfer (skip / next) unless a save needs it; slskd keeps the partial file, so playing it again resumes |
| `GET /api/soulseek/diagnostics` | What slskd itself sees, since its UI and logs are out of reach in the deploy kit: `{ slskd: { version, server, distributedNetwork, connectionWatchdog, pendingReconnect }, searches: [{ query, state, peers, files, at }], log: [{ timestamp, level, message }] }`. `searches` is how the latest network searches ended, newest last; `log` is slskd's latest lines (empty without an administrator key) |
| `GET /api/transfers` · `DELETE /api/transfers/{jobId}` · `POST /api/transfers/{jobId}/retry` | `{ items, total }`: running then recently ended transfers, each the `job` row `{ jobId, kind: "transfer", trackId, peer, speed (MB/s), pct, state, queuePos, started, error }` (also published as `job` events). Cancel: 204. Retry (a failed or cancelled one): `{ jobId }` of the new transfer |

## Library (`crates/library`)

The library is the music folder or Jellyfin (`LULL_LIBRARY`, ADR-0003); every route below works in both modes. Lists and lookups come from lull's index of the library, so in Jellyfin mode they keep working while Jellyfin is down. Details: [library.md](library.md).

| Method + path | Returns |
|---|---|
| `GET /api/tracks/{id}` | `{ track, entities }`. A saved track (also under the id it was saved as, see save-flow.md#ids), else the catalog's with `saved: false`; 404 if neither knows it |
| `GET /api/library/songs?sort=album\|title\|artist\|recent` | `{ items: [trackId], total, entities: { tracks }, songs: { trackId: { addedAt, plays } }, stale }`. `addedAt`: when the song entered the library (ISO 8601 UTC: the file's modified time in folder mode, Jellyfin's DateCreated in Jellyfin mode). `plays`: its rows in `plays`, counting plays under ids that alias it. Both stay out of the track entity (per-listener state) |
| `GET /api/library/albums?sort=recent\|title\|artist\|year` | `{ items: [albumId], total, entities: { albums }, stale }`. Library albums are `saved: "all"` |
| `GET /api/library/artists?sort=name` | `{ items: [artistId], total, entities: { artists }, stale }` |
| `GET /api/library/genres?sort=songs\|name` | `{ items: [{ id, name, artists, songs, trackIds }], total, entities: { tracks }, stale }` (`trackIds`: a few, for the mosaic) |
| ↳ every list | Also `filter=` (words in title, artist or album), `limit=` (default 200, max 5000), `offset=`. An unknown `sort` is 400. `stale: true` while Jellyfin is down (Jellyfin mode) and the list is the last one read |
| `GET /api/library/stream/{trackId}` | The saved file with Range support (200, 206 + `Content-Range`, 416). Folder mode serves the file; Jellyfin mode passes `Range` through and streams the body without buffering. 404 if not saved |
| `GET /api/art/{albumId}?size=96\|256\|512` | The cover image (`Cache-Control: private, max-age=86400`). For saved albums the library's (Jellyfin's image, or the folder's `cover.jpg`/`folder.jpg` or embedded picture, scaled), else the Cover Art Archive's (found through the catalog and MusicBrainz for albums that aren't saved). Cached on disk; 404 when there is none, and also while an outside lookup takes over 3 s (it finishes in the background, so a later request gets the cover) or after one failed in the last minute |
| `POST /api/library/duplicates { ids, source?: { trackId: { peer, path, size } } }` | `{ items: [{ trackId, existing: Track, better }] }` for the ids that are saved already. `better`: the offered file is clearly better (lossless over lossy, or a fifth more bitrate) |
| `POST /api/saves { ids, source?: { trackId: { peer, path, size } }, duplicate?: { trackId: separate\|replace } }` | `{ jobs: [jobId], skipped: [trackId] }`. One job per id; progress as `save` events. `source` is a row of `/api/tracks/{id}/sources` (`sizeBytes` is accepted for `size`); without one, the saving profile picks (`Transfers::save_candidate`: Soulseek searches when it knows no copy), and its `below` rule decides when none passes the minimum: save the best (`saveBestUpgrade`), wait (`waiting`), or fail at step 1 (`kind: transfer`) for the listener to pick (`ask`), as when no peer shares the song (save-flow.md#picking-the-copy). An id already saved is skipped unless `duplicate` says what to do (the duplicate dialog's answer; "keep" = don't send it). An id already saving returns its running job |
| `GET /api/saves/{jobId}/releases` | `{ items: [{ mbid, recording, releaseGroup, title, artist, kind, date, country, tracks, score }], track }`: the release picker after a `match` failure, best first (`kind`: "album, CD"); `track` is what lull knows of the file |
| `POST /api/saves/{jobId}/release { mbid } \| { keepTags: true }` | `{ jobId }`. Continues the save with that release, or with the file's own tags. 409 if the job isn't waiting for a release |
| `POST /api/saves/{jobId}/retry [{ source }]` | `{ jobId, step, status }`. Restarts a failed save from its failed step (a new `source` for a transfer failure), or picks up a pending one again. 409 otherwise |
| `DELETE /api/saves/{jobId}` | 204. Gives up on a save that is `waiting` for a copy that passes the saving minimum, or `failed`: one `save` event with `cancelled`, then the job is gone. 409 while it transfers or writes |
| `GET /api/jobs` · `DELETE /api/jobs` | `{ items: [row] }`, newest first: library's save and rescan jobs plus the latest `job` event of every other job. `DELETE` clears finished rows (failed saves stay) → 204 |
| `POST /api/library/rescan` | 202 `{ jobId }`. Jellyfin mode: runs Jellyfin's library scan; the rescan job follows its progress and ends with "Jellyfin found N new songs". Folder mode: walks the music folder again ("Found N new songs in the music folder") |
| `GET /api/library/source` | `{ mode: folder\|jellyfin, musicDir, jellyfin: { url, user, connected } \| null, files, lastScan, switching }`. `jellyfin` is null when it isn't configured; `connected`: lull reads it right now (Jellyfin mode, status `ok`). `files`: songs in the folder index (folder mode, else null); `lastScan`: unix time of the last whole-folder walk, or null |
| `PUT /api/library/source { mode }` | 202. Switches in the background: reads the other source, reconciles by path into `library_aliases`, stores `LULL_LIBRARY` through `core::settings`, then publishes `hub.restart`; progress as `library.source` events. 409 when already in that mode, while a switch runs, or when the environment sets `LULL_LIBRARY`; 400 for `jellyfin` before Jellyfin is connected |
| `GET /api/jellyfin/discover` | `{ items: [{ name, url, version }] }`: Jellyfin servers answering on the local network (UDP 7359; empty from inside a bridge-networked container) |
| `POST /api/jellyfin/quickconnect { url }` | `{ code, secret }`: show `code`; the listener approves it in Jellyfin › Quick Connect. 409 when Quick Connect is off on that server (use sign-in) |
| `GET /api/jellyfin/quickconnect/{secret}` | `{ state: waiting\|approved\|expired }`. On `approved` lull stores `LULL_JELLYFIN_URL`, `LULL_JELLYFIN_API_KEY` (the token) and `LULL_JELLYFIN_USER`, and switches to Jellyfin mode like `PUT /api/library/source`. 404 for a secret lull doesn't know (start again) |
| `POST /api/jellyfin/signin { url, username, password }` | 202, then the same switch: the fallback when Quick Connect is off. The password is sent to Jellyfin once and never stored. 400 when Jellyfin refuses it |

## Discovery (`crates/discovery`, `crates/lastfm`, `crates/lyrics`)

Sources and limits: [discovery.md](discovery.md); taste: [taste.md](taste.md); Last.fm: [scrobbling.md](scrobbling.md).

| Method + path | Returns |
|---|---|
| `GET /api/search?q=[&submitted=1]` (ADR-0008) | `{ query, top: { kind: artist\|album\|track, id } \| null, artists: [artistId], albums: [albumId], tracks: [trackId], pending: [deezer\|itunes\|library\|musicbrainz\|musicbrainz-albums\|lastfm], errors?: { same keys: message }, entities }`, best first (at most 20 artists, 20 albums, 50 tracks). `query` is the words as searched (lowercase, single spaces); `search.more` events carry the same. The fast catalog and saved songs answer at once (at 400 ms with whatever rows are in; an answer with none yet waits for them up to 1.5 s); the fast catalog is Deezer, or iTunes for tracks and albums plus Deezer for artists where Deezer is closed (status `catalog`, discovery.md#search). `pending` names the sources still searching, whose additions arrive as `search.more`. `albums` holds albums, EPs and singles: `Album.type` is `album`, `EP`, `single` or `compilation`. Albums and artists in `entities` carry `art` / `image` (`api/catalog/art/<id>`, relative to the page; add `?size=`) when lull has a picture for them. `errors.deezer` also names a partial answer (Deezer withholds tracks and albums where it isn't open). The same words within 10 minutes share one run. `submitted=1` (the words were submitted, as when Soulseek starts) adds MusicBrainz's release groups by title to the run, once (`musicbrainz-albums`, arriving as `search.more`); calling again with it for a run already answered is cheap. 400 for blank words |
| `GET /api/catalog/art/{id}?size=96\|256\|512` | The cover (`al-…`) or artist photo (`ar-…`) discovery found (Deezer, iTunes, Cover Art Archive), scaled to `size` (default 256), fetched once and cached; `Cache-Control: private, max-age=86400`. 404 at once when lull knows no source, or when the source has none. A picture lull hasn't fetched yet is waited for at most 3 s; if it isn't in by then the answer is 404 with `Cache-Control: no-store` while the fetch finishes in the background (it never answers 503). Every album in discovery's answers (search, feed, pages) with a source carries `art` |
| `GET /api/feed` | `{ updated, summary, newReleases: [albumId], picks: [{ trackId, reasons: [{ kind: like\|tag\|listeners\|fresh, name, ref? }], via }], radioStarters: [{ id, title, sub, seed: { kind: track\|artist\|tag, id } }], jumpBackIn: [{ kind: album\|artist, id }], followedTags, coldStartTags, errors?: { newReleases\|picks: message }, entities }`. `ref` is an artist id or a tag (absent for "listeners like you"); `via` names the source. The first call may take tens of seconds (the UI's loading state); afterwards it is instant |
| `GET /api/radio?kind=track\|artist\|tag&id=&steer=0..1&exclude=id,id` | `{ items: [trackId], entities }`: 12 tracks to top up the queue. `steer` defaults to 0.4. Answers within about 8 s: ListenBrainz, else Last.fm, else saved songs (discovery.md#radio). 404 when the seed is unknown or no source has anything; 503 only when no source answered at all |
| `GET /api/albums/{id}` | `{ album: Album (trackIds, duration, saved), entities }` (tracks with saved state) |
| `GET /api/artists/{id}` | `{ artist: Artist (tags, listeners, bio, image when Deezer has a photo), topTracks: [trackId], albums: [albumId] (newest first), similar: [artistId], errors?: { section: message }, entities }` |
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
| `GET /api/settings` · `PATCH /api/settings` (partial object) | `MOCK.settings` minus appearance (per device) and Last.fm switches (`/api/lastfm`). Stored: `playback { gapless, crossfade, normalisation, loudnessTarget, volumeStep, radioWhenQueueEnds, prefetchCount }` (`prefetchCount` 0-3, default 2: how many queued songs the player lists in `PUT /api/stream/prefetch`), `sources { stream: StreamProfile, save: SaveProfile, maxPeerQueue, stallTimeout }` (ADR-0008; the profile types are `lull_core::quality`; `formatPreset`, `minBitrate` and `autoPick` give way to the profiles: stored values and an older UI's `PATCH` migrate, a changed `minBitrate` to `stream.minimum.kbps` and `autoPick: false` to `below: ask` in both; minimum `kbps` 0-320, `stream.maxKbps` 0 (no cap, the default), 128, 192, 256 or 320 and never below `stream.minimum.kbps` (change both in one `PATCH`), `formats` from flac, wav, aiff (`.aif` counts), m4a, aac, mp3, ogg, opus and never a minimum nothing passes, `upgradeEveryH` 1-720), `search { showFiles }`, `library { duplicatePolicy }`, `sharing { uploadSlots, uploadLimitKBs }`, `discovery { adventurousness, on: { listenbrainz, musicbrainz, lastfm } }`. From config, read-only: `{ value, source: "config" }`, secrets `{ value: null, source: "config", secret: true, set }`. `cache.usedGB` is live (null until soulseek reports). A `PATCH` with an unknown key, a config key or a bad value is refused whole (400). Publishes `settings` |
| `GET /api/skins?q=&offset=&nsfw=` | `{ items: [{ md5, name, file, screenshotUrl, museumUrl, nsfw, favourite, downloaded }], more }`, 30 a page. Empty `q` = the museum's own order. NSFW hidden unless `nsfw=true` (the per-device setting). Screenshots load straight from the museum's CDN |
| `GET /api/skins/{md5}.wsz` | The skin file: downloaded once, validated (a zip with `main.bmp`), cached, same origin, `immutable` |
| `GET /api/skins/favourites` · `PUT/DELETE /api/skins/favourites/{md5}` | `{ items: [skin] }`, newest first · 204 (404 for a skin the museum doesn't have) |
| `POST /api/sync { ops: [{ id, at, op, ... }] }` | `{ applied: [id], rejected: [{ id, message }], playlists: { tempId: playlistId } }`. Ops apply in order, each in its own transaction, and are idempotent by `id` (a client-made uuid, 1-100 chars): a resent op is acknowledged, not applied again, and a resent `playlist.create` still reports its playlist. `at` (unix seconds) dates a like, follow or playlist change; a play uses its `startedAt`. A fault in one op (unknown track, bad shape, unknown `tempId`) rejects only that op; a missing `id` or more than 1000 ops is a 400 for the whole request. `op`: `play { trackId, startedAt, playedS, completed, skipped }`, `like { trackId, liked }`, `follow { kind, id, name?, followed }` (`name` required to follow an artist), `playlist.create { tempId, name, desc?, trackIds? }`, `playlist.edit { playlistId \| tempId, name?, desc?, trackIds? }`, `playlist.delete { playlistId \| tempId }`. A `tempId` resolves to the playlist an earlier `playlist.create` in any batch made. Publishes the same events as the matching routes (a synced play publishes `play.ended` only) |
| `POST /api/import/exportify` (body: Exportify CSV, up to 16 MB) | The M3U import preview shape (`name: null`, `lines`, `counts`, `libraryChecked`, `entities`; `location` is the Spotify URI); the UI then `POST`s the playlist with the ids it keeps. 400 when the file lacks the `Track Name` and `Artist Name(s)` columns |
| `POST /api/import/history { source: "lastfm", user }` or `POST /api/import/history` (body: a ListenBrainz export, up to 128 MB) | 202 `{ jobId }`. The plays are added as imported: they join History, Stats and taste, never scrobble, and publish no `play.*` event. Progress is `job` rows `{ jobId, kind: import, name, state: running\|done\|failed, started, pct, added, duplicates, invalid, error }`. Last.fm needs the server's API key (503 without) and a public profile (400 for an unknown or private user, checked before the 202); ListenBrainz takes the listens as a JSON array, JSON Lines or the API's `{ payload: { listens } }` (400 for a zip or anything else). 409 while another history import runs |

## Themes (`crates/themes`)

ADR-0004; format, hooks and slots: [themes.md](../themes.md). Built-in designs are static files of the UI (`/designs/<id>/`); these endpoints serve the listener's themes, stored in `<data_dir>/themes/<id>/`. A built-in id in a path is a 404.

| Method + path | Returns |
|---|---|
| `GET /api/themes` | `{ items: [{ id, name, author, api, extends, shell, swatches, modes, description, preview, slots, coverVariety, builtIn }] }`: the five designs, then the listener's themes (a folder whose `theme.json` no longer reads is left out and logged) |
| `GET /api/themes/{id}/{path}` | A file of a stored theme (`theme.json`, `theme.css`, `slots/*.html`, `assets/*`), ETag'd, `no-cache`, served under a sandboxing CSP |
| `POST /api/themes[?replace=true]` (body: a `.lulltheme` zip, up to 16 MB) | 200 manifest (with `builtIn: false`): stored under the id its `theme.json` names. 409 when that id exists, unless `replace` |
| `PUT /api/themes/{id}` (body: a `.lulltheme`) or `PUT /api/themes/{id}/{path}` (one file, from Theme Studio) | 200 manifest. A new theme starts with its `theme.json` (it gets an empty `theme.css`). Every write is checked as the whole package it makes: 400 names what's wrong (missing `theme.json`, an unknown field, a newer `api`, a script or handler, an outside URL, a file with no place in a theme) |
| `DELETE /api/themes/{id}` | 204 |
| `GET /api/themes/{id}.lulltheme` | The package as a zip, for sharing |

The hub derives a stored manifest's `slots` from `slots/`. Any change to a stored theme's files, by these endpoints or on disk, publishes `theme.changed` within a second.

## Subsonic (`crates/subsonic`)

OpenSubsonic (API 1.16.1) at `/rest/<endpoint>`, `.view` suffix accepted, by GET or form POST. XML by default, JSON with `f=json`. Auth on every call but `getOpenSubsonicExtensions`: `u=lull` with `p=` (plain or `enc:`) or `t=md5(password + salt)`+`s=`, against an app password from `POST /api/devices/subsonic`; or `apiKey=` with any paired device's token. Ids are lull's (`ar-`, `al-`, `t-`, `pl-`); cover art ids are album ids. Rules and spec gaps: [subsonic.md](subsonic.md).

| Group | Endpoints |
|---|---|
| System | `ping`, `getLicense`, `getOpenSubsonicExtensions` (`apiKeyAuthentication`, `formPost`, `songLyrics`), `getMusicFolders`, `getUser` (only `lull`) |
| Browse | `getIndexes`, `getArtists`, `getArtist`, `getAlbum`, `getSong`, `getAlbumList2`, `getRandomSongs`, `getGenres`, `getSongsByGenre` |
| Search, media | `search3` (empty query lists all), `stream` and `download` (the saved file, Range honoured), `getCoverArt` (`size`) |
| Likes, plays | `star`, `unstar` (songs only), `getStarred2` and `getStarred`, `scrobble` (a submission becomes `Collection::record_play`) |
| Playlists | `getPlaylists`, `getPlaylist`, `createPlaylist`, `updatePlaylist`, `deletePlaylist` (saved songs only; indexes count those) |
| Lyrics | `getLyricsBySongId` (through the `Lyrics` seam: synced lines in ms, plain lines untimed, none for instrumentals) |
