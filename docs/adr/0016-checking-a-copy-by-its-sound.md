# ADR-0016: Checking a copy is the song by its sound

- Status: accepted (the owner, 2026-11-05: "Yes, build it", after the trial's numbers below)
- Date: 2026-11-05

## Context

lull plays copies it did not make: YouTube uploads (Topic channels, artists' channels, YouTube Music, fans) and Soulseek files. Today a copy is taken on its words: the title, the artist and channel, the length (youtube.md#matching, soulseek.md#source-rules). Words fail both ways:
- **Right copies refused.** A fan upload titled its own way, a video with a few seconds of intro, a Topic upload under an unusual title. Germany Germany's "LENA" never played (fixed on 2026-11-05 by quoting the artist in YouTube Music's search; its own upload was not among those searched).
- **Wrong copies taken.** A re-recording under the same title and length, a same-title song with an uploader's wrong artist, a sped-up or live take not named as such, a mislabelled Soulseek file. The owner: "we have to make sure we don't play any random weirdo's track".

The owner asked (2026-11-05) to design and test checking the sound itself: fingerprint the candidate, compare it with an independent reference, and let length and titles only order the candidates, never refuse them. Measure first; build nothing until the numbers are seen.

## The measurement

Run on a workstation (Intel i7-13700K) on 2026-11-05, never against a listener's hub or Soulseek account; YouTube asked one request at a time, seconds apart. Tooling: [`tests/fingerprint-trial/`](../../tests/fingerprint-trial/README.md).

**Data.** 100 songs: one listener's 77 (likes, library and play history: indie, rap, film and game scores, fan-only and unreleased songs), 15 others across languages and scripts (Japanese, Korean, Chinese, Russian, Arabic, Turkish, Spanish, Portuguese, French, German, English from Nigeria and South Africa), and 8 famous songs with many versions on YouTube. Each song's reference is Apple's 30 s preview, picked by exact artist and title. Its candidates are YouTube uploads found by searching: the song itself (Topic, official, fan re-uploads) and wrong ones (other artists' songs of the same title, covers, live, sped-up, slowed, karaoke, instrumental, acoustic, remixes), plus remasters and radio edits. Every label was checked by hand against the upload's title, channel and length; 14 copies nobody can call without listening (another cut, a fan "remaster") were left out. That leaves 281 scored copies: 122 right, 159 wrong. Plus two re-recordings: Taylor Swift's "Love Story (Taylor's Version)" against the 2008 song, and Def Leppard's 2012 "Pour Some Sugar On Me" against 1987's.

