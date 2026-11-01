//! Getting a song onto disk: one slskd transfer at a time, from copies in the order the
//! listener's streaming or saving profile sets (ADR-0008), falling back to the next one when a
//! peer doesn't answer, queues us too long or stalls. When no copy passes the profile's minimum,
//! its `below` rule decides. A fetch is shared by everyone who wants the song (the player
//! streaming it, a save completing it), so the bytes come over the network once
//! (docs/context/streaming.md).

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use lull_core::events::Bus;
use lull_core::ids::TrackId;
use lull_core::model::{SourceRef, Track};
use lull_core::{Error, Result};
use serde::Serialize;
use serde_json::json;
use tokio::sync::watch;

use crate::cache::{Cache, Entry, Pin};
use crate::files;
use crate::layout::Layout;
use lull_core::quality::{SaveBelow, StreamBelow};

use crate::peers::Peers;
use crate::rules::{Current, Purpose, Rules};
use crate::search::Searches;
use crate::slskd::{Phase, Slskd, Transfer};
use crate::sources::{Ranked, Source, Sources};

/// Received bytes after which a stream counts as playing rather than buffering.
const BUFFERED: u64 = 256 * 1024;
const POLL: Duration = Duration::from_secs(1);
/// A peer that hasn't accepted the request by now is skipped.
const START_TIMEOUT: Duration = Duration::from_secs(20);
/// Waiting in a peer's queue longer than this moves on to the next source.
const QUEUE_TIMEOUT: Duration = Duration::from_secs(30);
/// No new bytes for this long shows as stalled; Settings › Sources' stall timeout falls back.
const STALL_NOTICE: Duration = Duration::from_secs(5);
const MAX_ATTEMPTS: usize = 5;
/// `keepLooking`: searches for a copy that passes the streaming minimum, this far apart.
const KEEP_LOOKING_SEARCHES: u32 = 3;
const KEEP_LOOKING_GAP: Duration = Duration::from_secs(20);
/// `ask`: the copies offered, and how long the listener has to choose one.
const CHOICES: usize = 8;
const CHOOSE_WAIT: Duration = Duration::from_secs(10 * 60);
/// A finished song nobody has read for this long is no longer "playing" (so it may be evicted).
const IDLE: Duration = Duration::from_secs(30 * 60);
const HISTORY: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Finding,
    /// `ask`: no copy passes the streaming minimum; waiting for the listener to pick one.
    Choosing,
    Queued,
    Buffering,
    Streaming,
    Stalled,
    Fallback,
    Done,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize)]
pub struct FailedPeer {
    pub peer: String,
    /// Percent of the file received when the peer gave up.
    pub at: u8,
}

/// Where a fetch stands. Readers serve bytes from `path` while `attempt` stays the same.
#[derive(Debug, Clone)]
pub struct Progress {
    /// Bumped on every fallback: a reader of an older attempt has the wrong file.
    pub attempt: u32,
    pub status: Status,
    pub source: Option<Source>,
    /// Bytes on disk so far.
    pub received: u64,
    /// Bytes per second.
    pub speed: u64,
    pub queue_pos: Option<u32>,
    /// The file to read: the growing incomplete file, then the finished one.
    pub path: Option<PathBuf>,
    pub error: Option<String>,
    pub failed_peers: Vec<FailedPeer>,
    pub peers_found: usize,
    /// The source doesn't pass the streaming minimum (`playBest`, or the listener's pick).
    pub below: bool,
    /// What `ask` offers while `Choosing`.
    pub choices: Vec<Ranked>,
}

impl Progress {
    fn new(status: Status) -> Self {
        Progress {
            attempt: 0,
            status,
            source: None,
            received: 0,
            speed: 0,
            queue_pos: None,
            path: None,
            error: None,
            failed_peers: Vec::new(),
            peers_found: 0,
            below: false,
            choices: Vec::new(),
        }
    }

    pub fn size(&self) -> u64 {
        self.source.as_ref().map_or(0, |s| s.size)
    }

    pub fn ended(&self) -> bool {
        matches!(self.status, Status::Done | Status::Failed | Status::Cancelled)
    }

    fn pct(&self) -> u8 {
        match self.size() {
            0 => 0,
            size => (self.received.min(size) * 100 / size) as u8,
        }
    }
}

