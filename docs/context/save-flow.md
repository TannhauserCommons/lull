# Save flow

`crates/library/src/save.rs`. Endpoints and events: [api.md](api.md#library-crateslibrary).

**What a save does.** It turns a Soulseek song into a properly tagged file in the music folder (`LULL_MUSIC_DIR`), in both library modes. It is one job with five steps. Each step's result is stored on the job (`library_jobs`), so a restart or a retry carries on from where it stopped. Every change is published as a `save` event and a `job` row.

| Step | What | Fails as |
|---|---|---|
| 1 Completing transfer | `Transfers::complete(track, source)`. The source is the listener's pick, else the saving profile's ([below](#picking-the-copy)). Soulseek's `transfer` events for the track become the job's `pct` | `transfer`: no peer shares it, no copy passes the minimum under `ask`, or the transfer failed. Retry with another `source` |
| 2 Matching on MusicBrainz | Recording search, then scoring (below). Skipped when the listener already picked a release. When MusicBrainz lists nothing like the song, the save goes on with the file's own tags (as "keep existing tags") | `match`: no sure match among several candidates. The release picker (`/releases`) or "keep existing tags" |
| 3 Fetching cover art | Release lookup (tags) and the Cover Art Archive front, 1200 px: the release's, else the release group's | Never fails on art: no cover sets `nocover: true` and the save carries on. A failed release lookup is `match` |
| 4 Tagging and writing | Copy, tag, rename into place (below) | `write`: unreadable file, no permission, disk full |
| 5 Pick-up | Jellyfin mode: ask Jellyfin to scan the music library; the job is `done` once the song is in the library. Folder mode: the folder index reads the file at once | Jellyfin mode never fails: the file is saved, the job is `pending` (below). Folder mode: `write` if the file can't be read back as the song; a retry only reads it again |

## Picking the copy

Without the listener's pick, step 1 asks Soulseek for the saving profile's copy (`Transfers::save_candidate`, ADR-0008). Soulseek searches when it knows no copy, and reuses the one the song streamed from when that passes ([soulseek.md](soulseek.md#source-rules)). When no copy passes the saving minimum, the profile's `below` decides:

- **`saveBestUpgrade`** (default): the best copy is saved now, the job says `belowMinimum`, and the song is watched for an upgrade.
- **`wait`**: the job is `waiting` at step 1 and searches again every 30 minutes (paced with auto-upgrade, at most one search a minute) until a copy passes. A restart carries on waiting. `DELETE /api/saves/{jobId}` gives up on it (and on a failed save): one `cancelled` event, then the job is gone.
- **`ask`**: step 1 fails as `transfer` ("No copy meets your saving minimum (320 kbps or lossless). Pick a source and retry."); the picker lists the copies with `GET /api/tracks/{id}/sources?for=save`.

With no peer sharing the song at all, `wait` waits too; the others fail as `transfer`.

## Auto-upgrade

`upgrade.rs`, ADR-0008.

- **What is watched.** A save that finishes with a file below the saving minimum is recorded in `library_upgrades`, with its extension and the release it was tagged from. One that passes is forgotten. Songs lull didn't save are never touched.
- **Searching again.** While `autoUpgrade` is on, every minute the most overdue song is checked: still saved, still below the minimum, no save of it running. Each song is searched every `upgradeEveryH` hours (24 by default), and all upgrade and `wait` searches share one pacer: at most one a minute.
- **Replacing.** A copy that passes the minimum and clearly beats the saved file (`Quality::beats`) starts a save job with `upgrade: true` (an Activity row like any save) and `duplicate: replace`. It keeps the saved release, so matching is skipped and the tags and cover come from the same release; "keep existing tags" saves fill blanks from the saved song. At the same path the file is overwritten, so Jellyfin keeps its entry; another extension writes a new file and removes the old one.
- **After the upgrade** the song is judged again: forgotten when it now passes, else searched again later.

## Duplicates

- **Before saving,** the UI asks `POST /api/library/duplicates`. The duplicate dialog's answer comes back in `POST /api/saves` as `duplicate: { id: separate | replace }`.
- **Keep** means the id isn't sent. A saved id sent without an answer is skipped (`skipped`).
- **`better`** means lossless over lossy, or at least a fifth more bitrate of the same kind (`lull_core::quality::Quality::beats`, the one rule soulseek and auto-upgrade use too). The offered file's bitrate is worked out from its size and the song's length. The margin keeps two rips of the same CD from counting as an upgrade.
- **`separate`** writes a second file. If the name is taken, it gets " (2)".
- **`replace`** removes the saved copy once the new file is in place, or overwrites it when both have the same path. The old file is the index entry's own path in folder mode, and found through the music folder mapping in Jellyfin mode ([library.md](library.md#music-folder-mapping)). If it can't be found, the save fails as `write` and nothing is removed.

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
- **Nothing at all** (MusicBrainz has no recording like it, common for small or local artists) isn't a question for the listener: the save continues with the file's own tags, the search result filling the gaps, and the file's own cover if it has one (`nocover` otherwise).
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

**"Keep existing tags"** leaves the tags alone. Only a missing title, artist or album is filled from the search result, so the library lists the song under the name it was saved as. No MusicBrainz call, no cover: `nocover` unless the file already embeds one.

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

## Folder pick-up

- **Step 5 in folder mode** reads the written file into the index straight away (`folder::pick_up`), without waiting for the watcher, and the job is `done`. The watcher sees the same file a moment later; reading it again changes nothing.
- **A restart during step 5,** and saves left `pending` by Jellyfin mode before a switch, are picked up from disk at start-up.

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
  - once the song is in the library, the saved track under the asked id in the catalog.
- **Effect.** `Library::saved([asked])`, `/api/tracks/{asked}` and `/api/library/stream/{asked}` answer with the saved song, under the asked id. The badge flips in place, and likes, plays and playlists on the old id keep working.

## Album and queue saves

- **Many jobs.** "Save album" and "save queue" are one `POST /api/saves` with many ids. Each id gets its own job, and the UI totals their progress.
- **Concurrency.** Jobs run at the same time. Transfers queue in Soulseek, and MusicBrainz calls queue on the shared pacer.
