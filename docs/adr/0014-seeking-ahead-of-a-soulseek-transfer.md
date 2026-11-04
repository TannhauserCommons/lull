# ADR-0014: Seeking ahead of a Soulseek transfer

- Status: accepted (owner, 2026-11-04)
- Date: 2026-11-04
- Ships: in the release after 0.2.2. Until then it lives on the `seek-ahead` branch; waiting for a seek ahead (part 1) ships in 0.2.2.

## Context

A Soulseek song plays while slskd downloads it front to back. Since 2026-11-04 a seek past what has arrived is kept and lands once the transfer gets there ("Jump to where I seek once it has downloaded", Settings › Playback; web.md). On a slow peer that can be minutes: a 40 MB FLAC at 150 KB/s takes four and a half minutes to reach its last minute. The owner asked for real seek-ahead: fetch from where the listener seeks.

What there is to work with:
- **The protocol allows it.** A Soulseek download starts at an offset the downloader names. Every client honours it, since it is how resuming works. Soulseek.NET's `DownloadAsync(..., size, startOffset, ...)` takes it.
- **slskd's enqueue doesn't.** `POST /api/v0/transfers/downloads/{user}` takes `[{ filename, size }]`. slskd sets the offset itself, only to resume.
- **But slskd resumes on every attempt, the first included.** Since 0.26.0 (lull pins 0.26.2), `DownloadService` looks at the incomplete file before each attempt. With `transfers.download.retry.partial: resume`, the default, it asks the peer to start at that file's length and appends (`FileMode.Append`). The incomplete file's path is fixed: `<incomplete>/<user>/<remote folders>/<file>`, which lull already computes (streaming.md, Where the bytes are). A download never deletes it; only slskd's age-based retention (`retention.files.incomplete`, off unless set) and its files API do.
- **Upstream contribution rules.** slskd's CONTRIBUTING.md asks for an issue or discussion before any pull request, assigns the contribution's copyright to the maintainer, and requires disclosure of AI tools. It says contributions made entirely by AI are not welcome and will be rejected. lull's code is written by agents, and lull's public work carries no mention of them. An honest pull request from lull can't meet both rules.
- **lull assumes "length on disk = bytes received"** (streaming.md, How it's written): `Progress.received`, the stream body's `range::sendable` and the cache's sizes all read the file's length.

## Decision

**Ranged transfers through slskd's resume, with no change to slskd.**
- **A leg** is one slskd transfer of the song's file that starts at a byte lull chooses. To start one at X, lull sets the incomplete file's length to X, then enqueues. slskd resumes at X. On Linux the extension is a hole and takes no space. On Windows the file is marked sparse first.
- **Bytes before X** in that file are whatever was there: an earlier leg's bytes, or the hole. lull never trusts them by position. It trusts only its own list of ranges (below).
- **One leg per peer and file at a time.** slskd refuses a duplicate. Before a new leg on the same peer, lull cancels the running one.
- **Detection, not configuration.** If slskd answers a leg by truncating the file (`partial: overwrite` on an existing slskd), the first bytes land at 0, not X. lull then marks that slskd as unable to seek ahead and falls back to waiting (below).

**slskd is pinned, by decision (owner, 2026-11-04).** lull depends on how this slskd resumes, so it ships the version that does it and never another one by itself.
- **The compose kit** names `slskd/slskd:0.26.2` by tag and digest (`deploy/compose.yaml`).
- **The desktop app's sidecar** stays at 0.26.2 with the release's own hashes (`app/build.rs`, desktop.md).
- **Nothing updates it automatically.** lull has no dependency bot, and the homelab's Renovate leaves slskd alone (homelab e196868).
- **A deliberate upgrade** (an upstream security fix, say) is taken by hand: tag, digest, `SLSKD_VERSION` and its hashes change in one commit, which lands only once the contract test below passes against the new version.
- **An slskd the listener already runs** (the kit's third shape) is theirs: lull detects what it does (above) and waits instead of seeking ahead when its resume differs.

**A contract test guards the resume.** It runs in CI against the pinned image, so an slskd that changes this fails CI instead of failing quietly for listeners.
- **The setup:** the real slskd container, logged in to a stand-in Soulseek server, downloading from a stand-in peer. Both stand-ins are small Node programs in `tests/slskd-contract/`, speaking the protocol as Soulseek.NET does. The peer serves a known file and records the offset it is asked for.
- **What it checks:**
  - an incomplete file pre-sized to X makes the peer send from X;
  - the file then holds the peer's bytes from X on, and its first X bytes are untouched;
  - with `retry.partial: overwrite`, the peer is asked for 0 and the file starts again: the signal lull's detection reads.
- **When it runs:** on every change to `deploy/compose.yaml`, `app/build.rs`, `crates/soulseek/` or the test itself. The desktop sidecar is the same release as the image, so one test covers both.

**The song's cache file is lull's: a sparse file plus a persisted list of byte ranges.**
- **Where:** each song's copy gets one file of lull's, `<transfers>/lull/<id>.<ext>`, sized to the full file. A leg's bytes are copied into it at their own offsets as they land, one 64 KiB chunk at a time, as the body is read today. slskd's files are only where legs land, and lull deletes a leg's file when the leg ends.
- **Why copy:** two legs from different starts can't share slskd's single incomplete path. A copy gives one file to serve, save and evict, whatever the legs did. It costs one extra write of each byte, on the same disk.
- **The ranges** live with the copy's row in `soulseek_files` (`ranges`, JSON) and are merged as they grow. They are written each time a range grows by a megabyte, and when a leg ends. After a restart, a range is trusted only up to what was written.
- **"Length on disk = bytes received" goes.**
  - `Progress` carries `ranges`; `received` is their sum.
  - The body sends up to the end of the range it is in, then waits for the gap to fill.
  - `stream` events add `ranges` as fractions, and the seek bar draws them as it draws `buffered` today.
  - The cache counts allocated bytes, not length.
- **Complete means no gaps.** Saving (`Transfers::complete`), the cache's `complete` flag and "best cached copy" all wait for the ranges to cover the whole file.

**What starts a leg.**
- **The browser's own request is the trigger.** It asks for byte X. Its demuxer's time-to-byte guess is better than the page's linear one, and it is what will be read.
- **The player decides whether to ask.** `stream` events carry `seekAheadS`: how many seconds of transfer away a seek must be for the hub to start a leg there (10 with another peer, 30 with only the same one), or `null`. A seek past what has arrived and further than that goes there at once, and the output's request starts the leg. A nearer one, or one with `null`, is kept until the bytes arrive (the waiting that ships in 0.2.2). The hub applies the same thresholds to the byte it is asked for, so the two agree.
- **Jumping costs a new transfer request,** about 1-3 s with a free slot, so a leg starts only when it wins. The running leg must be more than `JUMP_S` (10 s, measured at its current speed) away from X, and X must not already be on its way.
- **Probes collapse.** A request that starts in a gap waits 300 ms before it starts a leg. A newer request on the same song replaces an older one's leg plan. One seek gives one leg.
- **Order of legs:** first the leg the listener is waiting for. After it reaches the end of the file (or the next range), the gap nearest after the playing position, then the others in order. A gap is filled by a leg that starts a little inside the range before it (64 KiB), so the overlap can be compared.

**Which peer.**
- **Another peer first, when it can be the same file:** the same size, a free slot in its last search answer, and not failed before. That keeps the first leg running and its slot. The bytes are checked when the legs meet: the overlap must be identical, or the second peer's ranges are dropped, refetched from the first peer, and that peer is never used for ranges of this song again.
- **Otherwise the same peer, only when it is worth the slot.** Cancelling gives the peer's upload slot back, and the new request may wait behind other downloaders, for minutes on a busy peer. lull jumps on the same peer only when all of these hold:
  - **the wait it saves is long:** reaching X at the leg's current speed would take more than `JUMP_SAME_S` (30 s; another peer's jump needs only `JUMP_S`);
  - **the peer had room:** its last search answer said it had a free slot, and this song's first leg started without waiting in its queue;
  - **it hasn't queued us before:** a same-peer jump that queued, for this song, rules out any more on it.
- **When the jump queues anyway:** if the new leg is still waiting in the peer's queue after `stall_timeout`, lull cancels it and starts a leg at the old leg's front. That request may queue too, and the song then waits as any queued song does. Either way the seek is kept, so the song lands where the listener asked once the bytes arrive.

**The 416 rule under ranges.** A range starting past the end of the file is 416. Any range inside it waits for its start to be in a range, starting a leg as above. The body never fails at a gap; it waits there. A request already open when its leg is cancelled for a newer seek ends, as a fallback ends a response today. The browser has already moved on.

**Formats.**
- **MP3, CBR or VBR with a Xing/VBRI table:** the browser maps time to bytes well; one leg per seek.
- **MP3 VBR without a table:** the browser guesses by average bitrate. The guess lands near, and lull fetches from wherever it lands. The page's "Waiting for 2:40" estimate uses the same linear map; only that estimate is approximate.
- **FLAC with a SEEKTABLE** (the reference encoder writes one every 10 s by default): one leg per seek.
- **FLAC without one:** the browser bisects, about 10-15 reads at different offsets for one seek, each blocking on data. The 300 ms collapse can't merge reads that come one after another, so this would start a dozen legs. lull reads the STREAMINFO and SEEKTABLE blocks from the first bytes. With no seek table, the song doesn't seek ahead and waits as part 1 does.
- **M4A with `moov` at the end:** the browser asks for the tail before it can play anything. Until now that request waited for the whole file (since part 1; before, it got a 416 and "failed"). A leg for the tail fetches the index, a few hundred KB, and the song starts at once. This is the biggest single win.
- **Ogg and Opus, and YouTube's WebM:** YouTube copies arrive in seconds through yt-dlp and get no legs. Ogg from Soulseek follows the browser's bisection rule like FLAC without a seek table, so it waits.

**Measured in Chromium (2026-11-04)**, on the fixture hub trickling real files (`POST /__fixture/trickle { file }`, `GET /__fixture/reads`), with a seek to 60%:
- **MP3 VBR with a Xing table:** one read at open, from 0 (no probe at the end); the seek is one read, at 0.588 of the file, and the song plays from there.
- **MP3 VBR without one, with a 100 KB cover:** one read at open, but playback starts only once about 1 MB has arrived (the browser probes that much to estimate the length). The seek is one read at 0.721 of the file for 0.6 of the time: the browser's own guess, which lull follows.
- **M4A with `moov` at the end:** reads at 0, then at 0.99 (the index), then 0.006; with the tail fetched by a leg the song starts, and the seek is one read at 0.591.
- **FLAC without a seek table:** the seek is five reads, at 0.998, 0.996, 0.72, 0.393 and 0.557: the bisection above, which is why it doesn't jump. (A FLAC file with a seek table wasn't measured: no tool here writes one.)

