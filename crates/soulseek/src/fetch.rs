//! Getting a song onto disk: one slskd transfer at a time, from copies in the order the
//! listener's streaming or saving profile sets (ADR-0008), falling back to the next one when a
//! peer doesn't answer, queues us too long or stalls. When no copy passes the profile's minimum,
//! its `below` rule decides. A fetch is shared by everyone who wants the song (the player
//! streaming it, a save completing it), so the bytes come over the network once
//! (docs/context/streaming.md).
//!
//! YouTube is the second source (docs/context/youtube.md): with `fallback` it plays when no
//! Soulseek copy does in time; with `startFast` YouTube and Soulseek race, and whichever sends the
//! song's first bytes first plays, the other carrying on beside it (ADR-0012). Its audio lands in
//! the same cache, so everything downstream (ranges, preloading, pins, eviction) is unchanged.
//!
//! What a song starts on and what it switches to are the streaming policy's (`policy`): this
//! module gathers copies, from Soulseek and YouTube alike, and asks it.
//!
//! Better copies: while the policy knows a copy worth switching to from the one the player
//! streams (`policy::better`), an upgrade fetch gets it beside it; the player switches to it
//! seamlessly once enough has arrived (docs/context/streaming.md#better-copies).

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

mod legs;

use crate::cache::{Cache, Entry, Pin};
use crate::files;
use crate::layout::Layout;
use lull_core::quality::{SaveBelow, StreamBelow};

use crate::budget::Lane;
use crate::peers::Peers;
use crate::policy::{self, Bar};
use crate::ranges::Ranges;
use crate::rules::{Current, Purpose, Rules};
use crate::search::Searches;
use crate::slskd::Slskd;
use crate::sources::{self, Ranked, Source, Sources};

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
/// Why a YouTube attempt ends when its copy's sound isn't the song (ADR-0016): the next copy
/// plays, and the player reopens the song on it.
const NOT_THE_SONG: &str = "the YouTube copy isn't the song by its sound";
/// YouTube copies tried in turn when one turns out not to be the song.
const YOUTUBE_COPIES: usize = 2;
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
    /// The bytes of the copy that have arrived (ADR-0014): legs that start where the listener
    /// seeks leave gaps, so a file's length says nothing about it.
    pub ranges: Ranges,
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
    /// A seek past what has arrived starts a leg there when the leg running would take longer
    /// than this many seconds to get there; `None` while it can't (ADR-0014). The player reads
    /// it to choose between going there now and keeping the seek.
    pub ahead: Option<u32>,
    /// A YouTube copy no reference could check by its sound: the player asks the listener
    /// ("Right song?", ADR-0016).
    pub unverified: bool,
    /// A device whose browser or app couldn't play this copy ("this copy can't be played in
    /// <device> (<format>)"): the job's record of it, for Activity and the device that said so.
    /// Other devices' playback is untouched.
    pub unplayable: Option<String>,
    /// What became of a better copy, or of start fast's race, for Activity: "switched to MP3 320
    /// from <peer>", or why the player couldn't switch to it.
    pub note: Option<String>,
}

impl Progress {
    fn new(status: Status) -> Self {
        Progress {
            attempt: 0,
            status,
            source: None,
            ranges: Ranges::default(),
            speed: 0,
            queue_pos: None,
            path: None,
            error: None,
            failed_peers: Vec::new(),
            peers_found: 0,
            below: false,
            choices: Vec::new(),
            unverified: false,
            unplayable: None,
            ahead: None,
            note: None,
        }
    }

    pub fn size(&self) -> u64 {
        self.source.as_ref().map_or(0, |s| s.size)
    }

    /// Bytes arrived in all.
    pub fn received(&self) -> u64 {
        self.ranges.total()
    }

    pub fn ended(&self) -> bool {
        matches!(self.status, Status::Done | Status::Failed | Status::Cancelled)
    }

    fn pct(&self) -> u8 {
        match self.size() {
            0 => 0,
            size => (self.received().min(size) * 100 / size) as u8,
        }
    }
}

pub struct Fetch {
    pub job_id: String,
    pub track: Track,
    /// Whose profile orders the copies: the player's or a save's.
    pub purpose: Purpose,
    pub started: i64,
    /// When it began: a search for the song that ended since then is its own (`searched_since`).
    opened: Instant,
    progress: watch::Sender<Progress>,
    stop: watch::Sender<bool>,
    want: Mutex<Want>,
    readers: AtomicUsize,
    last_read: Mutex<Instant>,
    /// It has turned to YouTube (finding or fetching a YouTube copy).
    on_youtube: AtomicBool,
    /// The listener picked this copy: it is never switched away from by itself.
    picked: AtomicBool,
    /// The copy the listener picked to start this fetch: asking for the song with that pick
    /// again (the player reopening it after a fallback) is this fetch, whichever peer sends now.
    picked_copy: std::sync::OnceLock<SourceRef>,
    /// An upgrade's bar: it fetches only a copy better than the one the player has.
    bar: Option<Bar>,
    /// What it was started for beside the player's fetch, if it was: fixed when it starts, so
    /// its route never depends on what happens to it later (a hand-over before it even began).
    beside: Option<Beside>,
    /// Soulseek's side of start fast's race, still racing: only a copy that passes the streaming
    /// minimum, and no `below` rule, until it becomes the player's fetch (`hand_over`).
    racing: AtomicBool,
    /// The player's fetch this one took over (start fast's race won by Soulseek, the better copy
    /// switched to): its `stream` events name that job, so the player keeps following the song.
    replaces: std::sync::OnceLock<String>,
    /// The fetch that took over from this one before it sent a byte (start fast's race): a
    /// response still waiting on this one serves that one.
    successor: std::sync::OnceLock<Arc<Fetch>>,
    /// Why YouTube couldn't start the song, when this fetch took over for that reason: its own
    /// failure then says why for both.
    youtube_failed: std::sync::OnceLock<String>,
    /// Woken when the last response reading it ends.
    unread: Notify,
    /// Bytes readers wait for that haven't arrived, each with its ask's id and since when, newest
    /// last: where a leg may start (ADR-0014, What starts a leg).
    asked: Mutex<Vec<(u64, u64, Instant)>>,
    asks: AtomicU64,
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

    /// A reader waits for `byte`, which hasn't arrived, for as long as it holds the `Waiting`: a
    /// request the browser drops (a probe it gave up on) asks for nothing any more.
    pub fn ask_for(self: &Arc<Self>, byte: u64) -> Waiting {
        let id = self.asks.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut asked) = self.asked.lock() {
            asked.push((id, byte, Instant::now()));
        }
        Waiting {
            fetch: self.clone(),
            id,
        }
    }

    /// The newest byte a reader still waits for, and since when.
    fn asked(&self) -> Option<(u64, Instant)> {
        self.asked
            .lock()
            .ok()
            .and_then(|a| a.last().map(|&(_, byte, since)| (byte, since)))
    }

    fn idle(&self) -> bool {
        self.readers.load(Ordering::Relaxed) == 0 && self.last_read.lock().is_ok_and(|t| t.elapsed() > IDLE)
    }

    /// The fetch that took over from this one before it sent a byte, when one did.
    pub fn successor(&self) -> Option<Arc<Fetch>> {
        self.successor.get().cloned()
    }
}

/// A reader's wait for a byte that hasn't arrived; dropped when the reader stops waiting.
pub struct Waiting {
    fetch: Arc<Fetch>,
    id: u64,
}

impl Drop for Waiting {
    fn drop(&mut self) {
        if let Ok(mut asked) = self.fetch.asked.lock() {
            asked.retain(|&(id, _, _)| id != self.id);
        }
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
    /// Another fetch became the player's before this one sent a byte (start fast's race).
    HandedOver,
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
    /// The latest fetches that ended, for a player asking what became of its job.
    ended_fetches: Arc<Mutex<VecDeque<Arc<Fetch>>>>,
    /// slskd resumes a download where lull sized its file: until one is seen not to (an slskd
    /// set to overwrite partial files), legs may start anywhere (ADR-0014).
    resumes: Arc<AtomicBool>,
    next: Arc<AtomicU64>,
    /// Openings of the player's stream take turns (`stream`).
    opening: Arc<tokio::sync::Mutex<()>>,
}

/// Which map a new fetch goes in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Slot {
    Live,
    Upgrade,
}

