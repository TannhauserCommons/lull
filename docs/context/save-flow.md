# Save flow

`crates/library/src/save.rs`. Endpoints and events: [api.md](api.md#library-crateslibrary).

**What a save does.** It turns a Soulseek song into a properly tagged file in Jellyfin's music folder. It is one job with five steps. Each step's result is stored on the job (`library_jobs`), so a restart or a retry carries on from where it stopped. Every change is published as a `save` event and a `job` row.

| Step | What | Fails as |
|---|---|---|
| 1 Completing transfer | `Transfers::complete(track, source)`. The source is the listener's pick, else `Transfers::source_for` (the peer it streamed from, else the best known). Soulseek's `transfer` events for the track become the job's `pct` | `transfer`: no peer known, or the transfer failed. Retry with another `source` |
| 2 Matching on MusicBrainz | Recording search, then scoring (below). Skipped when the listener already picked a release | `match`: no sure match. The release picker (`/releases`) or "keep existing tags" |
| 3 Fetching cover art | Release lookup (tags) and the Cover Art Archive front, 1200 px: the release's, else the release group's | Never fails on art: no cover sets `nocover: true` and the save carries on. A failed release lookup is `match` |
| 4 Tagging and writing | Copy, tag, rename into place (below) | `write`: unreadable file, no permission, disk full |
| 5 Jellyfin rescan | Ask Jellyfin to scan the music library; the job is `done` once the song is in the library | Never fails: the file is saved, the job is `pending` (below) |

## Duplicates

- **Before saving,** the UI asks `POST /api/library/duplicates`. The duplicate dialog's answer comes back in `POST /api/saves` as `duplicate: { id: separate | replace }`.
- **Keep** means the id isn't sent. A saved id sent without an answer is skipped (`skipped`).
- **`better`** means lossless over lossy, or at least a fifth more bitrate of the same kind. The offered file's bitrate is worked out from its size and the song's length. The margin keeps two rips of the same CD from counting as an upgrade.
- **`separate`** writes a second file. If the name is taken, it gets " (2)".
- **`replace`** removes the saved copy once the new file is in place, or overwrites it when both have the same path. The old file is found through the music folder mapping ([library.md](library.md#music-folder-mapping)). If it can't be found, the save fails as `write` and nothing is removed.

## Matching

- **What lull looks for.** The file's own tags come first; the search result recorded in the catalog fills the gaps. That gives a title, artist, album, length and track number.
- **The search.** MusicBrainz is asked for recordings by title and artist, 25 at most. Every (recording, release) pair is scored 0-1 over the parts that are known:

  | Part | Weight |
  |---|---|
  | title | 0.35 |
  | artist | 0.25 |
  | album | 0.20 |
  | length | 0.20 |
  | track number | 0.05 |

  - **Names** are compared by edit distance after `ids::normalize`. A bracketed suffix ("[2024 FLAC]", "(Deluxe)") costs 8% instead of everything.
  - **Length** scores 1 within 2 s and 0 beyond 15 s. That is what tells a live take or an edit from the studio recording.
  - **Track number** weighs 0.05: 1 when the release lists the song at the file's number, else 0.
  - **What MusicBrainz doesn't know** counts as half agreement (0.5), never as a match. A recording without a length must not outscore the one whose length agrees.
- **Sure** means the best score is at least 0.85 and leads every other release group by 0.08 or more.
  - Releases of the same album tie by design. Among them the official, earliest release wins, then the lowest id, so the choice is stable.
- **Not sure** fails step 2 as `match` with up to 8 candidates, one per release, for the picker.
  - Without an album name (a loose file), an album, its single and a best-of tie. The listener picks.
- **A resumed save whose transfer is gone.** Soulseek keeps the finished transfer pinned only until the save fails, pends or finishes. A save resumed from step 2-4 (a picked release, a retry) whose file was evicted transfers it again first; a chosen release is kept.
- **Pacing.** MusicBrainz allows one request per second per IP, shared by every crate: `Http::pace("musicbrainz", MUSICBRAINZ_PACE)`. A save makes two requests (search, release lookup).

## Tags

**After a match,** these keys go into the file's primary tag (ID3v2, Vorbis comments, MP4...):

- title, artist (the track's credit with join phrases), album, album artist;
- track/total, disc/total, date (the release's), original date (the release group's first);
- genre (the release's top 5, written as one "a; b" value), label;
- MusicBrainz recording, release track, release, release group, artist and album artist ids;
- the front cover.

**What else changes:**

- Other keys in the old tag stay (ReplayGain, lyrics, ...).
- Other tag types are removed (ID3v1, APE on MP3), so nothing contradicts the new values.

**"Keep existing tags"** leaves the tags alone. Only a missing title, artist or album is filled from the search result, so Jellyfin lists the song under the name it was saved as. No MusicBrainz call, no cover: `nocover` unless the file already embeds one.

## Naming

- **The template.** `LULL_NAMING`, default `{albumartist}/{album} ({year})/{disc}{track:02} - {title}`, relative to `LULL_MUSIC_DIR`. It is checked at start-up: an unknown field, `.` or `..`, a leading `/` or a bad padding stops lull.
- **Fields.**
  - `artist`, `albumartist` (falls back to artist), `album`, `title`;
  - `year`: the original release year;
  - `track`;
  - `disc`: "2-" on a release with more than one disc, else empty, so the default reads "05 - …" or "2-05 - …";
  - `ext`.

  Numbers take `:02` to `:04`. Without `{ext}`, ".ext" is appended.
- **Sanitising.** A value never adds a folder:
  - `/ \ : * ? " < > |` and control characters become `_`;
  - brackets left empty by a missing value disappear ("Album ()" → "Album");
  - a leading `.` becomes `_` (no hidden folders);
  - trailing dots go;
  - Windows device names get `_`;
  - an empty name becomes `_`;
  - a component is cut at 200 bytes.
- **No overwrite surprises.** An existing file is never overwritten, except by "replace" at the same path. Otherwise the new file gets " (2)", " (3)"...
- **Writing.** The transfer is copied next to its destination as a hidden `.lull-<job>.<ext>`, which Jellyfin ignores. lull tags the copy, then renames it into place: Jellyfin never sees a half-written or untagged file. The transfer itself stays with Soulseek's stream cache.

## Jellyfin pick-up and `pending`

- **Asking for a scan.** Step 5 refreshes the Music library over `LULL_MUSIC_DIR` (`/Items/{library}/Refresh?Recursive=true`, the library's own "Scan library"). Without the music folder mapping, lull starts a scan of every library instead.
  - Not `/Library/Media/Updated`: Jellyfin ignores those notifications on libraries without realtime monitoring, which is off by default on many servers.
  - **Batched.** Saves ask at most once every 30 s: one scan covers every file written before it starts, so an album's songs share a few scans instead of queueing one each.
  - **Full scan as the fallback.** If the song isn't in the library 60 s after the library refresh, lull starts Jellyfin's full scan once ("Scan media library"). Seen live: a Music library that was never scanned has no folders to refresh yet, so its refresh finds nothing, while a full scan picked the file up in 3 s.
- **Done.** The sync loop polls the newest items every 5 s. The job is `done` once a song with the saved id is in the library.
- **`pending`** means the file is saved, Jellyfin hasn't got it yet, and it is retried by itself:
  - **Jellyfin couldn't be asked** (down, refused): asked again by the first check after Jellyfin is back (checks run every minute).
  - **Jellyfin was asked but doesn't show the file after 5 minutes:** the song appears with the next full library read (every 30 minutes, or after a rescan).
  - `POST /api/saves/{job}/retry` asks again now.

## Ids

- **Why a save can change a song's id.** Tagging may change artist, album or title ("OK Computer [1997 FLAC]" → "OK Computer"), and with them the content-derived id.
- **What lull records** when the saved id differs from the requested one (`library_aliases`):
  - the alias (asked id → saved id);
  - once the song is in Jellyfin, the saved track under the asked id in the catalog.
- **Effect.** `Library::saved([asked])`, `/api/tracks/{asked}` and `/api/library/stream/{asked}` answer with the saved song, under the asked id. The badge flips in place, and likes, plays and playlists on the old id keep working.

## Album and queue saves

- **Many jobs.** "Save album" and "save queue" are one `POST /api/saves` with many ids. Each id gets its own job, and the UI totals their progress.
- **Concurrency.** Jobs run at the same time. Transfers queue in Soulseek, and MusicBrainz calls queue on the shared pacer.