pub struct Fetch {
    pub job_id: String,
    pub track: Track,
    /// Whose profile orders the copies: the player's or a save's.
    pub purpose: Purpose,
    pub started: i64,
    progress: watch::Sender<Progress>,
    stop: watch::Sender<bool>,
    want: Mutex<Want>,
    readers: AtomicUsize,
    last_read: Mutex<Instant>,
    _pin: Pin,
}

#[derive(Default)]
struct Want {
    streaming: bool,
    saving: u32,
}

impl Fetch {
    pub fn progress(&self) -> watch::Receiver<Progress> {
        self.progress.subscribe()
    }

    pub fn now(&self) -> Progress {
        self.progress.borrow().clone()
    }

    /// Marks a response body in flight; the fetch counts as playing while any is.
    pub fn reading(self: &Arc<Self>) -> Reader {
        self.readers.fetch_add(1, Ordering::Relaxed);
        Reader(self.clone())
    }

    fn idle(&self) -> bool {
        self.readers.load(Ordering::Relaxed) == 0 && self.last_read.lock().is_ok_and(|t| t.elapsed() > IDLE)
    }
}

pub struct Reader(Arc<Fetch>);

impl Drop for Reader {
    fn drop(&mut self) {
        self.0.readers.fetch_sub(1, Ordering::Relaxed);
        if let Ok(mut t) = self.0.last_read.lock() {
            *t = Instant::now();
        }
    }
}

enum Stop {
    Cancelled,
    Failed(String),
}

enum Skip {
    /// The peer stalled or failed mid-way: remembered against it.
    Stalled(String),
    /// The peer is offline or refused: try the next one, no grudge.
    Unavailable(String),
}

#[derive(Clone)]
pub struct Fetcher {
    pub slskd: Slskd,
    pub layout: Arc<RwLock<Layout>>,
    pub cache: Cache,
    pub peers: Peers,
    pub sources: Sources,
    pub searches: Searches,
    pub bus: Bus,
    pub rules: Current,
    live: Arc<Mutex<HashMap<TrackId, Arc<Fetch>>>>,
    /// Ended fetches, newest last, for the Activity panel.
    ended: Arc<Mutex<VecDeque<serde_json::Value>>>,
    next: Arc<AtomicU64>,
}

