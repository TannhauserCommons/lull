//! A Soulseek copy arriving in legs (ADR-0014). A leg is one slskd transfer of the copy that
//! starts at a byte lull picks: lull sizes slskd's incomplete file to that byte, queues the
//! download, and slskd asks the peer to resume from there. Each leg's bytes are copied into lull's
//! own file for the copy, at their own offsets, and `Ranges` records what has arrived; slskd's
//! file is only where a leg lands, deleted once the leg ends. So legs from different bytes, in any
//! order, add up to one file that never moves, which the stream serves, a save copies and the
//! cache evicts.

use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::time::Instant;

use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::sync::watch;

use super::{
    BUFFERED, Fetch, Fetcher, POLL, QUEUE_TIMEOUT, STALL_NOTICE, START_TIMEOUT, Skip, Status, Stop, reason, stopped,
};
use crate::cache::Entry;
use crate::ranges::Ranges;
use crate::slskd::{Phase, Transfer};
use crate::sources::Source;
use lull_core::Error;
use lull_core::model::SourceRef;

/// slskd's file is read this often while a leg runs; slskd itself is asked every `POLL`.
const COPY_TICK: std::time::Duration = std::time::Duration::from_millis(250);
/// At most this much of one leg is copied per tick, so a fast leg can't hold the loop.
const COPY_MAX: u64 = 4 * 1024 * 1024;
/// slskd reports a download done just before it moves the file and names where: this long at most.
const LAND_WAIT: std::time::Duration = std::time::Duration::from_secs(60);
/// What arrived is written to the cache index each time it grows this much, and when a leg ends.
const SAVE_EVERY: u64 = 1024 * 1024;
/// Another peer's bytes count as the copy's once this many matched the own peer's where both
/// brought them (ADR-0014, Which peer).
const VERIFY: u64 = 64 * 1024;
/// Another peer is asked to start where a reader waits when the leg running would take longer
/// than this to get there: the own peer's leg keeps running.
pub(super) const JUMP_S: u32 = 10;
/// A reader's ask stands this long before a leg starts for it: the browser's reads around one
/// seek come together, and a newer one replaces an older (ADR-0014, Probes collapse).
const COLLAPSE: std::time::Duration = std::time::Duration::from_millis(300);
/// The same peer is asked to start over at the byte a reader waits for only when the leg running
/// would take longer than this to get there: it gives the peer's upload slot back, and the new
/// request may wait in its queue (ADR-0014, Which peer).
pub(super) const JUMP_SAME_S: u32 = 30;

type Outcome<T> = std::result::Result<T, std::result::Result<Skip, Stop>>;

/// lull's file for one copy and what of it has arrived.
pub(super) struct Copy {
    pub path: PathBuf,
    file: File,
    /// The bytes from the copy's own peer, and from another peer once its bytes matched the
    /// own peer's where both brought them.
    pub ranges: Ranges,
    /// Bytes another peer brought for a seek ahead, not checked yet: served at once, trusted only
    /// once they match (ADR-0014, Which peer).
    side: Ranges,
    /// The other peer's leg, running beside the own peer's.
    side_leg: Option<Side>,
    /// How the other peer's bytes compared with the own peer's where both brought them.
    check: Check,
    /// `ranges.total()` when the cache index was last written.
    saved: u64,
    /// A leg of this copy waited in the peer's queue: the peer is busy, so no leg gives its
    /// slot back for a jump.
    queued: bool,
    /// Whether the browser seeks in this file in one read (`None` until its first bytes say):
    /// no jumps for one it bisects.
    seeks_in_one: Option<bool>,
    /// Peers whose bytes of this copy were different, or whose leg failed: not asked again for it.
    shunned: Vec<String>,
}

impl Copy {
    /// What a reader may read: the own peer's bytes and another peer's not checked yet.
    fn arrived(&self) -> Ranges {
        self.ranges.union(&self.side)
    }
}

/// Another peer's leg, for a seek ahead, beside the own peer's.
struct Side {
    source: Source,
    leg: Leg,
    t: Option<Transfer>,
    asked: Instant,
}

#[derive(Default)]
struct Check {
    /// Bytes compared and found the same.
    same: u64,
    differ: bool,
}

/// Whose bytes: the copy's own peer's, or another peer's not checked yet.
#[derive(Clone, Copy, PartialEq, Eq)]
enum From {
    Own,
    Side,
}

/// One slskd transfer of the copy, from `start`.
struct Leg {
    id: String,
    start: u64,
    /// The next byte of the copy this leg brings.
    front: u64,
    /// Where slskd may write it: each spelling of its path, all sized to `start`.
    files: Vec<PathBuf>,
    reading: Option<(PathBuf, File)>,
    begun: Instant,
    /// The last new byte, for stall and queue timeouts: from when the peer started sending.
    last: (u64, Instant),
    was_running: bool,
    /// When slskd said it finished, while it hasn't yet said where it put the file.
    landing: Option<Instant>,
}

/// How a leg ended without failing the copy.
enum Ended {
    /// slskd finished the transfer: everything from the leg's start to the end is here.
    Finished,
    /// The leg reached bytes that had arrived before: it has nothing new to bring.
    Joined,
    /// A reader waits for this byte, far from the leg: the next leg starts there.
    Jump(u64),
}