**Fallback: waiting, as part 1 has it.** Where seek-ahead can't jump (an slskd that overwrites, a format above, a peer that queues), a seek ahead is kept and waited for. Part 1's setting, "Jump to where I seek once it has downloaded", is gone (the owner, 2026-11-04: with real seeking there is nothing to switch off): a seek always goes where the listener asked, and stored settings lose the key by a collection migration. A kept seek shows as a still notch on the seek bar where it lands, and in words only for a long wait (estimated over 8 s, or lasting 3 s), so a short one isn't announced at all.

## Risks

- **Losing the peer's upload slot (same-peer jumps).** Cancelling the running leg gives the slot back, and the peer may queue the new request behind other downloaders. The song then stops arriving at all until the queue moves, which is worse than waiting. Mitigated by the rule in "Which peer": another peer first; the same peer only for waits over 30 s, with a free slot seen and no queueing so far; never again on this song once a jump queued; back to the old position after `stall_timeout`.
- **Bandwidth split.** Two legs share the downlink. The playing leg gets priority by order: a gap fill waits while the playing position is within a minute of a gap's start.
- **Files that differ by tag size.** Same audio with other tags is a different size, so it never qualifies. Same size with different bytes is caught by the overlap check. Until the legs meet, a second peer's bytes are trusted on size alone.
- **slskd's size check.** slskd sends the size from the search, and the peer refuses a mismatch (`TransferSizeMismatchException`). A leg uses the same size as the first one, so a peer whose file changed fails the leg, not the song.
- **Relying on slskd's resume.** This is documented behaviour (`retry.partial`), not an API made for ranges. A future slskd could change it. The pin keeps the tested version, the contract test fails a deliberate upgrade that changes it, and the detection above turns an slskd the listener runs into waiting rather than wrong bytes.
- **More transfers in slskd's list.** Each leg is a transfer record. lull cancels a leg with `remove=true`, as it cancels its fetches today, so the record goes with it.
- **Disk.** On Linux the holes take no space. On Windows (the desktop app's bundled slskd and cache) they are zeros on disk: lull's file grows to the furthest byte a leg brought, and slskd's file for a leg at X takes X bytes until the leg ends. Both are within the twice-the-copy room made before the fetch, so it is bounded; marking the files sparse would need a Windows-only system call for no change in that bound.

## Alternatives

- **A start offset on slskd's enqueue, upstream.** This is the clean API: `QueueDownloadRequest.StartOffset`, passed to `DownloadAsync`, with the file opened at that offset. But a ranged transfer also needs slskd not to move a file with a hole into the downloads folder as finished, so it is more than one field. More decisively, the contribution rules above rule out a pull request from lull that is honest about how it was written. If the owner still wants the API upstream, the honest route is an issue (a feature request) in the owner's own words. lull doesn't wait for it: the resume route works with the slskd lull already ships.
- **lull's own slskd fork.** It would carry the same patch, built into the compose kit and the desktop app. It is a C# codebase to rebase every release, and an AGPL source distribution to keep, for what resume already gives.
- **Talking to Soulseek directly** (a Rust client in lull instead of slskd): this reverses ADR-0010 for one feature.
- **Waiting only (part 1 alone).** Kept as the fallback. On slow peers it is minutes.
- **The page seeks by its own time-to-byte guess and asks the hub for a leg.** The browser reads where its demuxer decides, not where the page guessed. Two guesses would mean two legs.
