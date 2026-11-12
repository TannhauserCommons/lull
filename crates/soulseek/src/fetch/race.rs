//! The better-copy race (ADR-0018): the mechanics beneath `Fetch`. Given rounds of candidate
//! copies (a quality band each, in the order to ask), it runs up to `LANES_PER_ROUND` lanes at
//! once, each a request to a distinct peer. A lane with no byte `FIRST_BYTE` after its request
//! gives its place to the next candidate; one that has sent for `JUDGE_AFTER` passes when the
//! bytes of its last `JUDGE_WINDOW` are at least `OUTRUN` times what the song plays in that time
//! (lull's own count, never slskd's or the peer's figure); one that has sent for `SLOW_AFTER`
//! without passing gives way too. The first to pass wins and the others are cancelled at once,
//! their partial files forgotten. A round asks at most `ROUND_PEERS` peers; when it fails the
//! next (lower) round runs. When no round has a sender, one queue is kept.
//!
//! It knows nothing of settings: `plan.rs` reads the device's preset and hands it the rounds.
//! The winner's running leg goes to `Fetcher::legs`, which carries on and never asks again.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use futures_util::stream::FuturesUnordered;
use lull_core::quality::Band;
use tokio::sync::watch;

use super::lanes::{Permit, Priority};
use super::legs::{Copy, Leg, Started, Watch, watch_leg};
use super::{Fetch, Fetcher, OUTRUN, POLL, SLOW_AFTER, Stop, reason, stopped};
use crate::slskd::{Phase, Transfer};
use crate::sources::Source;

/// Lanes a round runs at once.
pub(super) const LANES_PER_ROUND: usize = 3;
/// Peers a round asks in all.
pub(super) const ROUND_PEERS: usize = 6;
/// A request with no byte this long after it was made gives way (queued remotely or not).
pub(super) const FIRST_BYTE: Duration = Duration::from_secs(6);
/// A lane is judged once it has sent this long...
const JUDGE_AFTER: Duration = Duration::from_secs(3);
/// ...by the bytes of its last this long.
const JUDGE_WINDOW: Duration = Duration::from_secs(2);
/// How often a lane's file is looked at.
const TICK: Duration = Duration::from_millis(250);

/// Candidates of one quality band, in the order to ask them.
pub(super) struct Round {
    pub band: Band,
    pub candidates: Vec<Source>,
}

/// What a race came to.
pub(super) enum Raced {
    /// A lane passed the gate.
    Won(Box<Winner>, Report),
    /// No lane sent; one queue is kept (the shortest, at the best band tried).
    Kept(Box<Lane>, Report),
    /// Nothing sent.
    Lost(Report),
}

/// The copy that won: its source and what it has running.
pub(super) struct Winner {
    pub source: Source,
    pub started: Started,
}

/// What a race did, for its one log line.
#[derive(Default, Clone)]
pub(super) struct Report {
    pub bands: Vec<Band>,
    pub asked: usize,
    pub cancelled: usize,
    pub took: Duration,
}

#[derive(Default)]
struct Counts {
    asked: AtomicUsize,
    cancelled: AtomicUsize,
}

/// One request to a peer, running.
pub(super) struct Lane {
    source: Source,
    copy: Copy,
    leg: Leg,
    t: Option<Transfer>,
    polled: Instant,
    asked: Instant,
    first_byte: Option<Instant>,
    /// What had arrived at each look, newest last: the gate counts over the last `JUDGE_WINDOW`.
    samples: VecDeque<(Instant, u64)>,
    /// The peer keeps the request queued (as slskd last said).
    queued: bool,
    /// Bytes a second the song plays at.
    needs: f64,
    /// The hub's lane budget; given back when the lane is kept as a queue.
    permit: Option<Permit>,
}

impl Lane {
    /// Whether the lane's own bytes of the last `JUDGE_WINDOW` outrun the song by `OUTRUN`.
    fn passes(&self, now: Instant) -> bool {
        let Some(first) = self.first_byte else {
            return false;
        };
        if now.duration_since(first) < JUDGE_AFTER {
            return false;
        }
        let before = self
            .samples
            .iter()
            .rev()
            .find(|(at, _)| now.duration_since(*at) >= JUDGE_WINDOW)
            .map_or(self.leg.start, |(_, front)| *front);
        let got = self.leg.front.saturating_sub(before);
        got as f64 >= OUTRUN * self.needs * JUDGE_WINDOW.as_secs_f64()
    }

    fn sending_for(&self, now: Instant) -> Option<Duration> {
        self.first_byte.map(|at| now.duration_since(at))
    }

    fn sample(&mut self, now: Instant) {
        self.samples.push_back((now, self.leg.front));
        while self.samples.len() > 1 && now.duration_since(self.samples[1].0) >= JUDGE_WINDOW {
            self.samples.pop_front();
        }
    }
}