impl Fetcher {
    /// Gets one copy onto disk in legs, each from the first byte still missing, until every
    /// byte is here. A peer that doesn't answer, queues us too long or stalls is skipped, as
    /// with one transfer; what its legs brought stays recorded for a later fetch of the copy.
    pub(super) async fn legs(
        &self,
        f: &Fetch,
        attempt: u32,
        source: &Source,
        stop: &mut watch::Receiver<bool>,
    ) -> Outcome<PathBuf> {
        let failed = |e: Error| Err(Stop::Failed(reason(&e)));
        // lull's file, and slskd's for the leg running, can both hold the whole copy.
        self.cache
            .make_room(source.size.saturating_mul(2))
            .await
            .map_err(failed)?;
        let rules = self.rules.get();
        let below = !source.meets(&rules.stream.minimum, f.track.duration);
        let mut copy = self.open_copy(f, source).await.map_err(failed)?;
        self.update(f, |p| {
            p.attempt = attempt;
            p.source = Some(source.clone());
            p.below = below;
            p.choices.clear();
            p.status = Status::Queued;
            p.ranges = copy.ranges.clone();
            p.speed = 0;
            p.queue_pos = None;
            p.path = (copy.ranges.total() > 0).then(|| copy.path.clone());
            p.error = None;
        });
        // Each leg starts where a reader waits, else at the first byte missing: after a jump
        // reaches the end, the gap it left fills from its start.
        let mut jump = None;
        let outcome = loop {
            let Some(start) = jump.take().or_else(|| copy.ranges.next_missing(0, source.size)) else {
                break Ok(());
            };
            let leg = match self.start_leg(source, start).await {
                Ok(leg) => leg,
                Err(e) => break Err(e),
            };
            let ended = self.run_leg(f, source, &mut copy, leg, stop).await;
            if let Err(e) = self.save_copy(f, source, &mut copy, false).await {
                break Err(failed(e));
            }
            match ended {
                Ok(Ended::Jump(at)) => jump = Some(at),
                Ok(Ended::Finished | Ended::Joined) => {}
                Err(e) => break Err(e),
            }
        };
        if let Some(mut side) = copy.side_leg.take() {
            self.drop_leg(&side.source, &mut side.leg).await;
        }
        outcome?;
        copy.file.flush().await.map_err(|e| failed(io(&copy.path, e)))?;
        self.save_copy(f, source, &mut copy, true).await.map_err(failed)?;
        if let Err(e) = self.cache.report().await {
            tracing::warn!(error = %e, "measuring the stream cache");
        }
        if let Err(e) = self.peers.delivered(&source.user).await {
            tracing::warn!(error = %e, "clearing a peer's stall record");
        }
        Ok(copy.path)
    }

    /// lull's file for the copy: the one a stopped fetch of it left, with what it holds, or a
    /// new one.
    async fn open_copy(&self, f: &Fetch, source: &Source) -> lull_core::Result<Copy> {
        let found = self.cache.partial(&f.track.id, &source.source_ref()).await?;
        let (path, ranges) = match found {
            Some(e) => (e.path, e.ranges.unwrap_or_default()),
            None => (self.cache.copy_file(&source.source_ref()).await?, Ranges::default()),
        };
        let mut open = OpenOptions::new();
        open.read(true).write(true).create(true);
        if ranges.total() == 0 {
            open.truncate(true);
        }
        let file = open.open(&path).await.map_err(|e| io(&path, e))?;
        let saved = ranges.total();
        Ok(Copy {
            path,
            file,
            ranges,
            side: Ranges::default(),
            side_leg: None,
            check: Check::default(),
            saved,
            queued: false,
            seeks_in_one: None,
            shunned: Vec::new(),
        })
    }

    /// Records the copy in the cache, with what has arrived while it is unfinished: the own
    /// peer's bytes only, since another peer's not checked yet don't outlive this fetch.
    async fn save_copy(&self, f: &Fetch, source: &Source, copy: &mut Copy, complete: bool) -> lull_core::Result<()> {
        copy.saved = copy.ranges.total();
        self.cache
            .record(Entry {
                track: f.track.id.clone(),
                source: source.source_ref(),
                path: copy.path.clone(),
                complete,
                used_at: lull_core::now(),
                ranges: (!complete).then(|| copy.ranges.clone()),
            })
            .await
    }

    /// Queues a leg of `source` from `start`: slskd's incomplete file is made exactly `start`
    /// bytes long, a hole, so slskd resumes there (`retry.partial: resume`, its default), and
    /// nothing before `start` in it is ever real audio. Both spellings of its path are (slskd on
    /// Windows spells some names its own way), so whichever slskd writes starts at `start`.
    async fn start_leg(&self, source: &Source, start: u64) -> Outcome<Leg> {
        let files = self
            .layout
            .read()
            .map(|l| l.incomplete(&source.user, &source.path))
            .unwrap_or_default();
        let fail = |e: Error| Err(Stop::Failed(reason(&e)));
        for path in &files {
            remove_when_free(path).await.map_err(|e| fail(io(path, e)))?;
            if start > 0 {
                presize(path, start).await.map_err(|e| fail(io(path, e)))?;
            }
        }
        let transfer = match self.slskd.enqueue(&source.user, &source.path, source.size).await {
            Ok(t) => t,
            Err(e @ Error::Unavailable { .. }) => return Err(fail(e)),
            Err(e) => return Err(Ok(Skip::Unavailable(format!("{}: {}", source.user, reason(&e))))),
        };
        Ok(Leg {
            id: transfer.id,
            start,
            front: start,
            files,
            reading: None,
            begun: Instant::now(),
            last: (start, Instant::now()),
            was_running: false,
            landing: None,
        })
    }

