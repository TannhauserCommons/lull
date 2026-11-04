//! Classic's own player: the web player's engine (web/app.js, the player) for a window with no
//! webview, its rules mirrored here and its sound made by decks and a mixer. A thread of its own
//! keeps the page's clock (a tick every 250 ms, sooner when told something) and does what the page
//! does on it:
//! - loads songs (a saved one from the library, the rest from the hub's stream, a picked copy with
//!   its `peer` and `path`), ends the open play with its reason and moves on by the queue's rules
//!   (queue.rs), with radio topping the queue up;
//! - plays the next song gaplessly or crossfades into it (lull's Settings › Playback), preloading it
//!   at once as the Android player does (`eagerNext`), and tells the hub what to prefetch and what to
//!   let go;
//! - follows a song still arriving: waiting states, a fallback's new attempt reopened at the same
//!   second, seeks held inside what has arrived, a failed song skipped after 2.5 s;
//! - switches to a better copy mid-song, lined up by correlation (align.rs) and faded over 200 ms;
//! - levels loudness (ReplayGain, or measured), plays through the equaliser at the slider's square;
//! - records listening history (listen.rs) and tells lull Connect what plays.

use super::align;
use super::deck::{Deck, Facts, Phase, Song};
use super::eq::Chain;
use super::events::{self, Heard, StreamEvent, Upgrade};
use super::hub::{Failure, Hub};
use super::listen::{How, Listening};
use super::mixer::{Event, Mixer};
use super::output::{Device, Output};
use super::queue::{Entry, Previous, Queue, Repeat, encode};
use super::remote::Refused;
use super::rules::{self, Switch};
use crate::desktop::classic::Equaliser;
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// lull Connect: what plays goes out at once on a song, play or pause; seeks, the volume and the
/// queue at most once a second; and every 30 s while playing.
const CONNECT_SOON: Duration = Duration::from_secs(1);
const CONNECT_EVERY: Duration = Duration::from_secs(30);
/// Decoded ahead: the playing deck, and a deck waiting its turn.
const AHEAD: f64 = 0.5;
const AHEAD_WAITING: f64 = 1.0;
/// A better copy's deck starts this far ahead of where the song is, so it is playing by the time
/// the song gets there; one that hasn't given enough sound to line up after this long is a try that
/// didn't work out (app.js `place`, 20 s).
const SWITCH_START: f64 = 1.0;
const SWITCH_SLOW: Duration = Duration::from_secs(20);

/// Settings › Playback, as the page reads them from `GET /api/settings` (app.js
/// `applyPlaybackSettings`).
#[derive(Clone, Debug, PartialEq)]
pub struct Playback {
    pub gapless: bool,
    pub crossfade: f64,
    pub normalisation: String,
    pub loudness_target: f64,
    pub radio_when_queue_ends: bool,
    pub prefetch_count: usize,
}

impl Default for Playback {
    fn default() -> Self {
        Self {
            gapless: true,
            crossfade: 0.0,
            normalisation: "off".into(),
            loudness_target: -14.0,
            radio_when_queue_ends: true,
            prefetch_count: 2,
        }
    }
}

impl Playback {
    /// From the hub's settings: a value may come as `{ value, source }`.
    pub fn from_settings(s: &Value) -> Self {
        let v = |x: &Value| {
            if x.is_object() && x.get("value").is_some() {
                x["value"].clone()
            } else {
                x.clone()
            }
        };
        let pb = &s["playback"];
        let defaults = Self::default();
        Self {
            gapless: v(&pb["gapless"]) != Value::Bool(false),
            crossfade: v(&pb["crossfade"]).as_f64().unwrap_or(0.0),
            normalisation: v(&pb["normalisation"]).as_str().unwrap_or("off").to_owned(),
            loudness_target: v(&pb["loudnessTarget"])
                .as_f64()
                .filter(|t| *t != 0.0)
                .unwrap_or(defaults.loudness_target),
            radio_when_queue_ends: v(&pb["radioWhenQueueEnds"]) != Value::Bool(false),
            prefetch_count: v(&pb["prefetchCount"])
                .as_u64()
                .map_or(defaults.prefetch_count, |n| n.min(3) as usize),
        }
    }
}

/// What the player knows of a song.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Track {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub artist_id: String,
    pub album_id: String,
    pub duration: Option<f64>,
    /// In the library: it plays from there, not from Soulseek.
    pub saved: bool,
    pub liked: bool,
    /// The cover, on the hub.
    pub art: Option<String>,
    /// ReplayGain for Settings › Playback's mode, when the library has it.
    pub gain_db: Option<f64>,
    /// The hub's own record of it, for the page to take back (songs radio added here).
    pub entity: Option<Value>,
}

/// Where the playing song stands, as the page's status says it.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Status {
    #[default]
    Idle,
    /// Opening the song.
    Loading,
    Library,
    Streaming,
    Finding,
    Queued(u32),
    Buffering,
    Stalled,
    /// No copy passes the streaming minimum: one of these to pick, or skip.
    Choose(Vec<Value>),
    Failed(String),
    Ended,
}

/// What the window and the tray show; the engine keeps it current.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub queue: Vec<Entry>,
    pub index: usize,
    pub position: f64,
    pub duration: Option<f64>,
    pub playing: bool,
    pub waiting: bool,
    pub status: Status,
    pub facts: Facts,
    pub shuffle: bool,
    pub repeat: Repeat,
    pub volume: f64,
    pub eq: Equaliser,
    /// Goes up when the queue changes, for the playlist.
    pub revision: u64,
    /// A line for the marquee for a few seconds (a seek held back, a copy switched to).
    pub notice: Option<String>,
    /// Whether the hub answered the last call.
    pub reachable: bool,
    /// Winamp's Stop: paused at the start, the time blank.
    pub stopped: bool,
}

pub enum Command {
    Play,
    Pause,
    /// Winamp's Stop: paused at the start.
    Stop,
    Next,
    Previous,
    Jump(usize),
    Seek(f64),
    Volume(f64),
    Equaliser(Equaliser),
    Shuffle(bool),
    CycleRepeat,
    Keep(Vec<bool>),
    Reorder(Vec<usize>, usize),
    /// A copy picked for a song waiting in `choose`.
    Choose(Value),
    /// The tray's Like (on or off) and Save, for the playing song.
    Like,
    Save,
    Quit,
}

/// What the page takes back when Classic closes.
pub struct Back {
    pub queue: Queue,
    pub position: f64,
    pub playing: bool,
    pub volume: f64,
    pub eq: Equaliser,
    /// The hub's records of songs this player added (radio), for the page's lookups.
    pub tracks: serde_json::Map<String, Value>,
    /// The outbox's ops, complete once `threads` are done.
    pub ops: Arc<Mutex<Vec<Value>>>,
    /// Calls still telling the hub about the last play.
    pub threads: Vec<JoinHandle<()>>,
}

/// The player, as the window and the session hold it.
pub struct Engine {
    /// Tests tell the engine what the hub's events would.
    #[cfg(test)]
    heard_to: Sender<Heard>,
    shared: Arc<Shared>,
    mixer: Arc<Mutex<Mixer>>,
    catalogue: Arc<Mutex<HashMap<String, Track>>>,
    thread: Option<JoinHandle<Back>>,
}

struct Shared {
    commands: Mutex<VecDeque<Command>>,
    wake: Condvar,
    snapshot: Mutex<Snapshot>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// What the engine starts with.
pub struct Start {
    pub queue: Queue,
    pub tracks: Vec<Track>,
    pub position: f64,
    pub playing: bool,
    pub volume: f64,
    pub eq: Equaliser,
}

impl Engine {
    /// Starts the player on the default output device.
    pub fn start(hub: Hub, start: Start) -> Result<Self, String> {
        let mixer = Arc::new(Mutex::new(Mixer::new(48_000)));
        let render = {
            let mixer = mixer.clone();
            Box::new(move |out: &mut [f32]| lock(&mixer).render(out))
        };
        let device = Device::open(render)?;
        *lock(&mixer) = Mixer::new(device.rate());
        Self::with_output(hub, start, Box::new(device), mixer, true)
    }

    /// Starts the player on any output (tests give one they pull from), following the hub's events
    /// when `follow` (tests feed them).
    fn with_output(
        hub: Hub,
        start: Start,
        output: Box<dyn Output + Send>,
        mixer: Arc<Mutex<Mixer>>,
        follow: bool,
    ) -> Result<Self, String> {
        let shared = Arc::new(Shared {
            commands: Mutex::new(VecDeque::new()),
            wake: Condvar::new(),
            snapshot: Mutex::new(Snapshot::default()),
        });
        let catalogue = Arc::new(Mutex::new(
            start.tracks.iter().map(|t| (t.id.clone(), t.clone())).collect(),
        ));
        let (heard_to, heard) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        if follow {
            events::follow(
                hub.origin().clone(),
                hub.token().to_owned(),
                heard_to.clone(),
                stop.clone(),
            );
        }
        #[cfg(test)]
        let tests_heard = heard_to.clone();
        let run = Run::new(
            hub,
            start,
            output,
            mixer.clone(),
            shared.clone(),
            catalogue.clone(),
            heard,
            heard_to,
            stop,
        );
        let thread = std::thread::Builder::new()
            .name("classic-engine".into())
            .spawn(move || run.run())
            .map_err(|e| e.to_string())?;
        Ok(Self {
            #[cfg(test)]
            heard_to: tests_heard,
            shared,
            mixer,
            catalogue,
            thread: Some(thread),
        })
    }

    pub fn send(&self, command: Command) {
        lock(&self.shared.commands).push_back(command);
        self.shared.wake.notify_all();
    }