/// How a lane ended.
enum LaneEnd {
    /// It outran the song, or all of it arrived.
    Passed,
    /// No byte in `FIRST_BYTE`.
    Silent,
    /// It sent for `SLOW_AFTER` without passing.
    Slow,
    Failed(String),
    /// The round was won elsewhere.
    Over,
    /// The fetch was stopped.
    Stopped,
}

/// How a lane is held to account.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// In a round: no byte in `FIRST_BYTE` ends it.
    Round,
    /// The kept queue: it waits for its turn as long as the song plays.
    Kept,
}

struct Ctx<'a> {
    candidates: Mutex<VecDeque<Source>>,
    left: AtomicUsize,
    over: watch::Sender<bool>,
    band: Band,
    priority: Priority,
    kept: &'a Mutex<Option<(Band, Box<Lane>)>>,
    counts: &'a Counts,
}

impl Ctx<'_> {
    fn next(&self) -> Option<Source> {
        if self.left.load(Ordering::Relaxed) == 0 {
            return None;
        }
        let source = self.candidates.lock().ok()?.pop_front()?;
        self.left.fetch_sub(1, Ordering::Relaxed);
        Some(source)
    }
}

impl Fetcher {
    /// Races the `rounds`, highest band first. `Won` hands the winner over; with none, `Kept` has
    /// the one queue to keep while the song plays (`wait_kept`).
    pub(super) async fn race_lanes(
        &self,
        f: &Fetch,
        rounds: Vec<Round>,
        priority: Priority,
        stop: &mut watch::Receiver<bool>,
    ) -> Result<Raced, Stop> {
        let began = Instant::now();
        let counts = Counts::default();
        let kept: Mutex<Option<(Band, Box<Lane>)>> = Mutex::new(None);
        let mut report = Report::default();
        let mut outcome: Result<Option<Lane>, Stop> = Ok(None);
        for round in rounds {
            report.bands.push(round.band);
            match self.round(f, round, priority, &kept, &counts, stop).await {
                Ok(None) => {}
                other => {
                    outcome = other;
                    break;
                }
            }
        }
        let kept = kept.into_inner().ok().flatten();
        let report = |counts: &Counts| Report {
            asked: counts.asked.load(Ordering::Relaxed),
            cancelled: counts.cancelled.load(Ordering::Relaxed),
            took: began.elapsed(),
            ..report.clone()
        };
        match outcome {
            Ok(Some(lane)) => {
                if let Some((_, lane)) = kept {
                    self.cancel_lane(*lane, &counts).await;
                }
                let Lane { source, copy, leg, .. } = lane;
                let winner = Winner {
                    source,
                    started: Started { copy, leg: Some(leg) },
                };
                Ok(Raced::Won(Box::new(winner), report(&counts)))
            }
            Ok(None) => Ok(match kept {
                Some((_, lane)) => Raced::Kept(lane, report(&counts)),
                None => Raced::Lost(report(&counts)),
            }),
            Err(e) => {
                if let Some((_, lane)) = kept {
                    self.cancel_lane(*lane, &counts).await;
                }
                Err(e)
            }
        }
    }

    /// One round: up to `LANES_PER_ROUND` lanes at once over `ROUND_PEERS` peers; the first lane
    /// to pass wins and the rest are cancelled together.
    async fn round(
        &self,
        f: &Fetch,
        round: Round,
        priority: Priority,
        kept: &Mutex<Option<(Band, Box<Lane>)>>,
        counts: &Counts,
        stop: &mut watch::Receiver<bool>,
    ) -> Result<Option<Lane>, Stop> {
        let ctx = Ctx {
            candidates: Mutex::new(round.candidates.into()),
            left: AtomicUsize::new(ROUND_PEERS),
            over: watch::channel(false).0,
            band: round.band,
            priority,
            kept,
            counts,
        };
        let mut workers = FuturesUnordered::new();
        for _ in 0..LANES_PER_ROUND {
            workers.push(self.worker(f, &ctx, stop.clone()));
        }
        let mut won: Option<Lane> = None;
        let mut error: Option<Stop> = None;
        while let Some(done) = workers.next().await {
            match done {
                Ok(Some(lane)) if won.is_none() => {
                    won = Some(lane);
                    ctx.over.send_replace(true);
                }
                // Two lanes passed together: the second didn't win.
                Ok(Some(lane)) => self.cancel_lane(lane, counts).await,
                Ok(None) => {}
                Err(stop) => {
                    error.get_or_insert(stop);
                    ctx.over.send_replace(true);
                }
            }
        }
        match (error, won) {
            (Some(e), won) => {
                if let Some(lane) = won {
                    self.cancel_lane(lane, counts).await;
                }
                Err(e)
            }
            (None, won) => Ok(won),
        }
    }

