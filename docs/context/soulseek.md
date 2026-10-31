# Soulseek: slskd, search, ranking, sharing

`crates/soulseek` is the only code that talks to slskd. Why slskd and not a native client: [ADR-0010](../adr/0010-soulseek-through-slskd.md). Streaming and fallback are in [streaming.md](streaming.md), the cache in [cache.md](cache.md).

## Map

| File | Holds |
|---|---|
| `slskd.rs` | The HTTP client and slskd 0.26's shapes (only the fields lull reads) |
| `files.rs` | Song identity from a peer path, format facts from file attributes |
| `search.rs` | Search sessions, merging, album folders, result rows |
| `sources.rs` | Every known peer file per song, ranked on read |
| `rank.rs` | The score and its factors |
| `peers.rs` | Peers that stalled recently (`soulseek_peers`) |
| `share.rs` | Sharing checks and rescans |
| `lib.rs` | Start-up, the `soulseek` status, the `Transfers` seam |

## slskd

- **Auth and status.** Auth is `X-API-Key: LULL_SLSKD_API_KEY` against `/api/v0`. The `soulseek` status reads `GET /application` (any key may read it); `/server` and `/options` need the administrator role. States:
  - `ok`: logged in;
  - `warn`: reconnecting, or connected with a problem (lull can't see slskd's folders, or shares don't match `LULL_SHARE`);
  - `down`: slskd unreachable or the key rejected.
- **State flags.** slskd sends state fields as comma-separated flag names (`Completed, Succeeded`, `Queued, Remotely`). A transfer slskd will retry itself still reads `Completed, Errored` while it waits, with `nextAttemptAt` set; lull treats that as queued. Only `Completed` without `nextAttemptAt` is final.
- **Enqueue.** Downloads are queued with `POST /transfers/downloads/batches`, which returns the transfer ids. Queueing a file that is already running returns a failure, and lull reuses the running transfer.

## Search

- **Library first.** `GET /api/search` answers at once with library hits (`Library::search`, 4 s budget). If the library errs or is missing, the answer carries `libraryError` and search goes on.
- **slskd hides answers until a search ends.** It also only runs two searches at a time. So each query runs one network search that ends after 6 s with no new answers, or after 15 s in all, at which point lull stops it.
  - While it runs, `search.results` events carry only the counters (`files`, `peers`, `done: false`).
  - The final event carries the merged `results` and `entities`.
- **Politeness:**
  - searches start at least 1 s apart;
  - a newer query stops the older one;
  - the same words within 10 minutes reuse the earlier search (the answer's `soulseek` field replays it);
  - queries under 3 characters search the library only;
  - lull deletes its searches from slskd when done.
- **Merging.** Songs merge across peers by `TrackId`. A peer folder holding 3 or more of the results becomes an album row; the same album on many peers is one row listing each folder, best first. Rows are capped at 200 songs and 40 albums. Every track handed out is recorded in `lull_core::catalog`.
- **Duplicates.** Saved songs among the results come from `Library::saved`: the row gets `dup: note`, or `dup: better` when the Soulseek file is lossless and the saved one isn't, or is lossy by 64 kbps or more.
- **Resolve searches.** A song nothing is known about (a queue item, a discovery pick) gets a search for `artist title`. Any file with the same artist and title counts, whatever its album. Sources and streams trigger it themselves.

### Reading paths

A song's id comes from its path, so the parser decides what merges (`files.rs`, tested on differently organised shares):
- `[@@root\]…\Artist\Album (1998) [FLAC]\CD2\03 - Title.flac`;
- `Artist - Album` folders;
- `Artist - 03 - Title` files;
- disc folders;
- `Various Artists` compilations, where the file names the artist.

Bracketed release tags (`[FLAC]`, `(1998)`, `[24-96]`) are dropped; brackets that belong to a title stay. Share roots such as `Music`, `share` or `Downloads` never count as an artist.

## Ranking

The score runs from 0 to 100 and is the sum of four factors (`rank.rs`):

| Factor | Points |
|---|---|
| format | Up to 40 by position in `LULL_PREFERRED_FORMATS`, minus 8 per step down. Lossy formats are scaled by bitrate over 320 kbps; hi-res lossless takes ×0.95. A format not in the list gets 0. |
| speed | Up to 30, full at 3.3 MB/s |
| slot | 15 with a free upload slot, else 4 |
| queue | 15 with a free slot, else 15 − 2 × queue length |

A peer that stalled on lull in the last 7 days loses 25 points (`factors.stalled`) and is flagged `stalledBefore`; a finished transfer from it clears the record. `preferred` marks the first preferred format.

## Sharing

slskd's shares come from its start-up settings: the kit sets `SLSKD_SHARED_DIR=/music`, the library mounted read-only, unless `LULL_SHARE=none`. slskd 0.26 can't change shares through an API key. So lull:
- checks `GET /shares` on each reconnect, and reports a mismatch with `LULL_SHARE` as `warn`;
- with `LULL_SHARE=library`, asks slskd to rescan (`PUT /shares`) 60 s after the last finished save, so new songs are shared.

## Tables

- `soulseek_files`: every file lull asked slskd for (the cache index; see [cache.md](cache.md)).
- `soulseek_peers`: stall memory.