impl Fetcher {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        slskd: Slskd,
        layout: Arc<RwLock<Layout>>,
        cache: Cache,
        peers: Peers,
        sources: Sources,
        searches: Searches,
        bus: Bus,
        rules: Current,
    ) -> Self {
        Fetcher {
            slskd,
            layout,
            cache,
            peers,
            sources,
            searches,
            bus,
            rules,
            live: Arc::default(),
            ended: Arc::default(),
            next: Arc::default(),
        }
    }

    fn live(&self, track: &TrackId) -> Option<Arc<Fetch>> {
        let live = self.live.lock().ok()?;
        live.get(track)
            .filter(|f| !matches!(f.now().status, Status::Failed | Status::Cancelled))
            .cloned()
    }

    /// The fetch the player reads: the running one, the cached file, or a new transfer.
    /// `pick` (the source picker's choice, mid-song or answering `ask`) replaces a running fetch
    /// from another peer.
    pub async fn stream(&self, track: Track, pick: Option<SourceRef>) -> Result<Arc<Fetch>> {
        if let Some(f) = self.live(&track.id) {
            let current = f.now().source.map(|s| s.source_ref());
            if pick.is_none() || pick == current {
                want(&f, |w| w.streaming = true);
                self.cache.touch(&track.id).await?;
                return Ok(f);
            }
            self.replace(&f);
        }
        let cached = self.cache.finished(&track.id, pick.as_ref()).await?;
        let f = match cached.filter(|e| pick.is_none() || pick.as_ref() == Some(&e.source)) {
            // The bytes are here, so the profile's `below` rule doesn't hold them back; the
            // player still learns which copy it is, and whether it's below the minimum.
            Some(entry) => {
                let f = self.cached(track, entry).await?;
                want(&f, |w| w.streaming = true);
                self.bus.publish("stream", stream_event(&f, &f.now()));
                return Ok(f);
            }
            None => self.start(track, pick, Purpose::Stream),
        };
        want(&f, |w| w.streaming = true);
        Ok(f)
    }

    /// Transfers the song from `source` to the end and returns the finished file, joining a
    /// running fetch or reusing the cached file from that same source.
    pub async fn complete(&self, track: Track, source: SourceRef) -> Result<PathBuf> {
        // Not one waiting for the listener's choice: the save already named its copy.
        let running = self.live(&track.id).filter(|f| {
            let p = f.now();
            p.status != Status::Choosing
                && (p.source.as_ref().is_none_or(|s| s.is(&source)) || p.status == Status::Finding)
        });
        let f = match running {
            Some(f) => f,
            None => match self.cache.finished(&track.id, Some(&source)).await? {
                Some(e) if e.source == source => return Ok(e.path),
                _ => self.start(track, Some(source), Purpose::Save),
            },
        };
        want(&f, |w| w.saving += 1);
        let mut rx = f.progress();
        let ended = rx.wait_for(Progress::ended).await.map(|p| p.clone());
        want(&f, |w| w.saving -= 1);
        let p = ended.map_err(|_| Error::internal("transfer ended without a result"))?;
        match (p.status, p.path) {
            (Status::Done, Some(path)) => Ok(path),
            (Status::Cancelled, _) => Err(Error::Conflict("the transfer was cancelled".into())),
            _ => Err(Error::upstream(
                "soulseek",
                p.error.unwrap_or_else(|| "the transfer failed".into()),
            )),
        }
    }

    /// The player switched to another copy: the old fetch stops, unless a save is completing
    /// it; then it runs on for the save, out of the player's way.
    fn replace(&self, f: &Arc<Fetch>) {
        let saving = want(f, |w| {
            w.streaming = false;
            w.saving
        });
        if saving == 0 {
            self.cancel_fetch(f);
        } else if let Ok(mut live) = self.live.lock() {
            live.retain(|_, other| !Arc::ptr_eq(other, f));
        }
    }

    /// The player moved on (skip, next): cancel unless a save still needs the song.
    pub fn stop_streaming(&self, track: &TrackId) {
        let Some(f) = self.live.lock().ok().and_then(|l| l.get(track).cloned()) else {
            return;
        };
        let saving = want(&f, |w| {
            w.streaming = false;
            w.saving
        });
        if saving == 0 {
            self.cancel_fetch(&f);
        }
    }

    pub fn cancel(&self, job_id: &str) -> Result<()> {
        let f = self
            .by_job(job_id)
            .ok_or_else(|| Error::NotFound(format!("no running transfer {job_id}")))?;
        self.cancel_fetch(&f);
        Ok(())
    }

    /// Runs a failed or cancelled transfer again, from the start of the source list.
    pub fn retry(&self, job_id: &str) -> Result<String> {
        let ended = self
            .ended
            .lock()
            .ok()
            .and_then(|e| e.iter().find(|r| r["jobId"] == job_id).cloned());
        let row = ended.ok_or_else(|| Error::NotFound(format!("no ended transfer {job_id}")))?;
        let track: Track =
            serde_json::from_value(row["track"].clone()).map_err(|e| Error::internal(format!("transfer row: {e}")))?;
        Ok(self.start(track, None, Purpose::Stream).job_id.clone())
    }

    /// Activity rows: running transfers first, then recently ended ones.
    pub fn rows(&self) -> Vec<serde_json::Value> {
        let mut rows: Vec<_> = self
            .live
            .lock()
            .map(|l| l.values().map(|f| job_row(f, &f.now())).collect())
            .unwrap_or_default();
        if let Ok(ended) = self.ended.lock() {
            rows.extend(ended.iter().rev().map(|r| {
                let mut r = r.clone();
                if let Some(o) = r.as_object_mut() {
                    o.remove("track");
                }
                r
            }));
        }
        rows
    }

    /// Whether bytes are moving for any song right now.
    pub fn transferring(&self) -> bool {
        self.live.lock().is_ok_and(|l| {
            l.values()
                .any(|f| matches!(f.now().status, Status::Buffering | Status::Streaming | Status::Stalled))
        })
    }

    /// The source the song is (or was last) fetched from, while its fetch is live.
    pub fn current_source(&self, track: &TrackId) -> Option<SourceRef> {
        self.live(track)?.now().source.map(|s| s.source_ref())
    }

    /// Lets go of finished songs nobody is reading, so the cache may evict them.
    pub fn sweep(&self) {
        let Ok(mut live) = self.live.lock() else { return };
        live.retain(|_, f| {
            let p = f.now();
            let saving = f.want.lock().map(|w| w.saving).unwrap_or(0);
            !(p.ended() && saving == 0 && f.idle())
        });
    }

    fn by_job(&self, job_id: &str) -> Option<Arc<Fetch>> {
        self.live.lock().ok()?.values().find(|f| f.job_id == job_id).cloned()
    }

    fn cancel_fetch(&self, f: &Arc<Fetch>) {
        if f.now().ended() {
            if let Ok(mut live) = self.live.lock() {
                live.retain(|_, other| !Arc::ptr_eq(other, f));
            }
        } else {
            f.stop.send_replace(true);
        }
    }

    fn new_fetch(&self, track: Track, purpose: Purpose, progress: Progress) -> Arc<Fetch> {
        let n = self.next.fetch_add(1, Ordering::Relaxed);
        let f = Arc::new(Fetch {
            job_id: format!("job-{:x}{n}", lull_core::now()),
            purpose,
            started: lull_core::now(),
            progress: watch::channel(progress).0,
            stop: watch::channel(false).0,
            want: Mutex::default(),
            readers: AtomicUsize::new(0),
            last_read: Mutex::new(Instant::now()),
            _pin: self.cache.pin(&track.id),
            track,
        });
        if let Ok(mut live) = self.live.lock() {
            live.insert(f.track.id.clone(), f.clone());
        }
        f
    }

    async fn cached(&self, track: Track, e: Entry) -> Result<Arc<Fetch>> {
        self.cache.touch(&track.id).await?;
        let source = self
            .sources
            .get(&track.id, &e.source)
            .or_else(|| bare_source(&e.source));
        let below = source
            .as_ref()
            .is_some_and(|s| !s.meets(&self.rules.get().stream.minimum, track.duration));
        Ok(self.new_fetch(
            track,
            Purpose::Stream,
            Progress {
                attempt: 1,
                received: e.source.size,
                source,
                path: Some(e.path),
                below,
                ..Progress::new(Status::Done)
            },
        ))
    }

    fn start(&self, track: Track, pick: Option<SourceRef>, purpose: Purpose) -> Arc<Fetch> {
        let f = self.new_fetch(track, purpose, Progress::new(Status::Finding));
        let this = self.clone();
        let fetch = f.clone();
        tokio::spawn(async move { this.run(fetch, pick).await });
        f
    }

    async fn run(self, f: Arc<Fetch>, pick: Option<SourceRef>) {
        let outcome = self.attempts(&f, pick).await;
        self.update(&f, |p| match outcome {
            Ok(path) => {
                p.status = Status::Done;
                p.received = p.size();
                p.path = Some(path);
            }
            Err(Stop::Cancelled) => p.status = Status::Cancelled,
            Err(Stop::Failed(e)) => {
                p.status = Status::Failed;
                p.error = Some(e);
            }
        });
        let p = f.now();
        if let Ok(mut ended) = self.ended.lock() {
            let mut row = job_row(&f, &p);
            row["track"] = json!(f.track);
            ended.push_back(row);
            while ended.len() > HISTORY {
                ended.pop_front();
            }
        }
        // A stopped fetch leaves even when it finished at the last moment: nobody is playing it.
        let stopped = *f.stop.borrow();
        if (p.status != Status::Done || stopped)
            && let Ok(mut live) = self.live.lock()
        {
            live.retain(|_, other| !Arc::ptr_eq(other, &f));
        }
    }

    async fn attempts(&self, f: &Fetch, pick: Option<SourceRef>) -> Result<PathBuf, Stop> {
        let mut tried: Vec<SourceRef> = Vec::new();
        let mut stop = f.stop.subscribe();
        for attempt in 1..=MAX_ATTEMPTS as u32 {
            let next = self.next_source(f, pick.as_ref(), &tried, &mut stop).await?;
            tried.push(next.source_ref());
            match self.attempt(f, attempt, &next, &mut stop).await {
                Ok(path) => return Ok(path),
                Err(Ok(skip)) => {
                    let (reason, stalled) = match skip {
                        Skip::Stalled(r) => (r, true),
                        Skip::Unavailable(r) => (r, false),
                    };
                    tracing::info!(peer = %next.user, reason, "falling back to the next source");
                    if stalled && let Err(e) = self.peers.stalled(&next.user).await {
                        tracing::warn!(error = %e, "recording a stalled peer");
                    }
                    self.update(f, |p| {
                        p.failed_peers.push(FailedPeer {
                            peer: next.user.clone(),
                            at: p.pct(),
                        });
                        p.status = Status::Fallback;
                        p.error = Some(reason);
                    });
                }
                Err(Err(stop)) => return Err(stop),
            }
        }
        Err(Stop::Failed(format!(
            "{MAX_ATTEMPTS} peers tried, none delivered the song"
        )))
    }

    /// The listener's pick first, then the first untried copy in the profile's order that
    /// passes its minimum. With nothing known about the song, searches for it first
    /// ("finding"). When copies exist but none passes, the profile's `below` rule decides.
    async fn next_source(
        &self,
        f: &Fetch,
        pick: Option<&SourceRef>,
        tried: &[SourceRef],
        stop: &mut watch::Receiver<bool>,
    ) -> Result<Source, Stop> {
        if let Some(pick) = pick.filter(|p| !tried.contains(p)) {
            return self
                .sources
                .get(&f.track.id, pick)
                .or_else(|| bare_source(pick))
                .ok_or_else(|| Stop::Failed(format!("{} is not audio lull can play", pick.path)));
        }
        let rules = self.rules.get();
        let mut searches = 0;
        loop {
            let (copies, queued_out) = self.untried(f, tried, &rules);
            if copies.is_empty() && queued_out == 0 && tried.is_empty() && searches == 0 {
                self.find(f, stop).await?;
                searches += 1;
                continue;
            }
            let peers = copies.len() + tried.len();
            self.update(f, |p| p.peers_found = peers);
            if let Some(passing) = copies.iter().find(|r| r.meets) {
                return Ok(passing.source.clone());
            }
            let Some(best) = copies.first() else {
                return Err(Stop::Failed(match (queued_out, tried.is_empty()) {
                    (0, true) => "no peer shares this song right now".into(),
                    (0, false) => "no other peer shares this song right now".into(),
                    (n, _) => format!(
                        "{n} more peers share this song, each with more than {} people waiting",
                        rules.max_peer_queue
                    ),
                }));
            };
            match below(f.purpose, &rules) {
                Below::Take => return Ok(best.source.clone()),
                Below::Search if searches < KEEP_LOOKING_SEARCHES => {
                    if searches > 0 {
                        tokio::select! {
                            _ = tokio::time::sleep(KEEP_LOOKING_GAP) => {}
                            _ = stopped(stop) => return Err(Stop::Cancelled),
                        }
                    }
                    self.find(f, stop).await?;
                    searches += 1;
                }
                Below::Search => {
                    return Err(Stop::Failed(format!(
                        "no copy meets your streaming minimum ({}) after {searches} searches",
                        rules.stream.minimum
                    )));
                }
                Below::Ask => return self.ask(f, copies, stop).await,
                Below::Refuse => {
                    return Err(Stop::Failed(format!(
                        "no other copy meets your saving minimum ({})",
                        rules.save.minimum
                    )));
                }
            }
        }
    }

    /// Untried copies in the profile's order, and how many were left out for a long queue.
    fn untried(&self, f: &Fetch, tried: &[SourceRef], rules: &Rules) -> (Vec<Ranked>, usize) {
        let (copies, queued_out): (Vec<Ranked>, Vec<Ranked>) = self
            .sources
            .ordered(&f.track.id, f.track.duration, rules, f.purpose)
            .into_iter()
            .filter(|r| !tried.iter().any(|t| r.source.is(t)))
            .partition(|r| !r.queue_too_long);
        (copies, queued_out.len())
    }

    /// Searches Soulseek for the song ("finding").
    async fn find(&self, f: &Fetch, stop: &mut watch::Receiver<bool>) -> Result<(), Stop> {
        self.update(f, |p| p.status = Status::Finding);
        tokio::select! {
            r = self.searches.resolve(&f.track) => r.map_err(|e| Stop::Failed(reason(&e))),
            _ = stopped(stop) => Err(Stop::Cancelled),
        }
    }

    /// `ask`: offers the copies found in the `stream` event. The listener answers by playing
    /// one (`GET /api/stream/{id}?peer=&path=`), which replaces this fetch, or by skipping.
    async fn ask(&self, f: &Fetch, copies: Vec<Ranked>, stop: &mut watch::Receiver<bool>) -> Result<Source, Stop> {
        self.update(f, |p| {
            p.status = Status::Choosing;
            p.choices = copies.into_iter().take(CHOICES).collect();
        });
        tokio::select! {
            _ = tokio::time::sleep(CHOOSE_WAIT) => Err(Stop::Failed(format!(
                "no copy meets your streaming minimum ({}), and none was chosen",
                self.rules.get().stream.minimum
            ))),
            _ = stopped(stop) => Err(Stop::Cancelled),
        }
    }

    /// One transfer from one source, watched until it finishes, stalls or is stopped.
    async fn attempt(
        &self,
        f: &Fetch,
        attempt: u32,
        source: &Source,
        stop: &mut watch::Receiver<bool>,
    ) -> Result<PathBuf, Result<Skip, Stop>> {
        self.cache
            .make_room(source.size)
            .await
            .map_err(|e| Err(Stop::Failed(reason(&e))))?;
        let rules = self.rules.get();
        let below = !source.meets(&rules.stream.minimum, f.track.duration);
        self.update(f, |p| {
            p.attempt = attempt;
            p.source = Some(source.clone());
            p.below = below;
            p.choices.clear();
            p.status = Status::Queued;
            p.received = 0;
            p.speed = 0;
            p.queue_pos = None;
            p.path = None;
            p.error = None;
        });
        let transfer = match self.slskd.enqueue(&source.user, &source.path, source.size).await {
            Ok(t) => t,
            Err(e @ Error::Unavailable { .. }) => return Err(Err(Stop::Failed(reason(&e)))),
            Err(e) => return Err(Ok(Skip::Unavailable(format!("{}: {}", source.user, reason(&e))))),
        };
        let candidates = self
            .layout
            .read()
            .map(|l| l.incomplete(&source.user, &source.path))
            .unwrap_or_default();
        let record = |path: PathBuf, complete| Entry {
            track: f.track.id.clone(),
            source: source.source_ref(),
            path,
            complete,
            used_at: lull_core::now(),
        };
        let begun = Instant::now();
        let mut last_bytes = (0u64, Instant::now());
        let mut recorded = false;
        let mut was_running = false;
        loop {
            tokio::select! {
                _ = tokio::time::sleep(POLL) => {}
                _ = stopped(stop) => {
                    // It may have finished since the last poll: keep track of a finished file
                    // (cached, evictable) rather than cancel what's already done.
                    if let Ok(t) = self.slskd.download(&source.user, &transfer.id).await
                        && t.phase() == Phase::Succeeded
                    {
                        return self.finish(f, source, &t).await.map_err(|e| Err(Stop::Failed(reason(&e))));
                    }
                    if let Err(e) = self.slskd.cancel(&source.user, &transfer.id).await {
                        tracing::warn!(error = %e, "cancelling a transfer in slskd");
                    }
                    return Err(Err(Stop::Cancelled));
                }
            }
            let t = match self.slskd.download(&source.user, &transfer.id).await {
                Ok(t) => t,
                Err(e @ Error::Unavailable { .. }) => return Err(Err(Stop::Failed(reason(&e)))),
                Err(e) => return Err(Ok(Skip::Unavailable(format!("{}: {}", source.user, reason(&e))))),
            };
            let on_disk = on_disk(&candidates).await;
            // Recorded once slskd has written it, so even a partial left by a cancel is evictable.
            if let Some((path, _)) = on_disk.as_ref().filter(|_| !recorded) {
                self.cache
                    .record(record(path.clone(), false))
                    .await
                    .map_err(|e| Err(Stop::Failed(reason(&e))))?;
                recorded = true;
            }
            let received = on_disk.as_ref().map_or(t.bytes_transferred, |(_, len)| *len);
            // Silence counts from the last new byte, or from when the peer started sending: a
            // slow connection set-up (or slskd retrying the peer) is not a stall.
            let running = t.phase() == Phase::InProgress;
            if received > last_bytes.0 || (running && !was_running) {
                last_bytes = (received, Instant::now());
            }
            was_running = running;
            let quiet = last_bytes.1.elapsed();
            match t.phase() {
                Phase::Starting if begun.elapsed() > START_TIMEOUT => {
                    self.give_up(source, &t).await;
                    return Err(Ok(Skip::Unavailable(format!("{} did not answer", source.user))));
                }
                // Measured from the last new byte: a resumed partial file starts above zero.
                Phase::Queued if quiet > QUEUE_TIMEOUT => {
                    self.give_up(source, &t).await;
                    let pos = t
                        .place_in_queue
                        .map(|q| format!(" at position {q}"))
                        .unwrap_or_default();
                    return Err(Ok(Skip::Unavailable(format!("{} kept us queued{pos}", source.user))));
                }
                Phase::InProgress if quiet > rules.stall_timeout() => {
                    self.give_up(source, &t).await;
                    let pct = received.saturating_mul(100) / source.size.max(1);
                    return Err(Ok(Skip::Stalled(format!("{} stalled at {pct}%", source.user))));
                }
                Phase::Failed => {
                    let why = t.exception.clone().unwrap_or_else(|| t.state.clone());
                    return Err(Ok(Skip::Stalled(format!("{}: {why}", source.user))));
                }
                Phase::Succeeded => {
                    return self
                        .finish(f, source, &t)
                        .await
                        .map_err(|e| Err(Stop::Failed(reason(&e))));
                }
                phase => self.update(f, |p| {
                    p.received = received;
                    p.speed = t.average_speed.max(0.0) as u64;
                    p.queue_pos = t.place_in_queue;
                    p.path = on_disk.as_ref().map(|(path, _)| path.clone());
                    p.status = match phase {
                        Phase::Starting | Phase::Queued => Status::Queued,
                        _ if quiet > STALL_NOTICE => Status::Stalled,
                        _ if received < BUFFERED => Status::Buffering,
                        _ => Status::Streaming,
                    };
                }),
            }
        }
    }

    /// A transfer slskd finished: the file is recorded in the cache where slskd moved it, the
    /// peer's stall record is cleared, and slskd's transfer record is removed.
    async fn finish(&self, f: &Fetch, source: &Source, t: &Transfer) -> Result<PathBuf> {
        let path = self.finished_path(t).await?;
        self.cache
            .record(Entry {
                track: f.track.id.clone(),
                source: source.source_ref(),
                path: path.clone(),
                complete: true,
                used_at: lull_core::now(),
            })
            .await?;
        if let Err(e) = self.cache.report().await {
            tracing::warn!(error = %e, "measuring the stream cache");
        }
        if let Err(e) = self.peers.delivered(&source.user).await {
            tracing::warn!(error = %e, "clearing a peer's stall record");
        }
        if let Err(e) = self.slskd.cancel(&source.user, &t.id).await {
            tracing::debug!(error = %e, "removing a finished transfer from slskd's list");
        }
        Ok(path)
    }

    async fn give_up(&self, source: &Source, t: &Transfer) {
        if let Err(e) = self.slskd.cancel(&source.user, &t.id).await {
            tracing::warn!(error = %e, "cancelling a stalled transfer in slskd");
        }
    }

    async fn finished_path(&self, t: &Transfer) -> Result<PathBuf> {
        let reported = self.slskd.finished_path(&t.id).await?.ok_or_else(|| {
            Error::upstream(
                "soulseek",
                format!("slskd finished {} but did not say where it put it", t.filename),
            )
        })?;
        let path = self
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
        if !path.is_file() {
            return Err(Error::unavailable(
                "soulseek",
                format!(
                    "lull cannot see slskd's finished file at {}; check LULL_TRANSFERS_DIR",
                    path.display()
                ),
            ));
        }
        Ok(path)
    }

    /// Applies a change and publishes `stream`, `transfer` and `job` events when it matters.
    fn update(&self, f: &Fetch, change: impl FnOnce(&mut Progress)) {
        let before = f.now();
        f.progress.send_modify(change);
        let p = f.now();
        let moved = p.status != before.status
            || p.received != before.received
            || p.queue_pos != before.queue_pos
            || p.attempt != before.attempt
            || p.failed_peers.len() != before.failed_peers.len();
        if !moved {
            return;
        }
        let streaming = f.want.lock().is_ok_and(|w| w.streaming);
        if streaming && p.status != Status::Cancelled {
            self.bus.publish("stream", stream_event(f, &p));
        }
        self.bus.publish("transfer", transfer_event(f, &p));
        self.bus.publish("job", job_row(f, &p));
    }
}

