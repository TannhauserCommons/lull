# Progressive streaming and peer fallback

How `GET /api/stream/{trackId}` plays a song while slskd is still downloading it, and from wherever the listener seeks (`fetch.rs`, `fetch/legs.rs`, `ranges.rs`, `stream.rs`, `range.rs`, `layout.rs` in `crates/soulseek`; ADR-0014).

## Fetches

A **fetch** gets one song onto disk. There is at most one live fetch per track, shared by everyone who wants the song (the player streaming it, a save completing it, a [preload](#preloading)), so the bytes cross the network once.

A finished copy already in the stream cache plays at once, whatever the `below` rule, with one `stream` event (`streaming`, and `belowMinimum` when it doesn't pass the streaming minimum). Otherwise:

1. **Pick a source.** The listener's pick (`?peer=&path=`) comes first, then the first untried copy that passes the profile's minimum, in its order ([soulseek.md](soulseek.md#source-rules)): the streaming profile when the player started the fetch, the saving one when a save did. With no known source the fetch searches for the song first (`finding`).
2. **None passes.** When copies exist but none passes the minimum, the profile's `below` decides:
   - `playBest`: the best of them plays; the `stream` event says `belowMinimum: true`, so the player can show a badge;
   - `keepLooking`: search again, up to 3 searches 20 s apart, then fail with "no copy meets your streaming minimum";
   - `ask`: the fetch waits as `choose`, with the copies found in the `stream` event's `choices` (up to 8). The listener answers by playing one (`?peer=&path=`), which replaces the fetch, or by skipping; after 10 minutes it fails;
   - a save's fallback (`saveBestUpgrade`) takes the best one; under `wait` and `ask` library decided before starting ([save-flow.md](save-flow.md)), so the fetch fails rather than save a copy below the minimum.
3. **Make room** in the cache for the file twice over (lull's copy and slskd's file for the leg running, [cache.md](cache.md)), then get it in [legs](#legs).
4. **Watch each leg every second**, and give up on this peer when:
   - it hasn't accepted the request within 20 s;
   - it has kept the transfer queued for 30 s;
   - no new bytes have arrived for Settings › Sources' stall timeout (`stallTimeout`, 15 s by default; shown as `stalled` after 5 s);
   - slskd reports the transfer failed;
   - after 10 s of sending it delivers under 1.25 times the song's rate (its size over its length) and an untried copy is expected to be twice as fast (ADR-0015, [soulseek.md](soulseek.md#ranking)); not for a copy the listener picked or a save named, and not remembered as a stall.

   However it ends, a transfer that sent 1 MB or for 10 s leaves its rate in the peer's record.
5. **Fall back** to the next copy in the same order. A stall is remembered against the peer (ranking demotes it); an offline peer or a long queue is not. A fetch gives up after 5 peers. Peers whose queue is longer than `maxPeerQueue` are never picked automatically.
6. **Done:** every byte of the copy is in lull's file, which the cache records as complete. The last leg's tail is read from where slskd moved its file: slskd marks a transfer Succeeded before it moves the file and raises `DownloadFileComplete`, so lull asks for the path again each second, for up to 60 s, rather than failing the song in that moment.

**YouTube.** With Settings › Sources › YouTube on, the player's fetch (and a preload) turns to YouTube when no peer delivers, or none starts within `waitS` (`fallback`), or starts with YouTube while an upgrade fetch gets Soulseek's best copy beside it (`startFast`). The YouTube copy is one more attempt (`attempt` goes up), written into the cache and served the same way: [youtube.md](youtube.md#modes).

**Better copies.** See [Better copies](#better-copies): a lesser copy playing gets a better one fetched beside it, and the player switches without a jump.

**Switching source mid-song.** `GET /api/stream/{id}?peer=&path=` with another copy than the one playing replaces the fetch: the old one is cancelled, unless a save is completing it; then it runs on for the save, out of the player's way (no more `stream` events). The old response ends; the player opens the new one and seeks back by time (ADR-0008).

**Stopping.** `DELETE /api/stream/{id}` (skip, next) cancels the fetch unless a save still needs it or a device lists it to preload; a better copy on its way for switching stops too, unless a save needs it. lull's file keeps what arrived, with its ranges, and asking for the song again fetches only what is missing.

**Events.** Each change publishes:
- `stream` (to the player, only while it streams);
- `transfer`;
- `job` (an Activity row).

Stream status follows the fetch: `finding → queued → buffering` (under 256 KB from the leg running) `→ streaming`, with `stalled`, `fallback`, `choose` and `failed`.

## Legs

A Soulseek copy arrives in **legs** (`fetch/legs.rs`, ADR-0014): each is one slskd transfer of the file that starts at a byte lull picks.
- **Starting one at X.** lull makes slskd's incomplete file exactly X bytes long, a hole (under both spellings of its path, [below](#where-the-bytes-are)), then queues the download. slskd 0.26 resumes a download from its incomplete file's length on every attempt (`retry.partial: resume`, its default), so the peer sends from X. This is pinned by the contract test (`tests/slskd-contract/`) against the pinned image.
- **Copying.** Every 250 ms what slskd wrote of the leg is copied into lull's own file for the copy (`<transfers>/lull/<user>/<remote folders>/<file>`), at its own offsets, and added to the copy's `Ranges`. When slskd finishes a leg, the rest comes from its downloads folder (the path from its `DownloadFileComplete` event), then slskd's file and transfer record go. The ranges are written to the cache index every megabyte and when a leg ends.
- **Which byte.** The first missing one; a leg that reaches bytes that arrived before stops (it has nothing new). A reader waiting for a byte that hasn't arrived asks for it (`Fetch::ask_for`, while it waits: a request the browser drops asks nothing). When that ask has stood 300 ms (the reads of one seek come together) and the leg running wouldn't bring the byte within 30 s at slskd's reported speed (`JUMP_SAME_S`; behind the leg is never), the same peer starts over there: the leg is cancelled and a new one starts at the byte. After that leg reaches the end, the gap it left fills from its start.
- **When a jump is allowed.** It gives the peer's upload slot back, so only when slskd resumes where lull sizes the file, the peer's search answer said it had a free slot, and no leg of this copy has waited in its queue. `stream` events carry `seekAheadS`: the 30 while a jump is allowed, `null` while it isn't, so the player chooses between going to a seek now and keeping it ([web.md](web.md)).
- **An slskd set to overwrite** (`retry.partial: overwrite`, possible on an existing slskd): it starts a leg over from byte 0. lull sees it (the file shorter than X, or its first bytes no longer the hole) and copies that leg from 0, which is still the copy's own bytes. Seeking ahead is then off until the hub restarts, and legs run to the end rather than stop at bytes that arrived.

## Better copies

When the copy the player streams is a lesser one, the best copy by the streaming profile is fetched beside it, and the player switches to it mid-song without an audible jump (ADR-0013). Settings › Sources › "Switch to a better copy while a song plays" (`sources.switchToBetter`, on by default) turns it off.

- **Lesser copies** (`fetch.rs::lesser`): YouTube's (a `fallback`, or `startFast`), one below the streaming minimum (`playBest`), or a peer the fetch fell back to after better ones failed.
- **Better** (`sources::better`): passes the streaming minimum, isn't busy beyond `maxPeerQueue` or one of the peers that failed this play; against a peer fallen back to, it must also come before it in the profile's order.
- **When it starts.** Whenever the player streams a lesser copy: on opening the song (a preload or cached copy it joins included) and on each new attempt. Only when a better copy is known, or nothing is known yet; never for a copy the listener picked. It is an upgrade fetch (`Slot::Upgrade`, out of the player's way), Soulseek only, and the profile's `below` rule never applies to it. With no better copy left it searches once more, in the `Prefetch` lane of the [search budget](soulseek.md#search-budget), then gives up quietly. Start fast's own upgrade (the saving profile's copy) is offered the same way when it passes the streaming minimum and the cap.
- **The offer.** The playing song's `stream` events carry `upgrade { peer, path, format, status, received, size, ready }` while one is on its way, sent again as it moves. `ready`: all of it is here, or it arrives at least 1.5 times faster than it plays (`upgrade_ready`). A finished upgrade stays in the map while the player streams the song, so it can be switched to.
- **The switch.** The player plays the copy silently from `GET /api/stream/{id}/copy?peer=&path=` (serves whichever fetch holds that copy, changing nothing), lines it up, and calls `POST /api/stream/{id}/switch { peer, path }` right before its crossfade: the upgrade becomes the song's live fetch, and the copy before it stops once its last response ends (at most 60 s later), unless a save needs it. A manual pick of the same copy adopts it too (`?peer=&path=`).
- **When the player can't line them up**, nothing changes mid-song and the better copy plays from the next play: the cache's choice is the best copy, not the latest ([cache.md](cache.md)).
- **Preloads.** A song whose only cached copy is a lesser one is preloaded again when a better copy is known, so it plays that from its start.
- **Saves.** `save_candidate` reuses the better copy on its way (or finished) before the one playing.

## Preloading

So that next plays at once, the player lists the songs queued after the current one with `PUT /api/stream/prefetch { tracks }` (`prefetch.rs`): at most 3, in queue order, as many as `playback.prefetchCount` (0-3, default 2). Each call replaces the calling device's list (the gate's device id; requests without one share a list).

- **What starts.** In order, each listed song that isn't saved, finished in the cache or already fetching gets a fetch. Its search runs in the `Prefetch` lane of the search budget, behind the player's searches ([soulseek.md](soulseek.md#search-budget)), and is skipped when one already found copies. The fetch then waits (up to 30 s) while the playing song is still finding a copy or waiting for its first bytes, and the next song waits until this one's bytes flow (up to 30 s), so one peer at a time is asked.
- **Quietly.** A preload publishes no `stream` events (`transfer` and `job` as usual) and never waits as `choose`. When no copy passes the streaming minimum it starts only under `playBest`; under `keepLooking` or `ask` it doesn't start (or, after fallbacks, fails), and the player's own fetch applies the rule when the song plays. Songs no peer shares are left to the player too.
- **Let go.** A song no device lists any more is cancelled after 10 s, unless by then the player streams it, a save needs it or a device lists it again; the grace lets the player open the next song before or after sending its new list. A finished preload leaves the live fetches at once, so its file is an ordinary unpinned cache entry ([cache.md](cache.md)), newest in the eviction order.
- **Playing it.** `GET /api/stream/{id}` joins a running preload, with one `stream` event saying where it stands, and from then on it is the player's fetch (`below` rules included). A finished one plays from the cache at once.

## Where the bytes are

`LULL_TRANSFERS_DIR` is the folder holding slskd's incomplete and downloads folders, as lull sees it.
- **Folder names.** lull reads slskd's folder names from `GET /options` (administrator key); without it, it assumes the kit's `/transfers/{incomplete,complete}`. A folder lull can't see turns the `soulseek` status to `warn`.
- **Incomplete path.** slskd 0.26 writes a running download to `<incomplete>/<user>/<remote folders>/<file>`, with the peer's root (`@@abcde`, `C:\`, `\\server`) dropped. Each segment is sanitised: `/` and NUL become `_` on Linux slskd; Windows slskd also replaces `<>:"|?*`, so both spellings are tried.
- **How it's written.** slskd appends from where the leg starts and never sizes the file up front. lull reads the spelling that changed from how it left it. slskd's lock is advisory on Linux, so lull opens it read-only and takes no lock. What has arrived is the copy's `Ranges`, never a file's length.

## Serving bytes

- **Before answering.** The response waits until the fetch has a source (which gives the size and type) and its ranges hold the requested start, asking for that byte while it waits ([Legs](#legs)). Any range inside the file waits as long as the fetch keeps trying, a request from byte 0 or a seek ahead of the transfer alike: those bytes are coming, so the browser buffers rather than failing (its time-to-byte guess for VBR files or a big embedded cover can land past what the player thought had arrived). Only a range past the end of the file, or one a finished file turned out too short for, answers 416 (`Content-Range: bytes */size`). When another source took over meanwhile (a fallback, YouTube), the size and type are worked out again before answering.
- **Ranges.** They resolve against the file's full size from the search (`range.rs`): 200 for no range, 206 for one range, and suffix ranges are supported. Multi-range and malformed headers get the whole file.
- **The body.** It reads at most 64 KiB at a time, never past the end of the range it is in (`range::sendable`), then waits at the gap, asking for the byte there. Memory per response is one chunk.
- **When the file is recreated.** A Soulseek copy's file is lull's and never moves. A YouTube copy's may be recreated by yt-dlp: whenever the ranges hold bytes the open handle doesn't show, or the handle fails while the transfer runs, the reader reopens by path at the same position (on a Windows host, Docker Desktop leaves old handles on the old copy).
- **Fallback ends a response** with an error, since another peer's file has different bytes. The player sees the `stream` fallback event, reopens the stream URL and seeks back by time.
- **Saved tracks** stream from the library crate (`/api/library/stream/{id}`), not here.
