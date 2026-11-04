# ADR-0015: Sources ranked by the speed lull measured, not the one peers report

- Status: accepted (owner, 2026-11-04: "without a bloaty way", for real gains, measuring once a download starts, and ranking and showing by measurements)
- Date: 2026-11-04

## Context

The owner played a song whose source picker showed one peer at 8.6 MB/s and another at 4.7. The first kept buffering, while the second was fast.
- **Where the figure comes from:** it is the search answer's `uploadSpeed`. The Soulseek server keeps it as an average of that peer's past uploads, to anyone, at any time, and the peer can report it however its client does. It says little about how fast that peer sends to this hub now.
- **What it decided:** it was the speed factor, up to 30 of the 100 ranking points (`rank.rs`), and it broke ties under `fastest`. A peer with a flattering figure was picked first, and the listener saw that figure as if it were a measurement.
- **What lull already had:** every transfer reports `averageSpeed` from slskd. For a running transfer that is a moving average; for a finished one, the bytes sent over the time since the peer started sending, queue time and resumed bytes excluded (Soulseek.NET 10.0.2, `TransferInternal.UpdateProgress`). lull showed it as the live rate, then forgot it.
- **What lull did with a slow source:** nothing. A source only gave way when it stalled (no bytes for `stallTimeout`) or kept us queued for 30 s. A peer that sends steadily, but slower than the song plays, kept the listener buffering all song long.

## Can slskd tell a peer's speed without downloading from it?

No: nothing measures throughput to this hub short of a transfer. What slskd 0.26.2 and Soulseek offer:
- **`GET /users/{u}/info`:** the peer's own free-slot flag, queue length and slot count. These are the same fields its search answer already carries, and ranking already uses them.
- **`GET /users/{u}/status`:** presence only.
- **The server's user statistics** (`averageSpeed`, upload, file and folder counts): the same server average as `uploadSpeed`, and slskd 0.26.2 has no endpoint for it anyway.
- **Ping:** Soulseek's is to the server, not to a peer. A round-trip time would say little about bandwidth anyway.

**Are the free signals weighted right?**
- **The weights:** a free slot and an empty queue give 30 points. A busy peer gets 4, plus 15 less 2 per person queued.
- **What that means:** a busy peer outranks a free one only when its speed points make up 11 to 26 points. That needs a speed at least 1.2 to 2.9 MB/s higher.
- **With measured speeds:** that is a real difference now, and its queue wait is bounded by the 30 s queue fallback.
- **Verdict:** they are weighted reasonably, and stay as they are.

## Decision

**Measured on the transfer itself, at no extra cost.**
- **The live rate** is slskd's `averageSpeed`, which the fetch already polls every second and now sends in `stream` events (`speed`).
- **When a transfer ends** (finished, stopped, failed, stalled or given up), the rate is remembered against the peer. It counts once it has sent 1 MB or for 10 s, whichever comes first: before that, the connection's start-up is most of it.
- **No new service, request or background traffic.**

**A source slower than the song gives way.** Once a transfer has sent for 10 s, if its live rate is under 1.25 times what the song needs (its size over its length), and the fetch has an untried copy expected to be at least twice as fast, the fetch moves on to that copy, as after a stall.
- **The peer is demoted by its measured rate,** not marked as stalled.
- **The player reopens at the same time,** as after any fallback.
- **It never applies** to a copy the listener picked or a save named.

**One small record per peer.** It lives in `soulseek_speeds` (user, rate, weight, reported, measured_at), beside the stall record, and is loaded into memory with it.
- **Each measurement's weight halves every 7 days,** so a peer that was fast last month and slow today reads slow.
- **`reported`** is what the peer said its speed was when last measured, kept for calibration.

