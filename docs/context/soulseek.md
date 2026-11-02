# Soulseek: slskd, search, ranking, sharing

`crates/soulseek` is the only code that talks to slskd. Why slskd and not a native client: [ADR-0010](../adr/0010-soulseek-through-slskd.md). Streaming and fallback are in [streaming.md](streaming.md), the cache in [cache.md](cache.md).

## Map

| File | Holds |
|---|---|
| `slskd.rs` | The HTTP client and slskd 0.26's shapes (only the fields lull reads) |
| `files.rs` | Song identity from a peer path, format facts from file attributes |
| `search.rs` | Search sessions, merging, album folders, result rows, resolve searches, `onlySoulseek` |
| `matching.rs` | Whether a file is the song asked for (loose names, length) |
| `prewarm.rs` | `search.catalog`: pre-warming the catalog's top tracks |
| `prefetch.rs` | Preloading the songs queued next (`PUT /api/stream/prefetch`) |
| `rules.rs` | The streaming and saving profiles and peer limits, from the `settings` event |
| `sources.rs` | Every known peer file per song, in a profile's order on read |
| `rank.rs` | The score and its factors |
| `peers.rs` | Peers that stalled recently (`soulseek_peers`) |
| `share.rs` | Sharing checks and rescans |
| `recover.rs` | When to ask slskd for a fresh login (stale sessions) |
| `lib.rs` | Start-up, the `soulseek` status, the `Transfers` seam |

## slskd

