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

## Plays and events

- **Lifecycle.** The UI calls `POST /api/plays` when a song starts and `PATCH /api/plays/{id}` once when it stops. The row is written before the event goes out, so a reader can find it; lastfm itself takes everything from the event payload.
- **`play.ended` goes out exactly once.** A second `PATCH` is 409, because otherwise a play would scrobble twice. A play the UI never ends (tab killed without `keepalive`) stays open: it shows in history and stats with `playedS: 0` and is never scrobbled. lull doesn't guess how much was heard.
- **`at` is always the start time**, in both events and history, since Last.fm's timestamp is the start.
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
- **Export writes stream links only.** The design also offers "library paths", but the `Library` seam doesn't expose file paths. Guessing them from `LULL_NAMING` would be wrong after MusicBrainz retagging, so library paths are not offered. With `LULL_ACCESS_TOKEN` set, outside players can't fetch the links.
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
