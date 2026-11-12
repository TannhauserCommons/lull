# ADR-0012: YouTube as a second source, through yt-dlp into the stream cache

- Status: accepted
- Date: 2026-11-02

## Context

The listener asked for YouTube as a source (2026-11-02): through yt-dlp, streamed into the existing stream cache so preloading, seeking and resume work unchanged, with three ranking modes (off by default, fallback only, start fast then upgrade), its own quality badge, and every choice a setting. (Corrected 2026-11-08: this line said "exempt from the minimum bitrate", which the owner never asked for. A YouTube copy is judged by the streaming settings as any copy is; see the update "one streaming policy".)

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

## Update 2026-11-08: one streaming policy, for every source

The owner: "why won't YouTube play even at start fast when YouTube's copy is found faster?", and "build it modularly so we don't have to keep doing this patchwork: the app already has clear foundations and intentions of how things should work". The race above, and the fixes after it, judged YouTube's copy by rules of their own:
- a race that was "YouTube's alone" when no known Soulseek copy passed;
- YouTube's copy marked a lesser one by its source;
- this ADR's own line making YouTube "exempt from the minimum", which the owner never asked for.

**Decision.** One module, `soulseek::policy`, decides what a song starts on and what it switches to, from the listener's streaming profile alone (`sources.stream`: `minimum`, `below`, `pick`, `maxKbps`). It is pure. The fetch, race, fallback and upgrade code only gather copies, from any source, and ask it. A copy is a copy, a Soulseek peer's or YouTube's.

**`youtube.mode` only says when YouTube is a source.** It never changes how a copy is judged.
- `off`: never.
- `fallback`: searched when Soulseek has nothing or is too slow (`waitS`).
- `startFast`: searched alongside Soulseek from the start.

**Starting a song** (`policy::start`). At a song's start lull can't know yet whether a copy passing the minimum exists. So a copy below it arriving first means "none is good enough yet", and `stream.below` decides, as its setting says:
- **`playBest`:** the first copy whose first bytes are ready plays, from any source, above or below the minimum.
  - A passing copy ready at the same moment wins.
  - A copy below the minimum is marked so (`belowMinimum`), and the upgrade fetches the best passing copy beside it.
- **`keepLooking`:** only passing copies play.
  - Under start fast they race, and the first ready wins, from any source.
  - With none, lull searches on. A copy below the minimum is never fetched to start the song.
- **`ask`:** the first passing copy plays. Once the search ends with none passing, the listener picks among the copies found, YouTube's with Soulseek's.
- **`pick`** keeps its meaning: it orders the copies tried first (`sources::order`), and so breaks ties.

**Switching mid-song** follows the same module (ADR-0013's update of the same day).

**Consequences.**
- **YouTube starts a song under start fast when its copy is ready first:**
  - always, when it passes the minimum;
  - under `playBest`, also when it doesn't, marked below the minimum.
- Under `keepLooking` or `ask`, with a minimum above YouTube's best (Opus 160, AAC 128), a passing Soulseek copy starts the song, and YouTube's copy is never fetched.
- **The same rule holds in `fallback`:**
  - YouTube's copy below the minimum plays only under `playBest`;
  - otherwise a Soulseek peer that was only slow goes on;
  - with nothing on Soulseek, `keepLooking` fails and `ask` offers it.
- **Cached copies:** a later play takes one within the cap first, then one passing the minimum, then the better sounding (`policy::cached_key`; order corrected 2026-11-12, below), whatever its source.
- **A better copy may be YouTube's.** While start fast makes YouTube a source, its copy is one of the targets the policy weighs mid-song. The upgrade fetches whatever copy the policy names, from either source, and the switch is the same: lined up, or the next play. Example: under a 128 minimum, a peer's MP3 128 that started the song is followed by YouTube's Opus 160.
- **A copy that has played through gives way to a better cached one** on the next play, whatever either's source. Before, only a finished YouTube copy did.

## Update 2026-11-12: the cap is a maximum

The owner: a song "upgraded above my maximum bitrate from YouTube Opus". Marlon Funaki's "Metaphor", under a 320 kbps minimum and a 320 kbps cap (`playBest`), started on a peer's FLAC: Soulseek's side of the race sent first, and the policy counted a copy passing the minimum before one within the cap, at the start, in the order copies are tried and among cached copies.

**Decision.** `maxKbps` bounds what streams. A copy above it starts the song only when no copy within it can (`policy::start`); the order tries every copy within the cap first, passing ones first on each side (`sources::order`); a later play takes a cached copy within the cap first (`policy::cached_key`). In the race, Soulseek's copy above the cap that sends first waits for YouTube's lookup, and starts the song only when YouTube has no copy that may. Under `playBest`, YouTube's Opus 160 below a 320 minimum therefore plays before a FLAC above a 320 cap; under `keepLooking` or `ask`, where a copy below the minimum never starts the song, the FLAC is the last resort.

**Alternative.** A cap no copy ever crosses: with only copies above it, the song wouldn't play. Not chosen without the owner.

## Update 2026-11-12: a format switched off never streams

A listener: "It's playing FLAC on my phone even though I have FLAC off and maximum set to 320." Formats were part of the streaming minimum only, so under `playBest` a FLAC outside the formats left on was a copy below the minimum like any other: it started the song when it sent first in the race, and a cached one played on later plays. And Settings showed "any format" (none stored) as every format chip off, so FLAC looked switched off when it counted.

**Decision.** A format the streaming profile switches off never streams (`policy::allowed`): it never starts the song under any `below` rule, isn't tried, raced, preloaded or offered as a better copy, a later play skips it in the cache, and a live fetch of it (a save's) isn't joined. Only the listener's own pick of that copy plays it. It is their explicit choice, made for data on a phone, unlike the bitrate floor, which `below` may set aside. A `.webm` counts as Opus (YouTube's), so Opus switched on keeps YouTube. Settings shows "any" as every chip on, and at least one stays on.

**Alternative.** Keep formats inside the minimum and let `below` decide: what played the FLAC. Not chosen.

The cap stays a maximum with a last resort (above); a hard cap is still the owner's call.

## Update 2026-11-12: once YouTube is ready, a queue is given up

On 2026-11-11 TLC's "No Scrubs" waited for YouTube's lookup (8 s) while Soulseek's side sat 30 s in the queue of the first peer it asked, though many peers shared the song. Asked whether a peer that only queues the race should be waited out, the owner: "Once YouTube is ready".

**Decision.** Once YouTube's copy is ready (its first bytes, or all of it), Soulseek's side gives up a peer that has only queued it, at once rather than after the 30 s queue limit, and asks the next peer with a free slot first. Still one peer at a time (ADR-0015 stands). Before YouTube is ready, the queue limit is unchanged, so a known Soulseek copy can still start the song first.

