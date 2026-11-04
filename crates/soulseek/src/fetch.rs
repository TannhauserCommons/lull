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
//!
//! Better copies: while the player streams a lesser copy (YouTube's, one below the minimum, a peer
//! fallen back to), an upgrade fetch gets the streaming profile's best one beside it; the player
//! switches to it seamlessly once enough has arrived (docs/context/streaming.md#better-copies).

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
use tokio::sync::{Notify, watch};

use crate::cache::{Cache, Entry, Pin};
use crate::files;
use crate::layout::Layout;
use lull_core::quality::{SaveBelow, StreamBelow};

use crate::budget::Lane;
use crate::peers::Peers;
use crate::rules::{Current, Purpose, Rules};
use crate::search::Searches;
use crate::slskd::{Phase, Slskd, Transfer};
use crate::sources::{self, Bar, Lesser, Ranked, Source, Sources};

/// Received bytes after which a stream counts as playing rather than buffering.
const BUFFERED: u64 = 256 * 1024;
const POLL: Duration = Duration::from_secs(1);
/// A source's live rate is judged once it has sent this long: slskd's average (Soulseek.NET's,
/// updated once a second with α 0.2) still holds 11 % of its first second's rate after ten.
const SLOW_AFTER: Duration = Duration::from_secs(10);
/// A source sending under this many times what the song needs gives way: the song plays at 1x,
/// and the margin covers the average's wobble and what the player reads ahead.
const SLOWER_THAN: f64 = 1.25;
/// ...to a copy expected at least this many times as fast: worth a new request's start-up and the
/// listener's rebuffer, and well clear of how far an expectation can be off.
const FASTER_BY: f64 = 2.0;
/// A transfer's rate is remembered once at least this much came (or it sent for `SLOW_AFTER`):
/// before, the connection's start-up is most of it.
const MEASURE_MIN: u64 = 1_000_000;
/// slskd reports a download done just before it moves the file and says where: this long at most.
const LAND_WAIT: Duration = Duration::from_secs(60);
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
/// A better copy arriving at least this many times faster than it plays can be switched to
/// before all of it is here: from where the song is, it stays ahead.
const OUTRUN: f64 = 1.5;
/// The copy the player switched away from stops once its response ends, or after this.
const RETIRE_WAIT: Duration = Duration::from_secs(60);

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
    /// The listener picked this copy: it is never switched away from by itself.
    picked: AtomicBool,
    /// An upgrade's bar: it fetches only a copy better than the one the player has.
    bar: Option<Bar>,
    /// Woken when the last response reading it ends.
    unread: Notify,
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
        let was = self.0.readers.fetch_sub(1, Ordering::Relaxed);
        if let Ok(mut t) = self.0.last_read.lock() {
            *t = Instant::now();
        }
        if was == 1 {
            self.0.unread.notify_waiters();
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
    /// The peer sends slower than the song plays, and another is expected to be much faster: its
    /// measured rate demotes it, no stall is recorded (ADR-0015).
    Slow(String),
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
    /// Better copies fetched beside the one the player has (start fast's Soulseek copy, the
    /// streaming profile's best beside a lesser copy), and saves started while YouTube plays the
    /// song: out of the player's way, so its song never jumps until the player switches.
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
        // The listener picked the better copy already on its way: that fetch plays.
        if let Some(copy) = &pick
            && self.upgrade_of(&track.id, copy).is_some()
        {
            let f = self.adopt(&track.id, copy, true)?;
            self.cache.touch(&track.id).await?;
            return Ok(f);
        }
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
                    want(&f, |w| w.streaming = true);
                    self.cache.touch(&track.id).await?;
                    // Every opening tells the player where the song stands now: a preload or save
                    // it joins, and equally a song it streamed before (a replay, a reload). A
                    // finished transfer sends nothing more, so without this the player only
                    // knows what it buffered itself and refuses to seek past it.
                    self.bus.publish("stream", self.player_event(&f, &f.now()));
                    self.better_later(&f);
                    return Ok(f);
                }
            } else {
                self.replace(&f);
            }
        }
        let cached = match &pick {
            Some(copy) => self.cache.finished(&track.id, copy).await?,
            None => self.best_cached(&track, Purpose::Stream).await?,
        }
        .filter(|e| mode != Mode::Off || !youtube::is_youtube(&e.source));
        let f = match cached {
            // The bytes are here, so the profile's `below` rule doesn't hold them back; the
            // player still learns which copy it is, and whether it's below the minimum.
            Some(entry) => {
                let on_youtube = youtube::is_youtube(&entry.source);
                let f = self.cached(track.clone(), entry).await?;
                f.picked.store(pick.is_some(), Ordering::Relaxed);
                want(&f, |w| w.streaming = true);
                self.bus.publish("stream", self.player_event(&f, &f.now()));
                if on_youtube && mode == Mode::StartFast {
                    self.upgrade_later(track).await?;
                }
                self.better_later(&f);
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
        let cached = self.best_cached(&f.track, Purpose::Stream).await?;
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
        let cached = self.best_cached(&track, Purpose::Save).await?;
        if cached.is_some_and(|e| !youtube::is_youtube(&e.source)) {
            return Ok(());
        }
        if let Some(f) = self.start_upgrade(track, Purpose::Save, None) {
            tracing::info!(track = %f.track.id, job = f.job_id, "fetching Soulseek's copy while YouTube plays");
        }
        Ok(())
    }

    /// The player streams `f`: when its copy is a lesser one, the streaming profile's best copy
    /// is fetched beside it, out of its way, so the player can switch to it seamlessly (and
    /// later plays and saves have it). Not when the listener picked the copy playing, and only
    /// when a better copy is known, or nothing is known yet (its own search may find one).
    fn better_later(&self, f: &Fetch) {
        let rules = self.rules.get();
        let p = f.now();
        let (Some(playing), Some(why)) = (p.source.as_ref(), lesser(&p)) else {
            return;
        };
        if !rules.switch_to_better
            || f.purpose != Purpose::Stream
            || f.picked.load(Ordering::Relaxed)
            || !want(f, |w| w.streaming)
        {
            return;
        }
        let bar = Bar {
            playing: playing.source_ref(),
            why,
            avoid: p.failed_peers.iter().map(|failed| failed.peer.clone()).collect(),
        };
        let known = self
            .sources
            .ordered(&f.track.id, f.track.duration, &rules, Purpose::Stream);
        if !known.is_empty() && sources::better(known, &bar).is_empty() {
            return;
        }
        if let Some(u) = self.start_upgrade(f.track.clone(), Purpose::Stream, Some(bar)) {
            tracing::info!(track = %u.track.id, job = u.job_id, ?why, "fetching a better copy beside the one playing");
        }
    }

    /// Whether a preload should fetch the song although a copy is cached: that copy is a lesser
    /// one and a better one is known (with switching to better copies on).
    pub fn better_than_cached(&self, track: &Track, cached: &SourceRef) -> bool {
        let rules = self.rules.get();
        let below = || {
            self.sources
                .get(&track.id, cached)
                .or_else(|| bare_source(cached, track.duration))
                .is_some_and(|s| !s.meets(&rules.stream.minimum, track.duration))
        };
        let why = if youtube::is_youtube(cached) {
            Lesser::Youtube
        } else if below() {
            Lesser::Below
        } else {
            return false;
        };
        let bar = Bar {
            playing: cached.clone(),
            why,
            avoid: Vec::new(),
        };
        let known = self.sources.ordered(&track.id, track.duration, &rules, Purpose::Stream);
        rules.switch_to_better && !sources::better(known, &bar).is_empty()
    }

    /// The better copy on its way for the song, when it is `copy`.
    fn upgrade_of(&self, track: &TrackId, copy: &SourceRef) -> Option<Arc<Fetch>> {
        self.upgrade(track).filter(|u| holds(u, copy))
    }

    /// The fetch holding `copy` of the song, live or on its way beside it: what
    /// `GET /api/stream/{id}/copy` serves, changing nothing.
    pub fn copy(&self, track: &TrackId, copy: &SourceRef) -> Result<Arc<Fetch>> {
        self.live(track)
            .filter(|f| holds(f, copy))
            .or_else(|| self.upgrade_of(track, copy))
            .ok_or_else(|| Error::NotFound("that copy isn't playing or on its way; search again".into()))
    }

    /// The player switched to the better copy on its way (seamlessly, or the listener picked
    /// it): that fetch becomes the song's live one, and the copy it replaces stops once the
    /// response still playing it has ended.
    pub fn adopt(&self, track: &TrackId, copy: &SourceRef, picked: bool) -> Result<Arc<Fetch>> {
        let u = self
            .upgrade_of(track, copy)
            .ok_or_else(|| Error::Conflict("that copy is no longer on its way".into()))?;
        let old = {
            let (Ok(mut live), Ok(mut upgrades)) = (self.live.lock(), self.upgrades.lock()) else {
                return Err(Error::internal("the fetch maps are poisoned"));
            };
            upgrades.retain(|_, other| !Arc::ptr_eq(other, &u));
            live.insert(track.clone(), u.clone())
        };
        u.picked.store(picked, Ordering::Relaxed);
        want(&u, |w| w.streaming = true);
        if let Some(old) = old.filter(|o| !Arc::ptr_eq(o, &u)) {
            self.retire(old);
        }
        tracing::info!(track = %track, job = u.job_id, picked, "the player switched to the better copy");
        self.bus.publish("stream", self.player_event(&u, &u.now()));
        Ok(u)
    }

    /// The copy the player switched away from: a save completing it carries on out of the
    /// player's way; otherwise it stops once nothing reads it (the player lets go of the
    /// response that played it right after the switch).
    fn retire(&self, old: Arc<Fetch>) {
        let saving = want(&old, |w| {
            w.streaming = false;
            w.saving
        });
        if saving > 0 {
            if let Ok(mut upgrades) = self.upgrades.lock() {
                upgrades.entry(old.track.id.clone()).or_insert(old);
            }
            return;
        }
        let this = self.clone();
        tokio::spawn(async move {
            let unread = async {
                loop {
                    // Created before looking: a response ending in between still wakes it.
                    let woken = old.unread.notified();
                    if old.readers.load(Ordering::Relaxed) == 0 {
                        return;
                    }
                    woken.await;
                }
            };
            let _ = tokio::time::timeout(RETIRE_WAIT, unread).await;
            if want(&old, |w| w.unwanted()) {
                this.cancel_fetch(&old);
            }
        });
    }

    /// The song's live fetch, while the player streams it and `f` is the better copy on its
    /// way beside it.
    fn played_beside(&self, f: &Fetch) -> Option<Arc<Fetch>> {
        let beside = self
            .upgrades
            .lock()
            .ok()?
            .get(&f.track.id)
            .is_some_and(|u| std::ptr::eq(Arc::as_ptr(u), f));
        let live = self.live.lock().ok()?.get(&f.track.id).cloned()?;
        (beside && want(&live, |w| w.streaming)).then_some(live)
    }

    /// The `stream` event for the song the player streams, with the better copy on its way
    /// when the player may switch to it.
    fn player_event(&self, f: &Fetch, p: &Progress) -> serde_json::Value {
        let mut event = stream_event(f, p);
        if let Some(offer) = self.offer(f, p) {
            event["upgrade"] = offer;
        }
        event
    }

    /// The better copy the player may switch to mid-song: switching is on, the listener didn't
    /// pick the copy playing, which is a lesser one, and the copy on its way passes the
    /// streaming minimum (start fast's, fetched by the saving profile, also within the cap).
    fn offer(&self, f: &Fetch, p: &Progress) -> Option<serde_json::Value> {
        let rules = self.rules.get();
        if !rules.switch_to_better || f.picked.load(Ordering::Relaxed) || lesser(p).is_none() {
            return None;
        }
        let u = self.upgrade(&f.track.id)?;
        let up = u.now();
        let duration = f.track.duration;
        let s = up.source.as_ref().filter(|s| {
            s.meets(&rules.stream.minimum, duration)
                && (u.purpose == Purpose::Stream || s.quality(duration).within(rules.stream.max_kbps))
        })?;
        Some(json!({
            "peer": s.user,
            "path": s.path,
            "format": s.facts.format,
            "status": stream_status(up.status),
            "received": up.received,
            "size": up.size(),
            "ready": upgrade_ready(&up, duration),
        }))
    }

    /// The song's best finished copy in the cache by `purpose`'s profile (none of YouTube's
    /// while YouTube is off): what a later play or a save takes.
    pub async fn best_cached(&self, track: &Track, purpose: Purpose) -> Result<Option<Entry>> {
        let rules = self.rules.get();
        let mut entries = self.cache.complete(&track.id).await?;
        entries.retain(|e| rules.youtube.mode != Mode::Off || !youtube::is_youtube(&e.source));
        let ordered = self.sources.ordered(&track.id, track.duration, &rules, purpose);
        let rank = |e: &Entry| {
            let meets = self
                .sources
                .get(&track.id, &e.source)
                .or_else(|| bare_source(&e.source, track.duration))
                .is_some_and(|s| s.meets(rules.minimum(purpose), track.duration));
            let at = ordered.iter().position(|r| r.source.is(&e.source));
            (!meets, at.unwrap_or(usize::MAX))
        };
        Ok(best_first(entries, rank).into_iter().next())
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
            None => match self.cache.finished(&track.id, &source).await? {
                Some(e) => return Ok(e.path),
                None => self.start_save(track.clone(), source.clone()),
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
    /// device lists it to preload. Start fast's upgrade runs on (it is for later plays and
    /// saves); a better copy fetched for switching mid-song stops unless a save needs it.
    pub fn stop_streaming(&self, track: &TrackId) {
        if let Some(u) = self.upgrade(track)
            && u.bar.is_some()
            && want(&u, |w| w.unwanted())
        {
            self.cancel_fetch(&u);
        }
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

    /// The copies of the song playing or on their way, the better one first: a save that can
    /// take one of them needs no second transfer.
    pub fn copies_here(&self, track: &TrackId) -> Vec<SourceRef> {
        [self.upgrade(track), self.live(track)]
            .into_iter()
            .flatten()
            .filter_map(|f| f.now().source.map(|s| s.source_ref()))
            .collect()
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
        let f = self.fetch(track, purpose, progress, wanted, None);
        let map = match slot {
            Slot::Live => &self.live,
            Slot::Upgrade => &self.upgrades,
        };
        if let Ok(mut m) = map.lock() {
            m.insert(f.track.id.clone(), f.clone());
        }
        f
    }

    /// A fetch in no map yet.
    fn fetch(&self, track: Track, purpose: Purpose, progress: Progress, wanted: Want, bar: Option<Bar>) -> Arc<Fetch> {
        let n = self.next.fetch_add(1, Ordering::Relaxed);
        Arc::new(Fetch {
            job_id: format!("job-{:x}{n}", lull_core::now()),
            purpose,
            started: lull_core::now(),
            progress: watch::channel(progress).0,
            stop: watch::channel(false).0,
            want: Mutex::new(wanted),
            readers: AtomicUsize::new(0),
            last_read: Mutex::new(Instant::now()),
            on_youtube: AtomicBool::new(false),
            picked: AtomicBool::new(false),
            bar,
            unread: Notify::new(),
            _pin: self.cache.pin(&track.id),
            track,
        })
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
        f.picked
            .store(pick.is_some() && purpose == Purpose::Stream, Ordering::Relaxed);
        self.spawn(f.clone(), pick);
        f
    }

    /// A better copy fetched beside the song's live fetch, unless one is on its way already.
    fn start_upgrade(&self, track: Track, purpose: Purpose, bar: Option<Bar>) -> Option<Arc<Fetch>> {
        let f = self.fetch(track, purpose, Progress::new(Status::Finding), Want::default(), bar);
        {
            let mut upgrades = self.upgrades.lock().ok()?;
            let running = upgrades
                .get(&f.track.id)
                .is_some_and(|u| !matches!(u.now().status, Status::Failed | Status::Cancelled));
            if running {
                return None;
            }
            upgrades.insert(f.track.id.clone(), f.clone());
        }
        self.spawn(f.clone(), None);
        Some(f)
    }

    fn spawn(&self, f: Arc<Fetch>, pick: Option<SourceRef>) {
        let this = self.clone();
        tokio::spawn(async move { this.run(f, pick).await });
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
        // A better copy beside the song the player streams stays, so the player can switch to it.
        let stopped = *f.stop.borrow();
        let idle = want(&f, |w| !w.streaming && w.saving == 0) && self.played_beside(&f).is_none();
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
        match route(
            f.purpose,
            pick.as_ref(),
            &self.rules.get().youtube,
            prefetch_only,
            f.bar.is_some(),
        ) {
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
        if let Some(e) = self.cache.finished(&f.track.id, copy_ref).await.map_err(failed)? {
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
        // What a cancelled download left is never served as this one's first bytes.
        // (Should it be open elsewhere, the download truncates it as it starts.)
        if let Err(e) = tokio::fs::remove_file(&dest).await
            && e.kind() != std::io::ErrorKind::NotFound
        {
            tracing::debug!(error = %e, path = %dest.display(), "removing an old partial YouTube file");
        }
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
            // A copy named for this fetch (the listener's pick, a save's) is kept however slow;
            // otherwise the fastest copy left is what a slow one could give way to.
            let faster = match pick {
                Some(_) => 0.0,
                None => {
                    let (left, _) = self.untried(f, &tried, &self.rules.get());
                    left.iter().map(|r| r.expected).fold(0.0, f64::max)
                }
            };
            match self.attempt(f, attempt, &next, faster, &mut stop).await {
                Ok(path) => return Ok(path),
                Err(Ok(skip)) => {
                    let (reason, stalled) = match skip {
                        Skip::Stalled(r) => (r, true),
                        Skip::Unavailable(r) | Skip::Slow(r) => (r, false),
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
        if let Some(bar) = &f.bar {
            return self.better_source(f, bar, tried, stop).await;
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

    /// An upgrade's next copy: the best untried one better than the copy playing. With none
    /// left, it searches once more, behind the player's and preloads' searches, then gives up;
    /// the profile's `below` rule never applies, since a copy below it is no upgrade.
    async fn better_source(
        &self,
        f: &Fetch,
        bar: &Bar,
        tried: &[SourceRef],
        stop: &mut watch::Receiver<bool>,
    ) -> Result<Source, Stop> {
        let rules = self.rules.get();
        let mut searched = false;
        loop {
            let (copies, _) = self.untried(f, tried, &rules);
            if let Some(best) = sources::better(copies, bar).into_iter().next() {
                return Ok(best.source);
            }
            if searched {
                return Err(Stop::Failed("no better copy than the one playing turned up".into()));
            }
            self.update(f, |p| p.status = Status::Finding);
            tokio::select! {
                r = self.searches.resolve_again(&f.track, Lane::Prefetch) => r.map_err(|e| Stop::Failed(reason(&e)))?,
                () = stopped(stop) => return Err(Stop::Cancelled),
            }
            searched = true;
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

    /// One transfer from one source, watched until it finishes, stalls, proves slower than the
    /// song when a copy expected at `faster` bytes a second is left, or is stopped. However it
    /// ends, the rate it sent at is remembered against the peer (ADR-0015).
    async fn attempt(
        &self,
        f: &Fetch,
        attempt: u32,
        source: &Source,
        faster: f64,
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
        // When the peer started sending, and what was on disk then (a resumed partial).
        let mut sending: Option<(Instant, u64)> = None;
        // What the song needs, bytes a second: slower, and it outruns its own download.
        let needs = f
            .track
            .duration
            .or(source.length)
            .filter(|&d| d > 0)
            .map_or(0.0, |d| source.size as f64 / f64::from(d));
        let mut last: Option<Transfer> = None;
        loop {
            tokio::select! {
                _ = tokio::time::sleep(POLL) => {}
                _ = stopped(stop) => {
                    // It may have finished since the last poll: keep track of a finished file
                    // (cached, evictable) rather than cancel what's already done.
                    if let Ok(t) = self.slskd.download(&source.user, &transfer.id).await
                        && t.phase() == Phase::Succeeded
                    {
                        self.measure(source, &t, sending, t.bytes_transferred).await;
                        return self.finish(f, source, &t).await.map_err(|e| Err(Stop::Failed(reason(&e))));
                    }
                    if let Some(t) = &last {
                        self.measure(source, t, sending, last_bytes.0).await;
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
            if running && sending.is_none() {
                sending = Some((Instant::now(), received));
            }
            was_running = running;
            let quiet = last_bytes.1.elapsed();
            last = Some(t.clone());
            let rate = t.average_speed.max(0.0);
            if running
                && sending.is_some_and(|(since, _)| since.elapsed() >= SLOW_AFTER)
                && rate < needs * SLOWER_THAN
                && faster >= rate * FASTER_BY
            {
                self.measure(source, &t, sending, received).await;
                self.give_up(source, &t).await;
                return Err(Ok(Skip::Slow(format!(
                    "{} sent {} KB/s, slower than the song plays",
                    source.user,
                    (rate / 1000.0).round()
                ))));
            }
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
                    self.measure(source, &t, sending, received).await;
                    self.give_up(source, &t).await;
                    let pct = received.saturating_mul(100) / source.size.max(1);
                    return Err(Ok(Skip::Stalled(format!("{} stalled at {pct}%", source.user))));
                }
                Phase::Failed => {
                    self.measure(source, &t, sending, received).await;
                    let why = t
                        .exception
                        .as_deref()
                        .map_or_else(|| t.state.clone(), crate::slskd::plain);
                    return Err(Ok(Skip::Stalled(format!("{}: {why}", source.user))));
                }
                Phase::Succeeded => {
                    self.measure(source, &t, sending, t.bytes_transferred.max(received))
                        .await;
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

    /// Remembers how fast the peer sent this transfer: slskd's average over the bytes that flowed
    /// (queue time excluded), once enough came to judge it: `MEASURE_MIN`, or `SLOW_AFTER` of
    /// sending, so a slow peer given up on is remembered too.
    async fn measure(&self, source: &Source, t: &Transfer, sending: Option<(Instant, u64)>, received: u64) {
        let Some((since, from)) = sending else { return };
        let judged = received.saturating_sub(from) >= MEASURE_MIN || since.elapsed() >= SLOW_AFTER;
        if !judged || t.average_speed <= 0.0 {
            return;
        }
        if let Err(e) = self
            .peers
            .sent(&source.user, t.average_speed, source.upload_speed)
            .await
        {
            tracing::warn!(error = %e, "remembering a peer's speed");
        }
    }

    async fn give_up(&self, source: &Source, t: &Transfer) {
        if let Err(e) = self.slskd.cancel(&source.user, &t.id).await {
            tracing::warn!(error = %e, "cancelling a stalled transfer in slskd");
        }
    }

    /// Where slskd put a finished download. slskd marks a transfer Succeeded before it moves the
    /// file to its downloads folder and says where (its `DownloadFileComplete` event, slskd 0.26's
    /// `DownloadService`), so a look in between finds nothing yet: lull asks again each `POLL`, up
    /// to `LAND_WAIT`, while a reader keeps reading the file it has open (on Linux the handle
    /// follows the file through the move; one that has to reopen waits for the new path).
    async fn finished_path(&self, t: &Transfer) -> Result<PathBuf> {
        let since = Instant::now();
        let reported = loop {
            if let Some(reported) = self.slskd.finished_path(&t.id).await? {
                break reported;
            }
            if since.elapsed() >= LAND_WAIT {
                return Err(Error::upstream(
                    "soulseek",
                    format!("slskd finished {} but did not say where it put it", t.filename),
                ));
            }
            tokio::time::sleep(POLL).await;
        };
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
            self.bus.publish("stream", self.player_event(f, &p));
            if p.attempt != before.attempt {
                self.better_later(f);
            }
        } else if let Some(live) = self.played_beside(f) {
            // The better copy moved: the player hears how far it is, and when it's ready.
            self.bus.publish("stream", self.player_event(&live, &live.now()));
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
    /// Soulseek peers only: YouTube is off, a save named a peer's copy, the listener picked one,
    /// or it fetches a better copy.
    Soulseek,
    /// Soulseek, then YouTube when no peer delivers; with a wait, also when none starts in time.
    SoulseekThenYoutube(Option<Duration>),
    /// Start fast: YouTube now, Soulseek's best copy fetched beside it.
    YoutubeFirst,
}

fn route(
    purpose: Purpose,
    pick: Option<&SourceRef>,
    yt: &youtube::Settings,
    prefetch_only: bool,
    upgrade: bool,
) -> Route {
    if let Some(copy) = pick.filter(|p| youtube::is_youtube(p)) {
        return Route::Youtube(copy.clone());
    }
    // A better copy is a Soulseek one: it replaces YouTube's, or a lesser peer's.
    if purpose != Purpose::Stream || pick.is_some() || upgrade {
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

/// Why the copy a fetch has is a lesser one: YouTube's, below the streaming minimum, or a peer
/// fallen back to. `None` while it has none, or has the profile's first choice.
fn lesser(p: &Progress) -> Option<Lesser> {
    let source = p.source.as_ref()?;
    if youtube::is_youtube(&source.source_ref()) {
        Some(Lesser::Youtube)
    } else if p.below {
        Some(Lesser::Below)
    } else if !p.failed_peers.is_empty() {
        Some(Lesser::FellBack)
    } else {
        None
    }
}

/// Whether the player can switch to a better copy: all of it is here, or it arrives at least
/// `OUTRUN` times faster than it plays, so from wherever the song is it stays ahead. The player
/// still checks that what arrived covers where the song is.
fn upgrade_ready(p: &Progress, duration: Option<u32>) -> bool {
    match p.status {
        Status::Done => true,
        Status::Streaming => duration
            .filter(|d| *d > 0)
            .is_some_and(|d| p.speed as f64 * f64::from(d) >= OUTRUN * p.size() as f64),
        _ => false,
    }
}

/// Whether the fetch has (or had) `copy`.
fn holds(f: &Fetch, copy: &SourceRef) -> bool {
    f.now().source.is_some_and(|s| s.is(copy))
}

/// Cached copies, best first: Soulseek's before YouTube's, then by `rank` (whether a copy fails
/// the profile's minimum, then its place in the profile's order), then the most recently played.
fn best_first(mut entries: Vec<Entry>, rank: impl Fn(&Entry) -> (bool, usize)) -> Vec<Entry> {
    entries.sort_by_cached_key(|e| (youtube::is_youtube(&e.source), rank(e), -e.used_at));
    entries
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
        "speed": p.speed,
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
        let player = |yt: &youtube::Settings, preload: bool| route(Purpose::Stream, None, yt, preload, false);

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
            route(Purpose::Stream, None, &yt, false, true),
            Route::Soulseek,
            "a better copy is a Soulseek one, whatever the mode"
        );
        assert_eq!(
            route(Purpose::Save, None, &yt, false, false),
            Route::Soulseek,
            "a save never turns to YouTube by itself"
        );
        assert_eq!(
            route(Purpose::Stream, Some(&peer), &yt, false, false),
            Route::Soulseek,
            "the listener's pick"
        );
        assert_eq!(
            route(
                Purpose::Save,
                Some(&yt_copy),
                &youtube::Settings::default(),
                false,
                false
            ),
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

    fn peer_copy(user: &str, ext: &str, size: u64) -> Source {
        copy_of(
            &SourceRef {
                user: user.into(),
                path: format!("Music\\A\\01 - Song.{ext}"),
                size,
            },
            ext,
        )
    }

    /// A copy as a search reports it; its bitrate comes from its size and the song's length.
    fn copy_of(r: &SourceRef, ext: &str) -> Source {
        let lossless = ext == "flac";
        Source {
            user: r.user.clone(),
            path: r.path.clone(),
            size: r.size,
            ext: ext.into(),
            facts: lull_core::model::FileFacts {
                format: ext.to_uppercase(),
                codec: ext.to_uppercase(),
                lossless,
                khz: Some(44.1),
                bits: lossless.then_some(16),
                kbps: None,
                size_bytes: Some(r.size),
            },
            upload_speed: 0,
            free_slot: true,
            queue: 0,
            length: Some(200),
        }
    }

    fn playing(source: Source, status: Status, received: u64, speed: u64) -> Progress {
        Progress {
            attempt: 1,
            received,
            speed,
            source: Some(source),
            ..Progress::new(status)
        }
    }

    #[test]
    fn a_better_copy_is_ready_once_all_of_it_is_here_or_it_outruns_the_song() {
        // 30 MB of FLAC for a 200 s song plays at 150 KB/s.
        let flac = peer_copy("peer", "flac", 30_000_000);
        let at = |status, speed| upgrade_ready(&playing(flac.clone(), status, 5_000_000, speed), Some(200));
        assert!(at(Status::Done, 0), "all of it is here");
        assert!(
            at(Status::Streaming, 400_000),
            "arriving 2.7 times faster than it plays"
        );
        assert!(
            !at(Status::Streaming, 200_000),
            "1.3 times: from late in the song it might not stay ahead"
        );
        assert!(!at(Status::Stalled, 400_000), "a stalled transfer isn't ready");
        assert!(!at(Status::Buffering, 400_000), "nor one that has only started");
        assert!(
            !upgrade_ready(&playing(flac, Status::Streaming, 5_000_000, 400_000), None),
            "without the song's length, only a finished copy"
        );
    }

    #[test]
    fn youtube_a_copy_below_the_minimum_and_a_peer_fallen_back_to_are_lesser_copies() {
        let yt = youtube::Copy {
            video: "LUjGtyYEi90".into(),
            format: "251".into(),
            ext: "webm".into(),
        }
        .source_ref(5_730_310);
        let from_youtube = playing(copy_of(&yt, "webm"), Status::Done, 0, 0);
        assert_eq!(lesser(&from_youtube), Some(Lesser::Youtube));
        let mut p = playing(peer_copy("peer", "mp3", 3_000_000), Status::Streaming, 0, 0);
        assert_eq!(lesser(&p), None, "the profile's first choice");
        p.failed_peers.push(FailedPeer {
            peer: "stalled".into(),
            at: 12,
        });
        assert_eq!(lesser(&p), Some(Lesser::FellBack));
        p.below = true;
        assert_eq!(lesser(&p), Some(Lesser::Below));
        assert_eq!(lesser(&Progress::new(Status::Finding)), None, "no copy yet");
    }

    #[test]
    fn a_later_play_takes_the_best_cached_copy_not_the_latest() {
        let entry = |user: &str, ext: &str, used_at| Entry {
            track: TrackId("t".into()),
            source: peer_copy(user, ext, 9_000_000).source_ref(),
            path: PathBuf::from(format!("{user}.{ext}")),
            complete: true,
            used_at,
        };
        let yt = Entry {
            source: youtube::Copy {
                video: "LUjGtyYEi90".into(),
                format: "251".into(),
                ext: "webm".into(),
            }
            .source_ref(5_730_310),
            ..entry("x", "mp3", 400)
        };
        let below = entry("below", "mp3", 300);
        let second = entry("second", "mp3", 200);
        let first = entry("first", "flac", 100);
        // The profile ranks first, then second; below fails the minimum.
        let rank = |e: &Entry| match e.source.user.as_str() {
            "first" => (false, 0),
            "second" => (false, 1),
            "below" => (true, 2),
            _ => (false, usize::MAX),
        };
        let users = |v: Vec<Entry>| v.into_iter().map(|e| e.source.user).collect::<Vec<_>>();
        let best = best_first(vec![yt, below, second, first], rank);
        assert_eq!(
            users(best),
            ["first", "second", "below", youtube::PEER],
            "Soulseek before YouTube, passing before below, then the profile's order, whatever was played last"
        );
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

    mod against_slskd {
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
        use crate::cache::Cache;
        use crate::fake_slskd::{Fake, Peer};
        use crate::peers::Peers;
        use crate::rules::Current;
        use crate::search::Searches;
        use crate::sources::{Source, Sources};

        const REMOTE: &str = "Music\\Artist\\Album\\01 - Song.mp3";

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

        async fn fetcher(fake: &Fake) -> Result<Fetcher> {
            let db = lull_core::db::Db::memory()?;
            db.migrate(&crate::MIGRATIONS)?;
            let bus = Bus::new();
            let cache = Cache::new(db.clone(), Services::new(bus.clone()), fake.root.clone(), u64::MAX / 2);
            let peers = Peers::load(db.clone()).await?;
            let sources = Sources::new(peers.clone(), Vec::new());
            let searches = Searches::new(fake.slskd.clone(), bus.clone(), db, sources.clone());
            Ok(Fetcher::new(
                fake.slskd.clone(),
                Arc::new(RwLock::new(fake.layout.clone())),
                cache,
                peers,
                sources,
                searches,
                bus,
                Current::default(),
                Arc::new(NoYoutube),
            ))
        }

        async fn ended(f: &Arc<Fetch>) -> Result<super::super::Progress> {
            let mut rx = f.progress();
            let ended = tokio::time::timeout(Duration::from_secs(30), rx.wait_for(|p| p.ended())).await;
            Ok(ended
                .map_err(|_| Error::internal("the fetch never ended"))?
                .map_err(|_| Error::internal("gone"))?
                .clone())
        }

        #[tokio::test]
        async fn a_download_slskd_calls_done_before_it_has_moved_the_file_still_lands_whole() -> Result<()> {
            let bytes: Vec<u8> = (0..300_000).map(|i| (i % 251) as u8).collect();
            // slskd reads Succeeded two seconds before it moves the file and says where: lull
            // looks once a second, so it sees the transfer done with nowhere to find it yet.
            let peer = Peer {
                per_tick: 32_768,
                overwrite: false,
                settles: Duration::from_secs(2),
            };
            let fake = Fake::start(HashMap::from([(REMOTE.to_owned(), bytes.clone())]), peer)
                .await
                .map_err(|e| Error::internal(e.to_string()))?;
            let fetcher = fetcher(&fake).await?;
            let pick = SourceRef {
                user: "peer".into(),
                path: REMOTE.into(),
                size: bytes.len() as u64,
            };
            let f = fetcher.stream(track(), Some(pick)).await?;
            let p = ended(&f).await?;
            assert_eq!(p.status, Status::Done, "the fetch failed: {:?}", p.error);
            let path = p.path.ok_or_else(|| Error::internal("no file"))?;
            assert_eq!(std::fs::read(&path).map_err(|e| Error::internal(e.to_string()))?, bytes);
            Ok(())
        }

        #[tokio::test]
        async fn a_reader_streaming_the_song_as_slskd_finishes_and_moves_it_gets_every_byte() -> Result<()> {
            let bytes: Vec<u8> = (0..600_000).map(|i| (i % 251) as u8).collect();
            let peer = Peer {
                per_tick: 8_192,
                overwrite: false,
                settles: Duration::from_secs(2),
            };
            let fake = Fake::start(HashMap::from([(REMOTE.to_owned(), bytes.clone())]), peer)
                .await
                .map_err(|e| Error::internal(e.to_string()))?;
            let fetcher = fetcher(&fake).await?;
            let pick = SourceRef {
                user: "peer".into(),
                path: REMOTE.into(),
                size: bytes.len() as u64,
            };
            let f = fetcher.stream(track(), Some(pick)).await?;
            // The player's request from the start, read while the transfer runs, finishes and
            // the file moves under it.
            let resp = crate::stream::serve(f.clone(), &axum::http::HeaderMap::new()).await?;
            let body = tokio::time::timeout(
                Duration::from_secs(30),
                axum::body::to_bytes(resp.into_body(), usize::MAX),
            )
            .await
            .map_err(|_| Error::internal("the body never ended"))?
            .map_err(|e| Error::internal(format!("the body failed: {e}")))?;
            assert!(
                body.as_ref() == bytes.as_slice(),
                "{} of {} bytes, or different ones",
                body.len(),
                bytes.len()
            );
            assert_eq!(ended(&f).await?.status, Status::Done);
            Ok(())
        }

        /// A copy as a search answer offers it: `claims` is the speed the peer reports, MB/s.
        fn offered(user: &str, size: u64, claims: f64) -> Source {
            Source {
                user: user.into(),
                path: REMOTE.into(),
                size,
                ext: "mp3".into(),
                facts: lull_core::model::FileFacts {
                    format: "MP3 320".into(),
                    codec: "MP3".into(),
                    lossless: false,
                    khz: Some(44.1),
                    bits: None,
                    kbps: Some(320),
                    size_bytes: Some(size),
                },
                upload_speed: (claims * 1_000_000.0) as u64,
                free_slot: true,
                queue: 0,
                length: Some(20),
            }
        }

        /// A 2 MB song of 20 s (it needs 100 KB/s), from a peer that claims 8.6 MB/s and sends
        /// 50 KB/s, and one that claims 4.7 MB/s and sends about 1 MB/s.
        async fn liar_and_honest() -> Result<(Fake, Fetcher, Track, Vec<u8>)> {
            let bytes: Vec<u8> = (0..2_000_000).map(|i| (i % 251) as u8).collect();
            let peer = Peer {
                per_tick: 20_000,
                overwrite: false,
                settles: Duration::from_millis(100),
            };
            let fake = Fake::start_with(
                HashMap::from([(REMOTE.to_owned(), bytes.clone())]),
                peer,
                HashMap::from([("liar".to_owned(), 1_000)]),
            )
            .await
            .map_err(|e| Error::internal(e.to_string()))?;
            let fetcher = fetcher(&fake).await?;
            let song = Track {
                duration: Some(20),
                ..track()
            };
            let size = bytes.len() as u64;
            fetcher.sources.add(HashMap::from([(
                song.id.clone(),
                vec![offered("liar", size, 8.6), offered("honest", size, 4.7)],
            )]));
            Ok((fake, fetcher, song, bytes))
        }

        #[tokio::test]
        async fn a_peer_sending_slower_than_the_song_gives_way_and_is_ranked_by_what_it_sent_next_time() -> Result<()> {
            let (_fake, fetcher, song, bytes) = liar_and_honest().await?;
            let f = fetcher.stream(song.clone(), None).await?;
            let p = ended(&f).await?;
            assert_eq!(p.status, Status::Done, "{:?}", p.error);
            assert_eq!(
                p.source.as_ref().map(|s| s.user.as_str()),
                Some("honest"),
                "the slow peer gave way"
            );
            assert_eq!(
                p.failed_peers.iter().map(|f| f.peer.as_str()).collect::<Vec<_>>(),
                ["liar"]
            );
            assert!(
                !fetcher.peers.stalled_before("liar"),
                "slow is remembered as a speed, not a stall"
            );
            let path = p.path.ok_or_else(|| Error::internal("no file"))?;
            assert_eq!(std::fs::read(&path).map_err(|e| Error::internal(e.to_string()))?, bytes);
            // Next time, the measured 50 KB/s ranks it under the honest peer's figure.
            let ranked = fetcher.sources.ordered(
                &song.id,
                song.duration,
                &fetcher.rules.get(),
                super::super::Purpose::Stream,
            );
            assert_eq!(ranked.first().map(|r| r.peer.as_str()), Some("honest"));
            let liar = ranked
                .iter()
                .find(|r| r.peer == "liar")
                .ok_or_else(|| Error::internal("no liar"))?;
            assert!(
                liar.speed_measured && liar.speed < 0.2,
                "shown as measured: {} MB/s",
                liar.speed
            );
            Ok(())
        }

        #[tokio::test]
        async fn a_peer_the_listener_picked_is_kept_however_slow() -> Result<()> {
            let (_fake, fetcher, song, _) = liar_and_honest().await?;
            let pick = SourceRef {
                user: "liar".into(),
                path: REMOTE.into(),
                size: 2_000_000,
            };
            let f = fetcher.stream(song, Some(pick)).await?;
            tokio::time::sleep(Duration::from_secs(13)).await;
            let p = f.now();
            assert_eq!(p.source.as_ref().map(|s| s.user.as_str()), Some("liar"));
            assert!(p.failed_peers.is_empty(), "{:?}", p.failed_peers);
            Ok(())
        }

        /// Plays the fetch as a listener would, from the progress it reports: starts at 256 KB, plays
        /// at the song's rate, stalls when it reaches what has arrived and resumes 2 s of song later.
        /// Returns seconds to first sound, the stalls, and seconds stalled.
        async fn listen(f: &Arc<Fetch>, size: f64, seconds: f64) -> (f64, u32, f64) {
            let rate = size / seconds;
            let start = tokio::time::Instant::now();
            let (mut first, mut stalls, mut stalled, mut pos, mut playing, mut attempt) = (None, 0, 0.0, 0.0, false, 0);
            let tick = Duration::from_millis(50);
            while pos < size && start.elapsed() < Duration::from_secs(120) {
                tokio::time::sleep(tick).await;
                let p = f.now();
                // A new source restarts the file: the player reopens at the same place.
                let arrived = if p.attempt == attempt { p.received as f64 } else { 0.0 };
                attempt = p.attempt.max(attempt);
                let have = if p.status == Status::Done { size } else { arrived };
                if playing {
                    pos += rate * tick.as_secs_f64();
                    if pos >= have && have < size {
                        playing = false;
                        stalls += 1;
                    }
                } else if have >= (pos + 2.0 * rate).min(size) && have >= 256_000.0 {
                    first.get_or_insert(start.elapsed().as_secs_f64());
                    playing = true;
                } else if first.is_some() {
                    stalled += tick.as_secs_f64();
                }
            }
            (first.unwrap_or(f64::NAN), stalls, stalled)
        }

        /// The before/after for ADR-0015, printed: `cargo test -p lull-soulseek before_and_after -- --ignored --nocapture`.
        /// Before: the peer's own figures rank the liar first and nothing makes it give way (as a
        /// pick, which the give-way rule leaves alone, it plays as the old fetch did). After: the
        /// first play gives way after 10 s; the second is ranked by what each peer sent.
        #[tokio::test]
        #[ignore = "a report, about two minutes"]
        async fn before_and_after_a_peer_that_claims_fast_and_sends_slow() -> Result<()> {
            let (_fake, fetcher, song, bytes) = liar_and_honest().await?;
            let size = bytes.len() as f64;
            let liar = SourceRef {
                user: "liar".into(),
                path: REMOTE.into(),
                size: bytes.len() as u64,
            };
            let f = fetcher.stream(song.clone(), Some(liar)).await?;
            let before = listen(&f, size, 20.0).await;
            fetcher.cancel_fetch(&f);
            // A fresh hub's first play of the song, then a second play once lull has measured both.
            let (fake2, fetcher2, song2, _) = liar_and_honest().await?;
            let f = fetcher2.stream(song2.clone(), None).await?;
            let first = listen(&f, size, 20.0).await;
            ended(&f).await?;
            fetcher2.cancel_fetch(&f);
            fetcher2.forget(&f);
            // Played again from Soulseek, not the cache: what is measured decides the order.
            fetcher2
                .cache
                .forget(&SourceRef {
                    user: "honest".into(),
                    path: REMOTE.into(),
                    size: bytes.len() as u64,
                })
                .await?;
            for e in std::fs::read_dir(fake2.root.join("complete")).map_err(|e| Error::internal(e.to_string()))? {
                let path = e.map_err(|e| Error::internal(e.to_string()))?.path();
                std::fs::remove_file(path).map_err(|e| Error::internal(e.to_string()))?;
            }
            let f = fetcher2.stream(song2, None).await?;
            let second = listen(&f, size, 20.0).await;
            for (what, (first_sound, stalls, stalled)) in [
                ("before", before),
                ("after, first play", first),
                ("after, measured", second),
            ] {
                println!("{what:>18}: first sound {first_sound:.1} s, {stalls} stalls, {stalled:.1} s stalled");
            }
            Ok(())
        }
    }
}
