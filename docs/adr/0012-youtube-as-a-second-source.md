# ADR-0012: YouTube as a second source, through yt-dlp into the stream cache

- Status: accepted
- Date: 2026-11-02

## Context

The listener asked for YouTube as a source (2026-11-02): through yt-dlp, streamed into the existing stream cache so preloading, seeking and resume work unchanged, with three ranking modes (off by default, fallback only, start fast then upgrade), its own quality badge, exempt from the minimum bitrate, and every choice a setting.

Facts that shaped it, checked on 2026-11-02 with yt-dlp 2026.10.26:
- YouTube's audio-only streams (Opus 251 in WebM, AAC 140 in M4A) come with an exact `filesize`, so byte ranges can be served before the download ends.
- `yt-dlp --load-info-json … -o -` writes the stream to stdout: first bytes in about 1 s, a whole song in under 2 s. Finding the song (a search and a full extraction) takes 6-7 s cold.
- YouTube Music's song search finds official audio (Topic channels) with music metadata; a plain YouTube search has more fan uploads and videos.
- Current yt-dlp needs a JavaScript runtime (Deno) for YouTube. The standalone `yt-dlp_linux` needs no Python, only glibc and zlib.
- lofty, which tags saves, reads and writes YouTube's M4A, but has no WebM support.

## Decision

- **A `youtube` crate behind a core seam.** `Youtube::find(track, want)` returns a copy (`SourceRef` with user `YouTube` and path `youtube:<video>:<format>.<ext>`), `download(source, dest)` writes it. Soulseek's fetches call it: the copy is one more source, written into the stream cache folder and recorded in `soulseek_files`, so serving, preloading, pins and eviction need no change.
- **Soulseek decides when** (`fetch.rs::route`), the youtube crate decides **which upload and stream**. Saves never pick YouTube while any Soulseek copy is known; with "Save from YouTube" on and none known, they take the AAC copy.
- **Start fast** plays YouTube at once and runs an upgrade fetch of Soulseek's best copy (the saving profile's) in a separate map, so a seek never lands on another file; later plays and saves use the Soulseek copy.
- **Matching** prefers official audio, checks length and refuses other versions unless the song is one; matches are remembered, extractions kept for 4 h.
- **yt-dlp and Deno are pinned** in the image by version and SHA-256. "Keep yt-dlp up to date" (on) fetches newer stable releases into the data folder, checks them against the published checksums, and falls back to the pinned one.
- **Politeness:** no cookies or accounts, paced lookups, and a back-off with a plain status on sign-in checks and 429s.

## Alternatives

- **Downloading YouTube's stream URLs ourselves (reqwest) after `yt-dlp -g`:** resume by Range for free, but YouTube throttles naive clients and the URL rules change with the player; yt-dlp keeps up with that.
- **A Python yt-dlp in the image:** the final stage is distroless; a Python runtime would be the largest part of the image.
- **Transcoding to one format (ffmpeg):** a second large dependency and CPU per play, for no gain in quality.
- **Showing YouTube copies in the source picker:** possible later; the picker's Source shape is Soulseek's (speed, slots, queue).

## Consequences

- **A third outside dependency** that breaks often. The status says so plainly, and keeping yt-dlp current is the default.
- **Two binaries in the image** (about 150 MB), and each yt-dlp run unpacks itself into `<data>/youtube/tmp`.
- **Start fast fetches each new song twice** (YouTube and Soulseek), by design: the second is the copy that's kept.
- **The desktop app's own hub** ships yt-dlp and Deno the same way (desktop.md).

## Update 2026-11-03: the play may move to the better copy

With Settings › Sources › "Switch to a better copy while a song plays" on (the default), a song YouTube plays (start fast or a fallback) moves to Soulseek's copy once it is ready, lined up and crossfaded so it doesn't jump: [ADR-0013](0013-switching-to-a-better-copy-mid-song.md). With it off, the current play never jumps, as above.

## Update 2026-11-05: start fast's upgrade keeps to the streaming cap

With "Switch to a better copy" on, start fast's upgrade is the better copy by the streaming profile, not the saving profile's: a FLAC above a 320 kbps cap was never offered to the player and held the song's one upgrade, so no better copy came mid-song. Saves fetch their own copy when they need one.

## Update 2026-11-05: a copy is checked by its sound

Matching by words now has a second check: each upload's sound against the catalog's 30 s preview of the song. Fan uploads are checked before they play, official ones while they play; a re-recording or another song under the right title is refused, and a right upload titled oddly is taken: [ADR-0016](0016-checking-a-copy-by-its-sound.md).

## Update 2026-11-08: start fast is a race

The owner: start fast "doesn't mean YouTube first, it's if YouTube finds it first". On 2026-11-07 Beach House's "Baseball Diamond" took about 16 s to start. Its Soulseek copies (40 peers, 320 kbps) were known from an earlier search, but the hub waited about 15 s for YouTube's lookup before it asked a peer. Nothing bounded that lookup in start fast, and the next queued songs' warm-ups were taking yt-dlp's two lookup turns at the same moment.

**Decision.**
- **Both sides start at once:** YouTube's lookup and download, and Soulseek's copy by the streaming profile (one that passes the minimum: known, or from the search beside YouTube's lookup as soon as it answers).
- **Whichever sends the song's first bytes first plays.**
  - **Soulseek first,** or YouTube has no copy before it sent a byte: Soulseek's fetch becomes the player's (the response already waiting serves it, and its events name the job they replace), and YouTube's download stops.
  - **YouTube first:** Soulseek's copy carries on beside it, the better copy the player switches to when it is better (ADR-0013's update of 2026-11-08), and the copy later plays and saves take.
- **Soulseek's side runs on after a skip,** as start fast always promised its copy for later plays and saves.
- **The playing song's YouTube lookup goes first.** Warm-ups and preloads look up behind it, one at a time.

**Alternatives.**
- **A time limit on YouTube's lookup** (as `fallback`'s `waitS`): it bounds the wait but still makes a known Soulseek copy wait for it, and picking the limit is a guess.
- **Soulseek first whenever a copy is known:** it trades one fixed order for another; a busy peer can be slower than YouTube.

**Consequences.**
- **No extra cost to Soulseek peers:** Soulseek's side is the one request start fast always made. It is just made at once, and it is never cancelled for losing. This is not the racing of two peers that ADR-0015 decided against.
- **YouTube is downloaded only when it is the first to send** or Soulseek has no copy that passes. A song Soulseek starts first costs YouTube only its lookup, which is stopped.
- **Measured** with the stand-in slskd: a known Soulseek copy and a 15 s YouTube lookup, the first byte of the player's response (`cargo test -p lull-soulseek start_fast_start_time -- --ignored --nocapture`, three runs each):

  | | first byte | from |
  |---|---|---|
  | before | 15.02 s | YouTube |
  | after | 0.27-0.28 s | Soulseek |
