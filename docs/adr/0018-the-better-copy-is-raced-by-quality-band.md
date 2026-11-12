# ADR-0018: The better copy is raced, by quality band

- Status: accepted (the owner, 2026-11-12: get the better copy "much faster than waiting the entire, for example, three minutes"; "not always the highest — consider whether peers have it, are available, fast enough to stream"; reviewed by Fable 5.2)
- Date: 2026-11-12
- Updates: ADR-0013 (switching to a better copy mid-song), ADR-0015 ("Racing: considered, not adopted"), ADR-0017 (presets decide the bands)

## Context

A song that starts on a lesser copy (YouTube's Opus 160, a peer's MP3 128) gets its better copy one peer at a time (`fetch.rs`, `soulseek`): up to 5 peers (`MAX_ATTEMPTS`), each allowed 20 s to accept and 30 s in its queue (`QUEUE_TIMEOUT`, dropped at once when YouTube is ready, `youtube_ready`). The owner waited minutes for copies that never came.

What the hub's own slskd showed (2026-11-12, 78 real fetches, 11 searches; `scratch/soulseek-speed/findings.md`):
- 47 % of requests sent bytes, a median 1.9 s after the request; 53 % sent nothing (queued out, errored after 20-25 s, refused).
- Dead peers dominate the wait: 20-30 s each, tried one at a time, up to ~150 s.
- 2 of 10 "free slot" peers had queues of 10-12: the flag doesn't mean "sends now". A peer's reported speed says little (ADR-0015).
- Searches answer in ~2 s with 10+ qualifying candidates on most songs. One peer often lists one file several times.
- Cancelling a queued or running download triggers nothing automatic in slskd, Nicotine+ or the server; the real limits are per-peer queue and failure counts, and how often lull searches.

The owner's words: a proper upgrade to the preset's best "when achievable, never small steps"; peer-reported speed is unreliable; modular around the source presets (ADR-0017); "don't overcomplicate"; the same for every listener; don't fear leaving a peer's queue; and "make sure everything works including seeking".

## Decision

**A better copy is raced: a few peers at once, the first that proves it can keep ahead of the song wins, the rest are cancelled at once.**

1. **Searches are unchanged** (a resolve stops at 15 peers after 2 s). Candidates are deduped per peer: one copy per peer, its best.
2. **Quality bands** are defined once, in `lull_core::quality::Band`: `Lossless`; `HighLossy` (220 kbps and up, so V0 counts with 320); `LowerLossy`. A lossy bitrate lull estimated from size ÷ length (the answer gave none) counts one band down; lossless is known by its format and never moves. The device's preset decides the allowed bands (`StreamProfile::bands`): with a cap (`maxKbps`), lossless is never a band; a band needs a format the preset leaves on; `LowerLossy` only when the minimum is under 220 kbps. **A better copy is a higher band than the one playing**: Opus 160 → MP3 192 is no switch, Opus 160 → MP3 320 is, MP3 256 → 320 no longer is.
3. **The race decides the target.** It tries the highest allowed band first; when that band's round fails, it steps down one band, never to the playing copy's band or below. A **round**: up to 3 **lanes** at once, each a request to a distinct peer, ranked free slot first, then the speed lull measured (ADR-0015); the reported speed only breaks ties. A lane with **no byte within 6 s** of its request, queued or not, is replaced by the next candidate (no instant drop for being remotely queued). A round asks at most 6 peers.
4. **The speed gate** counts lull's own bytes on the transfer, never slskd's or the peer's figure: judged once a lane has sent for 3 s, it passes when the bytes of its last 2 s are at least `OUTRUN` (1.5) times what the song plays in 2 s (size ÷ length). A copy that has all arrived passes. `OUTRUN` is the one constant the upgrade's ready rule uses too: the winner is `ready` from its first offer. A lane that has sent for 10 s (`SLOW_AFTER`) without passing gives its place to the next candidate. **The first lane to pass wins**; the others are cancelled at once and their partial files forgotten: deleted, not cached, their cache reservations released.
5. **One switch per play.** After the player switched (or the race failed), no other race runs for this play, even if a higher band turns up. When no allowed band has a peer that sends, one queue is kept (the shortest, at the best band) while the song plays: if it starts sending and passes the gate it is the winner; at the song's end a copy downloading finishes for the next play, and a mere queue position is left.
6. **The race sits beneath `Fetch`.** A fetch holds one source; the race hands the upgrade fetch the winner's running leg (its transfer, its bytes so far) and never requests it again. The playing fetch is never touched by the race: its source, attempt and job stay as they are until the player switches (`adopt`).
7. **A hub-wide lane budget of 4,** playing songs first, then preloads. Two presets on one song never fetch the same copy twice at once (`held_elsewhere`, ADR-0017). **Preloads** take one lane, ask one peer at a time, go straight for the preset's highest band, with the same 6 s rule, and wait as today while the playing song finds its copy.
8. **Politeness:** at most 3 cancels of an upload that was already sending, per peer per day. Beyond that, the peer isn't raced (it can still start a song or be picked).
9. **Removed:** the 30 s `QUEUE_TIMEOUT`, the `youtube_ready` special case, and the one-at-a-time `MAX_ATTEMPTS` loop for better copies. A listener's own pick keeps today's patience. A song's start is unchanged except for the shared lane rules: it counts in the budget, and its request with no byte in 6 s gives way (as a lane does) instead of after 20 s or 30 s.
10. **Said once, plainly.** One log line per race: song, band, lanes, winner, time to winner, cancels. When the song stays on its first copy, the playing job's note says why, starting "Stays on this copy: " (Activity shows it).