/// What to do when copies exist but none passes the profile's minimum.
enum Below {
    /// Take the best of them (`playBest`, `saveBestUpgrade`).
    Take,
    /// Search again for a passing copy (`keepLooking`).
    Search,
    /// Offer them to the listener (`ask`).
    Ask,
    /// A save's fallback: library already applied `wait` or `ask` before it started.
    Refuse,
}

fn below(purpose: Purpose, rules: &Rules) -> Below {
    match purpose {
        Purpose::Stream => match rules.stream.below {
            StreamBelow::PlayBest => Below::Take,
            StreamBelow::KeepLooking => Below::Search,
            StreamBelow::Ask => Below::Ask,
        },
        Purpose::Save => match rules.save.below {
            SaveBelow::SaveBestUpgrade => Below::Take,
            SaveBelow::Wait | SaveBelow::Ask => Below::Refuse,
        },
    }
}

/// An error as the fetch's `error`: the bare reason. "soulseek failed: " is added once, where
/// the error leaves this crate (the stream response, a save), never twice.
fn reason(e: &Error) -> String {
    match e {
        Error::Upstream { reason, .. } | Error::Unavailable { reason, .. } => reason.clone(),
        other => other.to_string(),
    }
}

/// Resolves once `stop` is set (or its sender is gone).
pub async fn stopped(stop: &mut watch::Receiver<bool>) {
    let _ = stop.wait_for(|s| *s).await;
}

