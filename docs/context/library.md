# Library: Jellyfin reads and saved-track streaming

`crates/library`. Endpoints and events: [api.md](api.md#library-crateslibrary). The save pipeline has its own page: [save-flow.md](save-flow.md).

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
  - `/Library/Media/Updated` to announce a saved file;
  - `/Library/Refresh` and `/ScheduledTasks` for a rescan and its progress.
- **Status `jellyfin`.**
  - `off`: not configured.
  - `warn`: connecting, at start-up.
  - `ok`: "Library online, N songs", with `extra: { songs }`.
  - `warn` after start-up: online, but no Music library over `LULL_MUSIC_DIR` ([below](#music-folder-mapping)).
  - `down`: the error, "showing the last known library".

## The index: lull's copy of the library

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

## Mapping (`items.rs`)

- **Basic fields.** `RunTimeTicks` becomes seconds, `IndexNumber` the track number, `ParentIndexNumber` the disc. `ProductionYear` gives the year, with `PremiereDate` as fallback. Genres become `tags`.
- **File facts** come from the first media source and its audio stream:
  - `format` is "FLAC 16/44.1" for lossless, "MP3 320" for lossy (VBR averages within 4 kbps snap to the nominal rate);
  - `codec`, `lossless`, `khz`, `bits` (lossless only), `kbps`, `sizeBytes`.
- **Albums** in library lists are `saved: "all"`. Jellyfin doesn't know a release's full tracklist; discovery's album pages work out `partial`.

## Saved-track streaming

- **The proxy.** `GET /api/library/stream/{id}` requests Jellyfin's static stream with the listener's `Range` header. It passes the status (200, 206 or 416) and `Content-Type`, `Content-Length`, `Content-Range`, `Accept-Ranges`, `ETag` and `Last-Modified` straight through.
- **No buffering.** The body is streamed chunk by chunk. A request may stay open for 4 hours, the length of a paused listener.

## Cover art (`art.rs`)

- **Saved albums.** The cover is Jellyfin's primary image for the album, else the track's embedded one. It is keyed by Jellyfin's image tag, so a changed cover is fetched again.
- **When Jellyfin has no cover, or the album isn't saved**, it comes from the Cover Art Archive (front-250 for 96/256, front-500 for 512). lull finds the release group by:
  - its MusicBrainz id, when Jellyfin knows it;
  - otherwise by name: `catalog::track_on_album` gives artist and album, then a MusicBrainz release-group search must score 90 or more.
- **Disk cache.** Covers live in `<LULL_DATA_DIR>/art/`, one file per album, size and source. "No cover anywhere" is remembered for a week. A cover costs at most one outside call per album and size.
- **Ids are checked** (`al-` plus hex) before they become file names.

## Rescan and jobs

- **Rescan.** `POST /api/library/rescan` starts Jellyfin's "Scan media library" task (all libraries, like Jellyfin's own button). The task's progress becomes the job's `pct`. When it ends, the library is read again and the job reports how many new songs it found.
- **Activity.** `GET /api/jobs` lists library's save and rescan jobs (`library_jobs`) and the latest `job` event of every other job seen since start-up. Crates that want rows in Activity publish `job` events.
- **Clean-up.** Finished jobs older than 7 days are dropped at start-up.

## Music folder mapping

- **Why.** Jellyfin and lull may mount the music at different paths (`/media/music` in Jellyfin, `/music` in lull).
- **How.** lull takes the music library's folder when Jellyfin has exactly one. With several, it takes the one whose files it can see under `LULL_MUSIC_DIR`.
- **Uses.** The mapping:
  - announces a saved file's own folder to Jellyfin;
  - finds the old file for "replace if better".
- **Without it,** saves ask for a full library scan instead, and the `jellyfin` status is `warn` ("no Music library over LULL_MUSIC_DIR"): saved songs can't show up until Jellyfin has a Music library over that folder.