    pub fn snapshot(&self) -> Snapshot {
        lock(&self.shared.snapshot).clone()
    }

    pub fn track(&self, id: &str) -> Option<Track> {
        lock(&self.catalogue).get(id).cloned()
    }

    /// The newest `count` mono samples played, for the visualiser.
    pub fn tap(&self, count: usize) -> Vec<f32> {
        let mixer = lock(&self.mixer);
        let skip = mixer.tap.len().saturating_sub(count);
        mixer.tap.iter().skip(skip).copied().collect()
    }

    /// Stops the player and takes what the page takes back.
    pub fn finish(mut self) -> Option<Back> {
        self.send(Command::Quit);
        self.thread.take().and_then(|t| t.join().ok())
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        if self.thread.is_some() {
            self.send(Command::Quit);
        }
    }
}

/// A song loaded: which load it is, its deck, and what the stream events say of it.
struct Loaded {
    load: u64,
    track_id: String,
    deck: Arc<Deck>,
    /// From Soulseek or YouTube (not the library): it may still be arriving.
    soulseek: bool,
    source: Option<super::queue::Source>,
    job: Option<String>,
    attempt: u32,
    /// How much of it is here, as a fraction, and whether all of it is.
    received: f64,
    complete: bool,
    /// The song's own time is the deck's less this (a better copy's own timeline).
    offset: f64,
}

impl Loaded {
    fn position(&self) -> f64 {
        (self.deck.position() - self.offset).max(0.0)
    }
}

/// A better copy on its way to being switched to (app.js `onUpgrade`, `trySwitch`, the output's
/// `switchTo`). `tries` counts attempts that didn't work out (a seek or pause in the middle, too
/// quiet to tell, too slow); a copy offered again keeps its count, another starts afresh.
enum Better {
    /// Offered by the hub; waiting for the moment.
    Offered { upgrade: Upgrade, tries: u32 },
    /// Its deck started at `start` of its own time; what plays of the song from `anchor` on is
    /// being recorded, to be measured against the copy from `anchor + shift` of its own.
    Lining {
        upgrade: Upgrade,
        deck: Arc<Deck>,
        anchor: f64,
        start: f64,
        shift: f64,
        recorded: Vec<f32>,
        began: Instant,
        tries: u32,
    },
    /// Lined up: copy frame for song time t = (t - anchor) * rate + frames, at the song's level
    /// times `matched`. Waiting for a quiet moment.
    Lined {
        upgrade: Upgrade,
        lined: Lined,
        since: Instant,
        levels: Vec<f64>,
        tries: u32,
    },
    /// Asking the hub to make it the song's stream.
    Committing {
        upgrade: Upgrade,
        lined: Lined,
        answer: Receiver<Result<(), Failure>>,
    },
    /// Fading in.
    Switching { offset: f64 },
    /// Not now: this copy plays from the song's next play.
    NextPlay { peer: String, path: String },
}

/// A copy lined up with the song.
struct Lined {
    deck: Arc<Deck>,
    anchor: f64,
    start: f64,
    frames: i64,
    matched: f32,
}

struct Run {
    hub: Hub,
    output: Box<dyn Output + Send>,
    running: bool,
    mixer: Arc<Mutex<Mixer>>,
    shared: Arc<Shared>,
    catalogue: Arc<Mutex<HashMap<String, Track>>>,
    heard: Receiver<Heard>,
    heard_to: Sender<Heard>,
    stop: Arc<AtomicBool>,
    rate: u32,
    settings: Playback,
    queue: Queue,
    playing: bool,
    status: Status,
    volume: f64,
    eq: Equaliser,
    current: Option<Loaded>,
    next: Option<Loaded>,
    loads: u64,
    listening: Listening,
    plays: Plays,
    revision: u64,
    stopped: bool,
    better: Option<Better>,
    failed_at: Option<Instant>,
    radio: Option<Receiver<RadioAnswer>>,
    prefetch: (Vec<String>, Option<Instant>),
    connect: Connect,
    notice: Option<(String, Instant)>,
    random: std::hash::RandomState,
    draws: u64,
}

/// Radio's answer: the songs it adds with the hub's records of them, or why not.
type RadioAnswer = Result<Vec<(String, Value)>, String>;

/// lull Connect's state: when it last went out, and when it should next.
#[derive(Default)]
struct Connect {
    sent: Option<Instant>,
    due: Option<Instant>,
}

impl Run {
    #[allow(clippy::too_many_arguments)]
    fn new(
        hub: Hub,
        start: Start,
        output: Box<dyn Output + Send>,
        mixer: Arc<Mutex<Mixer>>,
        shared: Arc<Shared>,
        catalogue: Arc<Mutex<HashMap<String, Track>>>,
        heard: Receiver<Heard>,
        heard_to: Sender<Heard>,
        stop: Arc<AtomicBool>,
    ) -> Self {
        let rate = output.rate();
        Self {
            plays: Plays::new(hub.clone()),
            hub,
            output,
            running: false,
            mixer,
            shared,
            catalogue,
            heard,
            heard_to,
            stop,
            rate,
            settings: Playback::default(),
            queue: start.queue,
            playing: start.playing,
            status: Status::Idle,
            volume: start.volume,
            eq: start.eq,
            current: None,
            next: None,
            loads: 0,
            listening: Listening::default(),
            revision: 0,
            stopped: false,
            better: None,
            failed_at: None,
            radio: None,
            prefetch: (Vec::new(), None),
            connect: Connect::default(),
            notice: None,
            random: std::hash::RandomState::new(),
            draws: 0,
            // set below
        }
        .started(start.position)
    }

    fn started(mut self, position: f64) -> Self {
        if let Ok(settings) = self.hub.json("GET", "/api/settings", None) {
            self.settings = Playback::from_settings(&settings);
        }
        self.apply_levels();
        if !self.queue.queue.is_empty() {
            let index = self.queue.index.min(self.queue.queue.len() - 1);
            let playing = self.playing;
            self.load(index, How::Skipped, position, playing);
        }
        self
    }

    fn run(mut self) -> Back {
        loop {
            let commands = self.wait();
            for command in commands {
                if let Command::Quit = command {
                    return self.finish();
                }
                self.command(command);
            }
            while let Ok(heard) = self.heard.try_recv() {
                self.hear(heard);
            }
            self.tick();
        }
    }

    /// Sleeps until a command, or the next tick.
    fn wait(&self) -> Vec<Command> {
        let commands = lock(&self.shared.commands);
        let (mut commands, _) = self
            .shared
            .wake
            .wait_timeout_while(commands, rules::TICK, |c| c.is_empty())
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        commands.drain(..).collect()
    }

    fn command(&mut self, command: Command) {
        if matches!(
            command,
            Command::Play | Command::Next | Command::Previous | Command::Jump(_)
        ) {
            self.stopped = false;
        }
        match command {
            Command::Play => self.play(),
            Command::Pause => self.pause(),
            Command::Stop => {
                self.pause();
                self.seek(0.0);
                self.stopped = true;
            }
            Command::Next => self.next(false),
            Command::Previous => match self.queue.previous(self.position()) {
                Previous::Restart => self.seek(0.0),
                Previous::Load(index) => self.load(index, How::Skipped, 0.0, self.playing),
            },
            Command::Jump(index) => {
                if index < self.queue.queue.len() {
                    self.playing = true;
                    self.load(index, How::Skipped, 0.0, true);
                }
            }
            Command::Seek(seconds) => self.seek(seconds),
            Command::Volume(volume) => {
                self.volume = volume.clamp(0.0, 1.0);
                self.apply_levels();
                self.connect_soon();
            }
            Command::Equaliser(eq) => {
                self.eq = eq;
                self.apply_levels();
            }
            Command::Shuffle(on) => {
                let (random, draws) = (&self.random, &mut self.draws);
                self.queue.set_shuffle(on, &mut |below| {
                    *draws += 1;
                    use std::hash::BuildHasher;
                    (random.hash_one(*draws) % below.max(1) as u64) as usize
                });
                self.queue_changed();
            }
            Command::CycleRepeat => {
                self.queue.repeat = self.queue.repeat.cycled();
                self.release_next();
                self.queue_changed();
            }
            Command::Keep(keep) => {
                self.queue.keep(&keep);
                self.queue_changed();
            }
            Command::Reorder(rows, to) => {
                self.queue.reorder(&rows, to);
                self.queue_changed();
            }
            Command::Choose(source) => self.choose(source),
            Command::Like => self.like(),
            Command::Save => self.save(),
            Command::Quit => {}
        }
        self.publish();
    }

    /// Play: the start again after the queue ended, the same song again after it failed, else
    /// on (app.js `P.play`).
    fn play(&mut self) {
        if self.queue.queue.is_empty() {
            return;
        }
        match &self.status {
            Status::Ended => return self.load(0, How::Skipped, 0.0, true),
            Status::Failed(_) => {
                let (index, at) = (self.queue.index, self.position());
                return self.load(index, How::Skipped, at, true);
            }
            _ => {}
        }
        self.playing = true;
        self.run_output();
        self.connect_now();
    }

    fn pause(&mut self) {
        self.playing = false;
        lock(&self.mixer).abandon_fade();
        self.abandon_switch();
        self.run_output();
        self.connect_now();
    }

    fn run_output(&mut self) {
        if self.running != self.playing {
            self.running = self.playing;
            if let Err(e) = self.output.run(self.playing) {
                self.status = Status::Failed(e);
            }
        }
    }

