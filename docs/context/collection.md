# Collection: likes, follows, plays, history, playlists, M3U, stats, settings

`crates/collection`. Endpoints and shapes: [api.md](api.md#collection-cratescollection-cratesskins). This page has the rules that the shapes don't show.

## Tables

| Table | Owner | What |
|---|---|---|
| `likes`, `follows`, `plays` | core (written only by collection) | Readable by every crate: discovery reads them for taste |
| `collection_plays` | collection | Per play: `recommended`, `ended_at` (null while playing), `scrobble` (latest status) |
| `collection_saves` | collection | Songs saved through lull, from library's `save` events (step 5 `done`, or `pending` = written, rescan failed) |
| `collection_playlists`, `collection_playlist_items` | collection | `pl-<seq>` ids from an AUTOINCREMENT column, so a deleted playlist's id is never reused. Items are keyed by position, so a song can appear twice |
| `collection_settings` | collection | One JSON row of the editable settings |

## Writes and events

Every change runs in one transaction and publishes its events after the commit (`Service::write`). The routes, the `Collection` seam (`seam.rs`, for Subsonic) and `/api/sync` call the same functions, so each publishes the same events.

## Sync (`POST /api/sync`, ADR-0005)

- **Each op is its own transaction**, applied with a row in `collection_sync_ops` (the op id), so a crash never leaves an op applied but unrecorded, and a resend finds it.
- **Rejected vs failed.** An op that is wrong (unknown track, bad shape, a `tempId` nothing made) is rejected with a message and the rest go on; the device drops it. A database failure fails the request: earlier ops are already applied and recorded, so the resend skips them.
- **`tempId`s** live in `collection_sync_playlists`, so an edit in a later batch finds the playlist an earlier batch made. A resent create answers with the same playlist.
- **Retention.** Op ids and `tempId`s are forgotten after 90 days (`sync::RETENTION`), pruned at the start of each sync. A device offline longer than that and resending the same op would apply it twice; an outbox is days old at worst.
- **Times.** A play keeps its real `startedAt`; likes, follows and playlists are dated `at`. Neither may be in the future (a device clock ahead is clamped to now).
- **A synced play** goes through the same `record` as the `Collection` seam: `play.ended` only, so lastfm scrobbles it at its real start.

## History imports (`POST /api/import/history`)

- **Imported plays** are rows in `plays` with `collection_plays.imported = 1` and an end time already set. Nothing is published for them, so lastfm never sees them (no scrobble, and no echo of Last.fm's own history back to Last.fm) and no `play.*` event reaches other devices. They do join History, Stats and the taste model, which is the point: a new listener starts with their taste.
- **A listen counts as heard** (`completed`), since that is what a scrobble means and the taste model reads. `played_s` is the duration when the source or the catalog knows it, else 0, so Stats minutes undercount imported listening.
- **Ids** are `TrackId::of(artist, album, title)` as the source spells them, recorded in the catalog unless a richer record is there. Library matching is not tried per listen (a long history is tens of thousands of songs): a saved song whose album is spelled differently than in Last.fm gets another id, and shows as not saved.
- **Duplicates.** A listen at the same second as a play with the same track id or the same title (case aside) is skipped. That is how a re-run adds nothing, and how the plays lull itself scrobbled are not counted twice.
- **Last.fm** (`user.getRecentTracks`, 200 a page, paced at 300 ms). The first page is fetched before the 202, so an unknown user, a private profile or a missing server key is an answer, not a failed job. Listens are limited to those before the import began, so new scrobbles can't shift later pages. The song playing now has no time and is skipped.
- **ListenBrainz** takes the export's JSON, JSON Lines, or the API's payload. Listens without `listened_at` are left out.
- **One at a time** (409 otherwise), as a `job` row per batch of 500.

## Private listening

Spotify's Private Session, per paired device: `core_devices.private`, switched with `PATCH /api/devices/{id|me} { private }` and shown on `GET /api/devices`. The hub enforces it, so it holds whatever the device's UI does or doesn't.
- **The device that sends the play decides**, read inside the transaction that would record it. Every path that records a play asks (`devices::is_private`): `POST /api/plays`, `PATCH /api/plays/{id}`, `/api/sync` play ops (the outbox of a device that switched on later), and the `Collection` seam's `record_play` (Subsonic `scrobble`, which names the device that signed in). `PlayReport` carries the device for that.
- **A private play leaves nothing:** no `plays` row, so no History, Stats or taste, and no `play.*` event, so nothing for lastfm (now-playing or scrobble) or discovery. Playback is unchanged.
- **Answers stay ordinary** so clients don't break: `POST /api/plays` gives `{ playId: null, private: true }`, the others succeed and do nothing; a synced play counts as applied, so the device's outbox drops it.
- **A play started before the switch stays as it was:** it is open (`playedS: 0`, never ended) and its end is ignored. Nothing already recorded is deleted.
- **A device that isn't paired (any more) counts as private.**
- **Not covered:** history imports (a listener's own action, not listening) and Subsonic's `submission=false`, which has no effect anyway.
- **Send to Last.fm** is refused from a private device (scrobbling.md).

## Plays and events

- **Lifecycle.** The UI calls `POST /api/plays` when a song starts and `PATCH /api/plays/{id}` once when it stops. The row is written before the event goes out, so a reader can find it; lastfm itself takes everything from the event payload.
- **`play.ended` goes out exactly once.** A second `PATCH` is 409, because otherwise a play would scrobble twice. A play the UI never ends (tab killed without `keepalive`) stays open: it shows in history and stats with `playedS: 0` and is never scrobbled. lull doesn't guess how much was heard.
- **`at` is always the start time**, in both events and history, since Last.fm's timestamp is the start.
- **Plays from elsewhere.** The `Collection` seam's `record_play` (Subsonic) records a finished play at its real start and publishes only `play.ended`: "now playing" for a song that already finished would be wrong.
- **History is in order of start** (`started_at`, then id), not of arrival, since plays can arrive late. `next` is still a play id.
- **Scrobble status.** lastfm owns the scrobble rules. It publishes `scrobble { playId, status: pending|done|short }` (api.md#events), and collection stores the latest status per play. `short` = Last.fm's rule failed (track of 30 s or less, or less than half and less than 4 min heard); a skip after the half still scrobbles. `null` in history means lastfm never reported: not connected, or scrobbling off.
- **History and stats need the listener's timezone** (`?tz=`, minutes east of UTC). The server has no other way to know where a day ends.

## Playlists and M3U

- **Every track id in a playlist resolves through the catalog.** Ids the catalog doesn't know are refused with 400, so the UI can always show a playlist.
- **Import is a preview.** `POST /api/playlists/import` parses the file and checks each line, then the UI creates the playlist from the ids it keeps. Each line is tried in this order:
  1. A lull stream link (an export read back): the catalog id, `library` if saved.
  2. `Library::search(artist title)`: same normalised title, and the same artist when the line names one. That makes it `library`.
  3. Artist and title known: `will-search`. The song is recorded in the catalog with `TrackId::of(artist, album or "", title)`, so its id resolves and plays search Soulseek. A richer catalog record already there is kept.
  4. Otherwise `not-found` (a bare `track01.mp3`, a radio URL).
- **Reading a line.** `#EXTINF` "Artist - Title" (split at the first " - ") and `#EXTART`/`#EXTALB` win over the path. The path gives `Artist/Album/NN - Title.ext`:
  - the track number is stripped: `01 - `, `01. `, `1-03 `, `07_`;
  - a year is stripped from the album folder: `Album (2019)`, `Album [2019]`, `2019 - Album`;
  - a loose `Artist - Title.mp3` is split.

  Known limit: a title that starts with a number ("99 Luftballons") loses it when it comes from the file name.
- **Encodings.** A BOM wins (UTF-8/UTF-16). Otherwise the file is read as UTF-8 if it is valid, else as Windows-1252, which is what old players write in `.m3u`.
- **The library is optional.** If a library call fails once, the rest of the file is matched without it and the response says `libraryChecked: false`; the import doesn't fail.
- **Exportify CSV** (`POST /api/import/exportify`) is read into the same entries and goes through the same preview. Columns are found by header, so order and extra columns don't matter. A song's first artist (Spotify joins them with `;`) is its artist, which is what the id and the library match use. Rows without a title or artist are dropped.
- **Export writes stream links only.** The design also offers "library paths", but the `Library` seam doesn't expose file paths. Guessing them from `LULL_NAMING` would be wrong after MusicBrainz retagging, so library paths are not offered. With `LULL_ACCESS_TOKEN` set, outside players can't fetch the links.
- **`playlist` event.** `{ playlistId, deleted }` goes out once per operation (create, rename, track change, delete), from the routes, the seam and sync alike, so a PATCH that changes name and tracks is one event. Library mirrors playlists into Jellyfin from it.
- **Activity.** A playlist created with `imported: { file, library, willSearch, notFound }` publishes an import `job` event, and library's `/api/jobs` lists it.

## Stats

Computed from `plays` on each request; one listener's history stays small.
- `minutes` = Σ `played_s` / 60.
- `artists` and `newArtists` count artist ids, so "The Cure" = "Cure". An artist is new if their first play ever falls inside the period; for `all`, every artist is new.
- `skipRate` = skipped / plays, 0 for an empty period.
- `topTags`: each play adds its track's catalog tags. A tag's value is its share of all tag mentions, in percent.
- `hitRate`: distinct tracks started with `recommended: true` in the period, and how many of those are now liked or were saved through lull.
- `saves` = saves in the period, from `collection_saves`.
- The taste block is discovery's (`GET /api/taste`).

## Settings

- **What is stored here:** what must follow the listener across devices and isn't owned elsewhere.
  - Appearance (theme, design, motion, text size, NSFW skins, default skin) stays per device in the browser (DESIGN.md §7.5, PROJECT.md).
  - Last.fm's switches live in lastfm (`GET/PATCH /api/lastfm`).
- **Config always wins.** Cache size, share mode, Jellyfin, slskd, naming and keys come from `LULL_*` and are shown read-only. Secrets only say whether they are set.
- **Reaching other crates.** Crates that honour a setting subscribe to the `settings` event, published at start-up and on every change. Soulseek reads `sources` and `sharing`; discovery reads `discovery`. See [ADR-0050](../adr/0050-settings-reach-crates-by-event.md).
