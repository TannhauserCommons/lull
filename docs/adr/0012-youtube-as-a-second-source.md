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

## Update 2026-11-05: a copy is checked by its sound

Matching by words now has a second check: each upload's sound against the catalog's 30 s preview of the song. Fan uploads are checked before they play, official ones while they play; a re-recording or another song under the right title is refused, and a right upload titled oddly is taken: [ADR-0016](0016-checking-a-copy-by-its-sound.md).