/// What a fetch beside the player's is for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Beside {
    /// A better copy for the player to switch to: it stops when the player moves on.
    Better,
    /// Start fast's Soulseek copy beside a cached YouTube copy: for later plays and saves too.
    StartFast,
    /// Soulseek's side of start fast's race.
    Racer,
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
            ended_fetches: Arc::default(),
            resumes: Arc::new(AtomicBool::new(true)),
            next: Arc::default(),
            opening: Arc::default(),
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
    ///
    /// One opening at a time: two at once (a browser's audio element asking for another range,
    /// another device) each found no fetch running and started their own. Start fast's race then
    /// handed the song to Soulseek's copy as the first one's, and the player, following the
    /// second, dropped every event of the copy playing: it showed what had arrived frozen and
    /// kept seeks that never landed (2026-11-08).
    pub async fn stream(&self, track: Track, pick: Option<SourceRef>) -> Result<Arc<Fetch>> {
        let _turn = self.opening.lock().await;
        self.open(track, pick).await
    }

    async fn open(&self, track: Track, pick: Option<SourceRef>) -> Result<Arc<Fetch>> {
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
            // The pick this fetch started with is this fetch, though it fell back to another peer
            // since: the player reopens the song after every fallback with its pick still in the
            // address, and replacing the fetch then started the stalled peer again, in a loop.
            let started_with = pick.is_some() && pick.as_ref() == f.picked_copy.get();
            if pick.is_none() || pick == current || started_with {
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
                let playing = entry.source.clone();
                let f = self.cached(track.clone(), entry).await?;
                f.picked.store(pick.is_some(), Ordering::Relaxed);
                want(&f, |w| w.streaming = true);
                self.bus.publish("stream", self.player_event(&f, &f.now()));
                if on_youtube && mode == Mode::StartFast {
                    self.upgrade_later(track, playing, Instant::now()).await?;
                }
                self.better_later(&f);
                return Ok(f);
            }
            None => self.start(track, pick, Purpose::Stream, Want::default(), Slot::Live),
        };
        want(&f, |w| w.streaming = true);
        Ok(f)
    }

    /// A copy that has played through while a better one arrived beside it (`best_cached`, the
    /// policy's order): asking for the song now is a later play, which takes the better one,
    /// whatever either's source. Within the song's length (and a minute) it is the same play
    /// seeking, which never jumps to another file.
    async fn superseded(&self, f: &Arc<Fetch>) -> Result<bool> {
        let length = i64::from(f.track.duration.unwrap_or(600));
        let played_out = f.now().status == Status::Done
            && f.readers.load(Ordering::Relaxed) == 0
            && lull_core::now() - f.started > length + 60
            && want(f, |w| w.saving == 0);
        if !played_out {
            return Ok(false);
        }
        let cached = self.best_cached(&f.track, Purpose::Stream).await?;
        Ok(cached.is_some_and(|e| !holds(f, &e.source)))
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

    /// Start fast, with a cached YouTube copy playing: fetches Soulseek's copy in the background,
    /// for this play and later ones, unless one is running or a Soulseek copy is already cached.
    /// With "Switch to a better copy" on it is the better copy by the streaming profile (within
    /// its cap), so the player can move to it mid-song; off, the saving profile's, for later
    /// plays and saves. Either way it runs on when the player moves on.
    async fn upgrade_later(&self, track: Track, playing: SourceRef, since: Instant) -> Result<()> {
        if self.upgrade(&track.id).is_some() {
            return Ok(());
        }
        let cached = self.best_cached(&track, Purpose::Save).await?;
        if cached.is_some_and(|e| !youtube::is_youtube(&e.source)) {
            return Ok(());
        }
        let bar = self.bar(&track, &playing, Vec::new());
        let (purpose, bar) = match bar {
            Some(bar) if self.rules.get().switch_to_better => (Purpose::Stream, Some(bar)),
            _ => (Purpose::Save, None),
        };
        if let Some(f) = self.start_upgrade(track, purpose, bar, since, Beside::StartFast) {
            tracing::info!(track = %f.track.id, job = f.job_id, "fetching Soulseek's copy while YouTube plays");
        }
        Ok(())
    }

    /// The player streams `f`: when the policy knows a copy worth switching to from the one
    /// playing (`policy::better`, whatever either's source), it is fetched beside it, out of its way, so the player can switch to it seamlessly (and later
    /// plays and saves have it). Not when the listener picked the copy playing. When none is
    /// known, the copy playing fails the streaming minimum and the song hasn't been searched
    /// during this play, the upgrade searches again.
    fn better_later(&self, f: &Fetch) {
        let rules = self.rules.get();
        let p = f.now();
        let Some(playing) = p.source.as_ref() else {
            return;
        };
        if !rules.switch_to_better
            || f.purpose != Purpose::Stream
            || f.picked.load(Ordering::Relaxed)
            || !want(f, |w| w.streaming)
        {
            return;
        }
        let duration = f.track.duration;
        let avoid = p.failed_peers.iter().map(|failed| failed.peer.clone()).collect();
        let Some(bar) = self.bar(&f.track, &playing.source_ref(), avoid) else {
            return;
        };
        if let Some(u) = self.upgrade(&f.track.id) {
            let up = u.now();
            let better = up
                .source
                .as_ref()
                .is_none_or(|s| policy::switch_to(&rules.stream, s, &bar.playing, duration));
            if !up.ended() || better {
                return;
            }
            // Start fast's copy, finished and no better than the one playing: it is in the cache
            // for later plays, and out of the way of a better one.
            self.forget(&u);
        }
        let known = self.sources.ordered(&f.track, &rules, Purpose::Stream);
        let worth = !policy::better(&rules.stream, self.sources.preferred(), known, &bar, duration).is_empty();
        if worth || (p.below && !self.sources.searched_since(&f.track.id, f.opened)) {
            if let Some(u) = self.start_upgrade(f.track.clone(), Purpose::Stream, Some(bar), f.opened, Beside::Better) {
                tracing::info!(track = %u.track.id, job = u.job_id, "fetching a better copy beside the one playing");
            }
            return;
        }
        // Start fast makes YouTube a source alongside Soulseek: its copy may be the target. Its
        // match is looked up behind the listener's own lookups (remembered from the race, mostly).
        if rules.youtube.mode != Mode::StartFast {
            return;
        }
        let (this, track, opened) = (self.clone(), f.track.clone(), f.opened);
        tokio::spawn(async move {
            let still = || this.live(&track.id).is_some_and(|l| want(&l, |w| w.streaming));
            if this.targets(&track, &bar, &[]).await.is_empty() || !still() {
                return;
            }
            if let Some(u) = this.start_upgrade(track, Purpose::Stream, Some(bar), opened, Beside::Better) {
                tracing::info!(track = %u.track.id, job = u.job_id, "fetching a better copy beside the one playing");
            }
        });
    }

    /// The copies worth switching to from `bar` (`policy::better`), best first, whatever their
    /// source: Soulseek's known copies, and YouTube's while start fast makes YouTube a source
    /// alongside Soulseek. None of `tried`.
    async fn targets(&self, track: &Track, bar: &Bar, tried: &[SourceRef]) -> Vec<Ranked> {
        let rules = self.rules.get();
        let (duration, cap) = (track.duration, rules.stream.max_kbps);
        let untried = |r: &SourceRef| !tried.iter().any(|t| t.user == r.user && t.path == r.path);
        let mut copies: Vec<Ranked> = self
            .sources
            .ordered(track, &rules, Purpose::Stream)
            .into_iter()
            .filter(|r| untried(&r.source.source_ref()))
            .collect();
        if rules.youtube.mode == Mode::StartFast
            && let Ok(Some(found)) = self
                .youtube
                .find_later(track, YoutubeWant::Stream { max_kbps: cap })
                .await
            && untried(&found.source)
            && let Some(theirs) = bare_source(&found.source, duration)
        {
            copies.push(self.sources.rank_for(theirs, cap, duration));
        }
        let copies = sources::order(copies, duration, &rules, Purpose::Stream);
        policy::better(&rules.stream, self.sources.preferred(), copies, bar, duration)
    }

    /// What a better copy has to beat: the copy `playing`, as known (a peer's from its search,
    /// YouTube's from its own facts), and the peers that failed this play.
    fn bar(&self, track: &Track, playing: &SourceRef, avoid: Vec<String>) -> Option<Bar> {
        let playing = self
            .sources
            .get(track, playing)
            .or_else(|| bare_source(playing, track.duration))?;
        Some(Bar { playing, avoid })
    }

    /// Whether a preload should fetch the song although a copy is cached: the policy knows a copy
    /// worth switching to from it (`policy::better`; with switching to better copies on).
    pub fn better_than_cached(&self, track: &Track, cached: &SourceRef) -> bool {
        let rules = self.rules.get();
        let Some(bar) = self.bar(track, cached, Vec::new()) else {
            return false;
        };
        let known = self.sources.ordered(track, &rules, Purpose::Stream);
        rules.switch_to_better
            && !policy::better(&rules.stream, self.sources.preferred(), known, &bar, track.duration).is_empty()
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
        u.racing.store(false, Ordering::Relaxed);
        want(&u, |w| w.streaming = true);
        let old = old.filter(|o| !Arc::ptr_eq(o, &u));
        // The player follows this job from now on: its events name the one it replaces, which
        // the player was following, so none of them is taken for another fetch's.
        if let Some(old) = &old {
            let _ = u.replaces.set(old.job_id.clone());
        }
        if !picked && let Some(s) = u.now().source {
            let note = format!("switched to {} from {}", s.facts.format, s.user);
            self.update(&u, |p| p.note = Some(note));
        }
        if let Some(old) = old {
            self.retire(old);
        }
        tracing::info!(track = %track, job = u.job_id, picked, "the player switched to the better copy");
        self.bus.publish("stream", self.player_event(&u, &u.now()));
        Ok(u)
    }

    /// The player couldn't switch to the better copy on its way (`why`): the copy's job says so,
    /// for Activity, and it plays from the next play. Nothing else changes.
    pub fn not_switched(&self, track: &TrackId, copy: &SourceRef, why: NotSwitched) -> Result<()> {
        let u = self
            .upgrade_of(track, copy)
            .ok_or_else(|| Error::Conflict("that copy is no longer on its way".into()))?;
        let note = format!("not switched to mid-song: {}; it plays from the next play", why.words());
        tracing::info!(track = %track, job = u.job_id, note, "the player didn't switch to the better copy");
        self.update(&u, |p| p.note = Some(note));
        Ok(())
    }

    /// The `stream` event for the song's live fetch: what the player follows.
    pub fn event(&self, f: &Fetch) -> serde_json::Value {
        self.player_event(f, &f.now())
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
    /// pick the copy playing, and the copy on its way is worth switching to (`policy::switch_to`).
    fn offer(&self, f: &Fetch, p: &Progress) -> Option<serde_json::Value> {
        let rules = self.rules.get();
        if !rules.switch_to_better || f.picked.load(Ordering::Relaxed) {
            return None;
        }
        let playing = p.source.as_ref()?.source_ref();
        let u = self.upgrade(&f.track.id)?;
        let up = u.now();
        let duration = f.track.duration;
        let bar = self.bar(&f.track, &playing, Vec::new())?;
        let s = up
            .source
            .as_ref()
            .filter(|s| !s.is(&playing) && policy::switch_to(&rules.stream, s, &bar.playing, duration))?;
        Some(json!({
            "peer": s.user,
            "path": s.path,
            "format": s.facts.format,
            "status": stream_status(up.status),
            "received": up.received(),
            "size": up.size(),
            "ready": upgrade_ready(&up, duration),
        }))
    }

    /// The song's best finished copy in the cache by `purpose`'s profile (none of YouTube's
    /// while YouTube is off): what a later play (`policy::cached_key`, any source alike) or a save
    /// (passing the saving minimum, then its order) takes.
    pub async fn best_cached(&self, track: &Track, purpose: Purpose) -> Result<Option<Entry>> {
        let rules = self.rules.get();
        let duration = track.duration;
        let mut entries = self.cache.complete(&track.id).await?;
        entries.retain(|e| rules.youtube.mode != Mode::Off || !youtube::is_youtube(&e.source));
        let known = |e: &Entry| {
            self.sources
                .get(track, &e.source)
                .or_else(|| bare_source(&e.source, duration))
        };
        let best = match purpose {
            Purpose::Stream => best_first(entries, |e| {
                known(e).map(|s| policy::cached_key(&rules.stream, self.sources.preferred(), &s, duration))
            }),
            Purpose::Save => {
                let ordered = self.sources.ordered(track, &rules, purpose);
                best_first(entries, |e| {
                    let meets = known(e).is_some_and(|s| s.meets(rules.minimum(purpose), duration));
                    let at = ordered.iter().position(|r| r.source.is(&e.source));
                    (!meets, at.unwrap_or(usize::MAX))
                })
            }
        };
        Ok(best.into_iter().next())
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
            if let Err(e) = youtube.find_later(&track, YoutubeWant::Stream { max_kbps }).await {
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
    /// device lists it to preload. Start fast's Soulseek copy runs on (it is for later plays and
    /// saves); a better copy fetched only for switching mid-song stops unless a save needs it.
    pub fn stop_streaming(&self, track: &TrackId) {
        if let Some(u) = self.upgrade(track)
            && u.beside == Some(Beside::Better)
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
        let f = self.fetch(track, purpose, progress, wanted, None, Instant::now(), None);
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
    #[allow(clippy::too_many_arguments)]
    fn fetch(
        &self,
        track: Track,
        purpose: Purpose,
        progress: Progress,
        wanted: Want,
        bar: Option<Bar>,
        opened: Instant,
        beside: Option<Beside>,
    ) -> Arc<Fetch> {
        let n = self.next.fetch_add(1, Ordering::Relaxed);
        Arc::new(Fetch {
            job_id: format!("job-{:x}{n}", lull_core::now()),
            purpose,
            started: lull_core::now(),
            opened,
            progress: watch::channel(progress).0,
            stop: watch::channel(false).0,
            want: Mutex::new(wanted),
            readers: AtomicUsize::new(0),
            last_read: Mutex::new(Instant::now()),
            on_youtube: AtomicBool::new(false),
            picked: AtomicBool::new(false),
            picked_copy: std::sync::OnceLock::new(),
            bar,
            beside,
            racing: AtomicBool::new(beside == Some(Beside::Racer)),
            replaces: std::sync::OnceLock::new(),
            successor: std::sync::OnceLock::new(),
            youtube_failed: std::sync::OnceLock::new(),
            unread: Notify::new(),
            asked: Mutex::default(),
            asks: AtomicU64::new(0),
            _pin: self.cache.pin(&track.id),
            track,
        })
    }

    async fn cached(&self, track: Track, e: Entry) -> Result<Arc<Fetch>> {
        self.cache.touch(&track.id).await?;
        let source = self
            .sources
            .get(&track, &e.source)
            .or_else(|| bare_source(&e.source, track.duration));
        let on_youtube = youtube::is_youtube(&e.source);
        // Every source's copy is judged by the streaming profile, YouTube's too.
        let below = source
            .as_ref()
            .is_some_and(|s| !policy::passes(&self.rules.get().stream, s, track.duration));
        let f = self.new_fetch(
            track,
            Purpose::Stream,
            Progress {
                attempt: 1,
                ranges: Ranges::front(e.source.size),
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
        if let Some(p) = &pick {
            let _ = f.picked_copy.set(p.clone());
        }
        self.spawn(f.clone(), pick);
        f
    }

    /// A copy fetched beside the song's live fetch, unless one is on its way already.
    /// `opened`: since when a search for the song counts as its own.
    fn start_upgrade(
        &self,
        track: Track,
        purpose: Purpose,
        bar: Option<Bar>,
        opened: Instant,
        beside: Beside,
    ) -> Option<Arc<Fetch>> {
        let f = self.fetch(
            track,
            purpose,
            Progress::new(Status::Finding),
            Want::default(),
            bar,
            opened,
            Some(beside),
        );
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
        let arrived = outcome.as_ref().ok().cloned();
        let youtube_failed = f.youtube_failed.get().cloned();
        self.update(&f, |p| match outcome {
            Ok(path) => {
                p.status = Status::Done;
                p.ranges = Ranges::front(p.size());
                p.path = Some(path);
            }
            Err(Stop::Cancelled) => p.status = Status::Cancelled,
            Err(Stop::HandedOver) => p.status = Status::Cancelled,
            Err(Stop::Failed(e)) => {
                p.status = Status::Failed;
                p.error = Some(match youtube_failed {
                    Some(yt) => format!("{e}; YouTube: {yt}"),
                    None => e,
                });
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
        if let Ok(mut ended) = self.ended_fetches.lock() {
            ended.push_back(f.clone());
            while ended.len() > HISTORY {
                ended.pop_front();
            }
        }
        // A stopped fetch leaves even when it finished at the last moment: nobody is playing it.
        // So does a finished one nobody plays or saves (a preload, an upgrade): its file stays in
        // the cache, unpinned, and playing it later starts from there at once.
        // A better copy beside the song the player streams stays, so the player can switch to it.
        let stopped = *f.stop.borrow();
        let beside = self.played_beside(&f);
        let idle = want(&f, |w| !w.streaming && w.saving == 0) && beside.is_none();
        if p.status != Status::Done || stopped || idle {
            self.forget(&f);
        }
        // Start fast's copy finished beside the song: when it is no better than the one playing,
        // a better one may still be found.
        if p.status == Status::Done
            && let Some(live) = beside
        {
            self.better_later(&live);
        }
        // Checked once the maps say where the copy stands: a better copy beside the song that
        // isn't the song gives its place to the next.
        if let Some(path) = arrived {
            self.check_soulseek_copy(&f, &path);
        }
    }

    /// Gets the song onto disk by the listener's mode: Soulseek alone, Soulseek then YouTube
    /// (`fallback`), or YouTube and Soulseek at once (`startFast`, `race`). Only the player's own
    /// fetches (and preloads) turn to YouTube; a save names its copy.
    async fn attempts(&self, f: &Fetch, pick: Option<SourceRef>) -> Result<PathBuf, Stop> {
        let mut stop = f.stop.subscribe();
        let prefetch_only = want(f, |w| w.prefetch_only());
        match route(
            f.purpose,
            pick.as_ref(),
            &self.rules.get().youtube,
            prefetch_only,
            f.beside.is_some(),
        ) {
            // A copy the listener picked plays as it is.
            Route::Youtube(copy) => self.youtube_attempt(f, &copy, false, &mut stop).await,
            Route::Soulseek => self.soulseek(f, pick, stop).await,
            Route::SoulseekThenYoutube(wait) => self.soulseek_then_youtube(f, wait, true, &mut stop).await,
            Route::Race => self.race(f, &mut stop).await,
        }
    }

    /// Start fast is a race (ADR-0012): YouTube's copy and the streaming profile's Soulseek copy
    /// are fetched at once, and whichever sends the song's first bytes first plays. YouTube is not
    /// first by rule: a Soulseek copy known from an earlier search often starts before YouTube's
    /// lookup has ended (it took 15 s once, 2026-11-07).
    /// - **YouTube first:** Soulseek's copy carries on beside it, the better copy the player may
    ///   switch to (when it is better) and what later plays and saves take.
    /// - **Soulseek first,** or YouTube has no copy: its fetch becomes the player's (`hand_over`)
    ///   and YouTube's stops.
    async fn race(&self, f: &Fetch, stop: &mut watch::Receiver<bool>) -> Result<PathBuf, Stop> {
        self.search_beside(f);
        // A preload that made way is Soulseek's side already.
        let mut soulseek = self.upgrade(&f.track.id).or_else(|| self.racer(f));
        // Soulseek's copy sent first but is above the streaming cap: it starts the song only when
        // YouTube has no copy that may (`policy::start`, within the cap first).
        let mut held = false;
        // YouTube's lookup, while Soulseek's side may send first.
        let looked = {
            let lookup = self.youtube_copy(f, stop);
            tokio::pin!(lookup);
            loop {
                tokio::select! {
                    r = &mut lookup => break Some(r),
                    won = soulseek_sends(soulseek.clone()), if soulseek.is_some() && !held => {
                        if won && self.above_cap(f, soulseek.as_ref()) {
                            held = true;
                            continue;
                        }
                        if won {
                            break None;
                        }
                        soulseek = None;
                    }
                }
            }
        };
        let (outcome, youtube_sent) = match looked {
            None => (Err(()), false),
            Some(Err(e)) => (Ok(Err(e)), false),
            // A copy that may not start the song (`policy::may_start`: below the minimum, under
            // `keepLooking` or `ask`) doesn't race: Soulseek's side may still send a passing copy,
            // and when none comes the policy decides, with this copy one more found.
            Some(Ok(copy)) if !self.may_start(f, &copy) => {
                tracing::info!(track = %f.track.id, copy = copy.path, "YouTube's copy is below the streaming minimum: it doesn't race");
                let won = match soulseek.clone() {
                    Some(u) => tokio::select! {
                        won = soulseek_sends(Some(u)) => won,
                        () = stopped(stop) => return Err(Stop::Cancelled),
                    },
                    None => false,
                };
                if !won {
                    f.on_youtube.store(false, Ordering::Relaxed);
                    return self.race_below(f, copy, stop).await;
                }
                (Err(()), false)
            }
            Some(Ok(copy)) => {
                let theirs = copy.clone();
                let youtube = self.youtube_plays(f, copy, stop);
                tokio::pin!(youtube);
                let mut progress = f.progress();
                let mut youtube_sent = false;
                let outcome = loop {
                    tokio::select! {
                        r = &mut youtube => break Ok(r),
                        _ = progress.wait_for(sent), if !youtube_sent => {
                            // Soulseek's side ready at the same moment: the policy picks.
                            if self.soulseek_starts(f, &theirs, soulseek.as_ref()) {
                                break Err(());
                            }
                            youtube_sent = true;
                        }
                        won = soulseek_sends(soulseek.clone()), if !youtube_sent && soulseek.is_some() && !held => {
                            if won && self.above_cap(f, soulseek.as_ref()) {
                                held = true;
                                continue;
                            }
                            if won {
                                break Err(());
                            }
                            // It ended without sending (no copy passes, every peer failed):
                            // YouTube's copy plays alone.
                            soulseek = None;
                        }
                    }
                };
                (outcome, youtube_sent)
            }
        };
        // Soulseek's side, unless it failed or was stopped (finished is fine: it sent everything).
        let u = soulseek.filter(|u| !matches!(u.now().status, Status::Failed | Status::Cancelled));
        match (outcome, u) {
            (Err(()), Some(u)) => {
                self.hand_over(f, &u, "Soulseek's copy started the song first");
                Err(Stop::HandedOver)
            }
            (Ok(Err(Stop::Failed(yt))), Some(u)) if !youtube_sent => {
                tracing::info!(track = %f.track.id, why = yt, "YouTube didn't start the song; Soulseek's copy plays");
                let _ = u.youtube_failed.set(yt.clone());
                self.hand_over(f, &u, &format!("YouTube: {yt}"));
                Err(Stop::HandedOver)
            }
            (Ok(Err(Stop::Failed(yt))), _) => {
                tracing::info!(track = %f.track.id, why = yt, "YouTube didn't play the song; looking on Soulseek");
                f.on_youtube.store(false, Ordering::Relaxed);
                self.drop_upgrade(&f.track.id);
                match self.soulseek_then_youtube(f, None, false, stop).await {
                    Err(Stop::Failed(why)) => Err(Stop::Failed(format!("{why}; YouTube: {yt}"))),
                    other => other,
                }
            }
            (Ok(done), _) => done,
            // Soulseek sent first, then failed or was stopped before the hand-over: the player's
            // fetch looks on Soulseek itself, as when YouTube fails.
            (Err(()), None) => {
                f.on_youtube.store(false, Ordering::Relaxed);
                self.soulseek_then_youtube(f, None, false, stop).await
            }
        }
    }

    /// Soulseek's side of start fast's race: the streaming profile's copy, fetched beside the
    /// player's fetch from the start, in the profile's order, of copies that may start the song
    /// (`policy::may_start`: passing the minimum, or any under `playBest`).
    fn racer(&self, f: &Fetch) -> Option<Arc<Fetch>> {
        let u = self.start_upgrade(f.track.clone(), Purpose::Stream, None, f.opened, Beside::Racer)?;
        tracing::info!(track = %f.track.id, job = u.job_id, "racing Soulseek's copy against YouTube's");
        Some(u)
    }

    /// Whether Soulseek's side of the race (`u`) is on a copy above the streaming cap.
    fn above_cap(&self, f: &Fetch, u: Option<&Arc<Fetch>>) -> bool {
        let rules = self.rules.get();
        u.and_then(|u| u.now().source)
            .is_some_and(|s| !policy::within_cap(&rules.stream, &s, f.track.duration))
    }

    /// The copy `copy` as known: a peer's from its search, YouTube's from its own facts.
    fn known(&self, f: &Fetch, copy: &SourceRef) -> Option<Source> {
        self.sources
            .get(&f.track, copy)
            .or_else(|| bare_source(copy, f.track.duration))
    }

    /// Whether `copy` may start the song (`policy::may_start`).
    fn may_start(&self, f: &Fetch, copy: &SourceRef) -> bool {
        self.known(f, copy)
            .is_some_and(|s| policy::may_start(&self.rules.get().stream, &s, f.track.duration))
    }

    /// YouTube's copy (`youtube`) has its first bytes here: whether the policy starts the song on
    /// Soulseek's side instead, its bytes here at the same moment (`policy::start`).
    fn soulseek_starts(&self, f: &Fetch, youtube: &SourceRef, soulseek: Option<&Arc<Fetch>>) -> bool {
        let rules = self.rules.get();
        let duration = f.track.duration;
        let theirs = soulseek
            .map(|u| u.now())
            .filter(sent)
            .and_then(|p| p.source.filter(|s| !youtube::is_youtube(&s.source_ref())));
        let (Some(theirs), Some(ours)) = (theirs, self.known(f, youtube)) else {
            return false;
        };
        let ready: Vec<Ranked> = sources::order(
            vec![
                self.sources.rank_for(theirs.clone(), rules.stream.max_kbps, duration),
                self.sources.rank_for(ours, rules.stream.max_kbps, duration),
            ],
            duration,
            &rules,
            Purpose::Stream,
        );
        let seen: Vec<policy::Seen> = ready
            .iter()
            .map(|r| policy::Seen {
                copy: &r.source,
                ready: true,
            })
            .collect();
        matches!(policy::start(&rules.stream, &seen, false, duration), policy::Start::Play(c) if c.is(&theirs.source_ref()))
    }

    /// Start fast once neither side has a copy that may start the song, YouTube's (`youtube`)
    /// held: the policy decides with the copies found (`policy::start`). `keepLooking` searches
    /// on, so YouTube's copy below the minimum never plays; `ask` offers it with Soulseek's.
    async fn race_below(
        &self,
        f: &Fetch,
        youtube: SourceRef,
        stop: &mut watch::Receiver<bool>,
    ) -> Result<PathBuf, Stop> {
        let rules = self.rules.get();
        let duration = f.track.duration;
        let (copies, _) = self.untried(f, &[], &rules);
        let ours = self.known(f, &youtube);
        let seen: Vec<policy::Seen> = copies
            .iter()
            .map(|r| &r.source)
            .chain(ours.as_ref())
            .map(|copy| policy::Seen { copy, ready: false })
            .collect();
        match policy::start(&rules.stream, &seen, false, duration) {
            policy::Start::Search | policy::Start::Fail => self.soulseek_then_youtube(f, None, false, stop).await,
            policy::Start::Ask => self.ask_among(f, Some(&youtube), stop).await,
            policy::Start::Play(_) | policy::Start::Wait => self.youtube_plays(f, youtube, stop).await,
        }
    }

    /// `ask` among the copies found, YouTube's (`youtube`) with Soulseek's, in the profile's order;
    /// the listener's choice plays.
    async fn ask_among(
        &self,
        f: &Fetch,
        youtube: Option<&SourceRef>,
        stop: &mut watch::Receiver<bool>,
    ) -> Result<PathBuf, Stop> {
        let rules = self.rules.get();
        let (mut choices, _) = self.untried(f, &[], &rules);
        choices.extend(
            youtube
                .and_then(|y| self.known(f, y))
                .map(|s| self.sources.rank_for(s, rules.stream.max_kbps, f.track.duration)),
        );
        let choices = sources::order(choices, f.track.duration, &rules, Purpose::Stream);
        let chosen = self.ask(f, choices, stop).await?;
        self.soulseek(f, Some(chosen.source_ref()), stop.clone()).await
    }

    /// Soulseek's side of the race becomes the player's fetch, before `f` sent a byte: it takes
    /// `f`'s place, a response waiting on `f` serves it instead, and its events name `f`'s job,
    /// so the player follows it. `f` ends (`why`, for Activity).
    fn hand_over(&self, f: &Fetch, u: &Arc<Fetch>, why: &str) {
        if let (Ok(mut live), Ok(mut upgrades)) = (self.live.lock(), self.upgrades.lock()) {
            upgrades.retain(|_, other| !Arc::ptr_eq(other, u));
            let ours = live.get(&f.track.id).is_some_and(|l| std::ptr::eq(Arc::as_ptr(l), f));
            if ours {
                live.insert(f.track.id.clone(), u.clone());
            }
        }
        u.racing.store(false, Ordering::Relaxed);
        let _ = u.replaces.set(f.job_id.clone());
        let (streaming, prefetch) = want(f, |w| {
            let had = (w.streaming, w.prefetch);
            w.streaming = false;
            w.prefetch = false;
            had
        });
        want(u, |w| {
            w.streaming |= streaming;
            w.prefetch |= prefetch;
        });
        let _ = f.successor.set(u.clone());
        self.update(f, |p| p.note = Some(why.to_owned()));
        tracing::info!(track = %f.track.id, from = f.job_id, to = u.job_id, why, "the player's fetch is Soulseek's copy");
        if streaming {
            self.bus.publish("stream", self.player_event(u, &u.now()));
            // The policy may know a better copy than the one that won (YouTube's among them).
            self.better_later(u);
        }
    }

    /// The song's Soulseek search, in the player's lane, while YouTube's lookup runs: the one
    /// search the fallback or the upgrade would make after it (they take its copies, or join it
    /// while it runs). Not when copies are known or the song was searched since `f` began.
    fn search_beside(&self, f: &Fetch) {
        if self.sources.known(&f.track) || self.sources.searched_since(&f.track.id, f.opened) {
            return;
        }
        let (searches, track) = (self.searches.clone(), f.track.clone());
        tokio::spawn(async move {
            if let Err(e) = searches.resolve(&track).await {
                tracing::debug!(error = %e, track = %track.id, "searching Soulseek beside YouTube");
            }
        });
    }

    /// The YouTube copy, and when its sound turns out not to be the song, the next one `find`
    /// gives (the first is dropped for the song by then): a new attempt, which the player
    /// reopens the song on.
    async fn youtube_plays(
        &self,
        f: &Fetch,
        first: SourceRef,
        stop: &mut watch::Receiver<bool>,
    ) -> Result<PathBuf, Stop> {
        let mut copy = first;
        for left in (0..YOUTUBE_COPIES).rev() {
            match self.youtube_attempt(f, &copy, true, stop).await {
                Err(Stop::Failed(why)) if why == NOT_THE_SONG && left > 0 => {
                    tracing::info!(track = %f.track.id, copy = copy.path, "moving the song to the next YouTube copy");
                    copy = self.youtube_copy(f, stop).await?;
                }
                done => return done,
            }
        }
        Err(Stop::Failed(NOT_THE_SONG.into()))
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
            Err(Stop::HandedOver) => return Err(Stop::HandedOver),
        };
        if !youtube_after {
            return Err(Stop::Failed(why));
        }
        tracing::info!(track = %f.track.id, why, "playing from YouTube instead");
        let copy = match self.youtube_copy(f, stop).await {
            Ok(copy) => copy,
            Err(Stop::Failed(yt)) => return Err(Stop::Failed(format!("{why}; YouTube: {yt}"))),
            Err(other) => return Err(other),
        };
        // YouTube's copy is judged as any (`policy`): one below the minimum plays only under
        // `playBest`. Otherwise Soulseek, only slow, goes on; with nothing there, `keepLooking`
        // fails as Soulseek's search did, and `ask` offers YouTube's copy with Soulseek's.
        if !self.may_start(f, &copy) {
            f.on_youtube.store(false, Ordering::Relaxed);
            let rules = self.rules.get();
            if gave_way {
                tracing::info!(track = %f.track.id, "YouTube's copy is below the streaming minimum: Soulseek goes on");
                return self.soulseek(f, None, stop.clone()).await;
            }
            return match rules.stream.below {
                StreamBelow::Ask => self.ask_among(f, Some(&copy), stop).await,
                _ => Err(Stop::Failed(format!(
                    "{why}; YouTube's copy is below your streaming minimum ({})",
                    rules.stream.minimum
                ))),
            };
        }
        match self.youtube_plays(f, copy, stop).await {
            Err(Stop::Failed(yt)) => Err(Stop::Failed(format!("{why}; YouTube: {yt}"))),
            other => other,
        }
    }

    /// The song's YouTube copy, by the listener's format and the streaming cap ("finding").
    async fn youtube_copy(&self, f: &Fetch, stop: &mut watch::Receiver<bool>) -> Result<SourceRef, Stop> {
        f.on_youtube.store(true, Ordering::Relaxed);
        self.update(f, |p| p.status = Status::Finding);
        let wanted = YoutubeWant::Stream {
            max_kbps: self.rules.get().stream.max_kbps,
        };
        // A preload's lookup waits behind the player's (and a `find` of the same song raises it).
        let later = want(f, |w| w.prefetch_only());
        let lookup = async {
            match later {
                true => self.youtube.find_later(&f.track, wanted).await,
                false => self.youtube.find(&f.track, wanted).await,
            }
        };
        let found = tokio::select! {
            r = lookup => r,
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
    /// With `check`, the attempt ends only once the copy's sound check says it is the song (an
    /// official copy is checked while it plays, ADR-0016); one that isn't ends it with
    /// `NOT_THE_SONG`.
    async fn youtube_attempt(
        &self,
        f: &Fetch,
        copy_ref: &SourceRef,
        check: bool,
        stop: &mut watch::Receiver<bool>,
    ) -> Result<PathBuf, Stop> {
        f.on_youtube.store(true, Ordering::Relaxed);
        let failed = |e: Error| Stop::Failed(reason(&e));
        let (Some(copy), Some(source)) = (youtube::Copy::of(copy_ref), bare_source(copy_ref, f.track.duration)) else {
            return Err(Stop::Failed(format!("{} is not a YouTube copy", copy_ref.path)));
        };
        let attempt = f.now().attempt + 1;
        let below = !policy::passes(&self.rules.get().stream, &source, f.track.duration);
        let begin = |p: &mut Progress| {
            p.attempt = attempt;
            p.source = Some(source.clone());
            p.below = below;
            p.choices.clear();
            p.ranges = Ranges::default();
            p.speed = 0;
            p.queue_pos = None;
            p.path = None;
            p.error = None;
        };
        if let Some(e) = self.cache.finished(&f.track.id, copy_ref).await.map_err(failed)? {
            self.update(f, |p| {
                begin(p);
                p.ranges = Ranges::front(copy_ref.size);
                p.path = Some(e.path.clone());
                p.status = Status::Streaming;
            });
            return self.youtube_checked(f, copy_ref, check, e.path, stop).await;
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
            ranges: None,
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
                    return self.youtube_checked(f, copy_ref, check, dest.clone(), stop).await;
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
                p.ranges = Ranges::front(received);
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

    /// The copy's file once its sound check (if one runs) says it is the song; a copy that
    /// isn't leaves the cache.
    async fn youtube_checked(
        &self,
        f: &Fetch,
        copy_ref: &SourceRef,
        check: bool,
        path: PathBuf,
        stop: &mut watch::Receiver<bool>,
    ) -> Result<PathBuf, Stop> {
        if !check {
            return Ok(path);
        }
        let verdict = tokio::select! {
            v = self.youtube.checked(&f.track, copy_ref, &path) => v,
            () = stopped(stop) => return Err(Stop::Cancelled),
        };
        if verdict == Some(false) {
            if let Err(e) = self.cache.forget(copy_ref).await {
                tracing::warn!(error = %e, "dropping a YouTube copy that isn't the song");
            }
            return Err(Stop::Failed(NOT_THE_SONG.into()));
        }
        if verdict.is_none() {
            self.update(f, |p| p.unverified = true);
        }
        Ok(path)
    }

    /// The player's audio failed while it followed job `job_id` (ADR-0008: the job is the one
    /// record of a song's outcome). A job that failed answers its own state, its reason in it;
    /// one whose copy arrived records that `device` can't play it, for Activity, and answers
    /// that. Without a job id (the audio failed before the player heard of its job), the song's
    /// live job, else its latest. `None`: a job the hub no longer knows.
    pub fn output_error(&self, track: &TrackId, job_id: Option<&str>, device: &str) -> Option<serde_json::Value> {
        let is = |f: &Arc<Fetch>| match job_id {
            Some(id) => f.job_id == id,
            None => f.track.id == *track,
        };
        let f = match job_id {
            Some(_) => self.all().into_iter().find(is),
            None => self.live.lock().ok()?.get(track).cloned(),
        }
        .or_else(|| self.ended_fetches.lock().ok()?.iter().rev().find(|f| is(f)).cloned())?;
        if f.now().status != Status::Failed {
            let format = f.now().source.map(|s| s.facts.format).unwrap_or_default();
            let on = if device.is_empty() { "this player" } else { device };
            let why = format!("this copy can't be played in {on} ({format})");
            tracing::info!(track = %f.track.id, job = f.job_id, why, "a device can't play the copy");
            self.update(&f, |p| p.unplayable = Some(why.clone()));
        }
        Some(stream_event(&f, &f.now()))
    }

    /// A peer's copy the listener picked (`?peer=&path=`), as lull knows it: from the song's
    /// searches, else its finished file in the stream cache. A pick stays with its queue entry
    /// for as long as the queue does, and searches are kept 6 hours: before (2026-11-12), a song
    /// picked one day was refused the next ("not a known source") although all of it was here,
    /// and the player took the refusal for a copy it couldn't play.
    pub async fn picked(&self, track: &Track, wanted: &SourceRef) -> Result<Option<SourceRef>> {
        if let Some(s) = self.sources.get(track, wanted) {
            return Ok(Some(s.source_ref()));
        }
        Ok(self
            .cache
            .complete(&track.id)
            .await?
            .into_iter()
            .find(|e| e.source.user == wanted.user && e.source.path == wanted.path)
            .map(|e| e.source))
    }

    /// The copy the song's live fetch has now.
    pub fn playing_copy(&self, track: &TrackId) -> Option<SourceRef> {
        self.live(track)?.now().source.map(|s| s.source_ref())
    }

    /// The listener said a copy isn't the song ("Right song? No", ADR-0016): it leaves the cache
    /// and the song's live fetch lets go of it, so the player's next opening takes the next copy.
    pub async fn refused(&self, track: &TrackId, copy: &SourceRef) -> Result<()> {
        if let Some(f) = self.live(track).filter(|f| holds(f, copy)) {
            self.forget(&f);
        }
        self.cache.forget(copy).await
    }

    /// A Soulseek copy that arrived is checked by its sound, out of the way (ADR-0016): one that
    /// isn't the song is never offered for it again and leaves the cache. The play goes on.
    /// When it is the better copy beside the song the player streams, it leaves that place and
    /// the next copy worth switching to is fetched (`better_later`). Before (2026-11-11), it
    /// held the song's one upgrade: the hub went on offering a copy it knew wasn't the song, and
    /// a right copy shared by other peers was never fetched during the play.
    fn check_soulseek_copy(&self, f: &Arc<Fetch>, path: &std::path::Path) {
        let Some(copy) = f.now().source.map(|s| s.source_ref()) else {
            return;
        };
        if youtube::is_youtube(&copy) {
            return;
        }
        let (this, f, path) = (self.clone(), f.clone(), path.to_owned());
        tokio::spawn(async move {
            let track = &f.track;
            if this.youtube.check_file(track, &path).await != Some(false) {
                return;
            }
            tracing::info!(track = %track.id, peer = copy.user, path = copy.path, "a Soulseek copy isn't the song by its sound");
            this.sources.mark_wrong(&track.id, &copy);
            if let Err(e) = this.cache.forget(&copy).await {
                tracing::warn!(error = %e, "dropping a Soulseek copy that isn't the song");
            }
            if let Some(live) = this.played_beside(&f) {
                this.forget(&f);
                this.better_later(&live);
            }
        });
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
            // A copy of YouTube's (the better copy the policy named, or the listener's choice
            // when asked) downloads as YouTube's always do; failing, the next copy is tried.
            let got = if youtube::is_youtube(&next.source_ref()) {
                match self.youtube_attempt(f, &next.source_ref(), true, &mut stop).await {
                    Err(Stop::Failed(why)) => Err(Ok(Skip::Unavailable(why))),
                    Err(other) => Err(Err(other)),
                    Ok(path) => Ok(path),
                }
            } else {
                self.legs(f, attempt, &next, faster, &mut stop).await
            };
            match got {
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
                .get(&f.track, pick)
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
                // One beside YouTube's lookup that ended since the fetch began is its search.
                if !self.sources.searched_since(&f.track.id, f.opened) {
                    self.find(f, stop).await?;
                }
                searches += 1;
                continue;
            }
            let peers = copies.len() + tried.len();
            self.update(f, |p| p.peers_found = peers);
            if let Some(passing) = copies.iter().find(|r| r.meets && !r.above_max) {
                return Ok(passing.source.clone());
            }
            // A copy above the streaming cap only when none within it may start the song.
            let within_may_start = copies
                .iter()
                .any(|r| !r.above_max && policy::may_start(&rules.stream, &r.source, f.track.duration));
            if !within_may_start && let Some(passing) = copies.iter().find(|r| r.meets) {
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
            if f.racing.load(Ordering::Relaxed) && !policy::may_start(&rules.stream, &best.source, f.track.duration) {
                // Racing YouTube: a copy below the minimum races only under `playBest`.
                return Err(Stop::Failed(format!(
                    "no Soulseek copy meets your streaming minimum ({})",
                    rules.stream.minimum
                )));
            }
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
        let mut searched = false;
        loop {
            let targets = tokio::select! {
                t = self.targets(&f.track, bar, tried) => t,
                () = stopped(stop) => return Err(Stop::Cancelled),
            };
            if let Some(best) = targets.into_iter().next() {
                return Ok(best.source);
            }
            if searched {
                return Err(Stop::Failed("no better copy than the one playing turned up".into()));
            }
            self.update(f, |p| p.status = Status::Finding);
            tokio::select! {
                r = self.searches.resolve_again_since(&f.track, Lane::Prefetch, f.opened) => r.map_err(|e| Stop::Failed(reason(&e)))?,
                () = stopped(stop) => return Err(Stop::Cancelled),
            }
            searched = true;
        }
    }

    /// Untried copies in the profile's order, and how many were left out for a long queue.
    fn untried(&self, f: &Fetch, tried: &[SourceRef], rules: &Rules) -> (Vec<Ranked>, usize) {
        let (copies, queued_out): (Vec<Ranked>, Vec<Ranked>) = self
            .sources
            .ordered(&f.track, rules, f.purpose)
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

    /// Applies a change and publishes `stream`, `transfer` and `job` events when it matters.
    fn update(&self, f: &Fetch, change: impl FnOnce(&mut Progress)) {
        let before = f.now();
        f.progress.send_modify(change);
        let p = f.now();
        let moved = p.status != before.status
            || p.ranges != before.ranges
            || p.ahead != before.ahead
            || p.queue_pos != before.queue_pos
            || p.attempt != before.attempt
            || p.failed_peers.len() != before.failed_peers.len()
            || p.unverified != before.unverified
            || p.unplayable != before.unplayable
            || p.note != before.note;
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
    /// Start fast: YouTube and Soulseek at once, whichever sends first plays.
    Race,
}

fn route(
    purpose: Purpose,
    pick: Option<&SourceRef>,
    yt: &youtube::Settings,
    prefetch_only: bool,
    beside: bool,
) -> Route {
    if let Some(copy) = pick.filter(|p| youtube::is_youtube(p)) {
        return Route::Youtube(copy.clone());
    }
    // A copy beside the player's (a better copy, start fast's Soulseek side) is a Soulseek one.
    if purpose != Purpose::Stream || pick.is_some() || beside {
        return Route::Soulseek;
    }
    match yt.mode {
        Mode::Off => Route::Soulseek,
        Mode::Fallback => Route::SoulseekThenYoutube(Some(Duration::from_secs(u64::from(yt.wait_s)))),
        // A preload gets Soulseek's copy, YouTube's only when Soulseek has none; the song's
        // YouTube match is warmed meanwhile, so playing it still starts at once.
        Mode::StartFast if prefetch_only => Route::SoulseekThenYoutube(None),
        Mode::StartFast => Route::Race,
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
pub(crate) fn bare_source(r: &SourceRef, duration: Option<u32>) -> Option<Source> {
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

/// Cached copies, best first by `rank` (lowest first), then the most recently played.
fn best_first<K: Ord>(mut entries: Vec<Entry>, rank: impl Fn(&Entry) -> K) -> Vec<Entry> {
    entries.sort_by_cached_key(|e| (rank(e), -e.used_at));
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

/// Whether the song's first bytes are here to play: what the player's response waits for.
fn sent(p: &Progress) -> bool {
    p.path.is_some() && p.ranges.has(0)
}

/// Start fast's race: resolves with whether Soulseek's side (`u`) sent the song's first bytes
/// (true), or ended without them, or turned to YouTube itself (false). Never without a side.
async fn soulseek_sends(u: Option<Arc<Fetch>>) -> bool {
    let Some(u) = u else {
        return std::future::pending().await;
    };
    let mut progress = u.progress();
    let first = progress.wait_for(|p| p.ended() || sent(p)).await.map(|p| p.clone());
    first.is_ok_and(|p| (sent(&p) || p.status == Status::Done) && !u.on_youtube.load(Ordering::Relaxed))
}

/// Why the player couldn't switch to the better copy mid-song (`POST /api/stream/{id}/not-switched`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NotSwitched {
    /// The two copies didn't line up closely enough to switch without a jump.
    Mismatch,
    /// The song was too near its end.
    NearEnd,
    /// The player can't play two copies side by side (the Android app's own player).
    CantSwitch,
    /// It was tried and didn't work out (the player was busy, the copy too slow to start).
    Failed,
}

impl NotSwitched {
    fn words(self) -> &'static str {
        match self {
            NotSwitched::Mismatch => "the two copies couldn't be lined up closely enough to switch without a jump",
            NotSwitched::NearEnd => "the song was nearly over",
            NotSwitched::CantSwitch => "this player can't line two copies up",
            NotSwitched::Failed => "switching didn't work out",
        }
    }
}

/// Where a fetch is a running fetch in `map`, not one that failed or was cancelled.
fn running(map: &Mutex<HashMap<TrackId, Arc<Fetch>>>, track: &TrackId) -> Option<Arc<Fetch>> {
    let m = map.lock().ok()?;
    m.get(track)
        .filter(|f| !matches!(f.now().status, Status::Failed | Status::Cancelled))
        .cloned()
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
        "received": p.received(),
        "ranges": p.ranges.fractions(p.size()),
        "size": p.size(),
        "speed": p.speed,
        "attempt": p.attempt,
        "failedPeers": p.failed_peers,
        "error": p.error,
        "belowMinimum": p.below,
        "seekAheadS": p.ahead,
        "youtube": p.source.as_ref().is_some_and(|s| youtube::is_youtube(&s.source_ref())),
        "unverified": p.unverified,
        "unplayable": p.unplayable,
        "replaces": f.replaces.get(),
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
        "received": p.received(),
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
        "unplayable": p.unplayable,
        "note": p.note,
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
            section: None,
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
        assert_eq!(player(&yt, false), Route::Race);
        assert_eq!(
            player(&yt, true),
            Route::SoulseekThenYoutube(None),
            "a preload fetches Soulseek's copy; YouTube only when there is none"
        );
        assert_eq!(
            route(Purpose::Stream, None, &yt, false, true),
            Route::Soulseek,
            "a copy beside the player's (a better copy, start fast's Soulseek side) is a Soulseek one, whatever the mode"
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
            section: None,
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
            ranges: Ranges::front(received),
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
    fn a_later_play_takes_the_best_cached_copy_not_the_latest() {
        let entry = |user: &str, ext: &str, used_at| Entry {
            track: TrackId("t".into()),
            source: peer_copy(user, ext, 9_000_000).source_ref(),
            path: PathBuf::from(format!("{user}.{ext}")),
            complete: true,
            used_at,
            ranges: None,
        };
        let yt = Entry {
            source: youtube::Copy {
                video: "LUjGtyYEi90".into(),
                format: "251".into(),
                ext: "webm".into(),
                section: None,
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
            ["first", "second", youtube::PEER, "below"],
            "passing before below whatever the source, Soulseek before YouTube, then the profile's order, whatever was played last"
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
        use crate::rules::{Current, Rules};
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

        async fn fetcher(fake: &Fake, rules: Rules) -> Result<Fetcher> {
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
                Current::from(rules),
                Arc::new(NoYoutube),
            ))
        }

        /// How the song's fetch ended: the one the player follows, through start fast's race
        /// handing it to Soulseek's copy (`last`).
        async fn ended(f: &Arc<Fetch>) -> Result<super::super::Progress> {
            let mut f = f.clone();
            loop {
                let mut rx = f.progress();
                let ended = tokio::time::timeout(Duration::from_secs(30), rx.wait_for(|p| p.ended())).await;
                let p = ended
                    .map_err(|_| Error::internal("the fetch never ended"))?
                    .map_err(|_| Error::internal("gone"))?
                    .clone();
                match f.successor() {
                    Some(next) => f = next,
                    None => return Ok(p),
                }
            }
        }

        /// The fetch that took over from `f`, and from it, and so on: the one the player follows.
        fn last(f: &Arc<Fetch>) -> Arc<Fetch> {
            let mut f = f.clone();
            while let Some(next) = f.successor() {
                f = next;
            }
            f
        }

        /// Two uploads of the song: `find` gives the first until it is dropped for the song, then
        /// the second. The first's sound isn't the song (ADR-0016).
        struct TwoUploads {
            finds: std::sync::atomic::AtomicUsize,
        }

        const SIZE: u64 = 40_000;

        fn upload(video: &str) -> SourceRef {
            lull_core::youtube::Copy {
                video: video.into(),
                format: "140".into(),
                ext: "m4a".into(),
                section: None,
            }
            .source_ref(SIZE)
        }

        #[async_trait]
        impl Youtube for TwoUploads {
            async fn find(&self, _: &Track, _: Want) -> Result<Option<Found>> {
                let n = self.finds.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Ok(Some(Found {
                    source: upload(if n == 0 { "wrongupload" } else { "rightupload" }),
                    title: "Song".into(),
                    channel: "Artist - Topic".into(),
                    official: true,
                }))
            }
            async fn download(&self, _: &SourceRef, dest: &std::path::Path) -> Result<()> {
                std::fs::write(dest, vec![7u8; SIZE as usize]).map_err(|e| Error::internal(e.to_string()))
            }
            async fn checked(&self, _: &Track, source: &SourceRef, _: &std::path::Path) -> Option<bool> {
                Some(!source.path.contains("wrongupload"))
            }
        }

        async fn with_youtube(fake: &Fake, rules: Rules, youtube: Arc<dyn Youtube>) -> Result<Fetcher> {
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
                Current::from(rules),
                youtube,
            ))
        }

        #[tokio::test]
        async fn an_official_youtube_copy_that_isnt_the_song_gives_way_to_the_next() -> Result<()> {
            let peer = Peer {
                per_tick: 32_768,
                overwrite: false,
                reported: None,
                settles: Duration::ZERO,
            };
            let fake = Fake::start(HashMap::new(), peer)
                .await
                .map_err(|e| Error::internal(e.to_string()))?;
            let mut rules = Rules::default();
            rules.youtube.mode = lull_core::youtube::Mode::StartFast;
            let youtube = Arc::new(TwoUploads {
                finds: Default::default(),
            });
            let fetcher = with_youtube(&fake, rules, youtube).await?;
            let p = ended(&fetcher.stream(track(), None).await?).await?;
            assert_eq!(p.status, Status::Done, "{:?}", p.error);
            let playing = p.source.map(|s| s.source_ref().path).unwrap_or_default();
            assert!(playing.contains("rightupload"), "{playing}");
            // A second attempt: the player reopens the song on it.
            assert_eq!(p.attempt, 2);
            Ok(())
        }

        #[tokio::test]
        async fn a_youtube_copy_the_listener_picked_plays_whatever_its_check_says() -> Result<()> {
            let peer = Peer {
                per_tick: 32_768,
                overwrite: false,
                reported: None,
                settles: Duration::ZERO,
            };
            let fake = Fake::start(HashMap::new(), peer)
                .await
                .map_err(|e| Error::internal(e.to_string()))?;
            let mut rules = Rules::default();
            rules.youtube.mode = lull_core::youtube::Mode::StartFast;
            let youtube = Arc::new(TwoUploads {
                finds: Default::default(),
            });
            let fetcher = with_youtube(&fake, rules, youtube).await?;
            let p = ended(&fetcher.stream(track(), Some(upload("wrongupload"))).await?).await?;
            assert_eq!(p.status, Status::Done, "{:?}", p.error);
            assert_eq!(p.attempt, 1);
            Ok(())
        }

        /// YouTube's lookup as it was live for a song it couldn't match (2026-11-06): it takes
        /// `LOOKUP` (several yt-dlp runs), then finds an upload of the song, or (`found` false) none.
        struct SlowLookup {
            found: bool,
        }

        const LOOKUP: Duration = Duration::from_secs(3);

        #[async_trait]
        impl Youtube for SlowLookup {
            async fn find(&self, _: &Track, _: Want) -> Result<Option<Found>> {
                tokio::time::sleep(LOOKUP).await;
                Ok(self.found.then(|| Found {
                    source: upload("rightupload"),
                    title: "Song".into(),
                    channel: "Artist - Topic".into(),
                    official: true,
                }))
            }
            async fn download(&self, _: &SourceRef, dest: &std::path::Path) -> Result<()> {
                std::fs::write(dest, vec![7u8; SIZE as usize]).map_err(|e| Error::internal(e.to_string()))
            }
        }

        async fn start_fast(found: bool) -> Result<(Fake, Fetcher)> {
            let peer = Peer {
                per_tick: 32_768,
                overwrite: false,
                reported: None,
                settles: Duration::ZERO,
            };
            let fake = Fake::start(HashMap::from([(REMOTE.to_owned(), vec![5u8; 40_000])]), peer)
                .await
                .map_err(|e| Error::internal(e.to_string()))?;
            let mut rules = Rules::default();
            rules.youtube.mode = lull_core::youtube::Mode::StartFast;
            let fetcher = with_youtube(&fake, rules, Arc::new(SlowLookup { found })).await?;
            Ok((fake, fetcher))
        }

        #[tokio::test]
        async fn a_song_youtube_cant_match_starts_on_soulseek_without_waiting_out_the_lookup() -> Result<()> {
            // Live: 久石譲's 風のとおり道 (Acoustic Version) took 15 s to start, YouTube's lookup
            // first and only then Soulseek's search. Now they run at once.
            let (fake, fetcher) = start_fast(false).await?;
            let t0 = std::time::Instant::now();
            let p = ended(&fetcher.stream(track(), None).await?).await?;
            let took = t0.elapsed();
            assert_eq!(p.status, Status::Done, "{:?}", p.error);
            assert_eq!(p.source.map(|s| s.user), Some("peer".to_owned()));
            assert_eq!(fake.searches().len(), 1, "one search: {:?}", fake.searches());
            // In order it took the lookup and the search, one after the other.
            assert!(
                took < LOOKUP + crate::fake_slskd::SEARCH_TAKES,
                "{took:?}: the search waited for YouTube's lookup"
            );
            Ok(())
        }

        #[tokio::test]
        async fn start_fasts_race_takes_the_search_that_ran_beside_youtubes_lookup() -> Result<()> {
            // Nothing known: the one search finds the peer's copy (1 s) while YouTube's lookup
            // takes 3 s, so Soulseek's copy starts the song and YouTube's never downloads.
            let (fake, fetcher) = start_fast(true).await?;
            let opened = fetcher.stream(track(), None).await?;
            let p = ended(&opened).await?;
            assert_eq!(p.status, Status::Done, "{:?}", p.error);
            assert_eq!(p.source.map(|s| s.user), Some("peer".to_owned()));
            assert_eq!(fake.searches().len(), 1, "no second search: {:?}", fake.searches());
            assert_eq!(opened.now().status, Status::Cancelled, "YouTube's side stopped");
            Ok(())
        }

        /// One official upload of the song, always the song.
        struct OneUpload;

        #[async_trait]
        impl Youtube for OneUpload {
            async fn find(&self, _: &Track, _: Want) -> Result<Option<Found>> {
                Ok(Some(Found {
                    source: upload("rightupload"),
                    title: "Song".into(),
                    channel: "Artist - Topic".into(),
                    official: true,
                }))
            }
            async fn download(&self, _: &SourceRef, dest: &std::path::Path) -> Result<()> {
                std::fs::write(dest, vec![7u8; SIZE as usize]).map_err(|e| Error::internal(e.to_string()))
            }
        }

        fn shared(user: &str, path: &str, size: u64, ext: &str, kbps: Option<u32>, lossless: bool) -> Source {
            Source {
                user: user.into(),
                path: path.into(),
                size,
                ext: ext.into(),
                facts: lull_core::model::FileFacts {
                    format: ext.to_uppercase(),
                    codec: ext.to_uppercase(),
                    lossless,
                    khz: Some(44.1),
                    bits: lossless.then_some(16),
                    kbps,
                    size_bytes: Some(size),
                },
                upload_speed: 2_000_000,
                free_slot: true,
                queue: 0,
                length: Some(200),
            }
        }

        #[tokio::test]
        async fn a_pick_kept_past_its_search_plays_its_finished_copy() -> Result<()> {
            // Live (2026-11-12): a copy of TLC's "No Scrubs" picked the day before, all of it in
            // the cache, was refused ("not a known source for this song"): the search that found
            // it was forgotten after 6 hours, and the queue kept the pick.
            let bytes: Vec<u8> = vec![5u8; 300_000];
            let peer = Peer {
                per_tick: 32_768,
                overwrite: false,
                reported: None,
                settles: Duration::ZERO,
            };
            let fake = Fake::start(HashMap::from([(REMOTE.to_owned(), bytes.clone())]), peer)
                .await
                .map_err(|e| Error::internal(e.to_string()))?;
            let db = lull_core::db::Db::memory()?;
            db.migrate(&crate::MIGRATIONS)?;
            let bus = Bus::new();
            let cache = Cache::new(db.clone(), Services::new(bus.clone()), fake.root.clone(), u64::MAX / 2);
            let peers = Peers::load(db.clone()).await?;
            // The hub as it was, then as it is a day later: the same cache, its searches forgotten.
            let hub = || {
                let sources = Sources::new(peers.clone(), Vec::new());
                Fetcher::new(
                    fake.slskd.clone(),
                    Arc::new(RwLock::new(fake.layout.clone())),
                    cache.clone(),
                    peers.clone(),
                    sources.clone(),
                    Searches::new(fake.slskd.clone(), bus.clone(), db.clone(), sources),
                    bus.clone(),
                    Current::from(Rules::default()),
                    Arc::new(NoYoutube),
                )
            };
            let yesterday = hub();
            let copy = shared("peer", REMOTE, bytes.len() as u64, "mp3", Some(320), false);
            yesterday.sources.add(HashMap::from([(track().id, vec![copy.clone()])]));
            let f = yesterday.stream(track(), Some(copy.source_ref())).await?;
            assert_eq!(ended(&f).await?.status, Status::Done);

            let today = hub();
            // The pick as the player's address names it: peer and path.
            let named = SourceRef {
                user: "peer".into(),
                path: REMOTE.into(),
                size: 0,
            };
            let pick = today
                .picked(&track(), &named)
                .await?
                .ok_or_else(|| Error::internal("the picked copy in the cache isn't known"))?;
            let f = today.stream(track(), Some(pick)).await?;
            let (chunk, _) = first_bytes(&f).await?;
            assert!(chunk.iter().all(|b| *b == 5), "the picked copy plays");
            assert_eq!(
                fake.starts_of("peer").len(),
                1,
                "from the cache: the peer wasn't asked again"
            );

            let elsewhere = SourceRef {
                user: "nobody".into(),
                path: REMOTE.into(),
                size: 0,
            };
            assert!(
                today.picked(&track(), &elsewhere).await?.is_none(),
                "a copy lull never had"
            );
            Ok(())
        }

        #[tokio::test]
        async fn a_picked_flac_that_plays_beside_start_fasts_upgrade_plays_on_with_one_transfer() -> Result<()> {
            // The owner's path (2026-11-05): start fast plays the YouTube copy, then the listener
            // picks a peer's FLAC in Pick a source. It played, then fetches from that peer failed
            // and started again about 1,400 times.
            const FLAC: &str = "Music\\Artist\\Album\\01 - Song.flac";
            let bytes: Vec<u8> = (0..1_500_000).map(|i| (i % 251) as u8).collect();
            let peer = Peer {
                per_tick: 16_384,
                overwrite: false,
                reported: None,
                settles: Duration::ZERO,
            };
            let fake = Fake::start(HashMap::from([(FLAC.to_owned(), bytes.clone())]), peer)
                .await
                .map_err(|e| Error::internal(e.to_string()))?;
            let mut rules = Rules::default();
            rules.youtube.mode = lull_core::youtube::Mode::StartFast;
            let fetcher = with_youtube(&fake, rules, Arc::new(OneUpload)).await?;
            let flac = shared("trisska", FLAC, bytes.len() as u64, "flac", None, true);
            fetcher.sources.add(HashMap::from([(track().id, vec![flac.clone()])]));
            // The YouTube copy plays; start fast fetches Soulseek's copy beside it.
            let yt = fetcher.stream(track(), None).await?;
            let _ = ended(&yt).await?;
            tokio::time::sleep(Duration::from_millis(300)).await;
            // The listener picks the FLAC.
            let picked = fetcher.stream(track(), Some(flac.source_ref())).await?;
            let p = ended(&picked).await?;
            assert_eq!(p.status, Status::Done, "{:?}", p.error);
            tokio::time::sleep(Duration::from_secs(2)).await;
            let starts = fake.starts_of("trisska");
            assert!(
                starts.len() <= 2,
                "the peer was asked {} times: {starts:?}",
                starts.len()
            );
            Ok(())
        }

        #[tokio::test]
        async fn a_picked_peer_that_stalls_falls_back_once_and_the_players_reopen_starts_nothing_new() -> Result<()> {
            // The picked peer stalls, the fetch falls back to another peer (a new attempt), and the
            // player reopens the song with its pick, as it does on every new attempt.
            const FLAC: &str = "Music\\Artist\\Album\\01 - Song.flac";
            let bytes: Vec<u8> = (0..600_000).map(|i| (i % 251) as u8).collect();
            let peer = Peer {
                per_tick: 16_384,
                overwrite: false,
                reported: None,
                settles: Duration::ZERO,
            };
            let fake = Fake::start_with(
                HashMap::from([(FLAC.to_owned(), bytes.clone())]),
                peer,
                HashMap::from([("trisska".to_owned(), 0)]),
            )
            .await
            .map_err(|e| Error::internal(e.to_string()))?;
            let rules = Rules {
                stall_timeout: 1,
                ..Rules::default()
            };
            let fetcher = with_youtube(&fake, rules, Arc::new(NoYoutube)).await?;
            let picked = shared("trisska", FLAC, bytes.len() as u64, "flac", None, true);
            let other = shared("goodpeer", FLAC, bytes.len() as u64, "flac", None, true);
            fetcher
                .sources
                .add(HashMap::from([(track().id, vec![picked.clone(), other])]));
            let mut f = fetcher.stream(track(), Some(picked.source_ref())).await?;
            let mut jobs = vec![f.job_id.clone()];
            let mut seen = 0;
            let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
            while !f.now().ended() && tokio::time::Instant::now() < deadline {
                let mut rx = f.progress();
                let _ = tokio::time::timeout(Duration::from_millis(500), rx.changed()).await;
                let attempt = f.now().attempt;
                if attempt > seen {
                    seen = attempt;
                    // The player's reopen: the same song, its pick still in the address.
                    f = fetcher.stream(track(), Some(picked.source_ref())).await?;
                    if !jobs.contains(&f.job_id) {
                        jobs.push(f.job_id.clone());
                        seen = 0;
                    }
                }
            }
            assert!(jobs.len() == 1, "{} fetches started: {jobs:?}", jobs.len());
            let p = f.now();
            assert_eq!(p.status, Status::Done, "{:?}", p.error);
            Ok(())
        }

        /// YouTube as it answers: its lookup takes `lookup`, then an upload of the song is found
        /// (or, `found` false, none). Counts its downloads.
        struct Timed {
            lookup: Duration,
            found: bool,
            /// Its copy is Opus 160 (format 251), else AAC 128 (140).
            opus: bool,
            downloads: std::sync::atomic::AtomicUsize,
        }

        impl Timed {
            fn new(lookup: Duration, found: bool) -> Arc<Timed> {
                Arc::new(Timed {
                    lookup,
                    found,
                    opus: false,
                    downloads: Default::default(),
                })
            }

            fn opus(lookup: Duration) -> Arc<Timed> {
                Arc::new(Timed {
                    lookup,
                    found: true,
                    opus: true,
                    downloads: Default::default(),
                })
            }
        }

        #[async_trait]
        impl Youtube for Timed {
            async fn find(&self, _: &Track, _: Want) -> Result<Option<Found>> {
                tokio::time::sleep(self.lookup).await;
                let source = match self.opus {
                    true => lull_core::youtube::Copy {
                        video: "rightupload".into(),
                        format: "251".into(),
                        ext: "webm".into(),
                        section: None,
                    }
                    .source_ref(SIZE),
                    false => upload("rightupload"),
                };
                Ok(self.found.then(|| Found {
                    source,
                    title: "Song".into(),
                    channel: "Artist - Topic".into(),
                    official: true,
                }))
            }
            async fn download(&self, _: &SourceRef, dest: &std::path::Path) -> Result<()> {
                self.downloads.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                std::fs::write(dest, vec![7u8; SIZE as usize]).map_err(|e| Error::internal(e.to_string()))
            }
        }

        /// The first bytes a response to the player's request gets, and how long they took.
        async fn first_bytes(f: &Arc<Fetch>) -> Result<(Vec<u8>, Duration)> {
            use futures_util::StreamExt;
            let t0 = std::time::Instant::now();
            let response = crate::stream::serve(f.clone(), &axum::http::HeaderMap::new()).await?;
            let mut body = response.into_body().into_data_stream();
            let chunk = tokio::time::timeout(Duration::from_secs(30), body.next())
                .await
                .map_err(|_| Error::internal("no bytes"))?
                .ok_or_else(|| Error::internal("an empty response"))?
                .map_err(|e| Error::internal(e.to_string()))?;
            Ok((chunk.to_vec(), t0.elapsed()))
        }

        async fn racing(
            files: HashMap<String, Vec<u8>>,
            rules: Rules,
            youtube: Arc<dyn Youtube>,
        ) -> Result<(Fake, Fetcher)> {
            let peer = Peer {
                per_tick: 16_384,
                overwrite: false,
                reported: None,
                settles: Duration::ZERO,
            };
            let fake = Fake::start(files, peer)
                .await
                .map_err(|e| Error::internal(e.to_string()))?;
            let fetcher = with_youtube(&fake, rules, youtube).await?;
            Ok((fake, fetcher))
        }

        /// Start fast, with a minimum YouTube's copy here (AAC 128) passes: it may start the song.
        fn start_fast_rules() -> Rules {
            let mut rules = Rules::default();
            rules.youtube.mode = lull_core::youtube::Mode::StartFast;
            rules.stream.minimum.kbps = 128;
            rules
        }

        #[tokio::test]
        async fn start_fast_plays_a_known_soulseek_copy_without_waiting_for_youtubes_lookup() -> Result<()> {
            // Live (2026-11-07): Beach House's "Baseball Diamond" took 16 s to start. Its Soulseek
            // copies were known from an earlier search, but the hub waited for YouTube's lookup
            // (about 15 s) before it asked a peer. Now both start at once and the first to send plays.
            let bytes: Vec<u8> = vec![5u8; 300_000];
            let youtube = Timed::new(Duration::from_secs(15), true);
            let (_fake, fetcher) = racing(
                HashMap::from([(REMOTE.to_owned(), bytes.clone())]),
                start_fast_rules(),
                youtube.clone(),
            )
            .await?;
            let known = shared("peer", REMOTE, bytes.len() as u64, "mp3", Some(320), false);
            fetcher.sources.add(HashMap::from([(track().id, vec![known])]));
            fetcher.sources.mark_searched(&track().id);

            let opened = fetcher.stream(track(), None).await?;
            let (chunk, took) = first_bytes(&opened).await?;
            assert!(chunk.iter().all(|b| *b == 5), "the response serves the peer's copy");
            assert!(
                took < Duration::from_secs(3),
                "{took:?}: the song waited for YouTube's lookup"
            );

            let playing = last(&opened);
            assert!(
                !Arc::ptr_eq(&playing, &opened),
                "Soulseek's side became the player's fetch"
            );
            let event = fetcher.event(&playing);
            assert_eq!(event["peer"], "peer");
            assert_eq!(event["replaces"], serde_json::json!(opened.job_id), "{event}");
            assert_eq!(opened.now().status, Status::Cancelled);
            assert_eq!(
                opened.now().note.as_deref(),
                Some("Soulseek's copy started the song first"),
                "Activity says why YouTube's side stopped"
            );
            let p = ended(&opened).await?;
            assert_eq!(p.status, Status::Done, "{:?}", p.error);
            assert_eq!(
                youtube.downloads.load(std::sync::atomic::Ordering::Relaxed),
                0,
                "YouTube's copy was never downloaded"
            );
            Ok(())
        }

        /// The job a player follows through `stream` events, as `LullRules.followEvent` does
        /// (web/player-rules.js): the first it sees, then that job's, or one that took over from
        /// it. Panics on an event of `playing` it would drop.
        fn followed(events: &[serde_json::Value], playing: &str) -> Option<serde_json::Value> {
            let (mut job, mut last) = (None::<String>, None);
            for ev in events {
                let id = ev["jobId"].as_str().map(str::to_owned);
                let follows = match &job {
                    None => true,
                    Some(j) => id.as_ref() == Some(j) || ev["replaces"].as_str() == Some(j.as_str()),
                };
                assert!(
                    follows || id.as_deref() != Some(playing),
                    "the player drops the copy playing: following {job:?}, it got {ev}"
                );
                if follows {
                    job = id;
                    last = Some(ev.clone());
                }
            }
            last
        }

        #[tokio::test]
        async fn two_openings_at_once_are_one_fetch_and_the_player_follows_the_copy_playing() -> Result<()> {
            // Live (2026-11-08): Grouper's "Poison Tree" played on from Soulseek while the player
            // showed what had arrived frozen and kept a seek to 2:29 past 2:32. Two openings at once
            // had each started a fetch; the race handed the song to Soulseek's copy as the first
            // one's, and the player, following the second, dropped all of that copy's events.
            let bytes: Vec<u8> = vec![5u8; 2_000_000];
            let (_fake, fetcher) = racing(
                HashMap::from([(REMOTE.to_owned(), bytes.clone())]),
                start_fast_rules(),
                Timed::new(Duration::from_secs(15), true),
            )
            .await?;
            let known = shared("peer", REMOTE, bytes.len() as u64, "mp3", Some(320), false);
            fetcher.sources.add(HashMap::from([(track().id, vec![known])]));
            fetcher.sources.mark_searched(&track().id);
            let mut rx = fetcher.bus.subscribe();

            let (a, b) = tokio::join!(fetcher.stream(track(), None), fetcher.stream(track(), None));
            let (a, b) = (a?, b?);
            assert!(
                Arc::ptr_eq(&a, &b),
                "one song, one fetch: {} and {}",
                a.job_id,
                b.job_id
            );
            let (chunk, _) = first_bytes(&a).await?;
            assert!(chunk.iter().all(|x| *x == 5), "Soulseek's copy plays");
            let playing = last(&a);
            assert!(!Arc::ptr_eq(&playing, &a), "Soulseek's side won the race");
            assert_eq!(ended(&playing).await?.status, Status::Done);

            let mut events = Vec::new();
            while let Ok(ev) = rx.try_recv() {
                if ev.topic == "stream" {
                    events.push(ev.data);
                }
            }
            let seen = followed(&events, &playing.job_id)
                .ok_or_else(|| Error::internal("the player heard nothing of the song"))?;
            assert_eq!(seen["jobId"], serde_json::json!(playing.job_id), "{seen}");
            assert_eq!(
                seen["ranges"],
                serde_json::json!([[0.0, 1.0]]),
                "the player saw all of it arrive"
            );
            assert!(seen["speed"].as_u64().is_some_and(|s| s > 0), "and how fast: {seen}");
            Ok(())
        }

        #[tokio::test]
        async fn youtube_sending_first_plays_and_soulseeks_copy_carries_on_as_the_better_copy() -> Result<()> {
            // Nothing known: Soulseek's copy needs the search (1 s), YouTube's lookup is quick.
            // The owner streams at most 320 kbps: Soulseek's side takes the MP3 320, not the FLAC.
            // The FLAC is on another album: a peer sharing one song twice on one album counts once
            // in a search (`search::merge` keeps the first file it lists, and a folder's format tag
            // such as "(Lossless)" names no other album), so both there would leave whichever the
            // stand-in happened to list first.
            const FLAC: &str = "Music\\Artist\\Collected\\01 - Song.flac";
            let flac_bytes: Vec<u8> = (0..900_000).map(|i| (i % 251) as u8).collect();
            let mp3_bytes: Vec<u8> = (0..300_000).map(|i| (i % 241) as u8).collect();
            let mut rules = start_fast_rules();
            rules.stream.max_kbps = 320;
            let (fake, fetcher) = racing(
                HashMap::from([(FLAC.to_owned(), flac_bytes), (REMOTE.to_owned(), mp3_bytes)]),
                rules,
                Timed::new(Duration::ZERO, true),
            )
            .await?;
            let yt = fetcher.stream(track(), None).await?;
            let (chunk, _) = first_bytes(&yt).await?;
            assert!(chunk.iter().all(|b| *b == 7), "YouTube's copy plays");
            assert!(yt.successor().is_none());
            let up = fetcher
                .upgrade(&track().id)
                .ok_or_else(|| Error::internal("Soulseek's copy stopped"))?;
            let u = ended(&up).await?;
            assert_eq!(u.status, Status::Done, "{:?}", u.error);
            let found: Vec<String> = fetcher
                .sources
                .ordered(&track(), &fetcher.rules.get(), crate::rules::Purpose::Stream)
                .into_iter()
                .map(|r| r.source.ext)
                .collect();
            assert_eq!(
                found,
                ["mp3", "flac"],
                "the search found both, the one within the cap first"
            );
            assert_eq!(
                u.source.map(|s| s.ext).unwrap_or_default(),
                "mp3",
                "the copy within the cap"
            );
            let offer = fetcher.event(&yt);
            assert_eq!(offer["upgrade"]["peer"], "peer", "{offer}");
            assert_eq!(offer["upgrade"]["ready"], true);
            assert_eq!(fake.searches().len(), 1, "one search: {:?}", fake.searches());

            // The player switches: it follows the new job, which names the one it replaces, and
            // Activity says what happened.
            let copy = up
                .now()
                .source
                .map(|s| s.source_ref())
                .ok_or_else(|| Error::internal("no copy"))?;
            let switched = fetcher.adopt(&track().id, &copy, false)?;
            let event = fetcher.event(&switched);
            assert_eq!(event["replaces"], serde_json::json!(yt.job_id), "{event}");
            let note = switched.now().note.unwrap_or_default();
            assert!(
                note.starts_with("switched to ") && note.ends_with(" from peer"),
                "{note}"
            );
            Ok(())
        }

        #[tokio::test]
        async fn a_copy_above_the_streaming_cap_is_never_offered_mid_song() -> Result<()> {
            // Only a FLAC is shared and the cap is 320 kbps: Soulseek's side fetches it, as a
            // Soulseek play would with nothing within the cap, for later plays and saves; the
            // player is never offered it while YouTube's copy plays.
            const FLAC: &str = "Music\\Artist\\Album\\01 - Song.flac";
            let mut rules = start_fast_rules();
            rules.stream.max_kbps = 320;
            let (_fake, fetcher) = racing(
                HashMap::from([(FLAC.to_owned(), (0..900_000).map(|i| (i % 251) as u8).collect())]),
                rules,
                Timed::new(Duration::ZERO, true),
            )
            .await?;
            let yt = fetcher.stream(track(), None).await?;
            let (chunk, _) = first_bytes(&yt).await?;
            assert!(chunk.iter().all(|b| *b == 7), "YouTube's copy plays");
            let up = fetcher
                .upgrade(&track().id)
                .ok_or_else(|| Error::internal("Soulseek's copy stopped"))?;
            let mut rx = up.progress();
            let _ = tokio::time::timeout(Duration::from_secs(30), rx.wait_for(|p| p.source.is_some())).await;
            assert_eq!(up.now().source.map(|s| s.ext).unwrap_or_default(), "flac");
            for _ in 0..20 {
                assert!(fetcher.event(&yt)["upgrade"].is_null(), "offered a copy above the cap");
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Ok(())
        }

        #[tokio::test]
        async fn youtube_below_the_minimum_first_follows_the_below_rule_under_start_fast() -> Result<()> {
            // YouTube's AAC 128 is ready at once, below a 192 minimum; a passing MP3 320 from a
            // peer comes after it. At the start lull can't know a passing copy will come, so "none
            // is good enough yet", and `below` decides (policy.rs's table, run for real here).
            use lull_core::quality::StreamBelow;
            for below in [StreamBelow::PlayBest, StreamBelow::KeepLooking, StreamBelow::Ask] {
                let bytes: Vec<u8> = vec![5u8; 300_000];
                let youtube = Timed::new(Duration::ZERO, true);
                let mut rules = start_fast_rules();
                rules.stream.minimum.kbps = 192;
                rules.stream.below = below;
                let (_fake, fetcher) = racing(
                    HashMap::from([(REMOTE.to_owned(), bytes.clone())]),
                    rules,
                    youtube.clone(),
                )
                .await?;
                let known = shared("peer", REMOTE, bytes.len() as u64, "mp3", Some(320), false);
                fetcher.sources.add(HashMap::from([(track().id, vec![known])]));
                fetcher.sources.mark_searched(&track().id);

                let opened = fetcher.stream(track(), None).await?;
                let (chunk, _) = first_bytes(&opened).await?;
                let playing = last(&opened);
                let event = fetcher.event(&playing);
                if below == StreamBelow::PlayBest {
                    // The first ready plays, marked below; the passing copy is fetched beside it
                    // and offered as the better copy.
                    assert!(chunk.iter().all(|b| *b == 7), "{below:?}: YouTube's copy plays");
                    assert_eq!(event["belowMinimum"], true, "{below:?}: {event}");
                    let up = fetcher
                        .upgrade(&track().id)
                        .ok_or_else(|| Error::internal("no passing copy beside"))?;
                    assert_eq!(ended(&up).await?.status, Status::Done);
                    assert_eq!(fetcher.event(&playing)["upgrade"]["peer"], "peer", "{below:?}");
                } else {
                    // Only passing copies start the song: the peer's, and YouTube's is never fetched.
                    assert!(
                        chunk.iter().all(|b| *b == 5),
                        "{below:?}: the passing peer's copy plays"
                    );
                    assert_eq!(event["belowMinimum"], false, "{below:?}: {event}");
                    assert_eq!(ended(&playing).await?.status, Status::Done);
                    assert_eq!(
                        youtube.downloads.load(std::sync::atomic::Ordering::Relaxed),
                        0,
                        "{below:?}"
                    );
                }
            }
            Ok(())
        }

        #[tokio::test]
        async fn with_no_copy_passing_start_fast_follows_the_below_rule() -> Result<()> {
            // Nobody on Soulseek shares the song, and YouTube's AAC 128 is below a 192 minimum.
            let below = |rule| {
                let mut rules = start_fast_rules();
                rules.stream.minimum.kbps = 192;
                rules.stream.below = rule;
                rules
            };
            // `keepLooking`: only a passing copy plays, so YouTube's never does.
            let youtube = Timed::new(Duration::ZERO, true);
            let (_fake, fetcher) = racing(
                HashMap::new(),
                below(lull_core::quality::StreamBelow::KeepLooking),
                youtube.clone(),
            )
            .await?;
            let f = fetcher.stream(track(), None).await?;
            let p = ended(&f).await?;
            assert_eq!(p.status, Status::Failed, "{p:?}");
            assert_eq!(youtube.downloads.load(std::sync::atomic::Ordering::Relaxed), 0);

            // `playBest`: the best copy found plays, YouTube's here, marked below the minimum.
            let youtube = Timed::new(Duration::ZERO, true);
            let (_fake, fetcher) = racing(
                HashMap::new(),
                below(lull_core::quality::StreamBelow::PlayBest),
                youtube.clone(),
            )
            .await?;
            let f = fetcher.stream(track(), None).await?;
            let (chunk, _) = first_bytes(&f).await?;
            assert!(chunk.iter().all(|b| *b == 7), "YouTube's copy plays");
            assert_eq!(fetcher.event(&last(&f))["belowMinimum"], true, "and says it is below");

            // `ask`: YouTube's copy is one of the copies offered.
            let (_fake, fetcher) = racing(
                HashMap::new(),
                below(lull_core::quality::StreamBelow::Ask),
                Timed::new(Duration::ZERO, true),
            )
            .await?;
            let f = fetcher.stream(track(), None).await?;
            let mut rx = f.progress();
            let asked = tokio::time::timeout(Duration::from_secs(10), rx.wait_for(|p| p.status == Status::Choosing))
                .await
                .map_err(|_| Error::internal("the listener was never asked"))?
                .map(|p| p.choices.iter().map(|c| c.peer.clone()).collect::<Vec<_>>())
                .map_err(|e| Error::internal(e.to_string()))?;
            assert_eq!(asked, [lull_core::youtube::PEER]);
            Ok(())
        }

        #[tokio::test]
        async fn no_switch_mid_song_for_a_tiny_gain() -> Result<()> {
            // YouTube's AAC 128 starts the song; Soulseek's side brings an MP3 of 144 kbps, within
            // the cap and passing the minimum, but only an eighth more: no switch is offered.
            let bytes: Vec<u8> = vec![5u8; 300_000];
            let (_fake, fetcher) = racing(
                HashMap::from([(REMOTE.to_owned(), bytes.clone())]),
                start_fast_rules(),
                Timed::new(Duration::ZERO, true),
            )
            .await?;
            let known = shared("peer", REMOTE, bytes.len() as u64, "mp3", Some(144), false);
            fetcher.sources.add(HashMap::from([(track().id, vec![known])]));
            fetcher.sources.mark_searched(&track().id);
            let yt = fetcher.stream(track(), None).await?;
            let (chunk, _) = first_bytes(&yt).await?;
            assert!(chunk.iter().all(|b| *b == 7), "YouTube's copy plays");
            let up = fetcher
                .upgrade(&track().id)
                .ok_or_else(|| Error::internal("Soulseek's copy stopped"))?;
            assert_eq!(ended(&up).await?.status, Status::Done, "it is kept for later plays");
            for _ in 0..10 {
                assert!(fetcher.event(&yt)["upgrade"].is_null(), "offered a tiny gain");
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Ok(())
        }

        #[tokio::test]
        async fn a_copy_above_the_streaming_cap_never_starts_the_song_while_youtube_has_one_within_it() -> Result<()> {
            // Live (2026-11-12): Marlon Funaki's "Metaphor", under a 320 kbps minimum and cap and
            // `playBest`, started on a peer's FLAC: Soulseek's side sent before YouTube's Opus 160
            // and the race handed it the song, though the cap is a maximum.
            const FLAC: &str = "Music\\Artist\\Album\\01 - Song.flac";
            let mut rules = start_fast_rules();
            rules.stream.minimum.kbps = 320;
            rules.stream.max_kbps = 320;
            rules.stream.below = lull_core::quality::StreamBelow::PlayBest;
            let bytes: Vec<u8> = (0..900_000).map(|i| (i % 251) as u8).collect();
            let (_fake, fetcher) = racing(
                HashMap::from([(FLAC.to_owned(), bytes.clone())]),
                rules,
                Timed::opus(Duration::from_secs(1)),
            )
            .await?;
            let flac = shared("peer", FLAC, bytes.len() as u64, "flac", None, true);
            fetcher.sources.add(HashMap::from([(track().id, vec![flac])]));
            fetcher.sources.mark_searched(&track().id);

            let opened = fetcher.stream(track(), None).await?;
            let (chunk, _) = first_bytes(&opened).await?;
            let playing = last(&opened);
            let event = fetcher.event(&playing);
            assert_eq!(event["peer"], lull_core::youtube::PEER, "{event}");
            assert!(chunk.iter().all(|b| *b == 7), "YouTube's copy within the cap plays");
            assert!(event["upgrade"].is_null(), "the FLAC is never offered: {event}");
            Ok(())
        }

        /// YouTube's AAC 128 found at once; a Soulseek file in a remaster's folder isn't the song
        /// by its sound (ADR-0016), any other is.
        struct RefusesRemasters;

        #[async_trait]
        impl Youtube for RefusesRemasters {
            async fn find(&self, _: &Track, _: Want) -> Result<Option<Found>> {
                Ok(Some(Found {
                    source: upload("rightupload"),
                    title: "Song".into(),
                    channel: "Artist - Topic".into(),
                    official: true,
                }))
            }
            async fn download(&self, _: &SourceRef, dest: &std::path::Path) -> Result<()> {
                std::fs::write(dest, vec![7u8; SIZE as usize]).map_err(|e| Error::internal(e.to_string()))
            }
            async fn check_file(&self, _: &Track, path: &std::path::Path) -> Option<bool> {
                Some(!path.to_string_lossy().contains("Remaster"))
            }
        }

        #[tokio::test]
        async fn a_better_copy_that_isnt_the_song_gives_way_to_the_next() -> Result<()> {
            // Live (2026-11-11): TLC's "No Scrubs" started on YouTube; Soulseek's side brought a
            // peer's "re-mastered version", which the player couldn't line up and the sound check
            // then refused. It kept the song's one upgrade, so the hub went on offering it and
            // never fetched the copies other peers shared, which line up with YouTube's.
            const REMASTER: &str = "Music\\Artist\\Album (Remaster)\\01 - Song.mp3";
            let remaster: Vec<u8> = vec![3u8; 300_000];
            let right: Vec<u8> = vec![5u8; 300_000];
            let (_fake, fetcher) = racing(
                HashMap::from([
                    (REMASTER.to_owned(), remaster.clone()),
                    (REMOTE.to_owned(), right.clone()),
                ]),
                start_fast_rules(),
                Arc::new(RefusesRemasters),
            )
            .await?;
            // The remaster's peer is the faster, so Soulseek's side takes it first.
            let mut fast = shared("remasterer", REMASTER, remaster.len() as u64, "mp3", Some(320), false);
            fast.upload_speed = 8_000_000;
            let other = shared("peer", REMOTE, right.len() as u64, "mp3", Some(320), false);
            fetcher.sources.add(HashMap::from([(track().id, vec![fast, other])]));
            fetcher.sources.mark_searched(&track().id);

            let yt = fetcher.stream(track(), None).await?;
            let (chunk, _) = first_bytes(&yt).await?;
            assert!(chunk.iter().all(|b| *b == 7), "YouTube's copy plays");
            let up = fetcher
                .upgrade(&track().id)
                .ok_or_else(|| Error::internal("Soulseek's copy stopped"))?;
            assert_eq!(up.now().source.map(|s| s.user).as_deref(), Some("remasterer"));
            assert_eq!(ended(&up).await?.status, Status::Done);

            let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
            let offer = loop {
                let event = fetcher.event(&yt);
                let right_ready = event["upgrade"]["peer"] == "peer" && event["upgrade"]["ready"] == true;
                if right_ready || tokio::time::Instant::now() > deadline {
                    break event;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            };
            assert_eq!(
                offer["upgrade"]["peer"], "peer",
                "the copy that isn't the song is still offered: {offer}"
            );
            assert_eq!(offer["upgrade"]["ready"], true, "{offer}");
            Ok(())
        }

        #[tokio::test]
        async fn youtubes_copy_is_a_better_copy_like_any_and_the_upgrade_fetches_it() -> Result<()> {
            // A 128 minimum: a peer's MP3 128 starts the song first (YouTube's lookup is slower),
            // and YouTube's Opus 160, a fifth and more above it, is the better copy the policy
            // names. The upgrade fetches it from YouTube, and the player is offered it.
            let bytes: Vec<u8> = vec![5u8; 300_000];
            let youtube = Timed::opus(Duration::from_secs(1));
            let (_fake, fetcher) = racing(
                HashMap::from([(REMOTE.to_owned(), bytes.clone())]),
                start_fast_rules(),
                youtube.clone(),
            )
            .await?;
            let known = shared("peer", REMOTE, bytes.len() as u64, "mp3", Some(128), false);
            fetcher.sources.add(HashMap::from([(track().id, vec![known])]));
            fetcher.sources.mark_searched(&track().id);

            let opened = fetcher.stream(track(), None).await?;
            let (chunk, _) = first_bytes(&opened).await?;
            assert!(chunk.iter().all(|b| *b == 5), "the peer's MP3 128 starts the song");
            let playing = last(&opened);
            let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
            let offer = loop {
                let event = fetcher.event(&playing);
                if event["upgrade"]["ready"] == true || tokio::time::Instant::now() > deadline {
                    break event;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            };
            assert_eq!(offer["upgrade"]["peer"], lull_core::youtube::PEER, "{offer}");
            assert_eq!(offer["upgrade"]["format"], "YouTube · Opus 160", "{offer}");
            assert_eq!(youtube.downloads.load(std::sync::atomic::Ordering::Relaxed), 1);
            Ok(())
        }

        #[tokio::test]
        async fn a_skip_leaves_start_fasts_soulseek_copy_running_for_later_plays() -> Result<()> {
            let bytes: Vec<u8> = vec![5u8; 300_000];
            let (_fake, fetcher) = racing(
                HashMap::from([(REMOTE.to_owned(), bytes)]),
                start_fast_rules(),
                Timed::new(Duration::ZERO, true),
            )
            .await?;
            let yt = fetcher.stream(track(), None).await?;
            first_bytes(&yt).await?;
            let up = fetcher
                .upgrade(&track().id)
                .ok_or_else(|| Error::internal("no Soulseek copy beside"))?;
            // The listener skips while Soulseek's copy is still on its way.
            fetcher.stop_streaming(&track().id);
            let u = ended(&up).await?;
            assert_eq!(u.status, Status::Done, "{:?}", u.error);
            let cached = fetcher.best_cached(&track(), crate::rules::Purpose::Stream).await?;
            assert_eq!(
                cached.map(|e| e.source.user),
                Some("peer".to_owned()),
                "the next play takes it"
            );
            Ok(())
        }

        #[tokio::test]
        async fn youtube_with_no_copy_hands_the_song_to_soulseeks_copy_still_on_its_way() -> Result<()> {
            let bytes: Vec<u8> = vec![5u8; 300_000];
            let (_fake, fetcher) = racing(
                HashMap::from([(REMOTE.to_owned(), bytes)]),
                start_fast_rules(),
                Timed::new(Duration::ZERO, false),
            )
            .await?;
            let opened = fetcher.stream(track(), None).await?;
            // The player's request was made before YouTube said it has nothing: it gets the peer's copy.
            let (chunk, _) = first_bytes(&opened).await?;
            assert!(chunk.iter().all(|b| *b == 5));
            assert_eq!(
                opened.now().note.as_deref(),
                Some("YouTube: no YouTube upload matches this song")
            );
            assert_eq!(ended(&opened).await?.status, Status::Done);
            Ok(())
        }

        #[tokio::test]
        async fn a_copy_below_the_minimum_searches_again_for_a_better_one() -> Result<()> {
            // Before, a copy below the minimum was only ever upgraded to a better copy already
            // known, so with none known it never was.
            const LOW: &str = "Music\\Rips\\Album\\01 - Song.mp3";
            let bytes: Vec<u8> = vec![5u8; 200_000];
            let mut rules = Rules::default();
            rules.stream.minimum.kbps = 320;
            let (fake, fetcher) = racing(
                HashMap::from([
                    (format!("low|{LOW}"), bytes.clone()),
                    (REMOTE.to_owned(), bytes.clone()),
                ]),
                rules,
                Arc::new(NoYoutube),
            )
            .await?;
            let low = shared("low", LOW, bytes.len() as u64, "mp3", Some(128), false);
            fetcher.sources.add(HashMap::from([(track().id, vec![low])]));
            let f = fetcher.stream(track(), None).await?;
            assert_eq!(ended(&f).await?.source.map(|s| s.user), Some("low".to_owned()));
            let mut up = None;
            for _ in 0..100 {
                up = fetcher.upgrade(&track().id);
                if up.is_some() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            let up = up.ok_or_else(|| Error::internal("no better copy looked for"))?;
            let u = ended(&up).await?;
            assert_eq!(u.status, Status::Done, "{:?}", u.error);
            assert_eq!(
                u.source.map(|s| s.user),
                Some("peer".to_owned()),
                "the search's 320 kbps copy"
            );
            assert_eq!(fake.searches().len(), 1, "{:?}", fake.searches());
            assert_eq!(fetcher.event(&f)["upgrade"]["peer"], "peer");
            Ok(())
        }

        /// Start fast's start, before and after the race (ADR-0012): a Soulseek copy known from an
        /// earlier search and a YouTube lookup that takes 15 s, as Beach House's "Baseball
        /// Diamond" had (2026-11-07). Prints the time to the first byte of the player's response.
        /// `cargo test -p lull-soulseek start_fast_start_time -- --ignored --nocapture`
        #[tokio::test]
        #[ignore = "a measurement, not a check"]
        async fn start_fast_start_time() -> Result<()> {
            let bytes: Vec<u8> = vec![5u8; 300_000];
            let youtube = Timed::new(Duration::from_secs(15), true);
            let (_fake, fetcher) = racing(
                HashMap::from([(REMOTE.to_owned(), bytes.clone())]),
                start_fast_rules(),
                youtube,
            )
            .await?;
            let known = shared("peer", REMOTE, bytes.len() as u64, "mp3", Some(320), false);
            fetcher.sources.add(HashMap::from([(track().id, vec![known])]));
            fetcher.sources.mark_searched(&track().id);
            let opened = fetcher.stream(track(), None).await?;
            let (chunk, took) = first_bytes(&opened).await?;
            println!(
                "start fast, known Soulseek copy, 15 s YouTube lookup: first byte after {:.2} s, from {}",
                took.as_secs_f64(),
                if chunk.iter().all(|b| *b == 5) {
                    "Soulseek"
                } else {
                    "YouTube"
                }
            );
            Ok(())
        }

        /// Uploads no reference can check: the first, then (once it is refused) the second.
        struct Unchecked {
            finds: std::sync::atomic::AtomicUsize,
        }

        #[async_trait]
        impl Youtube for Unchecked {
            async fn find(&self, _: &Track, _: Want) -> Result<Option<Found>> {
                let n = self.finds.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Ok(Some(Found {
                    source: upload(if n == 0 { "firstupload" } else { "otherupload" }),
                    title: "Song".into(),
                    channel: "a fan".into(),
                    official: false,
                }))
            }
            async fn download(&self, _: &SourceRef, dest: &std::path::Path) -> Result<()> {
                std::fs::write(dest, vec![7u8; SIZE as usize]).map_err(|e| Error::internal(e.to_string()))
            }
        }

        #[tokio::test]
        async fn a_copy_no_reference_checks_plays_unverified_and_a_no_moves_the_song_on() -> Result<()> {
            let peer = Peer {
                per_tick: 32_768,
                overwrite: false,
                reported: None,
                settles: Duration::ZERO,
            };
            let fake = Fake::start(HashMap::new(), peer)
                .await
                .map_err(|e| Error::internal(e.to_string()))?;
            let mut rules = Rules::default();
            rules.youtube.mode = lull_core::youtube::Mode::StartFast;
            let youtube = Arc::new(Unchecked {
                finds: Default::default(),
            });
            let fetcher = with_youtube(&fake, rules, youtube).await?;
            let f = fetcher.stream(track(), None).await?;
            let p = ended(&f).await?;
            assert_eq!(p.status, Status::Done, "{:?}", p.error);
            assert!(p.unverified, "the player asks: right song?");
            assert_eq!(super::super::stream_event(&f, &p)["unverified"], true);
            // "No": the copy leaves, and the song's next opening takes the next upload.
            let first = upload("firstupload");
            fetcher.refused(&track().id, &first).await?;
            let next = ended(&fetcher.stream(track(), None).await?).await?;
            let playing = next.source.map(|s| s.source_ref().path).unwrap_or_default();
            assert!(playing.contains("otherupload"), "{playing}");
            Ok(())
        }

        #[tokio::test]
        async fn the_job_answers_why_a_song_stopped_and_records_a_device_that_cant_play_it() -> Result<()> {
            // The hub failed the song: its own reason answers, YouTube's part included, unchanged.
            // No search finds the song (the file is one peer's own); picking it still fetches it.
            let peer = Peer {
                per_tick: 32_768,
                overwrite: false,
                reported: None,
                settles: Duration::ZERO,
            };
            let fake = Fake::start(HashMap::from([(format!("peer|{REMOTE}"), vec![5u8; 50_000])]), peer)
                .await
                .map_err(|e| Error::internal(e.to_string()))?;
            let mut rules = Rules::default();
            rules.youtube.mode = lull_core::youtube::Mode::StartFast;
            let fetcher = fetcher(&fake, rules).await?;
            let opened = fetcher.stream(track(), None).await?;
            let p = ended(&opened).await?;
            // YouTube had no copy, so Soulseek's side of the race became the player's fetch.
            let failed = last(&opened);
            assert_eq!(p.status, Status::Failed);
            let answer = fetcher
                .output_error(&track().id, Some(&failed.job_id), "Kitchen")
                .ok_or_else(|| Error::internal("no answer"))?;
            assert_eq!(answer["status"], "failed");
            assert_eq!(answer["error"], serde_json::json!(p.error));
            assert!(
                answer["error"].as_str().is_some_and(|e| e.contains("YouTube:")),
                "{answer}"
            );
            assert_eq!(answer["unplayable"], serde_json::Value::Null);
            // The copy arrived, but this device can't play it: recorded on the job.
            let pick = SourceRef {
                user: "peer".into(),
                path: REMOTE.into(),
                size: 50_000,
            };
            let done = fetcher.stream(track(), Some(pick)).await?;
            assert_eq!(ended(&done).await?.status, Status::Done);
            let answer = fetcher
                .output_error(&track().id, Some(&done.job_id), "Kitchen")
                .ok_or_else(|| Error::internal("no answer"))?;
            assert_eq!(answer["status"], "streaming");
            let why = answer["unplayable"].as_str().unwrap_or_default();
            assert!(why.starts_with("this copy can't be played in Kitchen"), "{why}");
            assert_eq!(done.now().unplayable.as_deref(), Some(why), "the job's own record");
            assert!(fetcher.output_error(&track().id, Some("job-gone"), "Kitchen").is_none());
            // Before the player heard of its job: the song's own.
            let latest = fetcher
                .output_error(&track().id, None, "Kitchen")
                .ok_or_else(|| Error::internal("no answer"))?;
            assert_eq!(latest["jobId"], serde_json::json!(done.job_id));
            Ok(())
        }

        #[tokio::test]
        async fn start_fast_with_no_youtube_match_and_no_peer_says_why_youtube_failed_too() -> Result<()> {
            let peer = Peer {
                per_tick: 32_768,
                overwrite: false,
                reported: None,
                settles: Duration::ZERO,
            };
            let fake = Fake::start(HashMap::new(), peer)
                .await
                .map_err(|e| Error::internal(e.to_string()))?;
            let mut rules = Rules::default();
            rules.youtube.mode = lull_core::youtube::Mode::StartFast;
            let f = fetcher(&fake, rules).await?.stream(track(), None).await?;
            let p = ended(&f).await?;
            assert_eq!(p.status, Status::Failed);
            let error = p.error.unwrap_or_default();
            assert!(
                error.ends_with("; YouTube: no YouTube upload matches this song"),
                "{error}"
            );
            Ok(())
        }

        #[tokio::test]
        async fn a_download_slskd_calls_done_before_it_has_moved_the_file_still_lands_whole() -> Result<()> {
            let bytes: Vec<u8> = (0..300_000).map(|i| (i % 251) as u8).collect();
            // slskd reads Succeeded two seconds before it moves the file and says where: lull
            // looks once a second, so it sees the transfer done with nowhere to find it yet.
            let peer = Peer {
                per_tick: 32_768,
                overwrite: false,
                reported: None,
                settles: Duration::from_secs(2),
            };
            let fake = Fake::start(HashMap::from([(REMOTE.to_owned(), bytes.clone())]), peer)
                .await
                .map_err(|e| Error::internal(e.to_string()))?;
            let fetcher = fetcher(&fake, Rules::default()).await?;
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
                reported: None,
                settles: Duration::from_secs(2),
            };
            let fake = Fake::start(HashMap::from([(REMOTE.to_owned(), bytes.clone())]), peer)
                .await
                .map_err(|e| Error::internal(e.to_string()))?;
            let fetcher = fetcher(&fake, Rules::default()).await?;
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
                reported: None,
                settles: Duration::from_millis(100),
            };
            let fake = Fake::start_with(
                HashMap::from([(REMOTE.to_owned(), bytes.clone())]),
                peer,
                HashMap::from([("liar".to_owned(), 1_000)]),
            )
            .await
            .map_err(|e| Error::internal(e.to_string()))?;
            let fetcher = fetcher(&fake, Rules::default()).await?;
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
            let ranked = fetcher
                .sources
                .ordered(&song, &fetcher.rules.get(), super::super::Purpose::Stream);
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
                let arrived = if p.attempt == attempt { p.received() as f64 } else { 0.0 };
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