**Ranking expects the measured rate, and calibrates the reported one.**
- **Measured peers:** the speed factor uses the measured rate.
- **Unmeasured peers:** it uses the reported figure scaled by the median, over peers with both, of measured divided by reported. That is 1 until 5 peers have both. It is this hub's own record of how far to trust the figure, not a constant picked by hand.
- **Ties under `fastest`:** broken on the same expected rate.
- **Unchanged:** the format, slot, queue and stall factors stay as they are, and so does the 3.3 MB/s at which the speed factor is full.

**The listener sees which figure is which.**
- **Measured:** the source picker's speed column shows "1.2 MB/s", with when in its tip ("Measured yesterday").
- **Unmeasured:** it shows "~8.6 MB/s" in the muted text colour. Its tip says it is the peer's own figure, not measured yet, and its accessible name is "8.6 MB/s, as the peer says".
- **Sorting by speed** follows ranking's expectation.
- **The playing source** shows its live rate, as before.
- **In the API:** `/api/tracks/{id}/sources` rows gain `speedMeasured`, `measuredAt` and `expectedSpeed`.

## Why these numbers

- **10 s before a live rate is judged:** slskd's live average (one update a second, α 0.2) still holds 11 % of its first second's rate after ten, so a slow start no longer swings it. It is also long enough for a rate to be worth remembering.
- **1.25 times the song's rate:** the song plays at 1x. Below 1.25x the download barely stays ahead, and the 25 % covers the average's wobble and what the player reads ahead.
- **Twice as fast:** a switch costs a new request's start-up (queue, connection) and a rebuffer for the listener. Only a copy expected to be at least twice as fast wins that back for certain, and well beyond how far an expectation can be off.
- **1 MB:** under it, a transfer is mostly TCP start-up and the peer's first burst, not its sustained rate.
- **A 7-day half-life:** a peer's line, load and slot count change over days. A week keeps the peers a weekly listener meets known, and lets last month's measurements fade.
- **5 peers to calibrate:** fewer, and one odd peer (a liar, or a peer on a LAN) sets the scale for every unmeasured one. Five gives the median something to hold on to.

## Racing: considered, not adopted

Starting the top two candidates for a few seconds and keeping the faster would find the fast one at once, but:
- **It costs the peers.** Each request takes an upload slot. Cancelling after seconds spends the loser's bandwidth on bytes lull throws away (different peers' files differ, so they are no use to the winner). Many Soulseek users dislike clients that cancel a lot, and some clients ban them.
- **It doubles the hub's start-up demand** on the listener's line at the moment the song should start.
- **slskd allows it** (downloads from different users run side by side). This is a politeness and bandwidth question, not a technical one.
- **What replaces it.** Measured ranking finds the fast peer for any peer lull has met, and the give-way rule fixes a wrong pick within seconds, one request at a time.

If measured ranking proves insufficient, racing could come back narrowly: only when the top two are both unmeasured, both have free slots, and the song is lossless.

## Before and after

The test: a 2 MB, 20 s song (it needs 100 KB/s). Peer "liar" reports 8.6 MB/s and sends 50 KB/s. Peer "honest" reports 4.7 MB/s and sends about 1 MB/s. It runs against a stand-in slskd, and a listener is simulated from the fetch's progress: it starts at 256 KB, plays at 1x, stalls when it reaches what has arrived, and resumes 2 s later. To rerun it: `cargo test -p lull-soulseek before_and_after -- --ignored --nocapture`.

| | first sound | stalls | time stalled |
|---|---|---|---|
| Before (the report ranks the liar first; nothing makes it give way) | 8.1 s | 5 | 25.0 s |
| After, first play (the liar gives way after 10 s) | 8.1 s | 1 | 0.8 s |
| After, once measured (the honest peer ranks first) | 1.0 s | 0 | 0.0 s |

## Consequences

- **Rankings shift as lull learns:** the same search can order peers differently next week. The score's speed factor shows the figure it used.
- **A new hub knows nothing,** so ranking starts as before, by reported figures, until peers are measured. It calibrates once 5 are.
- **One more fallback reason:** "<peer> sent N KB/s, slower than the song plays".
