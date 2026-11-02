# Stream cache and eviction

Streamed-but-unsaved audio, capped at `LULL_CACHE_MAX_GB` (`crates/soulseek/src/cache.rs`).

## What it holds

- **Only lull's own files.** The cache is every file lull asked slskd for, recorded in `soulseek_files`: the source, the path on disk (the incomplete file while it runs, then the finished one), whether it's complete, and when it was last used.
- **Never anything else.** Nothing that isn't recorded is ever deleted, so an existing slskd's other downloads are safe.
- **The disk is the truth.** A row whose file is gone (deleted by hand, moved away) is dropped the next time usage is measured. Usage counts bytes actually on disk, partial files included.

## Eviction

Before each transfer and every 60 s, `make_room(incoming)` evicts least recently used files until:
- `used + incoming ≤ cap`;
- the transfers disk keeps 200 MB free beyond the incoming file (`fs4::available_space`).

**Pinned files are never evicted.** A track is pinned while:
- **it's playing:** its fetch is live. A finished song stays live until nobody has read it for 30 minutes, or until the player skips it;
- **it's preloading:** a preload ([streaming.md](streaming.md#preloading)) is pinned while it transfers. Once finished it is unpinned at once, like any file nobody plays;
- **it's being saved:** from `Transfers::complete` until that track's `save` event reports `failed` or `pending`, or `done` at step 5.

Pinned files may hold the cache over its cap; it reports `warn` rather than cut off the song that's playing.

## Full disk

If even evicting everything evictable can't fit the next file, the fetch fails with "the transfers disk is full" and the `cache` status goes `down`, instead of slskd failing mid-write.

## Status

`cache` status: `{ state, detail, extra: { usedBytes, capBytes } }`.
- `ok`: "3.1 of 5.0 GB used, oldest audio evicted first";
- `warn`: over 95 % of the cap, held there by pinned files;
- `down`: the disk is full.