    /// Moves on (app.js `P.next`): repeat one replays the same song as a new play; at the queue's
    /// end radio may add songs, else the queue ends.
    fn next(&mut self, by_itself: bool) {
        if self.queue.queue.is_empty() {
            return;
        }
        let how = if by_itself { How::Completed } else { How::Skipped };
        match self.queue.next_index(by_itself) {
            Some(i) if i == self.queue.index && by_itself && !matches!(self.better, Some(Better::NextPlay { .. })) => {
                self.end_play(How::Completed);
                self.loads += 1;
                if let Some(current) = self.current.as_mut() {
                    current.load = self.loads;
                }
                self.seek(0.0);
            }
            Some(i) => self.load(i, how, 0.0, self.playing || by_itself),
            None => {
                if self.top_up_radio_now() {
                    let index = self.queue.index + 1;
                    self.load(index, how, 0.0, self.playing || by_itself);
                } else {
                    self.end_play(how);
                    self.status = Status::Ended;
                    self.playing = false;
                    self.run_output();
                    self.connect_now();
                }
            }
        }
    }

    /// Loads queue entry `index` at `at` seconds (app.js `load`): the open play ends with `how`, the
    /// next deck is used if it holds this song, else a deck opens.
    fn load(&mut self, index: usize, how: How, at: f64, autoplay: bool) {
        self.end_play(how);
        let Some(entry) = self.queue.queue.get(index).cloned() else {
            return;
        };
        let was = self.current.take();
        self.queue.index = index;
        self.loads += 1;
        self.better = None;
        self.failed_at = None;
        let reuse = self
            .next
            .take()
            .filter(|n| n.track_id == entry.track_id && n.source == entry.source && at == 0.0);
        let loaded = match reuse {
            Some(mut next) => {
                next.load = self.loads;
                next.deck.set_ahead(AHEAD);
                next
            }
            None => self.open(&entry, at, AHEAD),
        };
        let level = self.level_for(&loaded.track_id);
        lock(&self.mixer).play(loaded.deck.clone(), level);
        if let Some(was) = was {
            self.let_go(&was);
        }
        self.status = if loaded.soulseek {
            Status::Loading
        } else {
            Status::Library
        };
        self.current = Some(loaded);
        self.playing = autoplay;
        self.run_output();
        self.top_up_radio();
        self.preload();
        self.prefetch_soon();
        self.connect_now();
        self.publish();
    }

    /// A deck for a queue entry: the library's copy of a saved song, else the hub's stream (a picked
    /// copy by its peer and path).
    fn open(&self, entry: &Entry, at: f64, ahead: f64) -> Loaded {
        let track = lock(&self.catalogue).get(&entry.track_id).cloned().unwrap_or_default();
        let id = encode(&entry.track_id);
        let soulseek = !track.saved;
        let path = match (&entry.source, soulseek) {
            (_, false) => format!("/api/library/stream/{id}"),
            (Some(s), true) => format!("/api/stream/{id}?peer={}&path={}", encode(&s.peer), encode(&s.path)),
            (None, true) => format!("/api/stream/{id}"),
        };
        let url = self.hub.origin().join(&path).map_or(path, |u| u.to_string());
        let deck = Arc::new(Deck::open(
            Song {
                url,
                duration: track.duration,
            },
            self.hub.token().to_owned(),
            at,
            self.rate,
            ahead,
        ));
        Loaded {
            load: self.loads,
            track_id: entry.track_id.clone(),
            deck,
            soulseek,
            source: entry.source.clone(),
            job: None,
            attempt: 0,
            received: if soulseek { 0.0 } else { 1.0 },
            complete: !soulseek,
            offset: 0.0,
        }
    }

    /// The same song again from where it was: a fallback's new attempt, a seek the hub couldn't
    /// serve yet, a cut response once the next attempt comes. The play goes on (app.js `reopen`).
    fn reopen(&mut self) {
        let at = self.position();
        self.reopen_at(at);
    }

    fn reopen_at(&mut self, at: f64) {
        let Some(current) = self.current.as_ref() else {
            return;
        };
        let Some(entry) = self.queue.queue.get(self.queue.index).cloned() else {
            return;
        };
        let (job, attempt, received, complete) =
            (current.job.clone(), current.attempt, current.received, current.complete);
        let mut loaded = self.open(&entry, at, AHEAD);
        (
            loaded.load,
            loaded.job,
            loaded.attempt,
            loaded.received,
            loaded.complete,
        ) = (self.loads, job, attempt, received, complete);
        let level = self.level_for(&loaded.track_id);
        lock(&self.mixer).play(loaded.deck.clone(), level);
        self.current = Some(loaded);
        self.better = None;
    }

    /// Seconds into the playing song.
    fn position(&self) -> f64 {
        self.current.as_ref().map_or(0.0, Loaded::position)
    }

    /// A seek, held inside what has arrived of a song still arriving (app.js `P.seek`): past it,
    /// the song goes to the edge of what is here and says so.
    fn seek(&mut self, seconds: f64) {
        let Some(current) = self.current.as_ref() else {
            return;
        };
        let duration = current.deck.duration();
        let mut to = seconds.max(0.0);
        let mut held = false;
        if let Some(length) = duration.filter(|l| *l > 0.0) {
            // What the deck holds, and what the transfer delivered (a song done arriving: all of it).
            let buffered = (current.position() + current.deck.buffered()) / length;
            let arriving = current.soulseek && !current.complete;
            let limit = rules::seek_limit(arriving, &[(0.0, buffered)], current.received);
            let (fraction, ok) = rules::seek_to(to / length, limit);
            to = fraction * length;
            held = !ok;
        }
        lock(&self.mixer).abandon_fade();
        current.deck.seek(to + current.offset);
        if held {
            self.notice("The song hasn't arrived that far yet".into());
        }
        self.abandon_switch();
        self.connect_soon();
    }

    /// Ends the open play, as the hub records it.
    fn end_play(&mut self, how: How) {
        if let Some(ended) = self.listening.end(how) {
            self.plays.end(ended.load, ended.heard, how);
        }
    }

    /// The next song set aside for gapless or crossfade (app.js `preloadNext`), every tick: the one
    /// after this, unless repeat one replays this one, once the playing song is close enough to its
    /// end (rules.rs `preload_lead`); not while a better copy is being lined up on another deck.
    fn preload(&mut self) {
        if self.switching() {
            return;
        }
        let wanted = (self.settings.gapless || self.settings.crossfade > 0.0)
            .then(|| self.queue.next_index(true))
            .flatten()
            .filter(|&i| i != self.queue.index)
            .and_then(|i| self.queue.queue.get(i).cloned());
        let Some(entry) = wanted else {
            self.release_next();
            return;
        };
        let due = self.current.as_ref().is_some_and(|c| {
            let lead = rules::preload_lead(self.settings.crossfade, c.soulseek, c.received);
            rules::preload_now(lead, c.deck.duration(), c.position())
        });
        if !due {
            return;
        }
        if self
            .next
            .as_ref()
            .is_some_and(|n| n.track_id == entry.track_id && n.source == entry.source)
        {
            return;
        }
        self.release_next();
        let loaded = self.open(&entry, 0.0, AHEAD_WAITING);
        let level = self.level_for(&loaded.track_id);
        {
            let mut mixer = lock(&self.mixer);
            mixer.next = Some(loaded.deck.clone());
            mixer.next_level = level;
        }
        self.next = Some(loaded);
    }

    /// The song set as next isn't any more (app.js `releaseNext`).
    fn release_next(&mut self) {
        lock(&self.mixer).next = None;
        if let Some(next) = self.next.take() {
            self.let_go(&next);
        }
    }

    /// A song that was playing or next and isn't any more: if it was still arriving, the hub may stop
    /// fetching it, unless it is the playing or the next song after all (app.js `letGo`).
    fn let_go(&self, loaded: &Loaded) {
        let in_use = |l: &Option<Loaded>| l.as_ref().is_some_and(|l| l.track_id == loaded.track_id);
        if !loaded.soulseek || loaded.complete || in_use(&self.current) || in_use(&self.next) {
            return;
        }
        let path = format!("/api/stream/{}", encode(&loaded.track_id));
        self.call("DELETE", path, None);
    }

    /// A hub call that nobody waits for: on a thread of its own.
    fn call(&self, method: &'static str, path: String, body: Option<Value>) {
        let hub = self.hub.clone();
        let spawned = std::thread::Builder::new().name("classic-call".into()).spawn(move || {
            if let Err(Failure::Refused(why)) = hub.json(method, &path, body) {
                eprintln!("lull: Classic {method} {path}: {why}");
            }
        });
        if let Err(e) = spawned {
            eprintln!("lull: Classic {method}: {e}");
        }
    }

    /// The queue changed (shuffle, repeat, songs taken out, moved, radio): the next song and what to
    /// prefetch follow.
    fn queue_changed(&mut self) {
        self.revision += 1;
        let next = self.queue.next_index(true).and_then(|i| self.queue.queue.get(i));
        if self
            .next
            .as_ref()
            .is_some_and(|n| next.is_none_or(|e| n.track_id != e.track_id || n.source != e.source))
        {
            self.release_next();
        }
        self.preload();
        self.prefetch_soon();
        self.connect_soon();
    }