**The method.** Decode (symphonia), fingerprint (rusty-chromaprint, Chromaprint's algorithm 2), slide the 30 s preview over the whole upload and keep the offset with the fewest differing bits. The score is that bit error rate: 0 is identical, about 0.5 unrelated.

### Reliability

| Copy | Scores (median, range) |
|---|---|
| The song (Topic, official, fan re-uploads) | 0.04 (0.003 to 0.19; 3 uploads of another mix: 0.37 to 0.41) |
| Remasters, radio edits of the same recording | 0.07 (0.02 to 0.16) |
| Other artists' songs of the title | 0.44 (0.28 to 0.48) |
| Covers, acoustic takes, remixes | 0.39 to 0.41 (lowest 0.26) |
| Sped-up, slowed, nightcore | 0.44 (lowest 0.29) |
| Live | 0.34 (lowest 0.15: sung over the record) |
| Re-recordings of the song | 0.275 to 0.28 |
| Instrumentals and karaoke made from the record | 0.21 and 0.29 (lowest 0.007) |

- **Sound alone** (under 0.20 = the song) accepts 18 wrong copies, all of them named for what they are: 14 instrumentals and karaoke tracks, 3 live takes played or sung over the record, 1 ten-minute "extended" loop. Chromaprint hears harmony, not voices: a track with the vocal removed is the same to it. It refuses 4 right copies.
- **The proposed rule** below accepts **0 of the 159 wrong copies** and plays **119 of the 122 right ones**. Not played: a 4K "official music video" re-upload of Fleetwood Mac's "Dreams" whose audio is another mix, and two uploads of one song that match no part of Apple's preview (probably another mix of it).
- **Today's rule** (version words, length within 5 s, title and artist read from the upload) also accepts 0 of the 159, but refuses 23 right copies; the proposed rule plays 22 of them (videos with a few seconds of intro, "Artist - Title (Lyrics)" re-uploads off by 6-9 s, Topic uploads titled unusually). And today's rule plays both "(Taylor's Version)" uploads for the 2008 "Love Story", whose sound scores 0.28 against it (the 2008 song's own uploads: 0.19).

### Which songs have a reference

- **Apple's preview, found by exact artist and title:** 59 of the listener's 77 songs, 7 of the 8 famous ones (Apple's search lacks Pablo Honey's "Creep"), and 7 of the 15 in other scripts (Apple spells many artists in Latin script: "Kenshi Yonezu", "Kino", "Jay Chou"). By storefront, on 30 songs: 21 in the US, 22 in Japan, 21 in Germany, 20 in Brazil, 0 in Korea (Apple's search answers nothing for that storefront). The songs without one are fan-only and unreleased songs, film and game score cues, and small rap releases.
- **Apple can hold a re-recording under the same name** (a band's later recording of its own song). A reference found by name can then refuse the right upload. The reference must be the catalog track the listener chose, by its id.
- **Deezer's preview:** Deezer's track search answers no tracks to requests from the network this ran on (`readable: false` on every track), so it could not be measured. Where Deezer answers, it is the same kind of reference.
- **AcoustID:** not measured: lull has no application key (`lull/ACOUSTID_API_KEY` is not in the vault), and none was made. From its documentation: free for non-commercial use with a registered key, at most 3 requests a second, built to identify whole files ("Can the service identify short audio snippets? No"), and it answers MusicBrainz recording ids, so the song needs one. The songs Apple lacks here (fan-only, unreleased, score cues) are the least likely to be in its crowd-sourced database too. Estimate: a small gain for older and well-catalogued songs Apple lacks, none for the rest.
- **Agreement between uploads:** uploads of one recording agree with each other at 0.005 to 0.07; covers, live takes and speed changes stay apart (0.32 to 0.49); instrumentals sit near the song (0.18). Agreement says "these uploads are one recording", never which: a viral sped-up edit can be uploaded more often than the song. Weakest tier: it orders uploads, and counts as a check only when an official upload (Topic, the artist's channel) is in the group.

### Speed

| Step | Measured |
|---|---|
| Fingerprint Apple's preview (about 1.1 MB) | 28 ms |
| Decode and fingerprint a whole upload, AAC (M4A 140) | 180 ms median, 230 ms p90 |
| The same, Opus (WebM 251, what lull plays), pure-Rust ruopus | 1.2 s for a 2:41 song (one song) |
| Align the preview in the upload | 0.5 ms |
| Where the preview sits in the song | 45 s median, 71 s p90, 190 s at most |
| YouTube: find the stream (playback does this anyway) | 3.4 to 6.5 s (5 songs) |
| YouTube: the whole AAC copy, after that | 1.5 to 3.1 s |
| Bytes of one upload, AAC | 4.0 MB median, 5.3 MB p90 |

CPU is per check on one core of that desktop; a small home server is likely 3 to 5 times slower (under 1 s for AAC, 4 to 6 s for Opus in pure Rust).

- **Strong copies** (YouTube Music songs, Topic channels, uploads with YouTube's music metadata, the artist's own channel): no change to the time before sound. The check runs on the bytes as they arrive and is done 2 to 3 s after the first ones.
- **Weak copies** (fan uploads): the check comes before the sound. It needs the audio up to the end of the preview (75 s of the song at the median, 100 s at p90), then 0.2 s of work. Estimated from the steps above, not timed end to end in a hub: about 1.5 s added at the median and about 3 s at p90.

## Decision

**Ship with changes**: the check is reliable for what it can hear, fast enough, and adds no wait for official copies. It doesn't replace the words that name a version, and it needs the right reference.

- **Where:** in the hub, in the youtube crate, after `judge` and before a copy is offered: the hub picks copies, keeps them, and is where every device's play comes from.
- **Libraries:** rusty-chromaprint (MIT, pure Rust, Chromaprint's algorithm 2: AcoustID-compatible fingerprints) and symphonia (pure Rust) for AAC, MP3, FLAC, Vorbis, ALAC and WAV, which covers YouTube's M4A and nearly every Soulseek file. Opus (what lull plays): ruopus (MIT, pure Rust), about 8 times slower than AAC but with no second download; libopus would be a C dependency. A fan upload, checked before it plays, is read on its AAC copy (140).
- **The reference:** Apple's preview of the song's own Apple track, by its id where the catalog row has one, never a search by name. Then Deezer's preview by its id where Deezer answers. AcoustID only once a key exists, as a second opinion for songs Apple lacks. No reference: today's rules, unchanged.
- **Alignment:** slide the preview over every offset of the upload (brute force: 0.5 ms), keep the best; an overlap of at least 15 s counts.
- **Thresholds (three zones):** under 0.15, the same recording. From 0.15 to 0.25, another master or mix of it: taken when today's rule also takes it. 0.25 and over: another recording, refused whatever its title says.
- **Words and length:** version words that the song lacks (live, karaoke, instrumental, sped up, slowed, remix, extended...) still refuse, because the sound can't hear a missing voice. Length refuses only beyond a quarter of the song (another cut, a loop, a compilation); within that it orders. Title and artist readings order the copies once the sound agrees.
- **Caching:** a verdict per (upload, catalog track): the score, the reference's id and when, kept a month like a match. Built as its own table, `youtube_checks` (a refused upload needs remembering too, not only the match), with the references' fingerprints per track in `youtube_references`.
- **What the listener sees:** a strong copy plays at once. If its check fails, which never happened for an official copy here, the copy is dropped for that song: the next play takes the next copy, and a song playing now moves to it at a quiet moment where ADR-0013 can line it up. A weak copy shows "Checking the YouTube copy…" for its 1.5 to 3 s, then plays. On a miss, the next upload is checked; none left is today's "No copy matched the song". Pick a source shows each YouTube copy's verdict ("Matches the song" / "Didn't match").
- **Soulseek:** check too, in the background, once a transfer has the preview's span: the bytes are local, decoding is cheap, and a mislabelled file is a wrong copy that filename rules can't catch. A miss marks that copy wrong for the song; it never blocks a play. Not measured here (no Soulseek searches were made for this).
- **Politeness:** previews come from Apple's CDN at one per song, cached with the verdict; no extra YouTube searches, and at most one extra AAC download per weak upload checked.

## As built (2026-11-05)

- **The reference is the catalog row that is the track:** lull's track id is made of the artist, album and title, so discovery's `Previews` keeps only the Apple (then Deezer) row whose own artist, album and title make that id. No new id is stored for tracks.
- **An official copy is checked on the file that plays** (Opus through ruopus, pure Rust, about 1.2 s of one core per song), not on a second AAC download as drafted: during the trial YouTube asked the workstation for a sign-in check after about 500 downloads in two hours, and start fast plays every new song from YouTube. Fan uploads, checked before they play, still take their AAC copy.
- **A miss on an official copy** fails soulseek's YouTube attempt once the download is done and the check has answered; the fetch takes the next copy `find` gives, and the player, which reopens a song on a new attempt, moves to it. A copy the listener picked plays as it is.
- **No reference** (Apple's search answers nothing for Korea's storefront; songs only fans uploaded): today's rules, and the `youtube` status says sound checks are idle once 10 tracks were looked up without one.
- **AcoustID** is behind `LULL_ACOUSTID_KEY` (Settings › Sources › YouTube), skipped without it.
- **Soulseek:** measured on fixture files (MP3 320 and FLAC made from the trial's uploads, no Soulseek involved): 0.3 s (MP3) and 0.23 s (FLAC) of one core per song, no download, the reference reused per song; scores 0.008 to 0.063 against the song's preview. So it is on, in the background.
- **Matches made before** by fan uploads are found again, checked; official ones are checked the next time they play.

## Update 2026-11-05: references for songs no catalog has

The owner (2026-11-05: "Yes, add them"): songs that Apple, Deezer and AcoustID all lack get references from what lull itself holds, and the listener decides only as the last resort. No tier loosens today's refusals: version words and far-off lengths refuse as before, and a candidate the words refuse is never put to the listener.

References, in order (`check::references`); a copy is the song when its sound matches any of them (the three zones above):
1. **The listener's own copy:** the song's saved library file (fingerprinted once, its first 4 minutes), and any copy the listener confirmed ("Right song? Yes").
2. **The catalog's previews** (Apple, then Deezer), as before.
3. **Soulseek's copies:** the song's finished Soulseek copies in the cache, each one a peer's file the song rule took (artist, title, version-aware, length within 6 s). One copy is a reference; two or more peers' copies that match each other are strong evidence and stand even against a catalog preview that disagrees. The other way round, a Soulseek copy is checked against the song's verified official YouTube upload.
4. **The artist's own uploads:** an official upload (a Topic channel, the artist's channel, YouTube Music's song, YouTube's music metadata) that was fingerprinted while it played is a reference for fan uploads, as an official upload anchors agreement.
5. **AcoustID** (with `LULL_ACOUSTID_KEY`), for a YouTube upload no reference above covers.

**Asking the listener, last:** a copy no reference covers, that today's word rules take, plays marked "Unverified" on its source chip, with a quiet "Right song? Yes / No" beside it. Nothing pops up, and it can be ignored. Yes keeps a verdict and makes that copy the song's reference (1). No refuses that copy for the song and moves the song to the next copy. Answers are this hub's. YouTube copies only: a Soulseek copy the words took was already a peer's file of that name and length, and asking about every niche song's file would nag.

**Contributing to AcoustID** (Settings › Sources › YouTube, off by default): when the listener confirms a song AcoustID didn't know, the hub submits the copy's fingerprint and length with the catalog's artist, title and album, so other hubs can verify it. It needs the application key and the listener's own AcoustID user key (`LULL_ACOUSTID_USER_KEY`, Settings). Nothing is sent with the switch off.

## Update 2026-11-05: songs cut from full-album uploads

Some songs are on YouTube only inside an upload of their whole album. The owner (2026-11-05): such a song plays as a cut of that upload, in the same release as the tiers above, and:
- **A cut needs no sound check when its bounds are reliable:** YouTube's chapters, else the upload's description timestamps, with an entry whose title matches the song (loosely, as an upload's title) and whose length agrees within the strictness's slack. That entry is the evidence; the cut is taken out of the fingerprint path (no check, no "Unverified" question).
- **Neither gives a reliable match: the upload is given up** for that song. No fingerprint search through a long video for a song with no track list.

As built: youtube.md#full-album-uploads. The album's audio is downloaded once per video and each song's AAC frames copied out without re-encoding, as yt-dlp's own sections need ffmpeg, which lull doesn't ship.

## Alternatives

- **Sound alone, with words and length only as hints:** what the owner asked for first; it accepts 18 of 159 wrong copies here, every one an instrumental, karaoke, a loop or a take sung over the record. The words that name them stay a refusal.
- **Chromaprint itself (C++, through FFI or the `fpcalc` binary):** the reference implementation, but a C++ build in the image, or a third binary next to yt-dlp and Deno. The pure-Rust port computes the same fingerprints (algorithm 2, which AcoustID uses).
- **AcoustID as the reference:** free for non-commercial use with a registered application key, at most 3 requests a second, and built for whole files ("Can the service identify short audio snippets? No"). It answers MusicBrainz recording ids, so a match needs the song's recording id; many of the songs here have none or aren't in its crowd-sourced database. Not measured: no key exists for lull (`lull/ACOUSTID_API_KEY` is not in the vault). Worth adding later as a second opinion for songs Apple has no preview of, not as the first check.
- **Deezer's 30 s preview:** Deezer's track search answers no tracks to requests from the homelab's network (every track `readable: false`), so it can't be a reference there. Where it answers, it is the same kind of reference as Apple's.
- **Agreement between several uploads plus their words:** with no reference, several independent uploads that fingerprint alike are at least one recording. It can't tell which recording (a viral sped-up edit is uploaded more often than the original), so it only orders candidates; it never makes a weak copy strong.
- **Checking on the listener's device (Web Audio):** devices differ, the check would run once per device, and the hub is where copies are chosen and cached.