**Seeking must hold through all of it.** Seeking while a race runs, while lanes are cancelled, during and after the switch keeps the listener's place: no jump to 0, no restart, no repeat beyond the switch's own handover. A seek into a part the playing copy hasn't downloaded yet works as today (it waits or starts a leg, ADR-0014). After the switch, a seek anywhere in the new copy works. A cancelled loser's file is never served. The mini player and the lock screen show the song's place.

## Interface

What the builder adds, and nothing more is assumed by the tests:

```rust
// lull_core::quality
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Band { LowerLossy, HighLossy, Lossless }      // ordered: a higher band is a better copy
pub const HIGH_LOSSY_KBPS: u32 = 220;
impl Band { pub fn of(q: Quality, estimated: bool) -> Band; }
impl StreamProfile { pub fn bands(&self) -> Vec<Band>; }  // allowed, highest first
```

Everything else is observed through what exists: `Fetcher::stream`/`play`, `upgrade`, `event` (the `upgrade` offer), `adopt`, `copy`, `stop_streaming`, `prefetch`, `Cache::partial`/`finished`/`copy_file`, the playing fetch's `note`. Contracts the tests read:
- **No offer while lanes race.** `stream` events carry `upgrade` only once a lane has won; it names the winner and is `ready: true` from its first event.
- **The note** of the playing fetch starts "Stays on this copy: " when the race ends without a winner.
- **A lane is cancelled before its replacement is requested,** so a peer count never overshoots for a moment.
- **Lanes freed at one moment go to playing songs' races first;** a preload asks only once none waits.
- **Constants:** 3 lanes a round, 6 peers a round, 6 s to a first byte, gate judged after 3 s over the last 2 s at `OUTRUN` 1.5, `SLOW_AFTER` 10 s, 4 lanes a hub, 3 cancels a peer a day.

Test infrastructure is the verifier's, not the builder's: `crates/soulseek/src/fake_slskd.rs` (many peers, `Behaviour`, the request log) and the `#[cfg(test)] mod race_tests;` line in `fetch.rs`.

## Acceptance criteria

Every file below begins "Acceptance tests for ADR-0018: the builder must not edit this file."