    /// Asks radio for more songs if the queue wants them (app.js `topUpRadio`); the answer comes on a
    /// later tick.
    fn top_up_radio(&mut self) {
        if self.radio.is_some() {
            return;
        }
        let catalogue = self.catalogue.clone();
        let name = |id: &str| lock(&catalogue).get(id).map(|t| t.title.clone());
        let Some(ask) = self.queue.radio_wanted(self.settings.radio_when_queue_ends, name) else {
            return;
        };
        let (to, answer) = channel();
        let hub = self.hub.clone();
        let spawned = std::thread::Builder::new().name("classic-radio".into()).spawn(move || {
            let got = hub.json("GET", &ask.path(), None).map_err(|e| match e {
                Failure::Offline => "the hub is out of reach".to_owned(),
                Failure::Refused(why) => why,
            });
            let songs = got.map(|v| {
                let ids: Vec<String> = v["items"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|i| i.as_str().map(str::to_owned)).collect())
                    .unwrap_or_default();
                ids.into_iter()
                    .map(|id| (id.clone(), v["entities"]["tracks"][&id].clone()))
                    .collect()
            });
            let _ = to.send(songs);
        });
        match spawned {
            Ok(_) => self.radio = Some(answer),
            Err(e) => eprintln!("lull: Classic radio: {e}"),
        }
    }

    /// At the queue's end radio must answer before the player knows whether to stop: wait for it.
    fn top_up_radio_now(&mut self) -> bool {
        self.top_up_radio();
        match self.radio.take().map(|r| r.recv()) {
            Some(Ok(answer)) => self.radio_answered(answer) > 0,
            _ => false,
        }
    }

    fn radio_answered(&mut self, answer: RadioAnswer) -> usize {
        match answer {
            Ok(songs) => {
                let mut catalogue = lock(&self.catalogue);
                for (id, entity) in &songs {
                    catalogue.entry(id.clone()).or_insert_with(|| track_from(id, entity));
                }
                drop(catalogue);
                let ids: Vec<String> = songs.into_iter().map(|(id, _)| id).collect();
                let added = self.queue.add_radio(&ids);
                if added > 0 {
                    self.queue_changed();
                }
                added
            }
            Err(why) => {
                self.queue.radio_failed(why);
                0
            }
        }
    }

    fn prefetch_soon(&mut self) {
        self.prefetch.1 = Some(Instant::now() + rules::PREFETCH);
    }

    /// The songs after this one the hub should fetch ahead (app.js `prefetch`): unsaved, distinct,
    /// not the playing one, up to Settings' count (the next only, with shuffle). Not while the playing
    /// song itself is still being found.
    fn prefetch(&mut self) {
        if self.prefetch.1.is_none_or(|due| Instant::now() < due) {
            return;
        }
        if matches!(
            self.status,
            Status::Finding | Status::Queued(_) | Status::Buffering | Status::Choose(_)
        ) {
            return;
        }
        self.prefetch.1 = None;
        let catalogue = lock(&self.catalogue);
        let ids: Vec<String> = self.queue.queue.iter().map(|e| e.track_id.clone()).collect();
        let ids = rules::upcoming(
            self.queue.index,
            &ids,
            self.queue.shuffle,
            self.queue.repeat,
            |id| catalogue.get(id).is_some_and(|t| t.saved),
            self.settings.prefetch_count,
        );
        drop(catalogue);
        if ids == self.prefetch.0 {
            return;
        }
        self.prefetch.0 = ids.clone();
        self.call("PUT", "/api/stream/prefetch".into(), Some(json!({ "tracks": ids })));
    }

    fn connect_now(&mut self) {
        self.connect.due = Some(Instant::now());
    }

    fn connect_soon(&mut self) {
        let soon = self
            .connect
            .sent
            .map_or(Instant::now(), |s| s + CONNECT_SOON)
            .max(Instant::now());
        self.connect.due = Some(self.connect.due.map_or(soon, |d| d.min(soon)));
    }

    /// lull Connect: what plays, for the hub to tell the other devices.
    fn connect(&mut self) {
        let now = Instant::now();
        if self.playing && self.connect.sent.is_some_and(|s| now >= s + CONNECT_EVERY) {
            self.connect.due = Some(now);
        }
        if self.connect.due.is_none_or(|d| now < d) {
            return;
        }
        self.connect = Connect {
            sent: Some(now),
            due: None,
        };
        let body = json!({
            "trackId": self.queue.queue.get(self.queue.index).map(|e| &e.track_id),
            "position": self.position(),
            "playing": self.playing,
            "volume": self.volume,
            "queue": self.queue.queue.iter().map(|e| &e.track_id).collect::<Vec<_>>(),
            "queueIndex": (!self.queue.queue.is_empty()).then_some(self.queue.index),
        });
        self.call("POST", "/api/devices/me/state".into(), Some(body));
    }

    /// Like, or unlike, the playing song: the hub at once, the outbox when it's out of reach, undone
    /// when the hub says no (app.js `setLiked` through the outbox).
    fn like(&mut self) {
        let Some(id) = self.queue.queue.get(self.queue.index).map(|e| e.track_id.clone()) else {
            return;
        };
        let on = {
            let mut catalogue = lock(&self.catalogue);
            let Some(track) = catalogue.get_mut(&id) else {
                return;
            };
            track.liked = !track.liked;
            track.liked
        };
        let (hub, ops, catalogue) = (self.hub.clone(), self.plays.ops.clone(), self.catalogue.clone());
        self.plays.spawn(move || match hub.like(&id, on) {
            Ok(()) => {}
            Err(Failure::Offline) => lock(&ops).push(json!({ "op": "like", "trackId": id, "liked": on })),
            Err(Failure::Refused(why)) => {
                eprintln!("lull: Classic like refused: {why}");
                if let Some(track) = lock(&catalogue).get_mut(&id) {
                    track.liked = !on;
                }
            }
        });
    }

    /// Save the playing song into the library (app.js `App.saves.start`). The outbox doesn't keep
    /// saves, so neither does this.
    fn save(&mut self) {
        let unsaved = self
            .queue
            .queue
            .get(self.queue.index)
            .map(|e| e.track_id.clone())
            .filter(|id| !lock(&self.catalogue).get(id).is_some_and(|t| t.saved));
        let Some(id) = unsaved else {
            return;
        };
        let hub = self.hub.clone();
        self.plays.spawn(move || {
            if let Err(e) = hub.save(&id) {
                eprintln!("lull: Classic save: {e:?}");
            }
        });
    }

    /// The listener picked one of the copies a `choose` offered: the song plays from it.
    fn choose(&mut self, source: Value) {
        let (Some(peer), Some(path)) = (source["peer"].as_str(), source["path"].as_str()) else {
            return;
        };
        if let Some(entry) = self.queue.queue.get_mut(self.queue.index) {
            entry.source = Some(super::queue::Source {
                peer: peer.to_owned(),
                path: path.to_owned(),
            });
        }
        self.reopen();
    }

    /// A `stream` event about the playing or the next song (app.js `onStreamEvent`).
    fn hear(&mut self, heard: Heard) {
        let event = match heard {
            // Missed events: one byte of the playing song makes the hub say where it stands again.
            Heard::Connected => {
                if let Some(current) = self.current.as_ref().filter(|c| c.soulseek && !c.complete) {
                    let path = format!("/api/stream/{}", encode(&current.track_id));
                    let hub = self.hub.clone();
                    let _ = std::thread::Builder::new().name("classic-call".into()).spawn(move || {
                        let _ = hub.json("GET", &path, None);
                    });
                }
                return;
            }
            Heard::Stream(event) => *event,
        };
        let is = |l: &Option<Loaded>| l.as_ref().is_some_and(|l| l.track_id == event.track_id && l.soulseek);
        if is(&self.next) && !is(&self.current) {
            if let Some(next) = self.next.as_mut() {
                adopt(next, &event);
            }
            return;
        }
        if !is(&self.current) {
            return;
        }
        let reopen = self.current.as_mut().is_some_and(|c| adopt(c, &event));
        if reopen {
            self.reopen();
        }
        // A copy offered again keeps its tries and its verdict; another starts afresh; none offered
        // is none (app.js `onUpgrade`). One being lined up or switched to carries on.
        self.better = match (self.better.take(), event.upgrade.clone()) {
            (Some(Better::Offered { upgrade: was, tries }), Some(upgrade)) => {
                let tries = if was.peer == upgrade.peer && was.path == upgrade.path {
                    tries
                } else {
                    0
                };
                Some(Better::Offered { upgrade, tries })
            }
            (Some(Better::NextPlay { peer, path }), Some(upgrade)) if peer != upgrade.peer || path != upgrade.path => {
                Some(Better::Offered { upgrade, tries: 0 })
            }
            (None, Some(upgrade)) => Some(Better::Offered { upgrade, tries: 0 }),
            (Some(Better::Offered { .. }), None) => None,
            (other, _) => other,
        };
        let heard = self.playing && !self.waiting();
        self.status = match event.status.as_str() {
            "choose" => Status::Choose(event.choices.clone()),
            "finding" if !heard => Status::Finding,
            "queued" if !heard => Status::Queued(event.queue_pos.unwrap_or(0)),
            "stalled" => Status::Stalled,
            "buffering" if !heard => Status::Buffering,
            "fallback" => Status::Buffering,
            "streaming" if heard || !self.playing => Status::Streaming,
            "streaming" => Status::Buffering,
            "failed" => {
                if self.playing {
                    self.failed_at = Some(Instant::now());
                }
                self.playing = false;
                self.run_output();
                Status::Failed(
                    event
                        .error
                        .clone()
                        .unwrap_or_else(|| "Couldn't play: no peer responded".into()),
                )
            }
            _ => self.status.clone(),
        };
        self.publish();
    }

    /// Whether the music should be sounding but isn't: the song is opening, or ran out of sound.
    fn waiting(&self) -> bool {
        self.current.as_ref().is_some_and(|c| {
            matches!(c.deck.phase(), Phase::Opening) || (c.deck.buffered() == 0.0 && !c.deck.drained())
        })
    }

