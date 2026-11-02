//! Getting a song onto disk: one slskd transfer at a time, from copies in the order the
//! listener's streaming or saving profile sets (ADR-0008), falling back to the next one when a
//! peer doesn't answer, queues us too long or stalls. When no copy passes the profile's minimum,
//! its `below` rule decides. A fetch is shared by everyone who wants the song (the player
//! streaming it, a save completing it), so the bytes come over the network once
//! (docs/context/streaming.md).
//!
//! YouTube is the second source (docs/context/youtube.md): with `fallback` it plays when no
//! Soulseek copy does in time; with `startFast` it starts the song at once while an upgrade fetch
//! gets Soulseek's best copy for later plays and saves. Its audio lands in the same cache, so
//! everything downstream (ranges, preloading, pins, eviction) is unchanged.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use lull_core::events::Bus;
use lull_core::ids::TrackId;
use lull_core::model::{SourceRef, Track};
use lull_core::seams::Youtube;
use lull_core::youtube::{self, Mode, Want as YoutubeWant};
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
/// A preload no device lists any more is cancelled after this long, so a player that opens the
/// song just before (or just after) sending its new list still finds it running.
const UNLIST_GRACE: Duration = Duration::from_secs(10);
const HISTORY: usize = 50;
/// A YouTube download is looked at this often: it arrives in a second or two.
const YOUTUBE_POLL: Duration = Duration::from_millis(250);
/// yt-dlp may extract the upload again before its first byte; silence counts after this.
const YOUTUBE_START: Duration = Duration::from_secs(30);

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
    /// It has turned to YouTube (finding or fetching a YouTube copy).
    on_youtube: AtomicBool,
    _pin: Pin,
}

#[derive(Default)]
struct Want {
    streaming: bool,
    saving: u32,
    /// A device lists the song to preload (`PUT /api/stream/prefetch`).
    prefetch: bool,
}

impl Want {
    /// Only preloading: no `stream` events, never `choose`, evictable once finished.
    fn prefetch_only(&self) -> bool {
        self.prefetch && !self.streaming && self.saving == 0
    }

    fn unwanted(&self) -> bool {
        !self.prefetch && !self.streaming && self.saving == 0
    }
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
    pub youtube: Arc<dyn Youtube>,
    /// What the player reads (or preloads, or a save completes): one fetch per song.
    live: Arc<Mutex<HashMap<TrackId, Arc<Fetch>>>>,
    /// Start fast's background fetches of Soulseek's best copy while YouTube plays the song, and
    /// saves started while YouTube plays it: out of the player's way, so its song never jumps.
    upgrades: Arc<Mutex<HashMap<TrackId, Arc<Fetch>>>>,
    /// Ended fetches, newest last, for the Activity panel.
    ended: Arc<Mutex<VecDeque<serde_json::Value>>>,
    next: Arc<AtomicU64>,
}

