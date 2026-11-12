# Stream cache and eviction

Streamed-but-unsaved audio, capped at `LULL_CACHE_MAX_GB` (`crates/soulseek/src/cache.rs`).

## What it holds

- **Only lull's own files.** The cache is every copy lull fetched, recorded in `soulseek_files`: the source, the path on disk, whether it's complete, when it was last used, and while it's unfinished which byte ranges have arrived (`ranges`, ADR-0014). A Soulseek copy is lull's own file under `lull/` in the transfers folder, filled in [legs](streaming.md#legs); slskd's files for a leg are deleted when the leg ends. A later fetch of an unfinished copy fetches only its missing ranges. YouTube copies are recorded the same way, written to `youtube/` in the same folder ([youtube.md](youtube.md#downloads)).
- **The best copy first.** Playing a song from the cache takes its best finished copy (`Fetcher::best_cached`): Soulseek's over YouTube's, one that passes the profile's minimum, then the profile's order, and only then the one used last. A save picks by the saving profile the same way.
- **Never anything else.** Nothing that isn't recorded is ever deleted, so an existing slskd's other downloads are safe.
- **The disk is the truth.** A row whose file is gone (deleted by hand, moved away) is dropped the next time usage is measured. Usage counts bytes on disk, partial files included: for a copy arriving in legs, the bytes its ranges hold, since its file has holes.

## Eviction

Before each transfer and every 60 s, `make_room(incoming)` evicts least recently used files until the following hold (a Soulseek copy's `incoming` is twice its size: lull's file and slskd's for the leg running):
- `used + incoming ≤ cap`;
- the transfers disk keeps 200 MB free beyond the incoming file (`fs4::available_space`).

**Pinned files are never evicted.** A track is pinned while:
- **it's playing:** its fetch is live. A finished song stays live until nobody has read it for 30 minutes, or until the player skips it;
- **it's preloading:** a preload ([streaming.md](streaming.md#preloading)) is pinned while it transfers. Once finished it is unpinned at once, like any file nobody plays;
- **it's being saved:** from `Transfers::complete` until that track's `save` event reports `failed` or `pending`, or `done` at step 5.

Pinned files may hold the cache over its cap rather than cut off the song that's playing; it reports `warn` then.

## Clearing

`POST /api/cache/clear` (Settings › Cache › Clear cache) removes every file nothing pins, finished or partial, and answers `{ files, freedBytes, keptBytes }`. The files of songs playing, preloading or being saved stay, and go by eviction once nothing holds them. Later plays of what went are fetched again.

## Full disk

If even evicting everything evictable can't fit the next file, the fetch fails with "the transfers disk is full" and the `cache` status goes `down`, instead of slskd failing mid-write.

## Status

`cache` status: `{ state, detail, extra: { usedBytes, capBytes } }`.
- `ok`: "9.8 of 10.0 GB used, oldest audio evicted first". A cache in use sits just under its cap, since eviction makes room for each file only as it comes: that is the cache working, not full.
- `warn`: pinned files alone are over the cap ("music that is playing or being saved holds it over its size until it ends"). Nothing stops playing.
- `down`: the disk is full.

Before (2026-11-12), `warn` was any use over 95 % of the cap, which a cache in use reaches and stays at, and the web said "Cache full and every item is in use, so playback may pause": neither was measured, and nothing was held.