    /// The page's clock: what the mixer did, the play, the decks, and everything that waits.
    fn tick(&mut self) {
        let events: Vec<Event> = std::mem::take(&mut lock(&self.mixer).events);
        for event in events {
            match event {
                // Gapless or a crossfade's end: the next song plays, its play a completed one's
                // successor (app.js `handOver`).
                Event::Handover => {
                    self.end_play(How::Completed);
                    if let (Some(next), Some(i)) = (self.next.take(), self.queue.next_index(true)) {
                        self.loads += 1;
                        let mut next = next;
                        next.load = self.loads;
                        next.deck.set_ahead(AHEAD);
                        self.queue.index = i;
                        self.current = Some(next);
                        self.status = if self.current.as_ref().is_some_and(|c| c.soulseek) {
                            Status::Streaming
                        } else {
                            Status::Library
                        };
                        self.better = None;
                        self.top_up_radio();
                        self.preload();
                        self.prefetch_soon();
                        self.connect_now();
                    }
                }
                Event::Ended => {
                    let early = self
                        .current
                        .as_ref()
                        .is_some_and(|c| rules::ended_early(c.soulseek, c.complete, c.deck.duration(), c.position()));
                    if early {
                        self.status = Status::Buffering;
                    } else {
                        self.next(true);
                    }
                }
                Event::Switched => {
                    if let Some(Better::Switching { offset }) = self.better.take()
                        && let Some(current) = self.current.as_mut()
                    {
                        current.deck = lock(&self.mixer)
                            .current
                            .clone()
                            .unwrap_or_else(|| current.deck.clone());
                        current.offset = offset;
                        self.notice("Switched to a better copy".into());
                    }
                }
            }
        }
        self.decks();
        if let Some(answer) = self.radio.as_ref().and_then(|r| r.try_recv().ok()) {
            self.radio = None;
            self.radio_answered(answer);
        }
        let waiting = self.waiting();
        let load = self.current.as_ref().map(|c| c.load);
        if let Some(load) = load {
            let position = self.position();
            if let Some(started) = self.listening.time(load, position, self.playing, waiting) {
                let track = self
                    .queue
                    .queue
                    .get(self.queue.index)
                    .and_then(|e| lock(&self.catalogue).get(&e.track_id).cloned());
                if let Some(track) = track {
                    let duration = self.current.as_ref().and_then(|c| c.deck.duration()).or(track.duration);
                    // Recommended: started from radio (app.js `beginPlay`).
                    let recommended = self
                        .queue
                        .queue
                        .get(self.queue.index)
                        .is_some_and(|e| e.group == "radio");
                    self.plays.start(started, &track.id, duration, recommended);
                }
            }
        }
        self.preload();
        self.crossfade();
        self.loudness();
        self.better();
        self.prefetch();
        self.connect();
        if self.failed_at.is_some_and(|at| at.elapsed() >= rules::SKIP_FAILED) {
            self.failed_at = None;
            if matches!(self.status, Status::Failed(_))
                && self.queue.next_index(false).is_some_and(|i| i != self.queue.index)
            {
                self.notice("Skipped a song no peer shares right now".into());
                self.playing = true;
                self.next(false);
            }
        }
        self.publish();
    }

    /// What the playing deck says: opened, failed, cut short, or a seek the hub couldn't serve.
    fn decks(&mut self) {
        let Some(current) = self.current.as_ref() else {
            return;
        };
        match current.deck.phase() {
            Phase::Playing | Phase::Done if self.status == Status::Loading => {
                self.status = if current.soulseek {
                    Status::Streaming
                } else {
                    Status::Library
                };
            }
            // A seek the hub couldn't serve: back where it played. Opened where nothing has arrived (a
            // song resumed where it was): from the start, as the page does.
            Phase::Denied => {
                self.notice("The song hasn't arrived that far yet".into());
                self.reopen();
            }
            Phase::Failed(Refused::Status(416, _)) => self.reopen_at(0.0),
            Phase::Failed(refused) => {
                let message = refused.to_string();
                if self.status != Status::Failed(message.clone()) {
                    if self.playing && current.soulseek {
                        self.failed_at = Some(Instant::now());
                    }
                    self.status = Status::Failed(message);
                    self.playing = false;
                    self.run_output();
                }
            }
            // A cut response: the next attempt's event reopens it; one that came already, now.
            Phase::Cut(_) if !current.soulseek || current.complete => self.reopen(),
            Phase::Cut(_) => self.status = Status::Buffering,
            _ => {}
        }
    }

    /// A crossfade starts when the song's last seconds are as long as Settings' crossfade and the
    /// next song is ready to sound; a pause abandons it (app.js `crossfade`).
    fn crossfade(&mut self) {
        let xfade = rules::crossfade_s(self.queue.repeat, self.settings.crossfade);
        if xfade <= 0.0 || !self.playing || self.switching() {
            return;
        }
        let (Some(current), Some(next)) = (self.current.as_ref(), self.next.as_ref()) else {
            return;
        };
        let Some(length) = current.deck.duration() else {
            return;
        };
        let left = length - current.position();
        if left <= xfade && next.deck.buffered() > 0.0 {
            lock(&self.mixer).crossfade(rules::crossfade_length_s(left));
        }
    }

    /// A better copy is being lined up and switched to on the other deck (app.js `switching`).
    fn switching(&self) -> bool {
        matches!(
            self.better,
            Some(Better::Lining { .. } | Better::Lined { .. } | Better::Committing { .. } | Better::Switching { .. })
        )
    }

    /// The playing deck's level: off, ReplayGain, or measured (app.js `normalise`): the mean loudness
    /// of what has played, after 3 s of it, levelled toward Settings' target within -12 to +6 dB.
    fn loudness(&mut self) {
        let Some(current) = self.current.as_ref() else {
            return;
        };
        if self.settings.normalisation != "loudness" || self.gain_db(&current.track_id).is_some() {
            return;
        }
        let mut mixer = lock(&self.mixer);
        let (sum, frames) = mixer.heard;
        // The page reads the level once a tick.
        let readings = frames as f64 / f64::from(self.rate) / rules::TICK.as_secs_f64();
        let rms = (sum / frames.max(1) as f64).sqrt();
        if let Some(level) = rules::loudness(self.settings.loudness_target, rms, readings) {
            mixer.level = level as f32;
        }
    }

    fn gain_db(&self, id: &str) -> Option<f64> {
        if !self.settings.normalisation.starts_with("replaygain") {
            return None;
        }
        lock(&self.catalogue).get(id).and_then(|t| t.gain_db)
    }

    /// A deck's level as it starts: its ReplayGain, else unity (measured loudness comes later).
    fn level_for(&self, id: &str) -> f32 {
        if self.settings.normalisation == "off" {
            return 1.0;
        }
        self.gain_db(id).map_or(1.0, |db| rules::gain(db) as f32)
    }

    /// The volume and the equaliser into the mixer.
    fn apply_levels(&mut self) {
        let mut mixer = lock(&self.mixer);
        mixer.volume = self.volume as f32;
        mixer.eq = Chain::new(&self.eq, self.rate);
    }

    /// The better copy, step by step (app.js `trySwitch`, the output's `switchTo`).
    fn better(&mut self) {
        let Some(better) = self.better.take() else {
            return;
        };
        self.better = Some(match better {
            Better::Offered { upgrade, tries } => self.consider(upgrade, tries),
            Better::Lining {
                upgrade,
                deck,
                anchor,
                start,
                shift,
                mut recorded,
                began,
                tries,
            } => {
                self.record(&mut recorded);
                let rate = f64::from(self.rate);
                let want = (align::RECORD_S * rate) as usize;
                // The copy from `anchor + shift` of its own time is this far into its deck.
                let lead = ((anchor + shift - start) * rate) as usize;
                if matches!(deck.phase(), Phase::Failed(_)) || began.elapsed() >= SWITCH_SLOW {
                    retry(upgrade, tries)
                } else if recorded.len() < want || deck.buffered() * rate < (lead + want) as f64 {
                    Better::Lining {
                        upgrade,
                        deck,
                        anchor,
                        start,
                        shift,
                        recorded,
                        began,
                        tries,
                    }
                } else {
                    let copy = deck.peek(lead + want);
                    let (song, copy) = (&recorded[..want], &copy[lead..]);
                    let m = align::measure(song, copy, self.rate, align::WIDE_S);
                    let (song_length, copy_length) = (
                        self.current.as_ref().and_then(|c| c.deck.duration()).unwrap_or(0.0),
                        deck.duration().unwrap_or(0.0),
                    );
                    if align::locked(&m) {
                        Better::Lined {
                            upgrade,
                            lined: Lined {
                                deck,
                                anchor,
                                start,
                                frames: lead as i64 + (m.lag * rate).round() as i64,
                                matched: rules::level_match(align::rms(song), align::rms(copy)) as f32,
                            },
                            since: Instant::now(),
                            levels: Vec::new(),
                            tries,
                        }
                    } else if m.quiet {
                        retry(upgrade, tries)
                    } else if shift == 0.0 && (copy_length - song_length).abs() > 0.8 * align::WIDE_S {
                        // One copy has an intro the other hasn't: look where the song is in the copy
                        // shifted by how much longer it is.
                        self.line_up(upgrade, tries, copy_length - song_length)
                    } else {
                        Better::NextPlay {
                            peer: upgrade.peer,
                            path: upgrade.path,
                        }
                    }
                }
            }
            Better::Lined {
                upgrade,
                lined,
                since,
                mut levels,
                tries,
            } => {
                levels.push(f64::from(self.level_now()));
                if rules::quiet(&levels) || since.elapsed().as_secs_f64() >= rules::SWITCH_QUIET_S {
                    let (to, answer) = channel();
                    let (hub, path) = (
                        self.hub.clone(),
                        format!("/api/stream/{}/switch", encode(&upgrade_track(&self.current))),
                    );
                    let body = json!({ "peer": upgrade.peer, "path": upgrade.path });
                    let _ = std::thread::Builder::new()
                        .name("classic-switch".into())
                        .spawn(move || {
                            let _ = to.send(hub.json("POST", &path, Some(body)).map(drop));
                        });
                    Better::Committing { upgrade, lined, answer }
                } else {
                    Better::Lined {
                        upgrade,
                        lined,
                        since,
                        levels,
                        tries,
                    }
                }
            }
            Better::Committing { upgrade, lined, answer } => match answer.try_recv() {
                Ok(Ok(())) => {
                    let level = lock(&self.mixer).level;
                    let offset = lined.start + lined.frames as f64 / f64::from(self.rate) - lined.anchor;
                    let position = self.position();
                    let skip = ((position - lined.anchor) * f64::from(self.rate)).round() as i64 + lined.frames;
                    lock(&self.mixer).switch(lined.deck, skip, level, lined.matched, rules::SWITCH_FADE_S);
                    Better::Switching { offset }
                }
                // The hub refused: the next play takes it.
                Ok(Err(_)) => Better::NextPlay {
                    peer: upgrade.peer,
                    path: upgrade.path,
                },
                Err(_) => Better::Committing { upgrade, lined, answer },
            },
            other => other,
        });
    }

