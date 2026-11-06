# YouTube: a second source through yt-dlp

`crates/youtube` finds the YouTube upload that is the song and downloads its audio; `crates/soulseek` decides when YouTube plays and writes the audio into the stream cache, so preloading, Range seeking, resume, pins and eviction treat it like a peer's file. Why it works this way: [ADR-0012](../adr/0012-youtube-as-a-second-source.md). Endpoints: [api.md](api.md#youtube-cratesyoutube).

## Map

| File | Holds |
|---|---|
| `core/src/youtube.rs` | The settings, `Copy` (how a YouTube copy is named as a `SourceRef`), its quality chip |
| `core/src/seams.rs` | `Youtube`: `find(track, want)`, `download(source, dest)`, `playlist(url, from, count)`, `checked(track, source)` and `check_file(track, path)`; `Previews` (discovery's, the references) |
| `youtube/src/search.rs` | Searching, confirming by full extraction, remembered matches, kept extractions |
| `youtube/src/matching.rs` | Whether an upload is the song, and how good a copy it is |
| `youtube/src/check.rs` | Sound checks: references, verdicts, the background check of official copies, AcoustID ([Sound checks](#sound-checks)) |
| `youtube/src/sound.rs` | Decoding, fingerprinting (Chromaprint's algorithm) and aligning a preview in a copy |
| `youtube/src/formats.rs` | Which audio stream to take |
| `youtube/src/cut.rs` | A full-album upload's track list, the song in it, and cutting its audio out ([Full-album uploads](#full-album-uploads)) |
| `youtube/src/playlist.rs` | Listing a playlist for link imports |
| `youtube/src/ytdlp.rs` | Running yt-dlp, its errors in plain words |
| `youtube/src/update.rs` | Keep yt-dlp up to date |
| `youtube/src/lib.rs` | The seam, back-off, the `youtube` status |
| `soulseek/src/fetch.rs` | The modes: `route`, `youtube_first`, `soulseek_then_youtube`, `youtube_attempt`, upgrades |

## Settings

`settings.sources.youtube` (collection stores it; soulseek and youtube read it from the `settings` event, ADR-0050). Settings › Sources › YouTube.

| Key | Default | Meaning |
|---|---|---|
| `mode` | `off` | `off`: never. `fallback`: when Soulseek has no playable copy, or none starts within `waitS`. `startFast`: YouTube starts the song at once; Soulseek's best copy is fetched for later plays and saves |
| `format` | `opus` | `opus` (WebM, itag 251 ≈ 160 kbps) or `aac` (M4A, itag 140 ≈ 128 kbps), YouTube's own stream either way |
| `match` | `close` | `official`: YouTube Music songs, Topic and VEVO channels, the artist's own channel, uploads with music metadata. `close`: also other uploads, length within 5 s. `loose`: length within 15 s |
| `waitS` | 20 | `fallback`: seconds Soulseek gets to start sending, 5-120 |
| `saves` | false | Saves may take a YouTube copy when no Soulseek peer shares the song |
| `autoUpdate` | true | Keep yt-dlp up to date |
| `contribute` | false | Contribute to AcoustID: a song the listener confirms that AcoustID didn't know is submitted (its whole fingerprint, length, the catalog's artist, title and album) under `LULL_ACOUSTID_USER_KEY`; needs `LULL_ACOUSTID_KEY` too ([Sound checks](#sound-checks)) |

## Modes

`fetch.rs::route` decides per fetch:

- **A save** gets the copy it named. `Transfers::save_candidate` never names a YouTube copy while any Soulseek copy is known; with `saves` on and none known, it names YouTube's AAC copy (the save pipeline can't tag WebM) as meeting the minimum, so `wait` doesn't hold it back. Auto-upgrade judges the saved file and replaces it once a peer has a passing copy.
- **The listener's pick** of a peer plays that peer. Pick a source lists the song's YouTube copy too while YouTube is on (any mode but `off`): one row after the peers, its chip ("YouTube · Opus 160"), "lossy", no score and never below the minimum; the copy the streaming path would play, matched in the background when the picker opens (`done: false` until then). Picking it plays YouTube (`?peer=YouTube&path=`), checked against the song's match.
- **`fallback`:** Soulseek as usual; YouTube when it fails (no peer shares it, none passes and the `below` rule gives up, every peer failed) or when no peer is sending `waitS` after the fetch began (still finding, queued, or between peers). The Soulseek attempt is cancelled in slskd first. Preloads do the same.
- **`startFast`:** the player's fetch finds the YouTube copy, starts an **upgrade** fetch, and plays YouTube. With "Switch to a better copy" on, the upgrade is the better copy by the streaming profile (its cap kept: a 320 cap takes the MP3 320 over a FLAC), the one the player moves to mid-song; off, the saving profile's, for later plays and saves. If YouTube has no match or fails, the upgrade is dropped and the player's fetch looks on Soulseek itself; when that fails too, the error says why for both ("no peer shares this song right now; YouTube: no YouTube upload matches this song"). A preload fetches Soulseek's copy (YouTube's only when Soulseek has none) and warms the YouTube match, so playing it still starts at once.

**The current play never jumps.** Upgrades live in their own map, out of the player's way: a seek joins the YouTube fetch, and the upgrade's bytes go to the cache only. A save started while YouTube plays the song runs there too. A preload still waiting for a peer when the song is played steps aside as the upgrade. A YouTube copy that has played through (its length plus a minute after it began, nobody reading) gives way to a cached Soulseek copy on the next play; until then a request is the same play seeking.

**Switching to the better copy.** With Settings › Sources › "Switch to a better copy while a song plays" on (the default), a song YouTube plays moves to Soulseek's copy once it is ready, lined up and crossfaded so nothing jumps ([streaming.md](streaming.md#better-copies)). In `fallback` the copy is fetched by the streaming profile as soon as YouTube steps in; in `startFast` it is the upgrade above, offered when it passes the streaming minimum and cap. With the setting off, or when the player can't line the copies up, the rule above holds: it plays from the next play.

**Later plays.** `Fetcher::best_cached` prefers a Soulseek copy over a YouTube one. With the mode off, cached YouTube copies are ignored; otherwise one plays at once (and `startFast` starts an upgrade beside it).

## Badges and quality

A YouTube copy's chip is its own: `YouTube · Opus 160`, `YouTube · AAC 128` (`Copy::facts`), with `peer: "YouTube"` and `youtube: true` in `stream` events. It is exempt from the minimum (`belowMinimum` is always false). The streaming cap (`stream.maxKbps`) picks the best stream within it (YouTube's average bitrate may run 10 % over), else the smallest.

## Matching

`find` remembers its answer per song and strictness in `youtube_matches` (a match for 30 days, no match for a day; a migration forgets the misses when the rules below find more). A match with no audio lull can stream is an error that says so, not a miss.

1. **YouTube Music songs** for `"artist" title`, the name quoted so it reads as one name (unquoted, "Germany Germany LENA" was the country and the singer Lena, and the song wasn't among the 5) (`music.youtube.com/search#songs`, first 5, no extraction): those whose title matches are extracted, at most 2, and judged on the full facts.
2. **Videos** (`ytsearch8:`, first 8 with channel and length): judged, best first, then those only their sound could take (`plausible`); at most 3 extracted and judged again.
3. **Full-album uploads**, when neither found the song ([below](#full-album-uploads)).

Each extracted upload then goes through `search::consider`: official and taken by the words, it is the match at once, and its sound is checked while it plays; any other the words take, or that could be the song (`plausible`: no other-version words, length within a quarter, the song's title in its title), is checked first ([Sound checks](#sound-checks)), at most 2 per search. An upload found not to be the song is never taken for it.

An upload is the song when (`matching.rs`):
- **the title matches** after `split` reads it (packaging brackets like "(Official Audio)", a translation's "(Sub.Español)" and quality tags like "HD 1080p" dropped, a " | …" tail and a leading "<show> Soundtrack - " dropped, an "Artist - " or "Artist • " prefix split off, a repeated artist and whole-title quotes dropped), with `ids::loose_title` and one slip per 8 letters; numbers must agree. A fan's "Title - Artist" reads the other way round when only that reading is the song and its artist (Clairo's "mount airy, georgia" is on YouTube only as such uploads);
- **the artist matches** YouTube's music metadata, the title's artist part, the channel ("- Topic", "VEVO", " Official", " Music" dropped), or the title itself;
- **the length agrees** within the strictness's slack;
- **it is no other version**: live (or a recorded live stream), cover, nightcore, sped up, slowed, reverb, 8D, karaoke, instrumental, remix, acoustic, piano, reaction and the like are refused unless the song's artist, title or album has the word.

`split` is the one reading of an upload's title: `read_upload` (the same, plus music metadata and Topic channels, and whether the length is the song's) gives the artist and title of each song `Youtube::playlist` lists, which collection's link imports and outside playlists take as they are.

Score: official +40, YouTube Music +10, exact title +5, minus 3 per second off, a music video (not audio) −10.


**Search's last gap-filler** (`Youtube::search_songs`, `matching::songs_of`): one flat video search (10 uploads, no extraction), each upload read as playback reads it (`read_upload`), a fan's "Title - Artist" turned round when the other uploads put its title part first more often (playback has the song it wants to tell; a search has the other uploads), and kept only when `judge` takes the upload for the song it reads as (close: the length its own, other versions refused). What search offers is what playback plays.
## Sound checks

Whether a copy is the song by its sound ([ADR-0016](../adr/0016-checking-a-copy-by-its-sound.md); measured with `tests/fingerprint-trial`).

- **References, in order** (`check::references`): the listener's own copy (copies confirmed with "Right song? Yes", the saved library file, `Library::stream`, its first 4 minutes); the catalog's previews (below); two or more of the song's finished Soulseek copies that match each other (`Copies::finished`, soulseek's cache); and, when no catalog has a preview, any one Soulseek copy and the artist's own uploads that played (an official upload's print is kept once it has been checked on the file that played). A copy is never its own reference. Their prints are kept in `youtube_prints` (`listener:<track>:<copy>`, `library:<track>`, `soulseek:<peer>\n<path>`, `youtube:<track>:<video>`).
- **The catalog's previews:** its own 30 s previews of the track (`Previews`, implemented in `discovery/src/search/previews.rs`): the Apple row, then the Deezer row, whose artist, album and title make the track's id, as search made it. A row found by name alone isn't one: Apple holds re-recordings under the same title. Each lookup waits for spare catalog calls (iTunes `Need::Background`) and is kept a week; the previews' fingerprints are kept per track in `youtube_references` a month (none: a day).
- **The check:** the copy is decoded and fingerprinted (symphonia; YouTube's Opus through ruopus; rusty-chromaprint), and each preview slid over it for the offset with the fewest differing bits (at least 15 s overlapping); the score is that bit error rate. Under 0.15: the recording, taken however it is titled. 0.15 to 0.25: another master or mix, taken when the words agree (`judge`). 0.25 and over: another recording, never taken. The words still refuse what the sound can't hear: an instrumental or karaoke of the record sounds the same (`check::takes`).
- **When:** a fan upload is checked before `find` gives it, on its AAC copy (the save format, itag 140) downloaded into `<data>/youtube/check` and removed after. An official upload the words take plays at once and is checked on the file that plays, once soulseek has downloaded it (`Youtube::checked`, `check::played`), so it costs YouTube nothing more; the attempt waits for that verdict. A miss drops the song's match and fails that attempt with "the YouTube copy isn't the song by its sound"; the fetch takes the next copy `find` gives, a new attempt the player reopens the song on. A copy the listener picked plays as it is.
- **Verdicts** are kept per (track, upload) in `youtube_checks` (`same`, `near`, `other`, `unchecked`, with the score and the reference) a month; `unchecked` a day.
- **No reference** (a store that answers nothing, as Apple's search does for Korea's storefront; a song only fans uploaded, with no Soulseek copy): the words decide, as before, and a YouTube copy they took plays **unverified**: `stream` events carry `unverified: true`, and the player bar shows "Unverified. Right song? Yes / No", quietly, ignorable. `POST /api/tracks/{id}/verify { answer }` answers for the YouTube copy playing: yes keeps a `same` verdict and the copy's print as the song's reference (`listener:`); no keeps `other`, drops the match and the cached copy, and the player opens the song again at the same time, on the next copy. A copy the listener picked is never asked about. After 10 tracks looked up with no reference and none with one, the `youtube` status says sound checks are idle.
- **AcoustID** (`LULL_ACOUSTID_KEY`, optional): with no catalog reference, the upload's whole fingerprint is looked up (at most 3 requests a second); AcoustID sure (score 0.8+) of a recording with the song's title and artist is the recording, sure of other names only is another recording. Without the key, skipped.
- **Soulseek copies:** a file that arrived is checked out of the way (`Youtube::check_file`, its first 4 minutes, about 0.3 s of one core): one that isn't the song is never offered for it again (`Sources::mark_wrong`) and leaves the cache. The play goes on.
- **Cost:** about 0.2 s of one core per AAC check (a fan upload, plus its 4 MB download), about 1.2 s per Opus one (an official copy, no download); a preview is about 1 MB, once per track. YouTube asked this project's workstation for a sign-in check after about 500 downloads in two hours (the trial): checks never add a download for an official copy for that reason.

## Full-album uploads

Some songs are on YouTube only inside an upload of their whole album (Mabisyo's "What Are We?"). The last matching step (`search::album`, `cut.rs`) searches `ytsearch5:artist album` and takes uploads at least twice the song's length (and 10 minutes) whose title has the album's; at most 2 are extracted, and the one whose artist agrees (official only, under Official) gives its track list: YouTube's chapters, else the timestamps in its description, each running to the next. The song is the one entry whose title matches (numbering like "03." dropped) and whose length agrees within the strictness's slack; no entry, or two, and the upload is passed over. Its bounds are the evidence, so a cut is never sound-checked and never "Unverified" (ADR-0016's update of 2026-11-05).

The match keeps its section (`youtube_matches.section_from`, `section_to`, in ms), and the copy names it: `youtube:<video>~<from_ms>-<to_ms>:140.m4a`. The album's AAC (itag 140) is downloaded once into `<data>/youtube/albums/<video>.m4a`, one download per video however many of its songs play at once, and kept a day; each song is its AAC frames copied out into an M4A of its own (`lull_core::m4a`: one track, its index before the audio), with no re-encoding (frame-accurate, about 23 ms), in `albums/<video>~<from>-<to>.m4a`, then into the stream cache like any copy. Measured 2026-11-05: a 40-minute album is 39 MB in about 3 s, a song's cut a few milliseconds. So a cut is like any YouTube AAC copy: saved as `.m4a`, tagged in MP4's own tags, cover included, read by the scan and by ffprobe (which Jellyfin uses), and it seeks by its index. Until 2026-11-06 a cut was raw AAC (ADTS, `.aac`). Tagged, that didn't decode in Chrome, and its length was only estimated. A cut an earlier lull named `.aac` still plays, and it's still written as ADTS.

## Formats

`formats.rs`: audio-only HTTPS streams with an exact `filesize` (the cache serves byte ranges against it while the download runs), no DRM, not YouTube's `-drc` copies, the original language track. Opus is WebM, AAC is M4A: `content-type` `audio/webm` or `audio/mp4`. A song cut from a full-album upload is M4A too (an earlier lull's `.aac` cut is ADTS, `audio/aac`).

## Downloads

`download` runs `yt-dlp --load-info-json <kept extraction> -f <format> -o -` and writes stdout to `<LULL_TRANSFERS_DIR>/youtube/<video>.<format>.<ext>` chunk by chunk, so the file grows front to back like slskd's. Extractions are kept 4 h (YouTube's stream URLs last about 6); an older one, or one whose URLs stopped working, is redone once. The fetch watches the file every 250 ms (`queued → buffering → streaming`), records it in `soulseek_files` once bytes arrive, and fails over the stall timeout after a 30 s grace. A finished copy is used from the cache as is. Cancelling drops the download, which kills yt-dlp; a partial file stays recorded and evictable. Measured on 2026-11-02 from this workstation: a cold song 6-7 s to find (search, extraction), then first bytes in about 1 s and a whole song in under 2 s.

## yt-dlp

- **Pinned in the image:** the standalone `yt-dlp_linux` (no Python) and Deno, the JavaScript runtime yt-dlp needs for YouTube, by version and SHA-256 in the `Dockerfile`; `LULL_YTDLP` and `LULL_DENO` point at them. `yt-dlp_linux` needs zlib, copied from the build stage. The image build runs both.
- **Every run:** `--ignore-config --no-update`, no cookies or accounts, `--retries 3 --extractor-retries 1`, `--js-runtimes deno:<LULL_DENO>`, a cache in `<data>/youtube/cache`; `TMPDIR` and `DENO_DIR` in `<data>/youtube` (the standalone binary unpacks itself there on every run; housekeeping removes leftovers after 30 minutes). No console window on Windows.
- **Keep yt-dlp up to date** (`update.rs`): a minute after start and daily, GitHub's latest stable release; a newer one is downloaded to `<data>/youtube/bin/yt-dlp-<version>`, checked against the release's `SHA2-256SUMS`, run (`--version` must say its tag), and only then used. The pinned one stays: it is used again when the setting is off, when the fetched one won't run, or when an image pins a newer one. `/api/youtube` shows which is in use.

## Politeness

- Lookups (searches, extractions) start at least 1.5 s apart lull-wide and at most 2 at once; downloads at most 3 at once. One search per song at a time.
- **Back-off:** "Sign in to confirm you're not a bot" or HTTP 429 stops every lookup and download for 15 minutes, doubling up to 6 hours, until YouTube answers again. The `youtube` status says so in plain words ("YouTube is asking for a sign-in check, try later (lull asks again at 14:05 UTC)"); a stream that needed YouTube fails with the same sentence, never a stack trace.
- An upload YouTube won't give (removed, private, age-restricted) is forgotten, so the next find searches again.

## Playlists for link imports

`Youtube::playlist(url, from, count)` lists a YouTube, YouTube Music or SoundCloud playlist for collection's link imports (collection.md), whatever the play mode. YouTube lists flat (`--flat-playlist`, no extraction); SoundCloud's flat listing names no song, so each song is read: metadata only (`--ignore-no-formats-error`, `soundcloud:formats=none`: one request a song) with `--sleep-requests 1`, about 3-4 s a song, because SoundCloud answered 403 to every set for some minutes after about 150 songs were read with their stream formats at full speed. `-i` leaves out a song blocked in this country instead of failing the page; a set it can't read at all comes back `null` and is an error. Runs share the lookups' pacing, limit and back-off.

## Status

`youtube`: `off` while the mode is off; `ok` "Ready, with yt-dlp 2026.10.26 (built in)", with ". Sound checks are idle: …" added while no track looked up has a reference ([Sound checks](#sound-checks)); `warn` while backing off; `down` when yt-dlp isn't installed or won't run. `extra { version, pinnedVersion, updated, backoffUntil, soundChecks: { checked, refused, withoutReference, withReference } }` (since the hub started).
