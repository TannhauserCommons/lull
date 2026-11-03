# Collection: likes, follows, plays, history, playlists, M3U, playlist links, stats, settings

`crates/collection`. Endpoints and shapes: [api.md](api.md#collection-cratescollection-cratesskins). This page has the rules that the shapes don't show.

## Tables

| Table | Owner | What |
|---|---|---|
| `likes`, `follows`, `plays` | core (written only by collection) | Readable by every crate: discovery reads them for taste |
| `collection_plays` | collection | Per play: `recommended`, `ended_at` (null while playing), `scrobble` (latest status) |
| `collection_saves` | collection | Songs saved through lull, from library's `save` events (step 5 `done`, or `pending` = written, rescan failed) |
| `collection_playlists`, `collection_playlist_items` | collection | `pl-<seq>` ids from an AUTOINCREMENT column, so a deleted playlist's id is never reused. Items are keyed by position, so a song can appear twice |
| `collection_settings` | collection | One JSON row of the editable settings |
| `collection_links`, `collection_link_songs`, `collection_link_choices` | collection | Playlist link imports: the link and its playlist, its songs with the catalog's match, the listener's fixes and skips |

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

## Outside playlists

`outside.rs`, `matches.rs`. Someone's playlist on YouTube or Deezer (any link lull imports from), opened from search's Playlists section, previews like an album page without becoming a lull playlist; it can be bookmarked like following a playlist on Spotify. "Make a copy" is the link import.
- **Kept, not imported:** `collection_outside` keeps the listing as last read (JSON). Opening it after an hour reads the source again; if the source fails, the kept one shows. A listing nobody bookmarked and nobody opened for 30 days is forgotten, with its picture.
- **Playable at once:** every song has a track id from the first answer: the catalog's match, else the song as the link names it (`TrackId::of(artist, album, title)`, recorded in the catalog, searched on Soulseek or YouTube when played, as an M3U line is). Its `status` says which: `matched`, `pending` (not looked up yet) or `unmatched`.
- **Matched in the background:** pending songs are looked up after the answer (4 at a time, the catalog paces itself), and go out as `outside` events, a batch a second. One run per playlist at a time.
- **Matches are shared** with link imports (`collection_song_matches`, `matches.rs`, by the source's song id): the one record of what a source's song matched; a match is kept, "nothing matches" for a day. A preview makes its copy's import instant, and the other way round. Matches link imports had kept per link before were moved in.
- **Pictures** are fetched when a listing is read (up to 4 s, else later) and kept as square JPEGs in `<data>/outside-covers/`, so the CSP stays same-origin and a bookmark keeps its picture if the source changes.
- **Bookmarks** (`collection_bookmarks`) hold the link, name, owner and size, refreshed whenever the listing is read; Library's Playlists shows them beside lull's own, marked with their site. Removing one keeps nothing.

## Covers

`covers.rs`. A playlist's own cover is an upload (`PUT /api/playlists/{id}/cover`) or a link import's source picture; without one the UI shows its songs' album covers (a 2×2 mosaic of the first four albums, one cover for a single album).
- **Stored on the hub** as `<data>/playlist-covers/pl-<n>.jpg`: the centred square, at most 600 px, JPEG 88, so it keeps working whatever the source does. `collection_playlists.cover_at` (ms) versions its URL; `cover_from` is `upload` or `link`, `cover_url` the source picture kept.
- **Checked before decoding:** JPEG, PNG or WebP only, at most 10 MB and 8000 px a side (a small file can claim a huge picture).
- **A link's picture** (Spotify's embed images, Deezer's `picture_xl`/`cover_xl`, YouTube's or SoundCloud's thumbnails; the first of up to three, largest first, that loads, since YouTube lists a `maxresdefault` that often 404s) is fetched once: again only when the source shows another picture, and never over an upload. If it can't be had, the import still ends and the songs' covers show.
- **Removing a cover** or **deleting the playlist** removes the file (deletion through the `playlist` event, whichever path deleted it).
- **Subsonic** asks through `Collection::playlist_cover` (subsonic.md).

## Playlist links

`POST /api/import/link` (`links/`): a pasted link becomes a lull playlist whose songs play from Soulseek (or YouTube) when played; nothing is saved. A `job` row shows it in Activity.
- **Sources, all keyless:**
  - **Spotify** playlists and albums from the public embed page (`open.spotify.com/embed/…`): title, artists, length; no album or ISRC, and **only the first 100 songs** (the job and the import carry a `note` saying so; Exportify's CSV has the rest). Spotify's Web API reads only playlists its app's own user owns or collaborates on since February 2026, so it can't import a shared link. Reading the page bends Spotify's no-scraping terms, with the user's OK (PROJECT.md decisions).
  - **Deezer** playlists and albums through its public API, with ISRCs, pages of 100 inside lull's one Deezer window (`http::DEEZER`, shared with discovery) (an id lookup answers where Deezer's catalog is closed).
  - **YouTube, YouTube Music, SoundCloud** playlists through yt-dlp (`Youtube::playlist`, youtube.md), YouTube 200 songs a page, SoundCloud 40 (about 3-4 s a song, so a 300-song set reads for about 20 minutes). If the site stops answering after the first page, the songs read so far are imported and the `note` says the rest are missing. Each upload comes with its artist and title as the youtube crate reads them, the same reading its own matching uses (youtube.md#matching: music metadata, a Topic channel, "Artist - Title", packaging dropped), and with whether its length is the song's; a music video's length isn't compared.
- **Before the 202:** the link is parsed (each link has one key, `spotify:playlist:<id>`, whatever its tracking parameters or app), short links followed, and the playlist's name read, so a wrong or private link is an answer. At most 2000 songs.
- **Matching** is discovery's `Catalog::find_song` (discovery.md): the library, then the catalog by ISRC, else by artist, title and length. A song matched before (by any link or preview) keeps its match without a lookup (`matches.rs`). A failed lookup leaves the song unmatched (a re-import tries again); the job fails only when every lookup does. Songs with an ISRC (Deezer's) match in seconds; in iTunes countries the others go at about 12 a minute, so a 100-song Spotify or YouTube playlist takes several minutes. Activity shows "Matching songs, n of m" from the start.
- **Unmatched songs** stay out of the playlist and are listed (`GET /api/import/links/{id}`). The listener picks the song from search, keeps it as the link names it (`as-is`: recorded as `TrackId::of(artist, album, title)`, searched on Soulseek when played, as an M3U line is), or skips it. Matched songs can be changed or skipped the same way.
- **Choices are kept by the source's song id** (`collection_link_choices`), so they outlive re-imports; `none` undoes one.
- **Name, description and cover** come from the source when the playlist is made (the description is "Imported from <link>." when the source has none; Spotify's embed has none). Later imports keep the listener's name and description; the cover follows covers' rules.
- **Re-importing** the same link updates its playlist (`collection_links.playlist_seq`) to the link's songs in the link's order, with the choices applied: no second playlist. Songs added or removed by hand in lull are overwritten; skip a song to drop it for good. A rename stays. A deleted playlist is made again. One import per link at a time (409).

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