/// Which map a new fetch goes in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Slot {
    Live,
    Upgrade,
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
        youtube: Arc<dyn Youtube>,
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
            youtube,
            live: Arc::default(),
            upgrades: Arc::default(),
            ended: Arc::default(),
            next: Arc::default(),
        }
    }

    fn live(&self, track: &TrackId) -> Option<Arc<Fetch>> {
        running(&self.live, track)
    }

    fn upgrade(&self, track: &TrackId) -> Option<Arc<Fetch>> {
        running(&self.upgrades, track)
    }

    /// Every fetch in both maps.
    fn all(&self) -> Vec<Arc<Fetch>> {
        [&self.live, &self.upgrades]
            .iter()
            .filter_map(|m| m.lock().ok().map(|m| m.values().cloned().collect::<Vec<_>>()))
            .flatten()
            .collect()
    }

    fn forget(&self, f: &Arc<Fetch>) {
        for map in [&self.live, &self.upgrades] {
            if let Ok(mut m) = map.lock() {
                m.retain(|_, other| !Arc::ptr_eq(other, f));
            }
        }
    }

    /// The fetch the player reads: the running one, the cached file, or a new transfer.
    /// `pick` (the source picker's choice, mid-song or answering `ask`) replaces a running fetch
    /// from another peer.
    pub async fn stream(&self, track: Track, pick: Option<SourceRef>) -> Result<Arc<Fetch>> {
        let mode = self.rules.get().youtube.mode;
        let live = match self.live(&track.id) {
            Some(f) if pick.is_none() && self.superseded(&f).await? => {
                self.forget(&f);
                None
            }
            other => other,
        };
        if let Some(f) = live {
            let current = f.now().source.map(|s| s.source_ref());
            if pick.is_none() || pick == current {
                // Start fast: a preload still waiting for a peer becomes the upgrade, and
                // YouTube starts the song below.
                if !(pick.is_none() && mode == Mode::StartFast && self.make_way(&f)) {
                    let joined = !want(&f, |w| std::mem::replace(&mut w.streaming, true));
                    self.cache.touch(&track.id).await?;
                    // A preload (or a save) the player joins: it learns where the song stands now.
                    if joined {
                        self.bus.publish("stream", stream_event(&f, &f.now()));
                    }
                    return Ok(f);
                }
            } else {
                self.replace(&f);
            }
        }
        let cached = self
            .cache
            .finished(&track.id, pick.as_ref())
            .await?
            .filter(|e| mode != Mode::Off || !youtube::is_youtube(&e.source));
        let f = match cached.filter(|e| pick.is_none() || pick.as_ref() == Some(&e.source)) {
            // The bytes are here, so the profile's `below` rule doesn't hold them back; the
            // player still learns which copy it is, and whether it's below the minimum.
            Some(entry) => {
                let on_youtube = youtube::is_youtube(&entry.source);
                let f = self.cached(track.clone(), entry).await?;
                want(&f, |w| w.streaming = true);
                self.bus.publish("stream", stream_event(&f, &f.now()));
                if on_youtube && mode == Mode::StartFast {
                    self.upgrade_later(track).await?;
                }
                return Ok(f);
            }
            None => self.start(track, pick, Purpose::Stream, Want::default(), Slot::Live),
        };
        want(&f, |w| w.streaming = true);
        Ok(f)
    }

    /// A YouTube copy that has played through while Soulseek's copy arrived: asking for the song
    /// now is a later play, which takes Soulseek's. Within the song's length (and a minute) it is
    /// the same play seeking, which never jumps to another file.
    async fn superseded(&self, f: &Arc<Fetch>) -> Result<bool> {
        let length = i64::from(f.track.duration.unwrap_or(600));
        let played_out = f.on_youtube.load(Ordering::Relaxed)
            && f.now().status == Status::Done
            && f.readers.load(Ordering::Relaxed) == 0
            && lull_core::now() - f.started > length + 60
            && want(f, |w| w.saving == 0);
        if !played_out {
            return Ok(false);
        }
        let cached = self.cache.finished(&f.track.id, None).await?;
        Ok(cached.is_some_and(|e| !youtube::is_youtube(&e.source)))
    }

    /// Start fast: a preload still waiting for a Soulseek peer steps out of the player's way
    /// and carries on as the song's upgrade. False when it has bytes coming, is on YouTube
    /// already, or someone else needs it.
    fn make_way(&self, f: &Arc<Fetch>) -> bool {
        let waiting = matches!(f.now().status, Status::Finding | Status::Queued | Status::Fallback);
        let aside = waiting
            && f.purpose == Purpose::Stream
            && !f.on_youtube.load(Ordering::Relaxed)
            && want(f, |w| w.prefetch_only());
        if !aside || self.upgrade(&f.track.id).is_some() {
            return false;
        }
        if let (Ok(mut live), Ok(mut upgrades)) = (self.live.lock(), self.upgrades.lock()) {
            live.retain(|_, other| !Arc::ptr_eq(other, f));
            upgrades.insert(f.track.id.clone(), f.clone());
            return true;
        }
        false
    }

    /// Start fast: fetches Soulseek's best copy (the saving profile's) in the background, for
    /// later plays and saves, unless one is running or a Soulseek copy is already cached.
    async fn upgrade_later(&self, track: Track) -> Result<()> {
        if self.upgrade(&track.id).is_some() {
            return Ok(());
        }
        let cached = self.cache.finished(&track.id, None).await?;
        if cached.is_some_and(|e| !youtube::is_youtube(&e.source)) {
            return Ok(());
        }
        let f = self.start(track, None, Purpose::Save, Want::default(), Slot::Upgrade);
        tracing::info!(track = %f.track.id, job = f.job_id, "fetching Soulseek's copy while YouTube plays");
        Ok(())
    }

    /// The upgrade is no longer wanted (YouTube didn't start the song after all, and the
    /// player's fetch looks for a Soulseek copy itself), unless a save is completing it.
    fn drop_upgrade(&self, track: &TrackId) {
        if let Some(f) = self.upgrade(track)
            && want(&f, |w| w.saving == 0)
        {
            self.cancel_fetch(&f);
        }
    }

    /// Warms a song's YouTube copy (the match and its extraction), so start fast starts it at once.
    pub fn warm(&self, track: Track) {
        let (youtube, max_kbps) = (self.youtube.clone(), self.rules.get().stream.max_kbps);
        tokio::spawn(async move {
            if let Err(e) = youtube.find(&track, YoutubeWant::Stream { max_kbps }).await {
                tracing::debug!(error = %e, track = %track.id, "warming a YouTube copy");
            }
        });
    }

    /// Preloads a song a device listed: a fetch that publishes no `stream` events and leaves
    /// once finished, so its file is evictable like any other. A live fetch is just marked; a
    /// running upgrade is the preload.
    pub fn prefetch(&self, track: Track) -> Arc<Fetch> {
        if let Some(f) = self.live(&track.id) {
            want(&f, |w| w.prefetch = true);
            return f;
        }
        if let Some(f) = self.upgrade(&track.id) {
            return f;
        }
        let listed = Want {
            prefetch: true,
            ..Want::default()
        };
        self.start(track, None, Purpose::Stream, listed, Slot::Live)
    }

    /// Whether the song has a live fetch (or an upgrade); a live one is marked as listed for
    /// preloading.
    pub fn list_live(&self, track: &TrackId) -> bool {
        self.live(track).inspect(|f| want(f, |w| w.prefetch = true)).is_some() || self.upgrade(track).is_some()
    }

    /// Preloads no device lists any more (not in `keep`) are cancelled after a grace period,
    /// unless by then the player streams them, a save needs them or a device lists them again.
    pub fn unlist(&self, keep: &std::collections::HashSet<TrackId>) {
        let dropped: Vec<Arc<Fetch>> = match self.live.lock() {
            Ok(live) => live
                .values()
                .filter(|f| !keep.contains(&f.track.id) && want(f, |w| std::mem::take(&mut w.prefetch)))
                .cloned()
                .collect(),
            Err(_) => return,
        };
        if dropped.is_empty() {
            return;
        }
        let this = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(UNLIST_GRACE).await;
            for f in dropped.iter().filter(|f| want(f, |w| w.unwanted())) {
                this.cancel_fetch(f);
            }
        });
    }

    /// Whether the player's song is still being found or waiting for its first bytes:
    /// preloads hold back until it plays.
    pub fn player_waiting(&self) -> bool {
        self.live.lock().is_ok_and(|l| {
            l.values().any(|f| {
                want(f, |w| w.streaming)
                    && matches!(f.now().status, Status::Finding | Status::Queued | Status::Fallback)
            })
        })
    }

    /// Transfers the song from `source` to the end and returns the finished file, joining a
    /// running fetch or reusing the cached file from that same source.
    pub async fn complete(&self, track: Track, source: SourceRef) -> Result<PathBuf> {
        let from_youtube = youtube::is_youtube(&source);
        // Not one waiting for the listener's choice: the save already named its copy. One still
        // finding may be joined, unless it is on YouTube while the save wants a peer's file.
        let joinable = |f: &Arc<Fetch>| {
            let p = f.now();
            p.status != Status::Choosing
                && (p.source.as_ref().is_some_and(|s| s.is(&source))
                    || (p.status == Status::Finding && f.on_youtube.load(Ordering::Relaxed) == from_youtube))
        };
        let running = self
            .live(&track.id)
            .filter(joinable)
            .or_else(|| self.upgrade(&track.id).filter(joinable));
        let f = match running {
            Some(f) => f,
            None => match self.cache.finished(&track.id, Some(&source)).await? {
                Some(e) if e.source == source => return Ok(e.path),
                _ => self.start_save(track.clone(), source.clone()),
            },
        };
        let (path, got) = self.wait_saving(&f).await?;
        // A fetch that was still finding may have ended on the other kind of copy: a save never
        // takes a YouTube copy it didn't ask for, so it fetches its own.
        if got.is_some_and(|g| youtube::is_youtube(&g) != from_youtube) {
            let own = self.start_save(track, source);
            return self.wait_saving(&own).await.map(|(path, _)| path);
        }
        Ok(path)
    }

    /// A save's own fetch: out of the player's way when the player is streaming something else.
    fn start_save(&self, track: Track, source: SourceRef) -> Arc<Fetch> {
        let slot = match self.live(&track.id) {
            Some(_) => Slot::Upgrade,
            None => Slot::Live,
        };
        self.start(track, Some(source), Purpose::Save, Want::default(), slot)
    }

    /// Holds the fetch for a save until it ends: its file and the copy it came from.
    async fn wait_saving(&self, f: &Arc<Fetch>) -> Result<(PathBuf, Option<SourceRef>)> {
        want(f, |w| w.saving += 1);
        let mut rx = f.progress();
        let ended = rx.wait_for(Progress::ended).await.map(|p| p.clone());
        want(f, |w| w.saving -= 1);
        let p = ended.map_err(|_| Error::internal("transfer ended without a result"))?;
        match (p.status, p.path) {
            (Status::Done, Some(path)) => Ok((path, p.source.map(|s| s.source_ref()))),
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

    /// The player moved on (skip, next): cancel unless a save still needs the song or a
    /// device lists it to preload. An upgrade runs on: it is for later plays and saves.
    pub fn stop_streaming(&self, track: &TrackId) {
        let Some(f) = self.live.lock().ok().and_then(|l| l.get(track).cloned()) else {
            return;
        };
        let unwanted = want(&f, |w| {
            w.streaming = false;
            w.unwanted()
        });
        if unwanted {
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
        Ok(self
            .start(track, None, Purpose::Stream, Want::default(), Slot::Live)
            .job_id
            .clone())
    }

    /// Activity rows: running transfers (upgrades too) first, then recently ended ones.
    pub fn rows(&self) -> Vec<serde_json::Value> {
        let mut rows: Vec<_> = self.all().iter().map(|f| job_row(f, &f.now())).collect();
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
        self.all()
            .iter()
            .any(|f| matches!(f.now().status, Status::Buffering | Status::Streaming | Status::Stalled))
    }

    /// The source the song is (or was last) fetched from, while its fetch is live.
    pub fn current_source(&self, track: &TrackId) -> Option<SourceRef> {
        self.live(track)?.now().source.map(|s| s.source_ref())
    }

    /// Lets go of finished songs nobody is reading, so the cache may evict them.
    pub fn sweep(&self) {
        for map in [&self.live, &self.upgrades] {
            let Ok(mut m) = map.lock() else { continue };
            m.retain(|_, f| {
                let p = f.now();
                let saving = f.want.lock().map(|w| w.saving).unwrap_or(0);
                !(p.ended() && saving == 0 && f.idle())
            });
        }
    }

    fn by_job(&self, job_id: &str) -> Option<Arc<Fetch>> {
        self.all().into_iter().find(|f| f.job_id == job_id)
    }

    fn cancel_fetch(&self, f: &Arc<Fetch>) {
        if f.now().ended() {
            self.forget(f);
        } else {
            f.stop.send_replace(true);
        }
    }

    fn new_fetch(&self, track: Track, purpose: Purpose, progress: Progress, wanted: Want, slot: Slot) -> Arc<Fetch> {
        let n = self.next.fetch_add(1, Ordering::Relaxed);
        let f = Arc::new(Fetch {
            job_id: format!("job-{:x}{n}", lull_core::now()),
            purpose,
            started: lull_core::now(),
            progress: watch::channel(progress).0,
            stop: watch::channel(false).0,
            want: Mutex::new(wanted),
            readers: AtomicUsize::new(0),
            last_read: Mutex::new(Instant::now()),
            on_youtube: AtomicBool::new(false),
            _pin: self.cache.pin(&track.id),
            track,
        });
        let map = match slot {
            Slot::Live => &self.live,
            Slot::Upgrade => &self.upgrades,
        };
        if let Ok(mut m) = map.lock() {
            m.insert(f.track.id.clone(), f.clone());
        }
        f
    }

    async fn cached(&self, track: Track, e: Entry) -> Result<Arc<Fetch>> {
        self.cache.touch(&track.id).await?;
        let source = self
            .sources
            .get(&track.id, &e.source)
            .or_else(|| bare_source(&e.source, track.duration));
        let on_youtube = youtube::is_youtube(&e.source);
        // YouTube copies are exempt from the minimum: they carry their own badge instead.
        let below = !on_youtube
            && source
                .as_ref()
                .is_some_and(|s| !s.meets(&self.rules.get().stream.minimum, track.duration));
        let f = self.new_fetch(
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
            Want::default(),
            Slot::Live,
        );
        f.on_youtube.store(on_youtube, Ordering::Relaxed);
        Ok(f)
    }

    fn start(&self, track: Track, pick: Option<SourceRef>, purpose: Purpose, wanted: Want, slot: Slot) -> Arc<Fetch> {
        let f = self.new_fetch(track, purpose, Progress::new(Status::Finding), wanted, slot);
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
        // So does a finished one nobody plays or saves (a preload, an upgrade): its file stays in
        // the cache, unpinned, and playing it later starts from there at once.
        let stopped = *f.stop.borrow();
        let idle = want(&f, |w| !w.streaming && w.saving == 0);
        if p.status != Status::Done || stopped || idle {
            self.forget(&f);
        }
    }

    /// Gets the song onto disk by the listener's mode: Soulseek alone, Soulseek then YouTube
    /// (`fallback`), or YouTube first with Soulseek's copy fetched beside it (`startFast`). Only
    /// the player's own fetches (and preloads) turn to YouTube; a save names its copy.
    async fn attempts(&self, f: &Fetch, pick: Option<SourceRef>) -> Result<PathBuf, Stop> {
        let mut stop = f.stop.subscribe();
        let prefetch_only = want(f, |w| w.prefetch_only());
        match route(f.purpose, pick.as_ref(), &self.rules.get().youtube, prefetch_only) {
            Route::Youtube(copy) => self.youtube_attempt(f, &copy, &mut stop).await,
            Route::Soulseek => self.soulseek(f, pick, stop).await,
            Route::SoulseekThenYoutube(wait) => self.soulseek_then_youtube(f, wait, true, &mut stop).await,
            Route::YoutubeFirst => match self.youtube_first(f, &mut stop).await {
                Err(Stop::Failed(why)) => {
                    tracing::info!(track = %f.track.id, why, "YouTube didn't start the song; looking on Soulseek");
                    f.on_youtube.store(false, Ordering::Relaxed);
                    self.drop_upgrade(&f.track.id);
                    self.soulseek_then_youtube(f, None, false, &mut stop).await
                }
                done => done,
            },
        }
    }

    /// Start fast: the YouTube copy plays now, and Soulseek's best copy is fetched beside it.
    async fn youtube_first(&self, f: &Fetch, stop: &mut watch::Receiver<bool>) -> Result<PathBuf, Stop> {
        let copy = self.youtube_copy(f, stop).await?;
        if let Err(e) = self.upgrade_later(f.track.clone()).await {
            tracing::warn!(error = %e, "starting the Soulseek upgrade");
        }
        self.youtube_attempt(f, &copy, stop).await
    }

    /// Soulseek, and YouTube when no peer delivers the song: none shares it, none passes the
    /// rules, every peer failed, or (`wait`) none started sending in time.
    async fn soulseek_then_youtube(
        &self,
        f: &Fetch,
        wait: Option<Duration>,
        youtube_after: bool,
        stop: &mut watch::Receiver<bool>,
    ) -> Result<PathBuf, Stop> {
        let (give_way, soulseek_stop) = watch::channel(false);
        let mut gave_way = false;
        let outcome = {
            let soulseek = self.soulseek(f, None, soulseek_stop);
            tokio::pin!(soulseek);
            let slow = too_slow(f, wait);
            tokio::pin!(slow);
            let mut stopping = false;
            loop {
                tokio::select! {
                    r = &mut soulseek => break r,
                    () = stopped(stop), if !stopping => {
                        stopping = true;
                        give_way.send_replace(true);
                    }
                    () = &mut slow, if !gave_way && !stopping => {
                        gave_way = true;
                        give_way.send_replace(true);
                    }
                }
            }
        };
        if *stop.borrow() {
            return Err(Stop::Cancelled);
        }
        let why = match outcome {
            Ok(path) => return Ok(path),
            Err(Stop::Cancelled) if !gave_way => return Err(Stop::Cancelled),
            Err(Stop::Cancelled) => format!(
                "no Soulseek peer started sending within {} s",
                wait.map_or(0, |w| w.as_secs())
            ),
            Err(Stop::Failed(why)) => why,
        };
        if !youtube_after {
            return Err(Stop::Failed(why));
        }
        tracing::info!(track = %f.track.id, why, "playing from YouTube instead");
        let tried = async {
            let copy = self.youtube_copy(f, stop).await?;
            self.youtube_attempt(f, &copy, stop).await
        };
        match tried.await {
            Err(Stop::Failed(yt)) => Err(Stop::Failed(format!("{why}; YouTube: {yt}"))),
            other => other,
        }
    }

    /// The song's YouTube copy, by the listener's format and the streaming cap ("finding").
    async fn youtube_copy(&self, f: &Fetch, stop: &mut watch::Receiver<bool>) -> Result<SourceRef, Stop> {
        f.on_youtube.store(true, Ordering::Relaxed);
        self.update(f, |p| p.status = Status::Finding);
        let want = YoutubeWant::Stream {
            max_kbps: self.rules.get().stream.max_kbps,
        };
        let found = tokio::select! {
            r = self.youtube.find(&f.track, want) => r,
            () = stopped(stop) => return Err(Stop::Cancelled),
        };
        match found {
            Ok(Some(found)) => Ok(found.source),
            Ok(None) => Err(Stop::Failed("no YouTube upload matches this song".into())),
            Err(e) => Err(Stop::Failed(reason(&e))),
        }
    }

    /// One YouTube download into the cache, watched like a transfer: the file grows front to
    /// back and readers serve it as it does. A finished copy in the cache is used as it is.
    async fn youtube_attempt(
        &self,
        f: &Fetch,
        copy_ref: &SourceRef,
        stop: &mut watch::Receiver<bool>,
    ) -> Result<PathBuf, Stop> {
        f.on_youtube.store(true, Ordering::Relaxed);
        let failed = |e: Error| Stop::Failed(reason(&e));
        let (Some(copy), Some(source)) = (youtube::Copy::of(copy_ref), bare_source(copy_ref, f.track.duration)) else {
            return Err(Stop::Failed(format!("{} is not a YouTube copy", copy_ref.path)));
        };
        let attempt = f.now().attempt + 1;
        let begin = |p: &mut Progress| {
            p.attempt = attempt;
            p.source = Some(source.clone());
            p.below = false;
            p.choices.clear();
            p.received = 0;
            p.speed = 0;
            p.queue_pos = None;
            p.path = None;
            p.error = None;
        };
        if let Some(e) = self.cache.finished(&f.track.id, Some(copy_ref)).await.map_err(failed)?
            && e.source == *copy_ref
        {
            self.update(f, |p| {
                begin(p);
                p.received = copy_ref.size;
                p.path = Some(e.path.clone());
                p.status = Status::Streaming;
            });
            return Ok(e.path);
        }
        self.cache.make_room(copy_ref.size).await.map_err(failed)?;
        let dest = self.cache.youtube_file(&copy).await.map_err(failed)?;
        self.update(f, |p| {
            begin(p);
            p.status = Status::Queued;
        });
        let record = |complete| Entry {
            track: f.track.id.clone(),
            source: copy_ref.clone(),
            path: dest.clone(),
            complete,
            used_at: lull_core::now(),
        };
        let download = self.youtube.download(copy_ref, &dest);
        tokio::pin!(download);
        let begun = Instant::now();
        let mut last = (0u64, Instant::now());
        let mut recorded = false;
        let stall = self.rules.get().stall_timeout();
        loop {
            tokio::select! {
                r = &mut download => {
                    r.map_err(|e| Stop::Failed(reason(&e)))?;
                    self.cache.record(record(true)).await.map_err(failed)?;
                    if let Err(e) = self.cache.report().await {
                        tracing::warn!(error = %e, "measuring the stream cache");
                    }
                    return Ok(dest.clone());
                }
                () = tokio::time::sleep(YOUTUBE_POLL) => {}
                // Dropping the download stops yt-dlp; a partial file stays recorded, so evictable.
                () = stopped(stop) => return Err(Stop::Cancelled),
            }
            let received = tokio::fs::metadata(&dest).await.map_or(0, |m| m.len());
            if received > 0 && !recorded {
                self.cache.record(record(false)).await.map_err(failed)?;
                recorded = true;
            }
            if received > last.0 {
                last = (received, Instant::now());
            }
            let quiet = last.1.elapsed();
            if quiet > stall && begun.elapsed() > YOUTUBE_START {
                return Err(Stop::Failed(format!(
                    "YouTube stopped sending at {}%",
                    received.saturating_mul(100) / copy_ref.size.max(1)
                )));
            }
            let speed = received * 1000 / (begun.elapsed().as_millis().max(1) as u64);
            self.update(f, |p| {
                p.received = received;
                p.speed = speed;
                p.path = (received > 0).then(|| dest.clone());
                p.status = match received {
                    0 => Status::Queued,
                    _ if quiet > STALL_NOTICE => Status::Stalled,
                    r if r < BUFFERED => Status::Buffering,
                    _ => Status::Streaming,
                };
            });
        }
    }

    /// Peers in the profile's order, falling back from one to the next.
    async fn soulseek(
        &self,
        f: &Fetch,
        pick: Option<SourceRef>,
        mut stop: watch::Receiver<bool>,
    ) -> Result<PathBuf, Stop> {
        let mut tried: Vec<SourceRef> = Vec::new();
        for _ in 0..MAX_ATTEMPTS {
            let next = self.next_source(f, pick.as_ref(), &tried, &mut stop).await?;
            tried.push(next.source_ref());
            // Counted across sources (YouTube included): a reader of an older attempt has the
            // wrong file.
            let attempt = f.now().attempt + 1;
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
                .or_else(|| bare_source(pick, f.track.duration))
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
            let prefetch_only = want(f, |w| w.prefetch_only());
            match below(f.purpose, &rules, prefetch_only) {
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
                Below::Later => {
                    return Err(Stop::Failed(format!(
                        "no copy meets your streaming minimum ({}); it is looked for again when it plays",
                        rules.stream.minimum
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
                    let why = t
                        .exception
                        .as_deref()
                        .map_or_else(|| t.state.clone(), crate::slskd::plain);
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

/// Where a fetch gets the song, by the listener's YouTube mode (docs/context/youtube.md).
#[derive(Debug, PartialEq, Eq)]
enum Route {
    /// The copy named is a YouTube one (a save's, or the listener's pick).
    Youtube(SourceRef),
    /// Soulseek peers only: YouTube is off, a save named a peer's copy, or the listener picked one.
    Soulseek,
    /// Soulseek, then YouTube when no peer delivers; with a wait, also when none starts in time.
    SoulseekThenYoutube(Option<Duration>),
    /// Start fast: YouTube now, Soulseek's best copy fetched beside it.
    YoutubeFirst,
}

fn route(purpose: Purpose, pick: Option<&SourceRef>, yt: &youtube::Settings, prefetch_only: bool) -> Route {
    if let Some(copy) = pick.filter(|p| youtube::is_youtube(p)) {
        return Route::Youtube(copy.clone());
    }
    if purpose != Purpose::Stream || pick.is_some() {
        return Route::Soulseek;
    }
    match yt.mode {
        Mode::Off => Route::Soulseek,
        Mode::Fallback => Route::SoulseekThenYoutube(Some(Duration::from_secs(u64::from(yt.wait_s)))),
        // A preload gets Soulseek's copy, YouTube's only when Soulseek has none; the song's
        // YouTube match is warmed meanwhile, so playing it still starts at once.
        Mode::StartFast if prefetch_only => Route::SoulseekThenYoutube(None),
        Mode::StartFast => Route::YoutubeFirst,
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
    /// A preload under `keepLooking` or `ask`: don't search on or wait for an answer nobody is
    /// there to give; the player's own fetch applies the rule when the song plays.
    Later,
}

fn below(purpose: Purpose, rules: &Rules, prefetch_only: bool) -> Below {
    if prefetch_only && purpose == Purpose::Stream {
        return match rules.stream.below {
            StreamBelow::PlayBest => Below::Take,
            StreamBelow::KeepLooking | StreamBelow::Ask => Below::Later,
        };
    }
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
pub(crate) fn reason(e: &Error) -> String {
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

/// A source known only by reference (picked before a restart, named by a save, a YouTube copy):
/// its facts come from the file name alone, a YouTube copy's from its format and the song's length.
fn bare_source(r: &SourceRef, duration: Option<u32>) -> Option<Source> {
    if let Some(copy) = youtube::Copy::of(r) {
        return Some(Source {
            user: r.user.clone(),
            path: r.path.clone(),
            size: r.size,
            facts: copy.facts(r.size, duration),
            ext: copy.ext,
            upload_speed: 0,
            free_slot: true,
            queue: 0,
            length: duration,
        });
    }
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

/// `fallback`: resolves once `wait` has passed and no peer is sending (still finding, queued, or
/// between peers). Never without a `wait`.
async fn too_slow(f: &Fetch, wait: Option<Duration>) {
    let Some(wait) = wait else {
        return std::future::pending().await;
    };
    tokio::time::sleep(wait).await;
    let mut progress = f.progress();
    let _ = progress
        .wait_for(|p| matches!(p.status, Status::Finding | Status::Queued | Status::Fallback))
        .await;
}

/// Where a fetch is a running fetch in `map`, not one that failed or was cancelled.
fn running(map: &Mutex<HashMap<TrackId, Arc<Fetch>>>, track: &TrackId) -> Option<Arc<Fetch>> {
    let m = map.lock().ok()?;
    m.get(track)
        .filter(|f| !matches!(f.now().status, Status::Failed | Status::Cancelled))
        .cloned()
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
        "youtube": p.source.as_ref().is_some_and(|s| youtube::is_youtube(&s.source_ref())),
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

    #[test]
    fn the_youtube_mode_decides_where_the_players_fetch_looks_and_saves_keep_their_copy() {
        let peer = SourceRef {
            user: "peer".into(),
            path: "Music\\A\\01 - Song.flac".into(),
            size: 1,
        };
        let yt_copy = youtube::Copy {
            video: "LUjGtyYEi90".into(),
            format: "251".into(),
            ext: "webm".into(),
        }
        .source_ref(5_730_310);
        let mut yt = youtube::Settings::default();
        let player = |yt: &youtube::Settings, preload: bool| route(Purpose::Stream, None, yt, preload);

        assert_eq!(player(&yt, false), Route::Soulseek, "off by default");
        yt.mode = Mode::Fallback;
        yt.wait_s = 25;
        assert_eq!(
            player(&yt, false),
            Route::SoulseekThenYoutube(Some(Duration::from_secs(25)))
        );
        assert_eq!(
            player(&yt, true),
            Route::SoulseekThenYoutube(Some(Duration::from_secs(25)))
        );
        yt.mode = Mode::StartFast;
        assert_eq!(player(&yt, false), Route::YoutubeFirst);
        assert_eq!(
            player(&yt, true),
            Route::SoulseekThenYoutube(None),
            "a preload fetches Soulseek's copy; YouTube only when there is none"
        );
        assert_eq!(
            route(Purpose::Save, None, &yt, false),
            Route::Soulseek,
            "a save never turns to YouTube by itself"
        );
        assert_eq!(
            route(Purpose::Stream, Some(&peer), &yt, false),
            Route::Soulseek,
            "the listener's pick"
        );
        assert_eq!(
            route(Purpose::Save, Some(&yt_copy), &youtube::Settings::default(), false),
            Route::Youtube(yt_copy.clone()),
            "a save that named a YouTube copy gets it"
        );
    }

    #[test]
    fn a_youtube_copy_carries_its_own_badge_and_plays_its_native_type() {
        let r = youtube::Copy {
            video: "LUjGtyYEi90".into(),
            format: "140".into(),
            ext: "m4a".into(),
        }
        .source_ref(5_152_490);
        let s = bare_source(&r, Some(318));
        assert_eq!(s.as_ref().map(|s| s.facts.format.as_str()), Some("YouTube · AAC 128"));
        assert_eq!(s.as_ref().map(|s| s.ext.as_str()), Some("m4a"));
        assert_eq!(s.map(|s| s.user), Some(youtube::PEER.to_owned()));
    }

    #[test]
    fn a_preload_never_waits_for_a_choice_or_keeps_searching() {
        let mut rules = Rules::default();
        rules.stream.below = StreamBelow::PlayBest;
        assert!(matches!(below(Purpose::Stream, &rules, true), Below::Take));
        rules.stream.below = StreamBelow::KeepLooking;
        assert!(matches!(below(Purpose::Stream, &rules, true), Below::Later));
        rules.stream.below = StreamBelow::Ask;
        assert!(matches!(below(Purpose::Stream, &rules, true), Below::Later));
        assert!(
            matches!(below(Purpose::Stream, &rules, false), Below::Ask),
            "once the player streams it, the listener is asked"
        );
    }
}