    /// Whether to start lining a better copy up now (rules.rs `switch_now`): ready, the song from
    /// Soulseek, no copy the listener picked, playing steadily, far enough from its end, and the
    /// copy arrived a little past where the song is.
    fn consider(&mut self, upgrade: Upgrade, tries: u32) -> Better {
        let now = self
            .current
            .as_ref()
            .filter(|c| upgrade.ready && c.soulseek)
            .map(|current| {
                let arrived = upgrade
                    .size
                    .filter(|s| *s > 0)
                    .zip(upgrade.received)
                    .map_or(0.0, |(size, received)| received as f64 / size as f64);
                let steady = self.playing && !self.waiting() && !lock(&self.mixer).fading();
                rules::switch_now(
                    current.source.is_some(),
                    true,
                    steady,
                    current.deck.duration(),
                    current.position(),
                    arrived,
                )
            });
        match now {
            Some(Switch::Now) => self.line_up(upgrade, tries, 0.0),
            Some(Switch::NextPlay) => Better::NextPlay {
                peer: upgrade.peer,
                path: upgrade.path,
            },
            Some(Switch::Wait) | None => Better::Offered { upgrade, tries },
        }
    }

    /// Opens the copy a little ahead of where the song is (`shift` further on in its own time) and
    /// records the song from a moment on, to measure one against the other.
    fn line_up(&mut self, upgrade: Upgrade, tries: u32, shift: f64) -> Better {
        let Some(current) = self.current.as_ref() else {
            return Better::Offered { upgrade, tries };
        };
        let anchor = current.position() + SWITCH_START + align::WIDE_S;
        let start = anchor + shift - align::WIDE_S;
        let path = format!(
            "/api/stream/{}/copy?peer={}&path={}",
            encode(&current.track_id),
            encode(&upgrade.peer),
            encode(&upgrade.path)
        );
        let url = self.hub.origin().join(&path).map_or(path, |u| u.to_string());
        let ahead = align::RECORD_S + 2.0 * align::WIDE_S + SWITCH_START + 4.0;
        let deck = Arc::new(Deck::open(
            Song {
                url,
                duration: current.deck.duration(),
            },
            self.hub.token().to_owned(),
            start.max(0.0),
            self.rate,
            ahead,
        ));
        lock(&self.mixer).record(anchor);
        Better::Lining {
            upgrade,
            deck,
            anchor,
            start: start.max(0.0),
            shift,
            recorded: Vec::new(),
            began: Instant::now(),
            tries,
        }
    }

    /// A seek or pause while a copy is being lined up: that try is over (app.js `abandonSwitch`).
    fn abandon_switch(&mut self) {
        self.better = match self.better.take() {
            Some(Better::Lining { upgrade, tries, .. } | Better::Lined { upgrade, tries, .. }) => {
                Some(retry(upgrade, tries))
            }
            other => other,
        };
    }

    /// What the mixer recorded of the playing song since the anchor it was given.
    fn record(&self, recorded: &mut Vec<f32>) {
        recorded.extend(lock(&self.mixer).recorded());
    }

    /// The level of what plays now, for a quiet moment.
    fn level_now(&self) -> f32 {
        let mixer = lock(&self.mixer);
        let n = mixer.tap.len().max(1) as f32;
        (mixer.tap.iter().map(|s| s * s).sum::<f32>() / n).sqrt()
    }

    fn notice(&mut self, line: String) {
        self.notice = Some((line, Instant::now()));
    }

    /// The snapshot the window and the tray read.
    fn publish(&mut self) {
        let (position, duration, facts) = self.current.as_ref().map_or((0.0, None, Facts::default()), |c| {
            (c.position(), c.deck.duration(), c.deck.facts())
        });
        self.notice = self
            .notice
            .take()
            .filter(|(_, at)| at.elapsed() < Duration::from_secs(6));
        let snapshot = Snapshot {
            queue: self.queue.queue.clone(),
            index: self.queue.index,
            position,
            duration,
            playing: self.playing,
            waiting: self.playing && self.waiting(),
            status: self.status.clone(),
            facts,
            shuffle: self.queue.shuffle,
            repeat: self.queue.repeat,
            volume: self.volume,
            eq: self.eq.clone(),
            revision: self.revision,
            notice: self.notice.as_ref().map(|(n, _)| n.clone()),
            reachable: self.plays.reachable(),
            stopped: self.stopped,
        };
        *lock(&self.shared.snapshot) = snapshot;
    }

    /// Classic closes: the play ends (stopped), the sound stops, and the queue goes back.
    fn finish(mut self) -> Back {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.heard_to.send(Heard::Connected);
        self.end_play(How::Stopped);
        self.playing_at_close();
        let position = self.position();
        let playing = self.playing;
        self.playing = false;
        self.run_output();
        let tracks = lock(&self.catalogue)
            .values()
            .filter_map(|t| t.entity.clone().map(|e| (t.id.clone(), e)))
            .collect();
        let (ops, threads) = self.plays.finish();
        Back {
            queue: self.queue.clone(),
            position,
            playing,
            volume: self.volume,
            eq: self.eq.clone(),
            tracks,
            ops,
            threads,
        }
    }

    fn playing_at_close(&mut self) {
        let mut mixer = lock(&self.mixer);
        mixer.next = None;
        mixer.abandon_fade();
    }
}

/// A `stream` event into what is known of a loaded song; whether the song must be reopened (a new
/// attempt of the same fetch: the old response ended). The first event of a fetch only says which
/// attempt the response belongs to.
fn adopt(loaded: &mut Loaded, event: &StreamEvent) -> bool {
    if let Some(fraction) = event.fraction() {
        loaded.received = fraction;
        loaded.complete = fraction >= 1.0;
    }
    let attempt = event.attempt.unwrap_or(0);
    if loaded.job != event.job_id {
        loaded.job = event.job_id.clone();
        loaded.attempt = attempt;
        false
    } else if attempt > loaded.attempt {
        loaded.attempt = attempt;
        true
    } else {
        false
    }
}

/// The playing song's id, for the better copy's switch.
fn upgrade_track(current: &Option<Loaded>) -> String {
    current.as_ref().map(|c| c.track_id.clone()).unwrap_or_default()
}

/// A try that didn't work out: another later, or after three the next play takes the copy.
fn retry(upgrade: Upgrade, tries: u32) -> Better {
    if tries + 1 >= rules::SWITCH_TRIES {
        Better::NextPlay {
            peer: upgrade.peer,
            path: upgrade.path,
        }
    } else {
        Better::Offered {
            upgrade,
            tries: tries + 1,
        }
    }
}

/// A song radio added, from the hub's record of it.
fn track_from(id: &str, entity: &Value) -> Track {
    let text = |k: &str| entity[k].as_str().unwrap_or_default().to_owned();
    Track {
        id: id.to_owned(),
        title: text("title"),
        artist: text("artist"),
        album: text("album"),
        artist_id: text("artistId"),
        album_id: text("albumId"),
        duration: entity["duration"].as_f64(),
        saved: entity["saved"].as_bool().unwrap_or(false),
        liked: false,
        art: None,
        gain_db: None,
        entity: (!entity.is_null()).then(|| entity.clone()),
    }
}

/// Listening history's calls: a play's start (POST) and, once it ends, its end (PATCH), each play on
/// a thread that waits for its end; what the hub can't take becomes an outbox op (web/app.js
/// `recordPlay`, web/offline.js).
struct Plays {
    hub: Hub,
    ends: HashMap<u64, Sender<(f64, How)>>,
    threads: Vec<JoinHandle<()>>,
    ops: Arc<Mutex<Vec<Value>>>,
    reachable: Arc<AtomicBool>,
}

impl Plays {
    fn new(hub: Hub) -> Self {
        Self {
            hub,
            ends: HashMap::new(),
            threads: Vec::new(),
            ops: Arc::default(),
            reachable: Arc::new(AtomicBool::new(true)),
        }
    }

    fn reachable(&self) -> bool {
        self.reachable.load(Ordering::Relaxed)
    }

