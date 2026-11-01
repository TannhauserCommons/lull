# Progressive streaming and peer fallback

How `GET /api/stream/{trackId}` plays a song while slskd is still downloading it (`fetch.rs`, `stream.rs`, `range.rs`, `layout.rs` in `crates/soulseek`).

## Fetches

A **fetch** gets one song onto disk. There is at most one live fetch per track, shared by everyone who wants the song (the player streaming it, a save completing it), so the bytes cross the network once.

A finished copy already in the stream cache plays at once, whatever the `below` rule, with one `stream` event (`streaming`, and `belowMinimum` when it doesn't pass the streaming minimum). Otherwise:

1. **Pick a source.** The listener's pick (`?peer=&path=`) comes first, then the first untried copy that passes the profile's minimum, in its order ([soulseek.md](soulseek.md#source-rules)): the streaming profile when the player started the fetch, the saving one when a save did. With no known source the fetch searches for the song first (`finding`).
2. **None passes.** When copies exist but none passes the minimum, the profile's `below` decides:
   - `playBest`: the best of them plays; the `stream` event says `belowMinimum: true`, so the player can show a badge;
   - `keepLooking`: search again, up to 3 searches 20 s apart, then fail with "no copy meets your streaming minimum";
   - `ask`: the fetch waits as `choose`, with the copies found in the `stream` event's `choices` (up to 8). The listener answers by playing one (`?peer=&path=`), which replaces the fetch, or by skipping; after 10 minutes it fails;
   - a save's fallback (`saveBestUpgrade`) takes the best one; under `wait` and `ask` library decided before starting ([save-flow.md](save-flow.md)), so the fetch fails rather than save a copy below the minimum.
3. **Make room** in the cache for the file ([cache.md](cache.md)), then queue it in slskd.
4. **Watch it every second**, and give up on this peer when:
   - it hasn't accepted the request within 20 s;
   - it has kept the transfer queued for 30 s;
   - no new bytes have arrived for Settings › Sources' stall timeout (`stallTimeout`, 15 s by default; shown as `stalled` after 5 s);
   - slskd reports the transfer failed.
5. **Fall back** to the next copy in the same order. A stall is remembered against the peer (ranking demotes it); an offline peer or a long queue is not. A fetch gives up after 5 peers. Peers whose queue is longer than `maxPeerQueue` are never picked automatically.
6. **Done:** slskd moves the file to its downloads folder. Its final path comes from slskd's `DownloadFileComplete` event; the cache records it, and slskd's transfer record is removed.

**Switching source mid-song.** `GET /api/stream/{id}?peer=&path=` with another copy than the one playing replaces the fetch: the old one is cancelled, unless a save is completing it; then it runs on for the save, out of the player's way (no more `stream` events). The old response ends; the player opens the new one and seeks back by time (ADR-0008).

**Stopping.** `DELETE /api/stream/{id}` (skip, next) cancels the fetch unless a save still needs it. slskd keeps the partial file, and asking for the song again resumes it.

**Events.** Each change publishes:
- `stream` (to the player, only while it streams);
- `transfer`;
- `job` (an Activity row).

Stream status follows the fetch: `finding → queued → buffering` (under 256 KB received) `→ streaming`, with `stalled`, `fallback`, `choose` and `failed`.

## Where the bytes are

`LULL_TRANSFERS_DIR` is the folder holding slskd's incomplete and downloads folders, as lull sees it.
- **Folder names.** lull reads slskd's folder names from `GET /options` (administrator key); without it, it assumes the kit's `/transfers/{incomplete,complete}`. A folder lull can't see turns the `soulseek` status to `warn`.
- **Incomplete path.** slskd 0.26 writes a running download to `<incomplete>/<user>/<remote folders>/<file>`, with the peer's root (`@@abcde`, `C:\`, `\\server`) dropped. Each segment is sanitised: `/` and NUL become `_` on Linux slskd; Windows slskd also replaces `<>:"|?*`, so both spellings are tried.
- **How it's written.** The file is written front to back and never sized up front, so its length on disk is the number of bytes received. slskd's lock is advisory on Linux, so lull opens it read-only and takes no lock.

## Serving bytes

- **Before answering.** The response waits until the fetch has a source (which gives the size and type) and bytes cover the requested start. A request from byte 0 waits as long as the fetch keeps trying. A seek past what has arrived waits 8 s, then answers 416 (`Content-Range: bytes */size`).
- **Ranges.** They resolve against the file's full size from the search (`range.rs`): 200 for no range, 206 for one range, and suffix ranges are supported. Multi-range and malformed headers get the whole file.
- **The body.** It reads at most 64 KiB at a time, never past the file's current length on disk (`range::sendable`), then waits for slskd to write more. Memory per response is one chunk; nothing is copied to a second file.
- **When the file moves or is recreated.** slskd moves the finished file, and may recreate the incomplete one when it retries a peer. Whenever the fetch sees more bytes at its path than the open handle shows, or the handle fails while the transfer runs, the reader reopens by path at the same position (on a Windows host, Docker Desktop leaves old handles on the old copy).
- **Fallback ends a response** with an error, since another peer's file has different bytes. The player sees the `stream` fallback event, reopens the stream URL and seeks back by time.
- **Saved tracks** stream from the library crate (`/api/library/stream/{id}`), not here.