    /// Runs a leg of the own peer until slskd finishes it, it reaches bytes that arrived before,
    /// a reader far ahead makes it start over there, or it fails. What it brings is copied as it
    /// lands and published as the copy's ranges. Beside it, another peer's leg may run for a
    /// reader far ahead (`tend_side`).
    async fn run_leg(
        &self,
        f: &Fetch,
        source: &Source,
        copy: &mut Copy,
        mut leg: Leg,
        stop: &mut watch::Receiver<bool>,
    ) -> Outcome<Ended> {
        let failed = |e: Error| Err(Stop::Failed(reason(&e)));
        let rules = self.rules.get();
        let mut asked = Instant::now() - POLL;
        let mut t: Option<Transfer> = None;
        loop {
            tokio::select! {
                () = tokio::time::sleep(COPY_TICK) => {}
                () = stopped(stop) => {
                    // It may have finished since the last look: keep what is done.
                    if let Ok(t) = self.slskd.download(&source.user, &leg.id).await
                        && t.phase() == Phase::Succeeded
                        && let Some(ended) = self.land(source, copy, &mut leg, &t, From::Own).await.map_err(failed)?
                    {
                        return Ok(ended);
                    }
                    self.drop_leg(source, &mut leg).await;
                    return Err(Err(Stop::Cancelled));
                }
            }
            self.copy_leg(copy, &mut leg, From::Own).await.map_err(failed)?;
            if copy.seeks_in_one.is_none() {
                copy.seeks_in_one = seeks_in_one(&source.ext, copy).await;
            }
            self.tend_side(copy).await;
            self.settle_check(copy).await;
            // An slskd that starts every download over from 0 brings each leg from 0: stopped at
            // what had arrived, the next would start over again, so it runs to the end.
            let resumes = self.resumes.load(std::sync::atomic::Ordering::Relaxed);
            if resumes && leg.front > leg.start && copy.ranges.has(leg.front) {
                self.drop_leg(source, &mut leg).await;
                return Ok(Ended::Joined);
            }
            let speed = t.as_ref().map_or(0, |t| t.average_speed.max(0.0) as u64);
            let waiting = f.asked().map(|(byte, since)| (byte, since.elapsed()));
            // A reader far ahead: another peer with the same file starts there, beside this
            // leg; with none, this peer starts over there, when that is worth its slot.
            if copy.side_leg.is_none()
                && self.seeks_ahead(copy)
                && let Some(at) = jump(waiting, &copy.arrived(), &leg, speed, JUMP_S)
                && let Some(other) = self.other_peer(f, source, copy)
            {
                tracing::info!(peer = %other.user, at, "another peer starts the copy where the listener seeks");
                match self.start_leg(&other, at).await {
                    Ok(leg) => {
                        copy.side_leg = Some(Side {
                            source: other,
                            leg,
                            t: None,
                            asked: Instant::now() - POLL,
                        })
                    }
                    Err(_) => copy.shunned.push(other.user),
                }
            }
            if copy.side_leg.is_none()
                && self.jumps_allowed(source, copy)
                && let Some(at) = jump(waiting, &copy.arrived(), &leg, speed, JUMP_SAME_S)
            {
                tracing::info!(peer = %source.user, from = leg.front, to = at, "starting the copy over where the listener seeks");
                self.drop_leg(source, &mut leg).await;
                return Ok(Ended::Jump(at));
            }
            if copy.ranges.total() >= copy.saved + SAVE_EVERY {
                self.save_copy(f, source, copy, false).await.map_err(failed)?;
            }
            if asked.elapsed() >= POLL {
                asked = Instant::now();
                t = Some(match self.slskd.download(&source.user, &leg.id).await {
                    Ok(t) => t,
                    Err(e @ Error::Unavailable { .. }) => return Err(failed(e)),
                    Err(e) => return Err(Ok(Skip::Unavailable(format!("{}: {}", source.user, reason(&e))))),
                });
            }
            let Some(t) = t.as_ref() else { continue };
            if t.phase() == Phase::Queued {
                copy.queued = true;
            }
            match watch_leg(&mut leg, t, rules.stall_timeout()) {
                Watch::Gone(gone) => {
                    self.drop_leg(source, &mut leg).await;
                    let pct = copy.ranges.total().saturating_mul(100) / source.size.max(1);
                    return Err(Ok(match gone {
                        Gone::Silent => Skip::Unavailable(format!("{} did not answer", source.user)),
                        Gone::Queued(pos) => Skip::Unavailable(format!("{} kept us queued{pos}", source.user)),
                        Gone::Stalled => Skip::Stalled(format!("{} stalled at {pct}%", source.user)),
                        Gone::Failed(why) => Skip::Stalled(format!("{}: {why}", source.user)),
                    }));
                }
                Watch::Done => {
                    if let Some(ended) = self.land(source, copy, &mut leg, t, From::Own).await.map_err(failed)? {
                        return Ok(ended);
                    }
                }
                Watch::Running(phase, quiet) => {
                    let brought = leg.front - leg.start;
                    let ahead = self.ahead(f, source, copy);
                    let arrived = copy.arrived();
                    self.update(f, |p| {
                        p.ahead = ahead;
                        p.speed = t.average_speed.max(0.0) as u64;
                        p.queue_pos = t.place_in_queue;
                        p.path = (arrived.total() > 0).then(|| copy.path.clone());
                        p.ranges = arrived;
                        p.status = match phase {
                            Phase::Starting | Phase::Queued => Status::Queued,
                            _ if quiet > STALL_NOTICE => Status::Stalled,
                            _ if brought < BUFFERED => Status::Buffering,
                            _ => Status::Streaming,
                        };
                    });
                }
            }
        }
    }

    /// Runs the other peer's leg a step: copies what it brought (kept apart until checked), asks
    /// slskd how it stands, and ends it when it is done, reaches the own peer's bytes, or fails;
    /// a peer whose leg failed isn't asked again for this copy. It never fails the copy.
    async fn tend_side(&self, copy: &mut Copy) {
        let Some(mut side) = copy.side_leg.take() else { return };
        let from = From::Side;
        let rules = self.rules.get();
        if let Err(e) = self.copy_leg(copy, &mut side.leg, from).await {
            tracing::info!(peer = %side.source.user, error = %e, "another peer's leg failed");
            return self.shun(copy, side).await;
        }
        // It ran into the own peer's bytes, and what overlapped was checked: it has nothing new.
        if side.leg.front > side.leg.start && copy.ranges.has(side.leg.front) && copy.side.is_empty() {
            self.drop_leg(&side.source, &mut side.leg).await;
            return;
        }
        if side.asked.elapsed() >= POLL {
            side.asked = Instant::now();
            match self.slskd.download(&side.source.user, &side.leg.id).await {
                Ok(t) => side.t = Some(t),
                Err(_) => return self.shun(copy, side).await,
            }
        }
        let Some(t) = side.t.clone() else {
            copy.side_leg = Some(side);
            return;
        };
        match watch_leg(&mut side.leg, &t, rules.stall_timeout()) {
            Watch::Gone(_) => return self.shun(copy, side).await,
            Watch::Done => match self.land(&side.source, copy, &mut side.leg, &t, from).await {
                Ok(Some(_)) => return,
                Ok(None) => {}
                Err(_) => return self.shun(copy, side).await,
            },
            Watch::Running(..) => {}
        }
        copy.side_leg = Some(side);
    }