    /// One lane's worker: asks the round's next candidate, tends it, and takes the next when it
    /// gives way, until the round is over or the candidates are. A lane is cancelled before its
    /// replacement is requested, so a peer count never overshoots for a moment.
    async fn worker(&self, f: &Fetch, ctx: &Ctx<'_>, mut stop: watch::Receiver<bool>) -> Result<Option<Lane>, Stop> {
        let mut over = ctx.over.subscribe();
        loop {
            if *over.borrow() {
                return Ok(None);
            }
            let Some(source) = ctx.next() else {
                return Ok(None);
            };
            let permit = tokio::select! {
                permit = self.lanes.acquire(ctx.priority) => permit,
                _ = over.wait_for(|o| *o) => return Ok(None),
                () = stopped(&mut stop) => return Err(Stop::Cancelled),
            };
            let mut lane = match self.open_lane(f, source, permit).await {
                Ok(lane) => lane,
                Err(Skipped::Peer(why)) => {
                    tracing::debug!(why, "a lane's peer didn't take the request");
                    continue;
                }
                Err(Skipped::Race(stop)) => return Err(stop),
            };
            ctx.counts.asked.fetch_add(1, Ordering::Relaxed);
            match self.tend(f, &mut lane, Mode::Round, &mut over, &mut stop).await {
                LaneEnd::Passed => return Ok(Some(lane)),
                LaneEnd::Silent if lane.queued => self.keep_queue(lane, ctx).await,
                LaneEnd::Silent | LaneEnd::Slow | LaneEnd::Failed(_) => self.cancel_lane(lane, ctx.counts).await,
                LaneEnd::Over => {
                    self.cancel_lane(lane, ctx.counts).await;
                    return Ok(None);
                }
                LaneEnd::Stopped => {
                    self.cancel_lane(lane, ctx.counts).await;
                    return Err(Stop::Cancelled);
                }
            }
        }
    }

    /// Requests `source`: room in the cache, lull's file for it, the first leg.
    async fn open_lane(&self, f: &Fetch, source: Source, permit: Permit) -> Result<Lane, Skipped> {
        let failed = |e: lull_core::Error| Skipped::Race(Stop::Failed(reason(&e)));
        self.cache
            .make_room(source.size.saturating_mul(2))
            .await
            .map_err(failed)?;
        let copy = self.open_copy(f, &source).await.map_err(failed)?;
        let leg = match self
            .start_leg(&source, copy.ranges.next_missing(0, source.size).unwrap_or(0))
            .await
        {
            Ok(leg) => leg,
            Err(Ok(skip)) => {
                let why = match skip {
                    super::Skip::Stalled(r) | super::Skip::Unavailable(r) | super::Skip::Slow(r) => r,
                };
                self.forget_copy(&source, copy).await;
                return Err(Skipped::Peer(why));
            }
            Err(Err(stop)) => {
                self.forget_copy(&source, copy).await;
                return Err(Skipped::Race(stop));
            }
        };
        let needs = f
            .track
            .duration
            .or(source.length)
            .filter(|&d| d > 0)
            .map_or(0.0, |d| source.size as f64 / f64::from(d));
        let now = Instant::now();
        Ok(Lane {
            samples: VecDeque::from([(now, leg.front)]),
            source,
            copy,
            leg,
            t: None,
            polled: now - POLL,
            asked: now,
            first_byte: None,
            queued: false,
            needs,
            permit: Some(permit),
        })
    }

    /// Watches a lane every `TICK` until it ends: its bytes are copied as they land (lull's own
    /// count), slskd is asked how it stands every `POLL`.
    async fn tend(
        &self,
        f: &Fetch,
        lane: &mut Lane,
        mode: Mode,
        over: &mut watch::Receiver<bool>,
        stop: &mut watch::Receiver<bool>,
    ) -> LaneEnd {
        let rules = self.rules.get();
        loop {
            tokio::select! {
                () = tokio::time::sleep(TICK) => {}
                _ = over.wait_for(|o| *o) => return LaneEnd::Over,
                () = stopped(stop) => return LaneEnd::Stopped,
            }
            if let Err(e) = self
                .copy_leg(&mut lane.copy, &mut lane.leg, super::legs::From::Own)
                .await
            {
                return LaneEnd::Failed(reason(&e));
            }
            let now = Instant::now();
            lane.sample(now);
            if lane.first_byte.is_none() && lane.leg.front > lane.leg.start {
                lane.first_byte = Some(now);
                if mode == Mode::Kept {
                    f.keeps_running.store(true, Ordering::Relaxed);
                }
            }
            if lane.polled.elapsed() >= POLL {
                lane.polled = now;
                let t = match self.slskd.download(&lane.source.user, &lane.leg.id).await {
                    Ok(t) => t,
                    Err(e) => return LaneEnd::Failed(reason(&e)),
                };
                lane.queued = t.phase() == Phase::Queued;
                let watched = watch_leg(&mut lane.leg, &t, rules.stall_timeout(), Duration::MAX);
                lane.t = Some(t);
                match watched {
                    // All of it arrived: the lane has outrun the song.
                    Watch::Done => return LaneEnd::Passed,
                    Watch::Gone(_) => return LaneEnd::Failed(format!("{} dropped the request", lane.source.user)),
                    Watch::Running(..) => {}
                }
            }
            if lane.passes(now) {
                return LaneEnd::Passed;
            }
            if mode == Mode::Round && lane.first_byte.is_none() && now.duration_since(lane.asked) > FIRST_BYTE {
                return LaneEnd::Silent;
            }
            if lane.sending_for(now).is_some_and(|s| s >= SLOW_AFTER) {
                return LaneEnd::Slow;
            }
        }
    }