| Id | Criterion | Test |
|---|---|---|
| R1 | Three bands; ≥ 220 kbps is high lossy (V0 with 320) | `crates/core/tests/adr0018_bands.rs` `bands_by_quality` |
| R2 | An estimated bitrate counts one band down; lossless never moves | `an_estimated_bitrate_counts_one_band_down` |
| R3 | The preset decides the bands, highest first; a cap means no lossless | `a_preset_allows_its_bands_highest_first` |
| R4 | A better copy is a higher band (Opus 160 → 192 no, → MP3 320 yes; 256 → 320 no) | `a_better_copy_is_a_higher_band` |
| R5 | Lanes race; the fast peer wins over silent, bursting and slow ones, offered `ready` within 14 s of the race's first request | `crates/soulseek/src/fetch/race_tests.rs` `r5_a_fast_peer_wins_over_silent_bursting_and_slow_ones` |
| R6 | A peer that queues, then sends at 2 s, keeps its lane and wins | `r6_a_peer_that_queues_then_sends_keeps_its_lane_and_wins` |
| R7 | ≤ 3 lanes; a lane with no byte in 6 s gives way to the next candidate | `r7_a_lane_with_no_byte_in_six_seconds_gives_way` |
| R8 | ≤ 6 peers a round; a failed round steps down one band | `r8_a_round_asks_at_most_six_peers_then_steps_down_a_band` |
| R9 | One request per peer; the winner is never requested again | `r9_one_request_per_peer_and_the_winner_is_never_asked_again` |
| R10 | Losers cancelled at once, their files deleted, not cached, never served | `r10_losers_are_cancelled_at_once_and_their_files_forgotten` |
| R11 | The gate counts lull's own bytes: a peer slskd calls fast but that sends slowly never wins | `r11_the_gate_counts_bytes_not_the_speed_slskd_reports` |
| R12 | One switch per play | `r12_one_switch_per_play` |
| R13 | No band sends: one queue kept (shortest, best band), left at the song's end; the note says why | `r13_with_no_band_sending_one_queue_is_kept_and_left_at_the_end` |
| R14 | A kept queue that started sending finishes after the song ends | `r14_a_kept_queue_that_sends_runs_on_after_the_song` |
| R15 | 4 lanes a hub, used across songs | `r15_two_songs_racing_share_four_lanes` |
| R16 | Playing songs get lanes before preloads | `r16_playing_songs_get_lanes_before_a_preload` |
| R17 | A preload asks one peer at a time, top band first, with the 6 s rule | `r17_a_preload_asks_one_peer_at_a_time_for_the_top_band` |
| R18 | ≤ 3 cancels of a sending upload a peer a day, then it isn't raced | `r18_a_peer_cancelled_three_times_today_is_not_raced` |
| R19 | A listener's pick keeps today's patience (guard) | `r19_a_pick_that_queues_is_waited_for` |
| R20 | A song's start gives way after 6 s with no byte | `r20_a_songs_start_moves_past_a_silent_peer_in_six_seconds` |
| R21 | One log line per race | review only (no subscriber in tests) |
| S1 | The race never touches the playing fetch (guard) | `s1_the_race_leaves_the_playing_copy_alone` |
| S2 | After the switch, a seek anywhere reads the winner's own bytes (guard) | `s2_after_the_switch_a_seek_anywhere_reads_the_winners_bytes` |
| S3 | A seek while the race runs keeps the place; no reload (guard) | `tests/ui/gates/race-seek.spec.mjs` "a seek while the better copy races keeps the listener's place" |
| S4 | A withdrawn offer is never switched to or served (guard) | "an offer the hub withdraws is never switched to" |
| S5 | A seek during the switch keeps the place (guard) | "a seek during the switch keeps the listener's place" |
| S6 | After the switch, a seek lands anywhere: at once where it arrived, kept until it arrives elsewhere (guard) | "after the switch a seek anywhere in the new copy lands" |
| S7 | A seek past what the playing copy has, while the race runs, is kept and lands (guard) | "a seek past what arrived, while the race runs, is kept and lands" |
| S8 | After the switch, the lock screen shows the song's place (guard) | "after the switch the lock screen shows the song's place" |
| S9 | Through the race and a seek, the desktop mini player shows the song's place (guard) | "through the race and a seek the mini player shows the song's place" |

## Running the acceptance tests

```sh
cargo test -p lull-core --test adr0018_bands   # R1-R4: "test result: ok. 4 passed; 0 failed"
cargo test -p lull-soulseek race_tests         # R5-R20, S1-S2: "test result: ok. 18 passed; 0 failed" (~30 s)
cd tests/ui && FIXTURE_PORT=8901 npx playwright test -c gates/playwright.config.mjs --project race-seek --reporter line   # S3-S9: "7 passed"
```

A failure names the criterion in its test name and message and says in one line what was seen (`R5 first offer: {"peer":"silent",...}`). On Windows, a Rust run may need `TMP`/`TEMP` under the checkout and an empty `RUSTC_WRAPPER`.

The Rust race tests run in real time against `fake_slskd` (a local HTTP stand-in for slskd): tokio's paused clock auto-advances while lull waits on the stand-in's sockets, so a paused-clock run reaches every timeout at once. They take about 30 s in all, run side by side, and keep a few seconds' slack on every bound. The rest of the soulseek suite runs against the same stand-in and must stay green (`cargo test -p lull-soulseek`).

## Rejected

- **Racing every copy at once:** costs peers' slots and the listener's line for nothing; 3 lanes find a sender ~85 % of the time within ~3 s at the measured 47 % send rate.
- **Always the highest quality:** the owner's words; a FLAC no peer sends is worth less than a 320 that plays now.
- **Small steps** (256 → 320, a second switch to a higher band later): each switch is a handover the listener may hear, for little; one switch, to a band clearly better.
- **slskd's or the peer's speed for the gate:** both measured wrong in practice (ADR-0015, findings). lull's own byte counts are free.
- **A user-info pre-check of slot and queue:** a median 1.07 s per peer and the free-slot flag doesn't mean "sends now"; a lane finds out in the same time.
- **A paused clock for the Rust race tests:** see above; real time with generous bounds instead.

## Consequences

- A typical better copy arrives in ~5-10 s instead of up to minutes. Losers send a few seconds of bytes lull throws away; the politeness cap bounds what any one peer pays.
- Up to 4 lanes run on the listener's line at once; playing songs take them first.
- MP3 256 no longer switches to 320 mid-song; a later play still takes the best cached copy.
- ADR-0015's "Racing: considered, not adopted" is superseded for better copies; a song's start still asks one peer at a time.