    /// The other peer's bytes, once compared with the own peer's where both brought them: the
    /// same over `VERIFY` bytes, they are the copy's; different, they go, and so does its leg.
    async fn settle_check(&self, copy: &mut Copy) {
        if copy.check.differ {
            tracing::warn!("another peer's copy differs from the own peer's where they overlap: its bytes are dropped");
            copy.side = Ranges::default();
            copy.check = Check::default();
            if let Some(side) = copy.side_leg.take() {
                self.shun(copy, side).await;
            }
        } else if copy.check.same >= VERIFY && !copy.side.is_empty() {
            copy.ranges = copy.ranges.union(&copy.side);
            copy.side = Ranges::default();
        }
    }

    /// Ends another peer's leg and doesn't ask that peer again for this copy.
    async fn shun(&self, copy: &mut Copy, mut side: Side) {
        self.drop_leg(&side.source, &mut side.leg).await;
        copy.shunned.push(side.source.user);
    }

    /// Another peer with the same file the copy can come from (ADR-0014, Which peer): the same
    /// size and kind, a free slot in its search answer, no stall on record, not tried for this copy.
    fn other_peer(&self, f: &Fetch, source: &Source, copy: &Copy) -> Option<Source> {
        let rules = self.rules.get();
        self.sources
            .ordered(&f.track.id, f.track.duration, &rules, f.purpose)
            .into_iter()
            .filter(|r| {
                r.peer != source.user
                    && r.size_bytes == source.size
                    && r.free_slot
                    && !r.stalled_before
                    && !copy.shunned.contains(&r.peer)
            })
            .find_map(|r| {
                let found = SourceRef {
                    user: r.peer,
                    path: r.path,
                    size: r.size_bytes,
                };
                self.sources.get(&f.track.id, &found).filter(|s| s.ext == source.ext)
            })
    }

    /// A seek further than this many seconds of transfer away is fetched from there: another
    /// peer's `JUMP_S`, the own peer's `JUMP_SAME_S`, or `None` while neither can.
    fn ahead(&self, f: &Fetch, source: &Source, copy: &Copy) -> Option<u32> {
        if !self.seeks_ahead(copy) {
            None
        } else if copy.side_leg.is_some() || self.other_peer(f, source, copy).is_some() {
            Some(JUMP_S)
        } else {
            self.jumps_allowed(source, copy).then_some(JUMP_SAME_S)
        }
    }

    /// Copies what slskd has written of the leg since the last look into lull's file. A file
    /// shorter than the leg's start, or one whose first bytes aren't the hole lull left, was
    /// started over from byte 0 (an slskd set to overwrite partial files): the leg then brings
    /// the copy from 0, which is still the copy's own bytes at their own offsets.
    async fn copy_leg(&self, copy: &mut Copy, leg: &mut Leg, from: From) -> lull_core::Result<()> {
        if leg.reading.is_none() {
            // The file slskd writes is the one that changed from how lull left it.
            for path in &leg.files {
                let changed = match tokio::fs::metadata(path).await {
                    Ok(m) => leg.start == 0 || m.len() != leg.start,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
                    Err(e) => return Err(io(path, e)),
                };
                if changed {
                    let file = File::open(path).await.map_err(|e| io(path, e))?;
                    leg.reading = Some((path.clone(), file));
                    break;
                }
            }
        }
        let Some((path, file)) = leg.reading.as_mut() else {
            return Ok(());
        };
        let on_disk = file.metadata().await.map_err(|e| io(path, e))?.len();
        if leg.start > 0 && leg.front == leg.start && (on_disk < leg.start || !hole(file, leg.start).await?) {
            tracing::warn!(file = %path.display(), "slskd started a download over from byte 0, not where lull sized it: no more seeking ahead");
            self.resumes.store(false, std::sync::atomic::Ordering::Relaxed);
            leg.start = 0;
            leg.front = 0;
        }
        let want = on_disk.saturating_sub(leg.front).min(COPY_MAX);
        if want == 0 {
            return Ok(());
        }
        file.seek(SeekFrom::Start(leg.front)).await.map_err(|e| io(path, e))?;
        let mut buf = vec![0; want as usize];
        let n = file.read(&mut buf).await.map_err(|e| io(path, e))?;
        bring(copy, leg.front, &buf[..n], from).await?;
        leg.front += n as u64;
        Ok(())
    }

    /// slskd finished the leg. It says so before it moves the file to its downloads folder, so
    /// the rest comes from the leg's open file while it reads (on Linux it follows the file through
    /// the move), then from where slskd put it. Once slskd names that place, its file and record
    /// go. `None`: slskd hasn't named it yet; ask again on the next look.
    async fn land(
        &self,
        source: &Source,
        copy: &mut Copy,
        leg: &mut Leg,
        t: &Transfer,
        from: From,
    ) -> lull_core::Result<Option<Ended>> {
        loop {
            let before = leg.front;
            self.copy_leg(copy, leg, from).await?;
            if leg.front == before {
                break;
            }
        }
        let Some(reported) = self.slskd.finished_path(&t.id).await? else {
            if leg.landing.get_or_insert_with(Instant::now).elapsed() < LAND_WAIT {
                return Ok(None);
            }
            return Err(Error::upstream(
                "soulseek",
                format!("slskd finished {} but did not say where it put it", t.filename),
            ));
        };
        let finished = self
            .layout
            .read()
            .ok()
            .and_then(|l| l.finished(&reported))
            .ok_or_else(|| {
                Error::unavailable(
                    "soulseek",
                    format!("slskd saved {reported} outside the downloads folder lull reads"),
                )
            })?;
        if leg.front < source.size {
            let mut file = File::open(&finished).await.map_err(|e| io(&finished, e))?;
            file.seek(SeekFrom::Start(leg.front))
                .await
                .map_err(|e| io(&finished, e))?;
            let mut rest = Vec::new();
            file.read_to_end(&mut rest).await.map_err(|e| io(&finished, e))?;
            bring(copy, leg.front, &rest, from).await?;
            leg.front += rest.len() as u64;
        }
        if leg.front < source.size {
            return Err(Error::upstream(
                "soulseek",
                format!("{} sent {} of {} bytes", source.user, leg.front, source.size),
            ));
        }
        leg.reading = None;
        for path in std::iter::once(&finished).chain(&leg.files) {
            if let Err(e) = tokio::fs::remove_file(path).await
                && e.kind() != std::io::ErrorKind::NotFound
            {
                tracing::warn!(error = %e, file = %path.display(), "removing a finished leg's file");
            }
        }
        if let Err(e) = self.slskd.cancel(&source.user, &t.id).await {
            tracing::debug!(error = %e, "removing a finished transfer from slskd's list");
        }
        Ok(Some(Ended::Finished))
    }