    /// A lane that gave way while queued at the peer: kept as the one queue when it is the best
    /// seen (the highest band, then the shortest), else cancelled.
    async fn keep_queue(&self, mut lane: Lane, ctx: &Ctx<'_>) {
        lane.permit = None;
        let rank = |band: Band, lane: &Lane| (band, std::cmp::Reverse(lane.source.queue));
        let loser = {
            let Ok(mut kept) = ctx.kept.lock() else {
                return self.cancel_lane(lane, ctx.counts).await;
            };
            match kept.take() {
                Some((band, old)) if rank(band, &old) >= rank(ctx.band, &lane) => {
                    *kept = Some((band, old));
                    Box::new(lane)
                }
                old => {
                    *kept = Some((ctx.band, Box::new(lane)));
                    match old {
                        Some((_, old)) => old,
                        None => return,
                    }
                }
            }
        };
        self.cancel_lane(*loser, ctx.counts).await;
    }

    /// The kept queue, while the song plays: it wins if it starts sending and passes the gate; one
    /// that sends but never outruns the song runs on quietly (`Raced::Won` with `quiet`), to be
    /// there for the next play; a queue place never reached is left when the fetch stops.
    pub(super) async fn wait_kept(
        &self,
        f: &Fetch,
        mut lane: Lane,
        stop: &mut watch::Receiver<bool>,
    ) -> Result<(Winner, bool), Stop> {
        let (_hold, mut over) = watch::channel(false);
        let counts = Counts::default();
        match self.tend(f, &mut lane, Mode::Kept, &mut over, stop).await {
            end @ (LaneEnd::Passed | LaneEnd::Slow) => {
                let quiet = matches!(end, LaneEnd::Slow);
                let Lane { source, copy, leg, .. } = lane;
                Ok((
                    Winner {
                        source,
                        started: Started { copy, leg: Some(leg) },
                    },
                    quiet,
                ))
            }
            LaneEnd::Stopped => {
                self.cancel_lane(lane, &counts).await;
                Err(Stop::Cancelled)
            }
            LaneEnd::Failed(why) => {
                self.cancel_lane(lane, &counts).await;
                Err(Stop::Failed(why))
            }
            LaneEnd::Silent | LaneEnd::Over => {
                self.cancel_lane(lane, &counts).await;
                Err(Stop::Cancelled)
            }
        }
    }

    /// Cancels a lane's request and forgets what it brought: its partial file is deleted, never
    /// cached. A peer cancelled while it was sending is counted against its day (`lanes`).
    async fn cancel_lane(&self, mut lane: Lane, counts: &Counts) {
        let sent = lane.leg.front > lane.leg.start;
        self.end_leg(&lane.source, &mut lane.leg, lane.t.as_ref()).await;
        if sent {
            self.lanes.cancelled(&lane.source.user);
        }
        counts.cancelled.fetch_add(1, Ordering::Relaxed);
        self.forget_copy(&lane.source, lane.copy).await;
    }

    /// Deletes lull's file for a copy nobody will play and its cache record, if any.
    async fn forget_copy(&self, source: &Source, copy: Copy) {
        let path = copy.path.clone();
        drop(copy);
        if let Err(e) = tokio::fs::remove_file(&path).await
            && e.kind() != std::io::ErrorKind::NotFound
        {
            tracing::debug!(error = %e, file = %path.display(), "removing a cancelled lane's file");
        }
        if let Err(e) = self.cache.forget(&source.source_ref()).await {
            tracing::debug!(error = %e, "forgetting a cancelled lane's copy");
        }
    }
}

/// Why a candidate wasn't raced.
enum Skipped {
    /// The peer refused or isn't there: the next candidate.
    Peer(String),
    /// Something that ends the race (slskd is down, the disk is full).
    Race(Stop),
}