    fn start(&mut self, load: u64, track: &str, duration: Option<f64>, recommended: bool) {
        let (end, ended) = channel::<(f64, How)>();
        let (hub, ops, reachable) = (self.hub.clone(), self.ops.clone(), self.reachable.clone());
        let id = track.to_owned();
        let started_at = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let spawned = std::thread::Builder::new().name("classic-play".into()).spawn(move || {
            let started = hub.start_play(&id, duration, recommended);
            reachable.store(!matches!(started, Err(Failure::Offline)), Ordering::Relaxed);
            let Ok((heard, how)) = ended.recv() else {
                return;
            };
            // The end as the hub takes it: `PATCH /api/plays/{id}`, or the outbox's `play` op.
            let body = rules::play_end(heard, how);
            match started {
                Ok(Some(play)) => {
                    if let Err(Failure::Refused(why)) = hub.end_play(&play, &body) {
                        eprintln!("lull: Classic play {play} not ended: {why}");
                    }
                }
                // Listening privately: the hub kept nothing, so there's nothing to end.
                Ok(None) => {}
                Err(Failure::Offline) => {
                    let mut op = json!({ "op": "play", "trackId": id, "startedAt": started_at });
                    if let (Some(op), Some(body)) = (op.as_object_mut(), body.as_object()) {
                        op.extend(body.clone());
                    }
                    lock(&ops).push(op);
                }
                Err(Failure::Refused(why)) => eprintln!("lull: Classic play not recorded: {why}"),
            }
        });
        match spawned {
            Ok(thread) => {
                self.threads.push(thread);
                self.ends.insert(load, end);
            }
            Err(e) => eprintln!("lull: Classic play not recorded: {e}"),
        }
    }

    /// Another call that must be done before the page takes the queue back (a like).
    fn spawn(&mut self, work: impl FnOnce() + Send + 'static) {
        match std::thread::Builder::new().name("classic-hub".into()).spawn(work) {
            Ok(thread) => self.threads.push(thread),
            Err(e) => eprintln!("lull: Classic can't reach the hub: {e}"),
        }
    }

    fn end(&mut self, load: u64, heard: f64, how: How) {
        if let Some(end) = self.ends.remove(&load) {
            let _ = end.send((heard, how));
        }
        self.threads.retain(|t| !t.is_finished());
    }

    fn finish(&mut self) -> (Arc<Mutex<Vec<Value>>>, Vec<JoinHandle<()>>) {
        (self.ops.clone(), std::mem::take(&mut self.threads))
    }
}

#[cfg(test)]
mod tests {
    use super::super::output::Manual;
    use super::super::queue::Entry;
    use super::super::testhub::{Book, Serve, TestHub, tone};
    use super::*;

    const RATE: u32 = 48_000;

    /// A player on the stand-in output, its queue the given songs (id, saved, seconds; `None`: the
    /// hub refuses it), from the first, playing.
    fn player(hub: &TestHub, songs: &[(&str, bool, Option<f64>)], repeat: Repeat) -> Option<(Engine, Arc<Manual>)> {
        {
            let mut book = hub.book.lock().ok()?;
            for (i, (id, _, seconds)) in songs.iter().enumerate() {
                let serve = match seconds {
                    Some(s) => Serve::Whole(tone(*s, 300.0 + 100.0 * i as f64, 0.5)),
                    None => Serve::Refuse("No peer delivered this song.".into()),
                };
                book.songs.insert((*id).to_owned(), serve);
            }
        }
        let tracks = songs
            .iter()
            .map(|(id, saved, seconds)| Track {
                id: (*id).to_owned(),
                title: (*id).to_owned(),
                saved: *saved,
                duration: *seconds,
                ..Track::default()
            })
            .collect();
        let queue = Queue {
            queue: songs
                .iter()
                .map(|(id, _, _)| Entry {
                    track_id: (*id).to_owned(),
                    group: "context".into(),
                    source: None,
                    orig: None,
                })
                .collect(),
            index: 0,
            shuffle: false,
            repeat,
            radio: None,
            context: Value::Null,
        };
        let start = Start {
            queue,
            tracks,
            position: 0.0,
            playing: true,
            volume: 1.0,
            eq: Equaliser::flat(),
        };
        let mixer = Arc::new(Mutex::new(Mixer::new(RATE)));
        let render = {
            let mixer = mixer.clone();
            Box::new(move |out: &mut [f32]| lock(&mixer).render(out))
        };
        let output = Arc::new(Manual::new(render, RATE));
        let hub = Hub::new(tauri::Url::parse(&hub.origin).ok()?, "token".into());
        let engine = Engine::with_output(hub, start, Box::new(output.clone()), mixer, false).ok()?;
        Some((engine, output))
    }

    /// `seconds` of sound, taken four times faster than it plays (the engine's clock still sees
    /// steps under 1.5 s, as listening history wants).
    fn listen(output: &Manual, seconds: f64) -> Vec<f32> {
        let mut sound = Vec::new();
        let chunk = RATE as usize / 25;
        for _ in 0..(seconds * 25.0) as usize {
            sound.extend(output.pull(chunk));
            std::thread::sleep(Duration::from_millis(10));
        }
        sound
    }

    /// Plays on until `done`, or for at most 20 s of sound (a slow machine gets the time it needs).
    fn listen_until(output: &Manual, mut done: impl FnMut() -> bool) -> bool {
        for _ in 0..40 {
            if done() {
                return true;
            }
            listen(output, 0.5);
        }
        done()
    }

    fn hub(settings: Value) -> Option<TestHub> {
        TestHub::start(Book {
            settings,
            ..Book::default()
        })
    }