    /// Legs may start elsewhere than where the transfer is: slskd resumes where lull sizes its
    /// file, and the browser seeks in this file with one read.
    fn seeks_ahead(&self, copy: &Copy) -> bool {
        self.resumes.load(std::sync::atomic::Ordering::Relaxed) && copy.seeks_in_one == Some(true)
    }

    /// The same peer may be asked to start over elsewhere: legs may, the peer said it had a free
    /// slot, and no leg of this copy has waited in its queue.
    fn jumps_allowed(&self, source: &Source, copy: &Copy) -> bool {
        self.seeks_ahead(copy) && source.free_slot && !copy.queued
    }

    /// Stops a leg in slskd and removes its record and its file: what it brought is in lull's.
    async fn drop_leg(&self, source: &Source, leg: &mut Leg) {
        if let Err(e) = self.slskd.cancel(&source.user, &leg.id).await {
            tracing::warn!(error = %e, "cancelling a leg in slskd");
        }
        leg.reading = None;
        for path in &leg.files {
            if let Err(e) = tokio::fs::remove_file(path).await
                && e.kind() != std::io::ErrorKind::NotFound
            {
                tracing::debug!(error = %e, file = %path.display(), "removing a leg's file");
            }
        }
    }
}

/// Where a leg stands, from slskd's transfer: running, done, or gone (with why).
enum Watch {
    Running(Phase, std::time::Duration),
    Done,
    Gone(Gone),
}

enum Gone {
    /// The peer didn't accept the request in time.
    Silent,
    /// It kept the leg queued; the place, when known, as " at position N".
    Queued(String),
    Stalled,
    Failed(String),
}

/// A leg's state from slskd's transfer, giving up on a peer that doesn't answer, queues it too
/// long or stalls. Silence counts from the last new byte, or from when the peer started sending:
/// a slow connection set-up (or slskd retrying the peer) is not a stall.
fn watch_leg(leg: &mut Leg, t: &Transfer, stall: std::time::Duration) -> Watch {
    let running = t.phase() == Phase::InProgress;
    if leg.front > leg.last.0 || (running && !leg.was_running) {
        leg.last = (leg.front, Instant::now());
    }
    leg.was_running = running;
    let quiet = leg.last.1.elapsed();
    match t.phase() {
        Phase::Starting if leg.begun.elapsed() > START_TIMEOUT => Watch::Gone(Gone::Silent),
        Phase::Queued if quiet > QUEUE_TIMEOUT => Watch::Gone(Gone::Queued(
            t.place_in_queue
                .map(|q| format!(" at position {q}"))
                .unwrap_or_default(),
        )),
        Phase::InProgress if quiet > stall => Watch::Gone(Gone::Stalled),
        Phase::Failed => Watch::Gone(Gone::Failed(
            t.exception
                .as_deref()
                .map_or_else(|| t.state.clone(), crate::slskd::plain),
        )),
        Phase::Succeeded => Watch::Done,
        phase => Watch::Running(phase, quiet),
    }
}

/// Where a leg starts for a reader that has waited (past `COLLAPSE`) for a byte that hasn't
/// arrived, when the leg running won't bring it within `within` seconds: it is behind the leg, or
/// too far ahead at the leg's speed (an unknown speed is too slow).
fn jump(
    asked: Option<(u64, std::time::Duration)>,
    arrived: &Ranges,
    leg: &Leg,
    speed: u64,
    within: u32,
) -> Option<u64> {
    let (byte, waited) = asked?;
    if waited < COLLAPSE || arrived.has(byte) {
        return None;
    }
    let soon = byte >= leg.front && speed > 0 && (byte - leg.front) / speed <= u64::from(within);
    (!soon).then_some(byte)
}

/// Whether a browser seeks in this file with one read (`None` while its first bytes haven't said),
/// from what has arrived from byte 0 (ADR-0014, Formats). A FLAC file without a SEEKTABLE block,
/// and Ogg, are bisected: a dozen reads at offsets one after another, each of which would start a
/// leg, so they don't seek ahead. MP3 and M4A (its index fetched by a leg too) do.
async fn seeks_in_one(ext: &str, copy: &Copy) -> Option<bool> {
    match ext {
        "ogg" | "opus" | "oga" => return Some(false),
        "flac" => {}
        _ => return Some(true),
    }
    let arrived = copy.ranges.end_from(0)?;
    let mut file = File::open(&copy.path).await.ok()?;
    if &four(&mut file, 0, arrived).await? != b"fLaC" {
        return Some(true);
    }
    // Each metadata block: a header (last flag, type, 24-bit length), then its body, stepped over.
    let mut at = 4;
    loop {
        let [head, l1, l2, l3] = four(&mut file, at, arrived).await?;
        match (head & 0x80 != 0, head & 0x7f) {
            (_, 3) => return Some(true),
            (true, _) => return Some(false),
            _ => at += 4 + (u64::from(l1) << 16 | u64::from(l2) << 8 | u64::from(l3)),
        }
    }
}

/// The four bytes at `at`, once they have arrived (`arrived`: the end of what came from byte 0).
async fn four(file: &mut File, at: u64, arrived: u64) -> Option<[u8; 4]> {
    let mut four = [0u8; 4];
    (at + 4 <= arrived).then_some(())?;
    file.seek(SeekFrom::Start(at)).await.ok()?;
    file.read_exact(&mut four).await.ok()?;
    Some(four)
}

/// Removes a file of slskd's, once slskd lets go of it: a leg just cancelled may still hold it
/// open a moment, which Windows refuses to delete (a sharing violation) until it closes.
async fn remove_when_free(path: &Path) -> std::io::Result<()> {
    let since = Instant::now();
    loop {
        match tokio::fs::remove_file(path).await {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e)
                if cfg!(windows) && e.raw_os_error() == Some(SHARING_VIOLATION) && since.elapsed() < RELEASE_WAIT =>
            {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            Err(e) => return Err(e),
        }
    }
}

