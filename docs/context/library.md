# Library: sources, switching and saved-track streaming

`crates/library`. Endpoints and events: [api.md](api.md#library-crateslibrary). The save pipeline has its own page: [save-flow.md](save-flow.md). Why two sources: ADR-0003.

## Two sources, one index

- **`LULL_LIBRARY`** = `folder` | `jellyfin` (`config.library.mode`). Unset, it is `jellyfin` when `LULL_JELLYFIN_URL` is set, else `folder`. `jellyfin` without Jellyfin configured stops lull at start-up.
- **One index type** (`index.rs`) answers every read in both modes, keyed by track id. Each entry (`items::Saved`) has a `location`: a Jellyfin item (and its path as Jellyfin sees it), or a file path under `LULL_MUSIC_DIR`. Every `/api/library/*` route and the `Library` seam work the same in both modes.
- **The mode is fixed for a run.** Switching stores the new mode and restarts the hub's crates ([below](#switching)).
- **Background work ends with the crates.** The sync loop, the folder watcher loop, the mirror, the job listener, saves and rescans run under `Ctx::spawn`, so a hub restart stops them before the new crates start theirs. An interrupted save carries on from its step at the next start (`save::resume`); an interrupted rescan is marked failed ("Stopped by a restart"). A switch is not cancelled: it ends by asking for the restart itself.
- **Status.** One `library` status in both modes; the mode leads the detail ("Jellyfin: library online, 2 songs", "Folder: 2 songs") and `extra` is `{ mode, songs }`. Folder mode: `ok`, `warn` when the folder can't be watched, `down` when `LULL_MUSIC_DIR` doesn't exist. Jellyfin mode: [below](#jellyfin). Lists are `stale` in Jellyfin mode whenever it isn't `ok`.

## Folder mode (`folder.rs`, `files.rs`)

- **Indexing.** A walk (walkdir, following links) of `LULL_MUSIC_DIR` reads every audio file (flac, mp3, ogg/oga, opus, m4a, aac, wav, aif/aiff, ape, wv) with lofty. Hidden files and folders are skipped: a save's `.lull-<job>.<ext>` temporary file is never indexed.
- **Persistence.** `library_files` keeps each file's row (modified time, size, whether it embeds a picture, the `Saved` JSON). The next walk reuses a row whose size and modified time haven't changed, so a restart lists at once and re-reads only what changed.
- **Mapping.** Ids follow [Ids](#ids), from the file's tags. An untagged file is listed under its file name and "Unknown Artist". Genres split on `;` and `/`. File facts come from lofty's properties (MP4 counts as ALAC when it reports a bit depth). `added` is the file's modified time.
- **Watching.** `notify` watches the folder recursively. Events are collected for 2 s, then the smallest covering paths are read again: a changed audio file, the folder of a changed cover image, a changed or vanished folder. Whatever was indexed under such a path and is gone is dropped.
  - When the folder can't be watched (no inotify watches left, a filesystem without notifications), the status warns and `POST /api/library/rescan` walks the folder again.
- **Rescan** in folder mode walks the whole folder and reports "Found N new songs".

## Jellyfin

- **Auth.** `LULL_JELLYFIN_API_KEY` in an `Authorization: MediaBrowser Token="…"` header, never in a URL.
- **User.** `LULL_JELLYFIN_USER` (case-insensitive name), else the first administrator. Item reads go through that user (`/Items?userId=`), so they respect its library access.
- **Calls.** `jellyfin.rs` holds every one:
  - `/System/Info` to check the key;
  - `/Users`;
  - `/Items` for audio, paged by 2,000;
  - `/Library/VirtualFolders` for the music folders;
  - `/Items/{id}/Images/Primary` for covers;
  - `/Audio/{id}/stream?static=true` for the original file;
  - `/Items/{library}/Refresh?Recursive=true` to scan the music library after a save;
  - `/Library/Refresh` and `/ScheduledTasks` for a rescan and its progress.
- **Status `library`** (detail prefixed "Jellyfin: ").
  - `warn`: connecting, at start-up.
  - `ok`: "library online, N songs".
  - `warn` after start-up: online, but no Music library over `LULL_MUSIC_DIR` ([below](#music-folder-mapping)).
  - `down`: the error, "showing the last known library".

## The index in Jellyfin mode

- **Every lookup reads it.** That covers lists, `/api/tracks/{id}`, search and `Library::saved` (other crates' saved badges and duplicate notes). None of them makes a Jellyfin call per request (`index.rs`).
- **Refreshing it** (`sync.rs`):
  - **Full read** at start-up, every 30 minutes, after a rescan, and as soon as Jellyfin is back after being down.
  - **While a save waits for Jellyfin:** the 200 newest audio items every 5 s, merged in.
  - **Otherwise:** a key check every 60 s.
- **Persistence.** The last full read is kept in `library_items`. While Jellyfin is down the lists still answer, with `stale: true`, and saved badges stay right.
- **Catalog.** Every read records its tracks in `lull_core::catalog`.
- **One song, several files.** The best copy wins (lossless, then bitrate, then newest).

## Ids

- **Track and artist ids.** A saved song's id is `TrackId::of(artist, album, title)`, where artist is the album artist when there is one. That is what a Soulseek folder gives (`Artist/Album/NN - Title`), so a search result and its saved copy share an id.
- **Compilations.** For an album artist of "Various Artists" (or "VA"), the track artist names the song. The album id still uses the album artist, so the compilation stays one album.
- **Retagged saves** get an alias. See [save-flow.md#ids](save-flow.md#ids).
- **Aliases and own ids.** A lookup tries the id itself first, then its alias. So after switching back to a source, its songs answer under their own ids even where an alias from the other source points away from them.

## Switching (`switch.rs`)

- **Asking.** `PUT /api/library/source { mode }`, or a Jellyfin connect ([below](#connecting-jellyfin)). Refused (409) when the mode is already that, a switch is running, or the environment sets one of the keys it would store (the environment wins over stored settings). Switching to Jellyfin needs it configured first.
- **What a switch does,** in the background, with `library.source` events (`switching`, then `done` or `failed`):
  1. checks that the settings it will store start the hub (`Config::from_vars` over env, stored and the change);
  2. reads the new source: a full Jellyfin read, or a walk of the folder. It is stored (`library_items` or `library_files`) and recorded in the catalog, so the restarted library lists at once;
  3. matches both sources **by path** under `LULL_MUSIC_DIR` (Jellyfin paths through the [music folder mapping](#music-folder-mapping)). A file whose id differs gets an alias old id → new id, and aliases into the old id follow it. An alias that would point at itself is removed;
  4. logs each song of the old source with no file in the new one ("dropped": its likes and plays stay, but it leaves the library views);
  5. stores the keys through `core::settings` and publishes `hub.restart`.
- **Without the mapping** (Jellyfin has no music library over `LULL_MUSIC_DIR`), the switch goes ahead without matching, and says so in the `done` detail.

## Connecting Jellyfin (`connect.rs`)

- **Discovery.** `GET /api/jellyfin/discover` broadcasts "Who is JellyfinServer?" on UDP 7359 and collects answers for 1.5 s, then asks each `/System/Info/Public` for its version. A hub in a container on a bridge network finds nothing: the broadcast doesn't leave the container network; the listener types the address instead.
- **Quick Connect.** `POST /QuickConnect/Initiate` gives a code and a secret; lull keeps the secret → address for 15 minutes. Polling `/QuickConnect/Connect` says `waiting`, `expired` (Jellyfin answers 404) or authenticated; then `/Users/AuthenticateWithQuickConnect` gives the token. Quick Connect switched off answers 409, and the UI offers the sign-in.
- **Sign-in fallback.** `/Users/AuthenticateByName` with the username and password, once. The password is never stored or logged.
- **Either way** lull stores `LULL_JELLYFIN_URL`, `LULL_JELLYFIN_API_KEY` (the access token), `LULL_JELLYFIN_USER` (the user who approved) and `LULL_LIBRARY=jellyfin`, through a switch. Jellyfin ties the token to the device `lull-hub`.

## Jellyfin mirror (`mirror.rs`)

- **Jellyfin mode only,** once the hub hands over the `Collection` seam (`attach_collection`) and Jellyfin's first full read is in.
- **Imported once per server** (`library_mirror` key `imported:<server id>`): Jellyfin's favourite songs become likes, and its audio playlists become lull playlists (songs the index doesn't hold are left out, empty playlists skipped). Each imported playlist is recorded (`playlist:<lull id>` → Jellyfin id), so an import that stopped part-way never makes one twice.
- **Mirrored after that** (Jellyfin 10.9 or later), from events seen after the import:
  - `like`: marks or unmarks the song's Jellyfin item as a favourite (`/UserFavoriteItems/{item}`);
  - `playlist { playlistId, deleted }`: the Jellyfin playlist mirroring it is made (`POST /Playlists`), replaced with the playlist's name and songs (`POST /Playlists/{id}`), or deleted (`DELETE /Items/{id}`).
  - Songs not in Jellyfin (Soulseek-only) are left out. Nothing flows back. A change made while the hub is down, or while Jellyfin is, isn't mirrored until that playlist changes again.

## Mapping (`items.rs`)

- **Basic fields.** `RunTimeTicks` becomes seconds, `IndexNumber` the track number, `ParentIndexNumber` the disc. `ProductionYear` gives the year, with `PremiereDate` as fallback. Genres become `tags`.
- **File facts** come from the first media source and its audio stream:
  - `format` is "FLAC 16/44.1" for lossless, "MP3 320" for lossy (VBR averages within 4 kbps snap to the nominal rate);
  - `codec`, `lossless`, `khz`, `bits` (lossless only), `kbps`, `sizeBytes`.
- **Albums** in library lists are `saved: "all"`. Jellyfin doesn't know a release's full tracklist; discovery's album pages work out `partial`.

## Saved-track streaming

- **Folder mode.** tower-http's `ServeFile`: 200, 206 for one range, 416 past the end, with `Accept-Ranges`, `Last-Modified` and `ETag`. A file gone since it was indexed is 404.
- **Jellyfin mode: the proxy.** `GET /api/library/stream/{id}` requests Jellyfin's static stream with the listener's `Range` header. It passes the status (200, 206 or 416) and `Content-Type`, `Content-Length`, `Content-Range`, `Accept-Ranges`, `ETag` and `Last-Modified` straight through.
- **No buffering.** The body is streamed chunk by chunk. A request may stay open for 4 hours, the length of a paused listener.

## Cover art (`art.rs`)

- **Saved albums, Jellyfin mode.** The cover is Jellyfin's primary image for the album, else the track's embedded one. It is keyed by Jellyfin's image tag, so a changed cover is fetched again.
- **Saved albums, folder mode.** The folder's image file (`cover`, `folder`, then `front`, `.jpg`/`.jpeg`/`.png`, any case), else the song's embedded picture (front cover, else the first). It is scaled down to fit the size asked (JPEG; smaller images and formats lull doesn't decode are served as they are) and cached keyed by the source file's modified time and size.
- **Sizes** are `lull_core::art`'s: 96, 256, 512, 1024 and 2048. Every source is scaled down to fit and never up, so an album whose only cover is 600 px answers 600 px at 1024 and 2048, and `X-Image-Size` says so (Jellyfin's `maxWidth`/`maxHeight` don't enlarge either). The `Library` seam's `cover` (Subsonic's `getCoverArt`) takes any size and serves the smallest of these that covers it.
- **When Jellyfin has no cover, or the album isn't saved**, it comes from the Cover Art Archive: front-250 for 96 and 256, front-500 for 512, front-1200 for 1024 and the original for 2048. A thumbnail the archive doesn't have falls back to the original, so a missing size never reads as "no cover". lull finds the release group by:
  - its MusicBrainz id, when Jellyfin knows it;
  - otherwise by name: `catalog::track_on_album` gives artist and album, then a MusicBrainz release-group search. Hits scoring 90 or more are tried in turn, albums first, and the first with a cover wins. A self-titled EP or single scores as high as the album and often has no cover.
- **Disk cache.** Covers live in `<LULL_DATA_DIR>/art/`, one file per album, size and source. "No cover anywhere" is remembered for a week, but only once every candidate was asked. An album the catalog doesn't know yet answers 404 without being remembered. A cover costs at most one MusicBrainz search per album and size.
- **Never a slow request.** A page asks for many covers at once, and a browser opens only six connections per host. So a request waits at most 3 s for MusicBrainz and the archive, then answers 404 "not yet" (`no-store`), and the lookup finishes in the background, caches its answer and announces a found cover with `art.ready` (api.md#events), so the page asks once more then, not on a timer. A cover asked for during the minute's pause after a failed lookup (or whose lookup failed) is looked up once the pause ends. "No cover" answers 404 with `Cache-Control: private, max-age=` the rest of its week. The archive's 250 px thumbnail makes both 96 and 256 (one lookup for rows and tiles).
- **Fetched before anyone asks** (`art::warm`, a `lull_core::warm` queue, 2 at a time): the playing song's album at 256, 512 and 1024 and the next 3 songs' at 256 and 512 (`device.state`, albums without a catalog picture, which discovery warms), the saved albums discovery hands out without one (`Library::warm_covers`), and every saved album's 96 and 256 px cover once per start, one at a time while nothing more urgent waits. Requests for the same cover share one lookup. A failed lookup (either service unreachable) caches nothing and pauses new lookups for a minute: during that time, uncached covers answer 404 at once.
- **Ids are checked** (`al-` plus hex) before they become file names.

## Rescan and jobs

- **Rescan** (Jellyfin mode). `POST /api/library/rescan` starts Jellyfin's "Scan media library" task (all libraries, like Jellyfin's own button). The task's progress becomes the job's `pct`. When it ends, the library is read again and the job reports how many new songs it found.
- **Activity.** `GET /api/jobs` lists library's save and rescan jobs (`library_jobs`) and the latest `job` event of every other job seen since start-up. Crates that want rows in Activity publish `job` events.
- **Clean-up.** Finished jobs older than 7 days are dropped at start-up.

## Music folder mapping

- **Why.** Jellyfin and lull may mount the music at different paths (`/media/music` in Jellyfin, `/music` in lull).
- **How.** lull takes the music library's folder when Jellyfin has exactly one. With several, it takes the one whose files it can see under `LULL_MUSIC_DIR`.
- **Uses.** The mapping:
  - picks the music library a save asks Jellyfin to scan;
  - finds the old file for "replace if better".
  - matches Jellyfin items with music-folder files when switching.
- **Without it,** saves ask for a full library scan instead, and the `jellyfin` status is `warn` ("no Music library over LULL_MUSIC_DIR"): saved songs can't show up until Jellyfin has a Music library over that folder.
