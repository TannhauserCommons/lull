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

/// slskd's file is read this often while a leg runs; slskd itself is asked every `POLL`.
const COPY_TICK: std::time::Duration = std::time::Duration::from_millis(250);
/// At most this much of one leg is copied per tick, so a fast leg can't hold the loop.
const COPY_MAX: u64 = 4 * 1024 * 1024;
/// What arrived is written to the cache index each time it grows this much, and when a leg ends.
const SAVE_EVERY: u64 = 1024 * 1024;
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
    pub ranges: Ranges,
    /// `ranges.total()` when the cache index was last written.
    saved: u64,
    /// A leg of this copy waited in the peer's queue: the peer is busy, so no leg gives its
    /// slot back for a jump.
    queued: bool,
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
        while let Some(start) = jump.take().or_else(|| copy.ranges.next_missing(0, source.size)) {
            let leg = self.start_leg(source, start).await?;
            let ended = self.run_leg(f, source, &mut copy, leg, stop).await;
            self.save_copy(f, source, &mut copy, false).await.map_err(failed)?;
            match ended {
                Ok(Ended::Jump(at)) => jump = Some(at),
                Ok(Ended::Finished | Ended::Joined) => {}
                Err(e) => return Err(e),
            }
        }
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
        open.write(true).create(true);
        if ranges.total() == 0 {
            open.truncate(true);
        }
        let file = open.open(&path).await.map_err(|e| io(&path, e))?;
        let saved = ranges.total();
        Ok(Copy {
            path,
            file,
            ranges,
            saved,
            queued: false,
        })
    }

    /// Records the copy in the cache, with what has arrived while it is unfinished.
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

    /// Queues a leg from `start`: slskd's incomplete file is made exactly `start` bytes long, a
    /// hole, so slskd resumes there (`retry.partial: resume`, its default), and nothing before
    /// `start` in it is ever real audio. Both spellings of its path are (slskd on Windows spells
    /// some names its own way), so whichever slskd writes starts at `start`.
    async fn start_leg(&self, source: &Source, start: u64) -> Outcome<Leg> {
        let files = self
            .layout
            .read()
            .map(|l| l.incomplete(&source.user, &source.path))
            .unwrap_or_default();
        let fail = |e: Error| Err(Stop::Failed(reason(&e)));
        for path in &files {
            match tokio::fs::remove_file(path).await {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(fail(io(path, e))),
            }
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
        })
    }

    /// Runs a leg until slskd finishes it, it reaches bytes that arrived before, or it fails.
    /// What it brings is copied as it lands and published as the copy's ranges.
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
                    {
                        return self.land(source, copy, &mut leg, &t).await.map_err(failed);
                    }
                    self.drop_leg(source, &mut leg).await;
                    return Err(Err(Stop::Cancelled));
                }
            }
            self.copy_leg(copy, &mut leg).await.map_err(failed)?;
            // An slskd that starts every download over from 0 brings each leg from 0: stopped at
            // what had arrived, the next would start over again, so it runs to the end.
            let resumes = self.resumes.load(std::sync::atomic::Ordering::Relaxed);
            if resumes && leg.front > leg.start && copy.ranges.has(leg.front) {
                self.drop_leg(source, &mut leg).await;
                return Ok(Ended::Joined);
            }
            let speed = t.as_ref().map_or(0, |t| t.average_speed.max(0.0) as u64);
            let room = self.jumps_allowed(source, copy);
            let waiting = f.asked().map(|(byte, since)| (byte, since.elapsed()));
            if let Some(at) = jump(waiting, &copy.ranges, &leg, speed, room) {
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
            // Silence counts from the last new byte, or from when the peer started sending: a
            // slow connection set-up (or slskd retrying the peer) is not a stall.
            let running = t.phase() == Phase::InProgress;
            if leg.front > leg.last.0 || (running && !leg.was_running) {
                leg.last = (leg.front, Instant::now());
            }
            leg.was_running = running;
            let quiet = leg.last.1.elapsed();
            if t.phase() == Phase::Queued {
                copy.queued = true;
            }
            match t.phase() {
                Phase::Starting if leg.begun.elapsed() > START_TIMEOUT => {
                    self.drop_leg(source, &mut leg).await;
                    return Err(Ok(Skip::Unavailable(format!("{} did not answer", source.user))));
                }
                Phase::Queued if quiet > QUEUE_TIMEOUT => {
                    self.drop_leg(source, &mut leg).await;
                    let pos = t
                        .place_in_queue
                        .map(|q| format!(" at position {q}"))
                        .unwrap_or_default();
                    return Err(Ok(Skip::Unavailable(format!("{} kept us queued{pos}", source.user))));
                }
                Phase::InProgress if quiet > rules.stall_timeout() => {
                    self.drop_leg(source, &mut leg).await;
                    let pct = copy.ranges.total().saturating_mul(100) / source.size.max(1);
                    return Err(Ok(Skip::Stalled(format!("{} stalled at {pct}%", source.user))));
                }
                Phase::Failed => {
                    let why = t
                        .exception
                        .as_deref()
                        .map_or_else(|| t.state.clone(), crate::slskd::plain);
                    self.drop_leg(source, &mut leg).await;
                    return Err(Ok(Skip::Stalled(format!("{}: {why}", source.user))));
                }
                Phase::Succeeded => return self.land(source, copy, &mut leg, t).await.map_err(failed),
                phase => {
                    let brought = leg.front - leg.start;
                    let ahead = self.jumps_allowed(source, copy).then_some(JUMP_SAME_S);
                    self.update(f, |p| {
                        p.ranges = copy.ranges.clone();
                        p.ahead = ahead;
                        p.speed = t.average_speed.max(0.0) as u64;
                        p.queue_pos = t.place_in_queue;
                        p.path = (copy.ranges.total() > 0).then(|| copy.path.clone());
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

    /// Copies what slskd has written of the leg since the last look into lull's file. A file
    /// shorter than the leg's start, or one whose first bytes aren't the hole lull left, was
    /// started over from byte 0 (an slskd set to overwrite partial files): the leg then brings
    /// the copy from 0, which is still the copy's own bytes at their own offsets.
    async fn copy_leg(&self, copy: &mut Copy, leg: &mut Leg) -> lull_core::Result<()> {
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
        write_at(copy, leg.front, &buf[..n]).await?;
        leg.front += n as u64;
        Ok(())
    }

    /// slskd finished the leg and moved its file to the downloads folder: the rest of it comes
    /// from there, then the file and slskd's record go.
    async fn land(&self, source: &Source, copy: &mut Copy, leg: &mut Leg, t: &Transfer) -> lull_core::Result<Ended> {
        let finished = self.finished_path(t).await?;
        let mut file = File::open(&finished).await.map_err(|e| io(&finished, e))?;
        file.seek(SeekFrom::Start(leg.front))
            .await
            .map_err(|e| io(&finished, e))?;
        let mut rest = Vec::new();
        file.read_to_end(&mut rest).await.map_err(|e| io(&finished, e))?;
        write_at(copy, leg.front, &rest).await?;
        leg.front += rest.len() as u64;
        if leg.front < source.size {
            return Err(Error::upstream(
                "soulseek",
                format!("{} sent {} of {} bytes", source.user, leg.front, source.size),
            ));
        }
        drop(file);
        if let Err(e) = tokio::fs::remove_file(&finished).await {
            tracing::warn!(error = %e, file = %finished.display(), "removing a finished leg's file");
        }
        leg.reading = None;
        for path in &leg.files {
            if let Err(e) = tokio::fs::remove_file(path).await
                && e.kind() != std::io::ErrorKind::NotFound
            {
                tracing::debug!(error = %e, file = %path.display(), "removing a leg's other file");
            }
        }
        if let Err(e) = self.slskd.cancel(&source.user, &t.id).await {
            tracing::debug!(error = %e, "removing a finished transfer from slskd's list");
        }
        Ok(Ended::Finished)
    }

    /// The same peer may be asked to start over elsewhere: slskd resumes where lull sizes its file,
    /// the peer said it had a free slot, and no leg of this copy has waited in its queue.
    fn jumps_allowed(&self, source: &Source, copy: &Copy) -> bool {
        self.resumes.load(std::sync::atomic::Ordering::Relaxed) && source.free_slot && !copy.queued
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

/// Where the next leg starts, when a reader has waited (past `COLLAPSE`) for a byte that hasn't
/// arrived and the leg running won't bring it within `JUMP_SAME_S`: it is behind the leg, or too
/// far ahead at the leg's speed (an unknown speed is too slow). `room`: jumping is allowed.
fn jump(asked: Option<(u64, std::time::Duration)>, ranges: &Ranges, leg: &Leg, speed: u64, room: bool) -> Option<u64> {
    let (byte, waited) = asked?;
    if !room || waited < COLLAPSE || ranges.has(byte) {
        return None;
    }
    let soon = byte >= leg.front && speed > 0 && (byte - leg.front) / speed <= u64::from(JUMP_SAME_S);
    (!soon).then_some(byte)
}

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

/// Writes bytes of the copy at their offset in lull's file and records them as arrived.
async fn write_at(copy: &mut Copy, at: u64, bytes: &[u8]) -> lull_core::Result<()> {
    if bytes.is_empty() {
        return Ok(());
    }
    copy.file
        .seek(SeekFrom::Start(at))
        .await
        .map_err(|e| io(&copy.path, e))?;
    copy.file.write_all(bytes).await.map_err(|e| io(&copy.path, e))?;
    // tokio's file writes in the background: flushed, the bytes are the OS's before readers
    // are told they arrived.
    copy.file.flush().await.map_err(|e| io(&copy.path, e))?;
    copy.ranges.add(at, at + bytes.len() as u64);
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
        settles: Duration::ZERO,
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
        let fake = Fake::start(HashMap::from([(REMOTE.to_owned(), bytes.clone())]), peer)
            .await
            .map_err(|e| Error::internal(e.to_string()))?;
        let (fetcher, _) = fetcher(&fake).await?;
        let pick = SourceRef {
            user: "peer".into(),
            path: REMOTE.into(),
            size: bytes.len() as u64,
        };
        fetcher.sources.add(HashMap::from([(track().id, vec![found(&pick)])]));
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
        Ok((fake, f, bytes))
    }

    #[tokio::test]
    async fn a_reader_far_ahead_of_the_transfer_gets_the_peer_started_there_and_the_gap_fills_after() -> Result<()> {
        let (fake, f, bytes) = seek_ahead(Peer {
            per_tick: 14_000,
            reported: Some(10_000.0),
            overwrite: false,
            settles: Duration::ZERO,
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
            settles: Duration::ZERO,
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
            settles: Duration::ZERO,
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
}