/// Windows' ERROR_SHARING_VIOLATION, and how long slskd gets to close a cancelled leg's file.
const SHARING_VIOLATION: i32 = 32;
const RELEASE_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// A new file of exactly `len` bytes, all of them a hole (zeros where the filesystem has no holes).
async fn presize(path: &Path, len: u64) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    let file = File::create(path).await?;
    file.set_len(len).await
}

/// The first bytes of slskd's file are still the hole lull left before the leg's start. No audio
/// file starts with 16 zero bytes, so a file slskd started over from 0 doesn't.
async fn hole(file: &mut File, start: u64) -> lull_core::Result<bool> {
    let mut head = [0u8; 16];
    let n = (start as usize).min(head.len());
    file.seek(SeekFrom::Start(0))
        .await
        .map_err(|e| Error::internal(format!("reading a leg: {e}")))?;
    file.read_exact(&mut head[..n])
        .await
        .map_err(|e| Error::internal(format!("reading a leg: {e}")))?;
    Ok(head[..n].iter().all(|&b| b == 0))
}

/// Writes bytes a leg brought at their offset in lull's file and records them as arrived. Where
/// the own peer's and another peer's bytes meet, they are compared (`Copy::check`): the own peer's
/// replace the other's, and the other's never replace the own peer's.
async fn bring(copy: &mut Copy, at: u64, bytes: &[u8], from: From) -> lull_core::Result<()> {
    let end = at + bytes.len() as u64;
    if bytes.is_empty() {
        return Ok(());
    }
    let theirs = match from {
        From::Own => copy.side.within(at, end),
        From::Side => copy.ranges.within(at, end),
    };
    for &(a, b) in &theirs {
        let mut held = vec![0; (b - a) as usize];
        copy.file
            .seek(SeekFrom::Start(a))
            .await
            .map_err(|e| io(&copy.path, e))?;
        copy.file.read_exact(&mut held).await.map_err(|e| io(&copy.path, e))?;
        if held == bytes[(a - at) as usize..(b - at) as usize] {
            copy.check.same += b - a;
        } else {
            copy.check.differ = true;
        }
    }
    let mut spans = vec![(at, end)];
    if from == From::Side {
        // Only where the own peer's bytes aren't.
        let mut gaps = Ranges::default();
        gaps.add(at, end);
        for &(a, b) in &theirs {
            gaps.remove(a, b);
        }
        spans = gaps.within(at, end);
    }
    for &(a, b) in &spans {
        copy.file
            .seek(SeekFrom::Start(a))
            .await
            .map_err(|e| io(&copy.path, e))?;
        copy.file
            .write_all(&bytes[(a - at) as usize..(b - at) as usize])
            .await
            .map_err(|e| io(&copy.path, e))?;
    }
    // tokio's file writes in the background: flushed, the bytes are the OS's before readers are
    // told they arrived.
    copy.file.flush().await.map_err(|e| io(&copy.path, e))?;
    for &(a, b) in &spans {
        match from {
            From::Own => {
                copy.ranges.add(a, b);
                copy.side.remove(a, b);
            }
            From::Side => copy.side.add(a, b),
        }
    }
    Ok(())
}