    #[test]
    fn songs_follow_each_other_without_a_gap_and_each_is_a_play() -> Result<(), String> {
        let hub = hub(json!({})).ok_or("no hub")?;
        let (engine, output) =
            player(&hub, &[("a", true, Some(2.0)), ("b", true, Some(2.0))], Repeat::Off).ok_or("no player")?;
        let mut sound = listen(&output, 5.0);
        listen_until(&output, || engine.snapshot().index == 1);
        sound.extend(listen(&output, 0.5));
        let first = sound.iter().position(|s| s.abs() > 0.01).unwrap_or(sound.len());
        // From the first sound to the second song's end, never a stretch of silence (a gap).
        let silent = sound[first..first + 3 * RATE as usize * 2]
            .as_chunks::<2>()
            .0
            .chunk_by(|a, b| (a[0].abs() < 1e-4) == (b[0].abs() < 1e-4))
            .filter(|run| run[0][0].abs() < 1e-4)
            .map(<[_]>::len)
            .max()
            .unwrap_or(0);
        assert!(silent < RATE as usize / 100, "{silent} silent frames between the songs");
        assert_eq!(engine.snapshot().index, 1);
        drop(engine);
        std::thread::sleep(Duration::from_millis(400));
        let started = hub.calls("POST", "/api/plays");
        assert_eq!(
            started
                .iter()
                .map(|p| p["trackId"].as_str().unwrap_or(""))
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
        // Completed by itself; its seconds counted by the shared rule (listen.rs). This test's clock
        // runs four times fast: a 2 s song gets two or three time reports, and the first two add
        // nothing, so at most a second.
        let ended = hub.calls("PATCH", "/api/plays/");
        let first = ended.first().ok_or("no end")?;
        assert_eq!(
            (first["completed"].clone(), first["skipped"].clone()),
            (json!(true), json!(false))
        );
        assert!(
            first["playedS"].as_i64().is_some_and(|s| (0..=1).contains(&s)),
            "{first}"
        );
        Ok(())
    }

    #[test]
    fn a_song_no_peer_delivers_is_skipped_after_a_beat_and_never_a_play() -> Result<(), String> {
        let hub = hub(json!({})).ok_or("no hub")?;
        let (engine, output) =
            player(&hub, &[("gone", false, None), ("b", true, Some(3.0))], Repeat::Off).ok_or("no player")?;
        let failed = || matches!(engine.snapshot().status, Status::Failed(ref why) if why.contains("No peer"));
        assert!(listen_until(&output, failed));
        let started = Instant::now();
        assert!(listen_until(&output, || engine.snapshot().index == 1));
        assert!(
            started.elapsed() >= Duration::from_millis(2000),
            "after a beat, not at once"
        );
        assert!(engine.snapshot().playing);
        drop(engine);
        std::thread::sleep(Duration::from_millis(300));
        assert!(hub.calls("POST", "/api/plays").iter().all(|p| p["trackId"] != "gone"));
        Ok(())
    }

    #[test]
    fn the_hub_prefetches_the_unsaved_songs_ahead() -> Result<(), String> {
        let hub =
            hub(json!({ "playback": { "prefetchCount": { "value": 2, "source": "config" } } })).ok_or("no hub")?;
        let songs = [
            ("a", true, Some(5.0)),
            ("s", true, Some(1.0)),
            ("u1", false, Some(1.0)),
            ("u2", false, Some(1.0)),
        ];
        let (engine, output) = player(&hub, &songs, Repeat::Off).ok_or("no player")?;
        assert!(listen_until(&output, || !hub
            .calls("PUT", "/api/stream/prefetch")
            .is_empty()));
        assert_eq!(
            hub.calls("PUT", "/api/stream/prefetch").last(),
            Some(&json!({ "tracks": ["u1", "u2"] }))
        );
        drop(engine);
        Ok(())
    }

    #[test]
    fn radio_carries_on_at_the_queues_end() -> Result<(), String> {
        let hub = TestHub::start(Book {
            settings: json!({}),
            radio: vec!["r1".into(), "r2".into()],
            ..Book::default()
        })
        .ok_or("no hub")?;
        hub.book
            .lock()
            .map(|mut b| {
                b.songs.insert("r1".into(), Serve::Whole(tone(2.0, 500.0, 0.5)));
                b.songs.insert("r2".into(), Serve::Whole(tone(2.0, 600.0, 0.5)));
            })
            .ok();
        let (engine, output) = player(&hub, &[("a", true, Some(1.0))], Repeat::Off).ok_or("no player")?;
        assert!(listen_until(&output, || engine.snapshot().index == 1));
        let now = engine.snapshot();
        assert_eq!(
            now.queue.iter().map(|e| e.track_id.as_str()).collect::<Vec<_>>(),
            ["a", "r1", "r2"]
        );
        assert_eq!((now.index, now.queue[1].group.as_str()), (1, "radio"));
        // Asked as the last song loaded (it seeds the radio), and again while fewer than three wait.
        assert!(!hub.calls("GET", "/api/radio").is_empty());
        Ok(())
    }

    #[test]
    fn repeat_one_replays_the_song_as_a_new_play() -> Result<(), String> {
        let hub = hub(json!({})).ok_or("no hub")?;
        let (engine, output) = player(&hub, &[("a", true, Some(3.0))], Repeat::One).ok_or("no player")?;
        assert!(listen_until(&output, || hub.calls("POST", "/api/plays").len() >= 2));
        assert_eq!(engine.snapshot().index, 0);
        drop(engine);
        std::thread::sleep(Duration::from_millis(300));
        assert!(hub.calls("POST", "/api/plays").len() >= 2);
        assert_eq!(
            hub.calls("PATCH", "/api/plays/")
                .first()
                .map(|e| e["completed"].clone()),
            Some(json!(true))
        );
        Ok(())
    }

    #[test]
    fn a_fallbacks_new_attempt_reopens_the_song_where_it_was() -> Result<(), String> {
        let hub = hub(json!({})).ok_or("no hub")?;
        let (engine, output) = player(&hub, &[("s", false, Some(6.0))], Repeat::Off).ok_or("no player")?;
        listen(&output, 1.0);
        let event = |attempt| {
            Heard::Stream(Box::new(StreamEvent {
                track_id: "s".into(),
                job_id: Some("j".into()),
                status: "streaming".into(),
                attempt: Some(attempt),
                received: Some(100),
                size: Some(1000),
                ..StreamEvent::default()
            }))
        };
        let _ = engine.heard_to.send(event(0));
        // The engine hears events on its tick (250 ms).
        std::thread::sleep(Duration::from_millis(600));
        let fetches = || {
            hub.book
                .lock()
                .map(|b| b.fetches.get("s").copied().unwrap_or(0))
                .unwrap_or(0)
        };
        let before = fetches();
        // The first event of a fetch only says which attempt it is: nothing reopens.
        assert_eq!(before, fetches());
        let _ = engine.heard_to.send(event(1));
        std::thread::sleep(Duration::from_millis(600));
        listen(&output, 0.5);
        assert!(fetches() > before, "a new attempt opens the song again");
        assert!(engine.snapshot().position > 1.0, "it carried on from where it was");
        Ok(())
    }

    fn stream_event(id: &str, received: u64, upgrade: Option<Upgrade>) -> Heard {
        Heard::Stream(Box::new(StreamEvent {
            track_id: id.into(),
            job_id: Some("j".into()),
            status: "streaming".into(),
            attempt: Some(0),
            received: Some(received),
            size: Some(1000),
            upgrade,
            ..StreamEvent::default()
        }))
    }

    #[test]
    fn a_better_copy_is_lined_up_and_switched_to_mid_song() -> Result<(), String> {
        let hub = hub(json!({})).ok_or("no hub")?;
        let (engine, output) = player(&hub, &[("s", false, Some(60.0))], Repeat::Off).ok_or("no player")?;
        // The song is noise; the better copy is the same noise from a rip that starts 0.2 s later.
        hub.book
            .lock()
            .map(|mut b| {
                b.songs
                    .insert("s".into(), Serve::Whole(super::super::testhub::noise(60.0, 0.0)));
                b.copies.insert("s".into(), super::super::testhub::noise(60.0, 0.2));
            })
            .map_err(|e| e.to_string())?;
        listen(&output, 1.0);
        let upgrade = Upgrade {
            peer: "better".into(),
            path: "rips/s.flac".into(),
            received: Some(1000),
            size: Some(1000),
            ready: true,
            ..Upgrade::default()
        };
        let _ = engine.heard_to.send(stream_event("s", 1000, Some(upgrade)));
        for _ in 0..40 {
            listen(&output, 1.0);
            if !hub.calls("POST", "/api/stream/s/switch").is_empty() {
                break;
            }
        }
        assert_eq!(
            hub.calls("POST", "/api/stream/s/switch").first(),
            Some(&json!({ "peer": "better", "path": "rips/s.flac" }))
        );
        let before = engine.snapshot().position;
        listen(&output, 2.0);
        let after = engine.snapshot();
        assert!(after.position > before, "the song carried on in its own time");
        assert_eq!(after.notice.as_deref(), Some("Switched to a better copy"));
        Ok(())
    }

    #[test]
    fn listening_privately_records_nothing_to_end() -> Result<(), String> {
        let hub = TestHub::start(Book {
            settings: json!({}),
            private: true,
            ..Book::default()
        })
        .ok_or("no hub")?;
        let (engine, output) =
            player(&hub, &[("a", true, Some(1.0)), ("b", true, Some(1.0))], Repeat::Off).ok_or("no player")?;
        assert!(listen_until(&output, || hub.calls("POST", "/api/plays").len() >= 2));
        drop(engine);
        std::thread::sleep(Duration::from_millis(300));
        assert!(!hub.calls("POST", "/api/plays").is_empty());
        assert!(hub.calls("PATCH", "/api/plays/").is_empty());
        Ok(())
    }

    #[test]
    fn a_seek_past_what_has_arrived_is_held_at_its_edge() -> Result<(), String> {
        let hub = hub(json!({})).ok_or("no hub")?;
        let (engine, output) = player(&hub, &[("s", false, Some(20.0))], Repeat::Off).ok_or("no player")?;
        listen(&output, 0.5);
        let _ = engine.heard_to.send(stream_event("s", 100, None));
        std::thread::sleep(Duration::from_millis(600));
        engine.send(Command::Seek(15.0));
        std::thread::sleep(Duration::from_millis(600));
        listen(&output, 0.2);
        let now = engine.snapshot();
        assert!(
            now.position < 2.5,
            "held inside the tenth that has arrived: {}",
            now.position
        );
        assert_eq!(now.notice.as_deref(), Some("The song hasn't arrived that far yet"));
        Ok(())
    }

    #[test]
    fn plays_the_hub_cant_take_go_back_for_the_outbox() -> Result<(), String> {
        let hub = TestHub::start(Book {
            settings: json!({}),
            unreachable: vec!["/api/plays".into()],
            ..Book::default()
        })
        .ok_or("no hub")?;
        let (engine, output) = player(&hub, &[("a", true, Some(5.0))], Repeat::Off).ok_or("no player")?;
        listen(&output, 2.0);
        let back = engine.finish().ok_or("no hand-back")?;
        for thread in back.threads {
            let _ = thread.join();
        }
        let ops = lock(&back.ops).clone();
        let op = ops.first().ok_or("no op")?;
        assert_eq!((op["op"].clone(), op["trackId"].clone()), (json!("play"), json!("a")));
        assert_eq!(
            (op["completed"].clone(), op["skipped"].clone()),
            (json!(false), json!(false))
        );
        assert!(op["startedAt"].as_u64().is_some_and(|t| t > 0) && op["playedS"].as_f64().is_some());
        Ok(())
    }

    #[test]
    fn lull_connect_hears_what_plays() -> Result<(), String> {
        let hub = hub(json!({})).ok_or("no hub")?;
        let (engine, output) =
            player(&hub, &[("a", true, Some(3.0)), ("b", true, Some(3.0))], Repeat::Off).ok_or("no player")?;
        assert!(listen_until(&output, || !hub
            .calls("POST", "/api/devices/me/state")
            .is_empty()));
        let state = hub.calls("POST", "/api/devices/me/state");
        let last = state.last().ok_or("no state")?;
        assert_eq!(
            (
                last["trackId"].clone(),
                last["queue"].clone(),
                last["queueIndex"].clone()
            ),
            (json!("a"), json!(["a", "b"]), json!(0))
        );
        drop(engine);
        Ok(())
    }

    #[test]
    fn a_like_reaches_the_hub_or_waits_in_the_outbox() -> Result<(), String> {
        let hub = hub(json!({})).ok_or("no hub")?;
        let (engine, output) = player(&hub, &[("a", true, Some(5.0))], Repeat::Off).ok_or("no player")?;
        listen(&output, 0.5);
        engine.send(Command::Like);
        assert!(listen_until(&output, || !hub.calls("PUT", "/api/likes/a").is_empty()));
        assert!(engine.track("a").is_some_and(|t| t.liked));
        hub.book
            .lock()
            .map(|mut b| b.unreachable = vec!["/api/likes".into()])
            .map_err(|e| e.to_string())?;
        engine.send(Command::Like);
        listen(&output, 0.5);
        let back = engine.finish().ok_or("no hand-back")?;
        for thread in back.threads {
            let _ = thread.join();
        }
        // The unlike couldn't reach the hub: it waits in the outbox, as the page's would.
        assert_eq!(
            lock(&back.ops).first(),
            Some(&json!({ "op": "like", "trackId": "a", "liked": false }))
        );
        Ok(())
    }
}