fn want<T>(f: &Fetch, change: impl FnOnce(&mut Want) -> T) -> T {
    let mut w = f.want.lock().unwrap_or_else(|p| p.into_inner());
    change(&mut w)
}

/// A source known only by reference (picked before a restart, or named by a save): its facts
/// come from the file name alone.
fn bare_source(r: &SourceRef) -> Option<Source> {
    let ext = files::extension(&r.path)?;
    let facts = files::facts(&ext, r.size, files::Attributes::default())?;
    Some(Source {
        user: r.user.clone(),
        path: r.path.clone(),
        size: r.size,
        ext,
        facts,
        upload_speed: 0,
        free_slot: false,
        queue: 0,
        length: None,
    })
}

/// The first candidate path that exists, with its length (the bytes received so far).
async fn on_disk(candidates: &[PathBuf]) -> Option<(PathBuf, u64)> {
    for p in candidates {
        if let Ok(m) = tokio::fs::metadata(p).await {
            return Some((p.clone(), m.len()));
        }
    }
    None
}

fn stream_status(s: Status) -> &'static str {
    match s {
        Status::Finding => "finding",
        Status::Choosing => "choose",
        Status::Queued => "queued",
        Status::Buffering => "buffering",
        Status::Streaming | Status::Done => "streaming",
        Status::Stalled => "stalled",
        Status::Fallback => "fallback",
        Status::Failed | Status::Cancelled => "failed",
    }
}