fn io(path: &Path, e: std::io::Error) -> Error {
    Error::unavailable("cache", format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, RwLock};
    use std::time::Duration;

    use async_trait::async_trait;
    use lull_core::events::Bus;
    use lull_core::ids::{AlbumId, ArtistId, TrackId};
    use lull_core::model::{SourceRef, Track};
    use lull_core::seams::Youtube;
    use lull_core::status::Status as Services;
    use lull_core::youtube::{Found, Want};
    use lull_core::{Error, Result};

    use super::super::{Fetch, Fetcher, Status};
    use crate::cache::{Cache, Entry};
    use crate::fake_slskd::{Fake, Peer};
    use crate::peers::Peers;
    use crate::ranges::Ranges;
    use crate::rules::Current;
    use crate::search::Searches;
    use crate::sources::{Source, Sources};

    const REMOTE: &str = "Music\\Artist\\Album\\01 - Song.mp3";
    const STEADY: Peer = Peer {
        per_tick: 16_384,
        reported: Some(800_000.0),
        overwrite: false,
        settles: Duration::from_millis(200),
    };

    struct NoYoutube;

    #[async_trait]
    impl Youtube for NoYoutube {
        async fn find(&self, _: &Track, _: Want) -> Result<Option<Found>> {
            Ok(None)
        }
        async fn download(&self, _: &SourceRef, _: &std::path::Path) -> Result<()> {
            Err(Error::unavailable("youtube", "not in tests"))
        }
    }

    /// The peer's file: every byte says where it is, so a byte copied to the wrong place shows.
    fn song(size: usize) -> Vec<u8> {
        (0..size).map(|i| (i % 251) as u8 + 1).collect()
    }

    fn track() -> Track {
        Track {
            id: TrackId("t-1".into()),
            title: "Song".into(),
            artist_id: ArtistId("ar-1".into()),
            artist: "Artist".into(),
            album_id: AlbumId("al-1".into()),
            album: "Album".into(),
            year: None,
            no: Some(1),
            disc: None,
            duration: Some(200),
            tags: Vec::new(),
            saved: false,
            file: None,
        }
    }

    /// The copy as a search found it, its peer with a free upload slot.
    fn found(r: &SourceRef) -> Source {
        Source {
            user: r.user.clone(),
            path: r.path.clone(),
            size: r.size,
            ext: "mp3".into(),
            facts: lull_core::model::FileFacts {
                format: "MP3 320".into(),
                codec: "MP3".into(),
                lossless: false,
                khz: Some(44.1),
                bits: None,
                kbps: Some(320),
                size_bytes: Some(r.size),
            },
            upload_speed: 0,
            free_slot: true,
            queue: 0,
            length: Some(200),
        }
    }

    async fn fetcher(fake: &Fake) -> Result<(Fetcher, Cache)> {
        let db = lull_core::db::Db::memory()?;
        db.migrate(&crate::MIGRATIONS)?;
        let bus = Bus::new();
        let cache = Cache::new(db.clone(), Services::new(bus.clone()), fake.root.clone(), u64::MAX / 2);
        let peers = Peers::load(db.clone()).await?;
        let sources = Sources::new(peers.clone(), Vec::new());
        let searches = Searches::new(fake.slskd.clone(), bus.clone(), db, sources.clone());
        let fetcher = Fetcher::new(
            fake.slskd.clone(),
            Arc::new(RwLock::new(fake.layout.clone())),
            cache.clone(),
            peers,
            sources,
            searches,
            bus,
            Current::default(),
            Arc::new(NoYoutube),
        );
        Ok((fetcher, cache))
    }

    async fn done(f: &Arc<Fetch>) -> Result<()> {
        let mut rx = f.progress();
        let ended = tokio::time::timeout(Duration::from_secs(30), rx.wait_for(|p| p.ended())).await;
        let p = ended
            .map_err(|_| Error::internal("the fetch never ended"))?
            .map_err(|_| Error::internal("gone"))?
            .clone();
        match p.status {
            Status::Done => Ok(()),
            s => Err(Error::internal(format!("the fetch ended {s:?}: {:?}", p.error))),
        }
    }

    #[tokio::test]
    async fn a_song_arrives_through_slskd_into_lulls_own_file_whole() -> Result<()> {
        let bytes = song(300_000);
        let fake = Fake::start(HashMap::from([(REMOTE.to_owned(), bytes.clone())]), STEADY)
            .await
            .map_err(|e| Error::internal(e.to_string()))?;
        let (fetcher, _) = fetcher(&fake).await?;
        let pick = SourceRef {
            user: "peer".into(),
            path: REMOTE.into(),
            size: bytes.len() as u64,
        };
        let f = fetcher.stream(track(), Some(pick)).await?;
        done(&f).await?;
        let p = f.now();
        let path = p.path.ok_or_else(|| Error::internal("no file"))?;
        assert!(
            path.starts_with(fake.root.join("lull")),
            "lull's own file, not slskd's: {}",
            path.display()
        );
        assert_eq!(std::fs::read(&path).map_err(|e| Error::internal(e.to_string()))?, bytes);
        assert_eq!(fake.starts(), [0]);
        let leftovers = std::fs::read_dir(fake.root.join("complete"))
            .map_err(|e| Error::internal(e.to_string()))?
            .count();
        assert_eq!(leftovers, 0, "slskd's finished file goes once its bytes are in lull's");
        Ok(())
    }

    #[tokio::test]
    async fn a_copy_stopped_part_way_fetches_only_what_is_missing() -> Result<()> {
        let bytes = song(300_000);
        let fake = Fake::start(HashMap::from([(REMOTE.to_owned(), bytes.clone())]), STEADY)
            .await
            .map_err(|e| Error::internal(e.to_string()))?;
        let (fetcher, cache) = fetcher(&fake).await?;
        let pick = SourceRef {
            user: "peer".into(),
            path: REMOTE.into(),
            size: bytes.len() as u64,
        };
        // What an earlier fetch of this copy left: its first 100 kB and a range near the end.
        let path = cache.copy_file(&pick).await?;
        let mut held = bytes.clone();
        held[100_000..250_000].fill(0);
        std::fs::write(&path, &held).map_err(|e| Error::internal(e.to_string()))?;
        let mut ranges = Ranges::default();
        ranges.add(0, 100_000);
        ranges.add(250_000, 300_000);
        cache
            .record(Entry {
                track: track().id,
                source: pick.clone(),
                path: path.clone(),
                complete: false,
                used_at: lull_core::now(),
                ranges: Some(ranges),
            })
            .await?;
        let f = fetcher.stream(track(), Some(pick)).await?;
        done(&f).await?;
        assert_eq!(std::fs::read(&path).map_err(|e| Error::internal(e.to_string()))?, bytes);
        assert_eq!(
            fake.starts(),
            [100_000],
            "one leg, from the first missing byte, which stops at the range after it"
        );
        Ok(())
    }

    /// A slow peer (by slskd's report) sending a 2 MB song over about three seconds, and a reader
    /// that waits for 1.5 MB in: a minute and more away at that speed.
    async fn seek_ahead(peer: Peer) -> Result<(Fake, Arc<Fetch>, Vec<u8>)> {
        let bytes = song(2_000_000);
        seek_ahead_among(HashMap::from([(REMOTE.to_owned(), bytes.clone())]), &[], peer)
            .await
            .map(|(fake, f)| (fake, f, bytes))
    }

    /// The same, with `others` also sharing the file (the same size), each with a free slot.
    async fn seek_ahead_among(
        files: HashMap<String, Vec<u8>>,
        others: &[&str],
        peer: Peer,
    ) -> Result<(Fake, Arc<Fetch>)> {
        let size = files.get(REMOTE).map_or(0, Vec::len) as u64;
        let fake = Fake::start(files, peer)
            .await
            .map_err(|e| Error::internal(e.to_string()))?;
        let (fetcher, _) = fetcher(&fake).await?;
        let pick = SourceRef {
            user: "peer".into(),
            path: REMOTE.into(),
            size,
        };
        let all = std::iter::once("peer")
            .chain(others.iter().copied())
            .map(|user| {
                found(&SourceRef {
                    user: user.into(),
                    ..pick.clone()
                })
            })
            .collect();
        fetcher.sources.add(HashMap::from([(track().id, all)]));
        let f = fetcher.stream(track(), Some(pick)).await?;
        let mut rx = f.progress();
        rx.wait_for(|p| p.received() > 100_000)
            .await
            .map_err(|_| Error::internal("gone"))?;
        let ask = f.ask_for(1_500_000);
        let arrived = tokio::time::timeout(Duration::from_secs(10), rx.wait_for(|p| p.ranges.has(1_500_000))).await;
        arrived
            .map_err(|_| Error::internal("the byte asked for never arrived"))?
            .map_err(|_| Error::internal("gone"))?;
        drop(ask);
        done(&f).await?;
        Ok((fake, f))
    }

    #[tokio::test]
    async fn a_reader_far_ahead_of_the_transfer_gets_the_peer_started_there_and_the_gap_fills_after() -> Result<()> {
        let (fake, f, bytes) = seek_ahead(Peer {
            per_tick: 14_000,
            reported: Some(10_000.0),
            overwrite: false,
            settles: Duration::from_millis(200),
        })
        .await?;
        let starts = fake.starts();
        assert_eq!(
            starts.len(),
            3,
            "the first leg, the jump, and the gap it left: {starts:?}"
        );
        assert_eq!(starts[..2], [0, 1_500_000]);
        assert!(
            starts[2] > 100_000 && starts[2] < 1_500_000,
            "the gap fills from where the first leg stopped: {starts:?}"
        );
        let path = f.now().path.ok_or_else(|| Error::internal("no file"))?;
        assert_eq!(std::fs::read(&path).map_err(|e| Error::internal(e.to_string()))?, bytes);
        Ok(())
    }

    #[tokio::test]
    async fn a_reader_the_transfer_reaches_soon_doesnt_restart_the_peer() -> Result<()> {
        let (fake, _, _) = seek_ahead(Peer {
            per_tick: 14_000,
            reported: Some(10_000_000.0),
            overwrite: false,
            settles: Duration::from_millis(200),
        })
        .await?;
        assert_eq!(
            fake.starts(),
            [0],
            "at the leg's speed the byte is seconds away: no jump"
        );
        Ok(())
    }

    #[tokio::test]
    async fn an_slskd_that_starts_over_from_zero_still_delivers_the_whole_song_and_stops_jumps() -> Result<()> {
        let (fake, f, bytes) = seek_ahead(Peer {
            per_tick: 14_000,
            reported: Some(10_000.0),
            overwrite: true,
            settles: Duration::from_millis(200),
        })
        .await?;
        let path = f.now().path.ok_or_else(|| Error::internal("no file"))?;
        assert_eq!(std::fs::read(&path).map_err(|e| Error::internal(e.to_string()))?, bytes);
        assert_eq!(
            fake.starts(),
            [0, 0],
            "the jump started over from 0, and no other followed"
        );
        assert_eq!(f.now().ahead, None, "the player hears it can't seek ahead");
        Ok(())
    }

    #[tokio::test]
    async fn a_flac_file_seeks_ahead_only_with_a_seektable_known_once_its_block_headers_are_here() -> Result<()> {
        let block = |kind: u8, last: bool, len: usize| {
            let mut b = vec![
                kind | if last { 0x80 } else { 0 },
                (len >> 16) as u8,
                (len >> 8) as u8,
                len as u8,
            ];
            b.resize(4 + len, 0);
            b
        };
        let flac = |blocks: &[Vec<u8>]| [b"fLaC".to_vec(), blocks.concat()].concat();
        let streaminfo = block(0, false, 34);
        let with = flac(&[streaminfo.clone(), block(6, false, 5000), block(3, true, 18)]);
        let without = flac(&[streaminfo, block(6, false, 5000), block(4, true, 40)]);
        let dir = std::env::temp_dir().join(format!("lull-legs-flac-{}", std::process::id()));
        std::fs::create_dir_all(&dir).map_err(|e| Error::internal(e.to_string()))?;
        let copy = |name: &str, bytes: &[u8], arrived: usize| {
            let path = dir.join(name);
            let written = std::fs::write(&path, bytes);
            async move {
                written.map_err(|e| Error::internal(e.to_string()))?;
                let file = tokio::fs::File::open(&path)
                    .await
                    .map_err(|e| Error::internal(e.to_string()))?;
                Ok::<_, Error>(super::Copy {
                    path,
                    file,
                    ranges: Ranges::front(arrived as u64),
                    side: Ranges::default(),
                    side_leg: None,
                    check: super::Check::default(),
                    saved: 0,
                    queued: false,
                    seeks_in_one: None,
                    shunned: Vec::new(),
                })
            }
        };
        assert_eq!(
            super::seeks_in_one("flac", &copy("with", &with, with.len()).await?).await,
            Some(true),
            "a seek table after a big picture"
        );
        assert_eq!(
            super::seeks_in_one("flac", &copy("without", &without, without.len()).await?).await,
            Some(false)
        );
        assert_eq!(
            super::seeks_in_one("flac", &copy("early", &with, 100).await?).await,
            None,
            "the picture hasn't all arrived: not known yet"
        );
        assert_eq!(
            super::seeks_in_one("ogg", &copy("ogg", b"OggS", 4).await?).await,
            Some(false)
        );
        assert_eq!(
            super::seeks_in_one("mp3", &copy("mp3", b"ID3", 3).await?).await,
            Some(true)
        );
        Ok(())
    }

    #[tokio::test]
    async fn another_peer_with_the_same_file_starts_where_the_reader_waits_and_the_first_leg_keeps_its_slot()
    -> Result<()> {
        let bytes = song(2_000_000);
        let files = HashMap::from([(REMOTE.to_owned(), bytes.clone())]);
        let (fake, f) = seek_ahead_among(
            files,
            &["peer2"],
            Peer {
                per_tick: 14_000,
                reported: 10_000.0,
                overwrite: false,
            },
        )
        .await?;
        assert_eq!(
            fake.starts_of("peer2"),
            [1_500_000],
            "the other peer started where the reader waited"
        );
        assert_eq!(
            fake.starts_of("peer"),
            [0],
            "the first peer's leg ran on: its slot kept"
        );
        let path = f.now().path.ok_or_else(|| Error::internal("no file"))?;
        assert_eq!(std::fs::read(&path).map_err(|e| Error::internal(e.to_string()))?, bytes);
        Ok(())
    }

    #[tokio::test]
    async fn another_peers_bytes_that_differ_where_the_legs_meet_are_dropped_and_fetched_again() -> Result<()> {
        let bytes = song(2_000_000);
        let mut other = bytes.clone();
        other[1_400_000..].iter_mut().for_each(|b| *b = b.wrapping_add(7)); // same size, other audio
        let files = HashMap::from([(REMOTE.to_owned(), bytes.clone()), (format!("peer2|{REMOTE}"), other)]);
        let (fake, f) = seek_ahead_among(
            files,
            &["peer2"],
            Peer {
                per_tick: 14_000,
                reported: 10_000.0,
                overwrite: false,
            },
        )
        .await?;
        assert_eq!(fake.starts_of("peer2"), [1_500_000]);
        let path = f.now().path.ok_or_else(|| Error::internal("no file"))?;
        assert_eq!(
            std::fs::read(&path).map_err(|e| Error::internal(e.to_string()))?,
            bytes,
            "every byte is the first peer's in the end"
        );
        Ok(())
    }
}
