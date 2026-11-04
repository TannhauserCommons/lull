# YouTube: a second source through yt-dlp

`crates/youtube` finds the YouTube upload that is the song and downloads its audio; `crates/soulseek` decides when YouTube plays and writes the audio into the stream cache, so preloading, Range seeking, resume, pins and eviction treat it like a peer's file. Why it works this way: [ADR-0012](../adr/0012-youtube-as-a-second-source.md). Endpoints: [api.md](api.md#youtube-cratesyoutube).

## Map

| File | Holds |
|---|---|
| `core/src/youtube.rs` | The settings, `Copy` (how a YouTube copy is named as a `SourceRef`), its quality chip |
| `core/src/seams.rs` | `Youtube`: `find(track, want)`, `download(source, dest)` and `playlist(url, from, count)` |
| `youtube/src/search.rs` | Searching, confirming by full extraction, remembered matches, kept extractions |
| `youtube/src/matching.rs` | Whether an upload is the song, and how good a copy it is |
| `youtube/src/formats.rs` | Which audio stream to take |
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

## Modes

`fetch.rs::route` decides per fetch:

- **A save** gets the copy it named. `Transfers::save_candidate` never names a YouTube copy while any Soulseek copy is known; with `saves` on and none known, it names YouTube's AAC copy (the save pipeline can't tag WebM) as meeting the minimum, so `wait` doesn't hold it back. Auto-upgrade judges the saved file and replaces it once a peer has a passing copy.
- **The listener's pick** of a peer plays that peer. Pick a source lists the song's YouTube copy too while YouTube is on (any mode but `off`): one row after the peers, its chip ("YouTube · Opus 160"), "lossy", no score and never below the minimum; the copy the streaming path would play, matched in the background when the picker opens (`done: false` until then). Picking it plays YouTube (`?peer=YouTube&path=`), checked against the song's match.
- **`fallback`:** Soulseek as usual; YouTube when it fails (no peer shares it, none passes and the `below` rule gives up, every peer failed) or when no peer is sending `waitS` after the fetch began (still finding, queued, or between peers). The Soulseek attempt is cancelled in slskd first. Preloads do the same.
- **`startFast`:** the player's fetch finds the YouTube copy, starts an **upgrade** fetch (the saving profile's order, so the best copy), and plays YouTube. If YouTube has no match or fails, the upgrade is dropped and the player's fetch looks on Soulseek itself; when that fails too, the error says why for both ("no peer shares this song right now; YouTube: no YouTube upload matches this song"). A preload fetches Soulseek's copy (YouTube's only when Soulseek has none) and warms the YouTube match, so playing it still starts at once.

**The current play never jumps.** Upgrades live in their own map, out of the player's way: a seek joins the YouTube fetch, and the upgrade's bytes go to the cache only. A save started while YouTube plays the song runs there too. A preload still waiting for a peer when the song is played steps aside as the upgrade. A YouTube copy that has played through (its length plus a minute after it began, nobody reading) gives way to a cached Soulseek copy on the next play; until then a request is the same play seeking.

**Switching to the better copy.** With Settings › Sources › "Switch to a better copy while a song plays" on (the default), a song YouTube plays moves to Soulseek's copy once it is ready, lined up and crossfaded so nothing jumps ([streaming.md](streaming.md#better-copies)). In `fallback` the copy is fetched by the streaming profile as soon as YouTube steps in; in `startFast` it is the upgrade above, offered when it passes the streaming minimum and cap. With the setting off, or when the player can't line the copies up, the rule above holds: it plays from the next play.

**Later plays.** `Fetcher::best_cached` prefers a Soulseek copy over a YouTube one. With the mode off, cached YouTube copies are ignored; otherwise one plays at once (and `startFast` starts an upgrade beside it).

## Badges and quality

A YouTube copy's chip is its own: `YouTube · Opus 160`, `YouTube · AAC 128` (`Copy::facts`), with `peer: "YouTube"` and `youtube: true` in `stream` events. It is exempt from the minimum (`belowMinimum` is always false). The streaming cap (`stream.maxKbps`) picks the best stream within it (YouTube's average bitrate may run 10 % over), else the smallest.

## Matching

`find` remembers its answer per song and strictness in `youtube_matches` (a match for 30 days, no match for a day; a migration forgets the misses when the rules below find more). A match with no audio lull can stream is an error that says so, not a miss.

1. **YouTube Music songs** for "artist title" (`music.youtube.com/search#songs`, first 5, no extraction): those whose title matches are extracted, at most 2, and judged on the full facts.
2. **Videos** (`ytsearch8:`, first 8 with channel and length): judged, best first; at most 2 extracted and judged again.

An upload is the song when (`matching.rs`):
- **the title matches** after `split` reads it (packaging brackets like "(Official Audio)", a translation's "(Sub.Español)" and quality tags like "HD 1080p" dropped, a " | …" tail and a leading "<show> Soundtrack - " dropped, an "Artist - " or "Artist • " prefix split off, a repeated artist and whole-title quotes dropped), with `ids::loose_title` and one slip per 8 letters; numbers must agree. A fan's "Title - Artist" reads the other way round when only that reading is the song and its artist (Clairo's "mount airy, georgia" is on YouTube only as such uploads);
- **the artist matches** YouTube's music metadata, the title's artist part, the channel ("- Topic", "VEVO", " Official", " Music" dropped), or the title itself;
- **the length agrees** within the strictness's slack;
- **it is no other version**: live (or a recorded live stream), cover, nightcore, sped up, slowed, reverb, 8D, karaoke, instrumental, remix, acoustic, piano, reaction and the like are refused unless the song's artist, title or album has the word.

`split` is the one reading of an upload's title: `read_upload` (the same, plus music metadata and Topic channels, and whether the length is the song's) gives the artist and title of each song `Youtube::playlist` lists, which collection's link imports and outside playlists take as they are.

Score: official +40, YouTube Music +10, exact title +5, minus 3 per second off, a music video (not audio) −10.

## Formats

`formats.rs`: audio-only HTTPS streams with an exact `filesize` (the cache serves byte ranges against it while the download runs), no DRM, not YouTube's `-drc` copies, the original language track. Opus is WebM, AAC is M4A: `content-type` `audio/webm` or `audio/mp4`.

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

`youtube`: `off` while the mode is off; `ok` "Ready, with yt-dlp 2026.10.26 (built in)"; `warn` while backing off; `down` when yt-dlp isn't installed or won't run. `extra { version, pinnedVersion, updated, backoffUntil }`.