fn transfer_state(s: Status) -> &'static str {
    match s {
        Status::Finding | Status::Choosing | Status::Queued => "queued",
        Status::Buffering | Status::Streaming => "transferring",
        Status::Stalled | Status::Fallback => "stalled",
        Status::Done => "done",
        Status::Failed => "failed",
        Status::Cancelled => "cancelled",
    }
}

pub fn stream_event(f: &Fetch, p: &Progress) -> serde_json::Value {
    let mut event = json!({
        "trackId": f.track.id,
        "jobId": f.job_id,
        "status": stream_status(p.status),
        "peer": p.source.as_ref().map(|s| &s.user),
        "format": p.source.as_ref().map(|s| &s.facts.format),
        "peersFound": p.peers_found,
        "queuePos": p.queue_pos,
        "received": p.received,
        "size": p.size(),
        "attempt": p.attempt,
        "failedPeers": p.failed_peers,
        "error": p.error,
        "belowMinimum": p.below,
    });
    if p.status == Status::Choosing {
        event["choices"] = json!(p.choices);
    }
    event
}

fn transfer_event(f: &Fetch, p: &Progress) -> serde_json::Value {
    json!({
        "jobId": f.job_id,
        "trackId": f.track.id,
        "peer": p.source.as_ref().map(|s| &s.user),
        "state": transfer_state(p.status),
        "queuePos": p.queue_pos,
        "received": p.received,
        "size": p.size(),
        "speed": p.speed,
        "error": p.error,
    })
}

/// The Activity panel's row (`MOCK.activity` shape), also published as the `job` event.
fn job_row(f: &Fetch, p: &Progress) -> serde_json::Value {
    json!({
        "jobId": f.job_id,
        "kind": "transfer",
        "trackId": f.track.id,
        "peer": p.source.as_ref().map(|s| &s.user),
        "speed": (p.speed as f64 / 100_000.0).round() / 10.0,
        "pct": p.pct(),
        "state": transfer_state(p.status),
        "queuePos": p.queue_pos,
        "started": f.started,
        "error": p.error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_stream_reads_as_one_sentence() {
        let inner = Error::upstream("soulseek", "answered 500 Internal Server Error");
        let shown = Error::upstream("soulseek", reason(&inner)).to_string();
        assert_eq!(shown, "soulseek failed: answered 500 Internal Server Error");
        let disk = Error::unavailable("soulseek", "the transfers disk is full");
        assert_eq!(reason(&disk), "the transfers disk is full");
    }
}