- **Auth and status.** Auth is `X-API-Key: LULL_SLSKD_API_KEY` against `/api/v0`. The `soulseek` status reads `GET /application` (any key may read it); `/server` and `/options` need the administrator role. States:
  - `ok`: logged in;
  - `warn`: reconnecting, or connected with a problem: lull can't see slskd's folders, shares don't match `LULL_SHARE`, or searches get no answers (below);
  - `down`: slskd unreachable, the key rejected, or slskd [stuck mid-transition](#stuck-transitions).
- **The account of an slskd lull runs** (`account.rs`, ADR-0002). With `LULL_SLSKD_CONFIG_FILE` (the desktop app's sidecar), every start writes `LULL_SOULSEEK_USERNAME`/`PASSWORD` into that `slskd.yml`, which holds nothing else. When the file changed, lull waits up to 60 s for slskd to reload it, then logs in again (or out, when the account was removed). Details: [hub.md](hub.md).
- **State flags.** slskd sends state fields as comma-separated flag names (`Completed, Succeeded`, `Queued, Remotely`). A transfer slskd will retry itself still reads `Completed, Errored` while it waits, with `nextAttemptAt` set; lull treats that as queued. Only `Completed` without `nextAttemptAt` is final.
- **Enqueue.** Downloads are queued with `POST /transfers/downloads/batches`, which returns the transfer ids. Queueing a file that is already running returns a failure, and lull reuses the running transfer.

## Stale sessions

The Soulseek server can keep a session that slskd reports as logged in while none of its searches reach peers; seen on the homelab on 2026-10-31, and cured by a fresh login (`recover.rs`).
- **The trigger.** 3 searches in a row get no answer from any peer.
- **The fix.** lull asks slskd for a fresh login (`DELETE /server`, then `PUT /server`, whose watchdog retries until it's in).
- **Limits.** At most once per 15 minutes, never while a song is transferring (a new login drops peer connections), and logged as a warning.
- **The status** says what happened ("logged in again at 04:05 UTC…") until a search is answered again. While a fresh login isn't due, it quotes slskd's latest warning instead.
- **Diagnostics.** `GET /api/soulseek/diagnostics` shows slskd's own state, how recent searches ended, and its latest log lines.
- **No setting needed.** slskd's connection watchdog has none: it starts by itself after an unexpected disconnect and stops once connected. It deliberately doesn't start after "another client logged in", so lull doesn't fight a second login either.

## Stuck transitions

slskd can stop halfway through connecting or disconnecting, logged out, with its own connection watchdog off. Seen live on 2026-11-01: `Disconnecting` for over 40 minutes after a `Sequence contains no elements` exception, while every stream failed (`recover.rs`).

- **The trigger.** `/application` says logged out and transitioning for 60 s. Shorter transitions are normal; plainly logged out (not transitioning) is left to slskd's watchdog, as before.
- **Fresh logins first.** lull asks for one: `DELETE /server` (a wedged slskd may refuse it), then `PUT /server` regardless. Again 2 minutes later if still stuck.
- **Then a restart.** After two fresh logins that didn't help, lull asks slskd to restart itself (`PUT /application`). slskd 0.26 allows that only to a web session (`AuthPolicy.JwtOnly`), never to an API key, so lull logs in at `POST /session` with slskd's web login, `LULL_SLSKD_WEB_USERNAME` and `LULL_SLSKD_WEB_PASSWORD`. In the compose kit slskd takes the same values as its login; under Docker the restarting process ends the container and `restart: unless-stopped` starts it again. At most one restart per 30 minutes: an slskd that wedges again within that gets fresh logins only (4, 8, then every 15 minutes).
- **The status** turns `down` from 60 s on, saying for how long and what lull did: "slskd has been stuck at "Disconnecting", logged out, since 19:24 UTC (5 min); lull asked it to log in again 2 times, then asked it to restart itself at 19:29 UTC". `extra` carries `{ stuck, stuckSince, relogins, restartedAt, restartSlskd }`.
- **`restartSlskd: true`** (in `/health`, which is open) means a person or the deploy has to restart slskd: there is no web login to restart it with, the restart was refused, or slskd wedged again soon after lull restarted it. The desktop app's slskd runs headless, which refuses web logins, so there it is always the signal, and the app restarts its slskd on it (docs/context/desktop.md).

## Search

Search is a catalog search owned by discovery (`/api/search`, ADR-0008); Soulseek runs behind it at `GET /api/soulseek/search`.

- **slskd hides answers until a search ends.** It also only runs two searches at a time. So each query runs one network search that ends after 6 s with no new answers, or after 15 s in all, at which point lull stops it (resolve searches stop sooner, below).
  - While it runs, `search.results` events carry only the counters (`files`, `peers`, `done: false`).
  - The final event carries the merged `results` and `entities`, or `error`.
- **Waiting for slskd's last step.** slskd calls a search `Completed` before it writes the responses (that step sets `endedAt`), and deleting the search in between loses them: seen live as "Failed to finalize search … affected 0 row(s)" and an empty result after 300 peers answered. lull reads the responses until they appear or `endedAt` is set, for up to 5 s. If peers answered (or the search `Errored`) and still no responses come back, the final event carries `error` ("slskd found 8217 files from 300 peers but handed over none; search again"), not an empty success.
- **Politeness:**
  - searches start at least 1 s apart;
  - searches take turns by lane (`search::Lane`), since slskd runs two at a time: a preload's or pre-warm's search waits until no search for the player (a song playing or saved, the listener's own search) is running, and is skipped if that one found the song;
  - a newer query stops the older one;
  - the same words within 10 minutes reuse the earlier search (the answer's `soulseek` field replays it), but only one still running or one that ended with results: an empty or failed search, or one a newer query stopped, runs again;
  - queries under 3 characters don't search;
  - lull deletes its searches from slskd when done.
- **Merging.** Songs merge across peers by `TrackId`. A peer folder holding 3 or more of the results becomes an album row; the same album on many peers is one row listing each folder, best first. Rows are capped at 200 songs and 40 albums. Every track handed out is recorded in `lull_core::catalog`.
- **Duplicates.** Saved songs among the results come from `Library::saved`: the row gets `dup: note`, or `dup: better` when the Soulseek file clearly beats the saved one (`Quality::beats`: lossless over lossy, or at least a fifth more bitrate).
- **Only on Soulseek.** Discovery publishes `search.catalog { query, trackIds }` for a query's top catalog tracks. The final results carry `onlySoulseek`: the unsaved result songs no catalog track matches (by `matching`). When the catalog arrives after the results, the final `search.results` is published again with it; before the catalog has answered it is `null`.
- **Pre-warming** (`prewarm.rs`). The top 3 tracks of each `search.catalog` get a resolve search in the background (the preload lane), one at a time and at least 5 s apart, so pressing play finds copies waiting. A newer query replaces what an older one left queued; songs with known copies, saved songs and an offline Soulseek are skipped.
- **Resolve searches.** A song nothing is known about (a queue item, a catalog pick, a save) gets a search for `artist title`. Files count as the song by [matching](#matching), whatever their album, and are kept under the song's id too. When the first search finds no copy, a looser second one runs: the title without its tags plus the artist's longest word. Sources, streams and saves trigger it themselves.
- **Resolve searches stop early.** They want a few good copies, not every answer, so lull stops one (slskd's stop call keeps what arrived; then the usual wait for slskd's last step) as soon as:
  - 15 peers have answered and it has run 2 s;
  - answers have been quiet for 1.5 s after the first ones;
  - nobody has answered within 4 s (the looser search goes next);
  - or 8 s have passed.

  A cold song typically has its copies in 2-4 s instead of 7-21 s. The listener's own search still takes every answer, since it fills a results page.

## Matching

`matching.rs`, ADR-0008. A file is the song asked for when:
- **the titles match** after `ids::loose_title` (the catalog merge's normalization: feat., version, remaster, edit and live tags and bare years dropped, accents folded, "&" = "and"), allowing for spacing ("dont" / "don t") and one slip per 8 letters. Numbers must agree ("Part 1" isn't "Part 2"), and a word such as remix, acoustic, instrumental, demo or reprise on one side only makes another recording;
- **the artists match** the same way (`loose_artist`), or one of several credited ("A, B & C", "A x B") does, or, when the path parser got the artist wrong (a compilation), the artist's name appears in the file's path;
- **the lengths agree** within 6 s, when both the song and the file report one. An edit or a live take is another recording.

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

## Source rules

The listener's streaming and saving profiles (ADR-0008, `lull_core::quality`) arrive in the `settings` event (`rules.rs`, ADR-0050); until collection's start-up copy, the defaults apply. `Sources::ordered` lists a song's copies for one profile:

1. **Gate.** A copy `meets` the profile's minimum by `Quality::meets` with the format from its extension (`quality::format_of`: `.aif` is AIFF). Its bitrate is the peer's, else worked out from size and length (the file's, else the song's); a lossy file of unknown bitrate fails a floor.
2. **Order.** Copies that pass come first, then the rest, each by the profile's `pick`:
   - `fastest`: the score, then speed;
   - `highest`: lossless first (hi-res above CD), then bitrate; the score breaks ties.

   **Streaming cap.** With `stream.maxKbps` set (128, 192, 256 or 320; 0 = none), streaming takes copies within the cap before copies above it, among passing copies and among the rest; lossless counts as above any cap, and a lossy copy of unknown bitrate as within it (`Quality::within`). Copies above it are marked `aboveMax`. It is a preference, not a gate: when only copies above the cap pass, one of them plays. Saving has no cap.
3. **Peer limit.** A busy peer (no free slot) with more people waiting than `maxPeerQueue` is marked `queueTooLong`, listed last and never picked automatically.

Playing and fallback take copies in this order ([streaming.md](streaming.md#fetches)). A save takes the first one (`Transfers::save_candidate`), reusing the copy already streamed or cached when it passes and, under `highest`, nothing beats it. `GET /api/tracks/{id}/sources` returns the streaming order, or the saving one with `?for=save`.

## Sharing

slskd's shares come from its start-up settings: the kit sets `SLSKD_SHARED_DIR=/music`, the library mounted read-only, unless `LULL_SHARE=none`. slskd 0.26 can't change shares through an API key. So lull:
- checks `GET /shares` on each reconnect, and reports a mismatch with `LULL_SHARE` as `warn`;
- with `LULL_SHARE=library`, asks slskd to rescan (`PUT /shares`) 60 s after the last finished save, so new songs are shared.

## Tables

- `soulseek_files`: every file lull asked slskd for (the cache index; see [cache.md](cache.md)).
- `soulseek_peers`: stall memory.
