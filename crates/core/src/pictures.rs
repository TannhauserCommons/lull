//! The hub's one service for outside pictures (ADR-0033): album covers and artist photos that
//! come from another site, whichever route asks for them (`/api/catalog/art`, library's
//! `/api/art` for an album without a picture of its own, Subsonic's `getCoverArt`).
//!
//! - **Instant:** a cached picture (or a larger cached one, scaled down) and a remembered "none"
//!   are answered at once and never queued.
//! - **The screen sets the order.** Each device reports what it shows (`report`). A job runs by
//!   its best place: the playing song's and the next ones' covers, then what is visible, then
//!   what is near, then what a waiting request asked for; newest screen first, then top to
//!   bottom. A job no screen shows, nothing plays and no request waits for is dropped before it
//!   starts. A retry keeps whatever place the screen gives it.
//! - **Lanes.** A job waits in the lane of the outside service it calls next (`FETCH`: a CDN;
//!   a finder's lanes: one per looked-up service), each with its own workers, so a slow service
//!   never holds up another. Paced services take their turn when the call goes out (`Http::pace`).
//! - **One fetch per picture:** the largest size wanted while it waited is fetched once, and
//!   every smaller served size is scaled from it.
//! - `art.ready` names each picture that came in.
//!
//! Where a picture comes from is a `Finder`'s (discovery registers one): the source it already
//! knows, or the steps to find one outside, each in its lane.

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use axum::response::Response;
use serde::Deserialize;
use tokio::sync::{Notify, broadcast, watch};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::art::{self, SIZES};
use crate::events::Bus;
use crate::http::Http;
use crate::{Error, Result};

/// The lane of jobs whose source is known: a fetch from a CDN or the Cover Art Archive.
pub const FETCH: &str = "fetch";
/// Fetches at once: several CDNs, each held to `http::PER_HOST`.
const FETCH_WORKERS: usize = 8;
/// How long a request waits for a picture lull doesn't have yet before answering "not yet".
const WAIT: Duration = Duration::from_secs(3);
const FETCH_TIMEOUT: Duration = Duration::from_secs(20);
/// A fetched picture is used this long before it is fetched again.
pub const KEEP: Duration = Duration::from_secs(30 * 24 * 3600);
/// "The source has none" is believed this long.
pub const NONE_FOR: Duration = Duration::from_secs(24 * 3600);
/// A failed step is tried again after this (twice that the second time).
const RETRY_AFTER: Duration = Duration::from_secs(20);
const RETRIES: u32 = 2;
/// A device's screen counts this long after its last report (a closed tab sends no more).
const SCREEN_KEEP: Duration = Duration::from_secs(10 * 60);
/// Pictures one report may name: a few screens of a dense grid.
const MAX_SHOWN: usize = 400;

/// Where a picture stands for the listener, most urgent first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Place {
    /// The playing song's album and the next ones'.
    Playing,
    Visible,
    /// Within about a screen of view.
    Near,
    /// A request waits for it, but no screen shows it (a Subsonic app, a page before its report).
    Asked,
}

/// The order jobs run in: the place, then the newest screen, then the order on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Rank {
    pub place: Place,
    newest: Reverse<u64>,
    pos: usize,
}

/// Where a picture comes from, as far as lull knows without an outside call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// A picture URL with `{size}` (`sized`).
    Url(String),
    /// Not known yet: the finder's `step` finds it, in `lane`.
    Find { lane: &'static str, step: u8 },
    /// lull has no picture for this id.
    Unknown,
}

/// What one step of finding a picture came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    Url(String),
    /// No source has one.
    Missing,
    /// Not this one: try `step` in `lane`.
    Next {
        lane: &'static str,
        step: u8,
    },
}

/// Knows where pictures come from (discovery's catalog and artist photos).
#[async_trait]
pub trait Finder: Send + Sync {
    /// Where `id`'s picture comes from, from what lull has stored: no outside call.
    async fn source(&self, id: &str) -> Result<Source>;
    /// One step of finding it outside. A source found is recorded by the finder, so the next
    /// `source` knows it.
    async fn find(&self, id: &str, step: u8) -> Result<Found>;
}

/// The route a picture on screen is drawn from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Route {
    /// `/api/catalog/art/{id}`: always an outside picture.
    Catalog,
    /// Library's `/api/art/{albumId}`: the album's own picture, else an outside one.
    Library,
}

/// A picture on a device's screen: its route, its id and the size it is drawn at. Size 0 is an
/// artist drawn without a photo (one may still be found).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Shown {
    pub route: Route,
    pub id: String,
    pub size: u32,
}

/// `PUT /api/pictures/screen`: what a device shows now, top to bottom. `gen` grows with every
/// report from the device, so a late one is ignored. Pictures are `[route/id, size]`, the route
/// as the picture's URL names it (`catalog/art/al-…`, `art/al-…`).
#[derive(Debug, Deserialize)]
pub struct Report {
    #[serde(rename = "gen")]
    pub generation: u64,
    #[serde(default)]
    pub visible: Vec<(String, u32)>,
    #[serde(default)]
    pub near: Vec<(String, u32)>,
}

impl Shown {
    /// A reported picture, checked: a known route, an id of the kind it serves (prefix and hex),
    /// a served size (0 only for an artist).
    pub fn parse(pic: &str, size: u32) -> Result<Shown> {
        let bad = || Error::BadRequest(format!("not a picture lull serves: {pic}"));
        let (route, id) = if let Some(id) = pic.strip_prefix("catalog/art/") {
            (Route::Catalog, id)
        } else if let Some(id) = pic.strip_prefix("art/") {
            (Route::Library, id)
        } else {
            return Err(bad());
        };
        let kind_ok = match route {
            Route::Catalog => is_id(id),
            Route::Library => id.starts_with("al-") && is_id(id),
        };
        if !kind_ok {
            return Err(bad());
        }
        let kind = id.split('-').next().unwrap_or_default();
        if !(SIZES.contains(&size) || (size == 0 && kind == "ar")) {
            return Err(Error::BadRequest(format!("{pic}: size must be one lull serves")));
        }
        Ok(Shown {
            route,
            id: id.to_owned(),
            size,
        })
    }
}

/// A picture id: `al-`, `ar-` or `pls-` and hex. Ids become file names, so nothing else is.
pub fn is_id(id: &str) -> bool {
    id.split_once('-').is_some_and(|(kind, hex)| {
        matches!(kind, "al" | "ar" | "pls") && !hex.is_empty() && hex.bytes().all(|b| b.is_ascii_hexdigit())
    })
}

/// `PUT /api/pictures/screen`: the calling device's screen (`Report`) → 204. 400 for a picture
/// lull doesn't serve or a size it doesn't; a report older than the device's last is ignored.
pub fn router(pictures: Pictures) -> axum::Router {
    axum::Router::new()
        .route("/api/pictures/screen", axum::routing::put(screen))
        .with_state(pictures)
}

async fn screen(
    axum::extract::State(pictures): axum::extract::State<Pictures>,
    axum::Extension(device): axum::Extension<crate::devices::Device>,
    body: std::result::Result<axum::Json<Report>, axum::extract::rejection::JsonRejection>,
) -> Result<axum::http::StatusCode> {
    let axum::Json(report) = body.map_err(|e| Error::BadRequest(e.body_text()))?;
    pictures.report(&device.id, report)?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// What a request for a picture gets.
#[derive(Debug)]
pub enum Answer {
    /// At the size asked, or smaller when its source is.
    Image(Vec<u8>),
    /// On its way; `art.ready` names it when it is in.
    NotYet,
    /// No source has one; believed this much longer.
    None(Duration),
}

impl Answer {
    pub fn into_response(self, id: &str) -> Response {
        match self {
            Answer::Image(bytes) => art::response(bytes),
            Answer::NotYet => art::not_yet(id),
            Answer::None(left) => art::none(id, left),
        }
    }
}

/// The outside-picture service, shared by every crate (`Ctx::pictures`).
#[derive(Clone)]
pub struct Pictures(Arc<Inner>);

struct Inner {
    dir: PathBuf,
    http: Http,
    shutdown: CancellationToken,
    queue: Mutex<Queue>,
    finder: OnceLock<Arc<dyn Finder>>,
    /// Per lane, what wakes its workers; a lane's workers start with its first job.
    lanes: Mutex<HashMap<&'static str, Arc<Notify>>>,
    /// Pictures that came onto a screen, for the crates that start work for them.
    shown: broadcast::Sender<Arc<Vec<Shown>>>,
    ready: Arc<art::Ready>,
}

impl Pictures {
    /// The service, caching under `dir`, until `shutdown`.
    pub fn new(dir: PathBuf, http: Http, bus: Bus, shutdown: CancellationToken) -> Pictures {
        Pictures(Arc::new(Inner {
            dir,
            http,
            ready: art::Ready::new(bus, shutdown.clone()),
            shutdown,
            queue: Mutex::default(),
            finder: OnceLock::new(),
            lanes: Mutex::default(),
            shown: broadcast::channel(64).0,
        }))
    }

    /// Where pictures come from; set once, by discovery at start.
    pub fn set_finder(&self, finder: Arc<dyn Finder>) {
        let _ = self.0.finder.set(finder);
    }

    /// The picture for `id` at `size` (one of `art::SIZES`): from the cache at once, else found and
    /// fetched, waiting at most `WAIT`; `Err(NotFound)` when lull has no picture for `id`.
    pub async fn get(&self, id: &str, size: u32) -> Result<Answer> {
        art::check(size)?;
        if !is_id(id) {
            return Err(Error::NotFound(format!("lull has no picture for {id}")));
        }
        if let Some(bytes) = self.cached(id, size).await {
            return Ok(Answer::Image(bytes));
        }
        if let Some(left) = self.none_left(id).await {
            return Ok(Answer::None(left));
        }
        let start = self.source(id).await?;
        if start == Source::Unknown {
            return Err(Error::NotFound(format!("lull has no picture for {id}")));
        }
        let (mut done, _asker) = self.ask(id, size, start, true);
        // Ended (or the hub stopping) either way; what matters is what is on disk now.
        let _ = tokio::time::timeout(WAIT, done.wait_for(|d| *d)).await;
        Ok(match self.cached(id, size).await {
            Some(bytes) => Answer::Image(bytes),
            None => match self.none_left(id).await {
                Some(left) => Answer::None(left),
                None => Answer::NotYet,
            },
        })
    }

    /// Fetches `id` at `size` in the background, unless it is here or surely missing: for what a
    /// screen shows and what plays. Its place comes from the screen (or `set_playing`); with
    /// neither it is dropped before it starts.
    pub async fn want(&self, id: &str, size: u32) {
        if !is_id(id) || !SIZES.contains(&size) {
            return;
        }
        if self.cached(id, size).await.is_some() || self.none_left(id).await.is_some() {
            return;
        }
        match self.source(id).await {
            Ok(Source::Unknown) => {}
            Ok(start) => drop(self.ask(id, size, start, false)),
            Err(e) => tracing::debug!(id, error = %e, "picture source unreadable"),
        }
    }

    /// A device's screen. Pictures new to it are announced to `shown()`'s receivers, and the
    /// catalog's are wanted here.
    pub fn report(&self, device: &str, report: Report) -> Result<()> {
        if report.visible.len() + report.near.len() > MAX_SHOWN {
            return Err(Error::BadRequest(format!("at most {MAX_SHOWN} pictures a screen")));
        }
        let parse = |l: Vec<(String, u32)>| {
            l.into_iter()
                .map(|(p, s)| Shown::parse(&p, s))
                .collect::<Result<Vec<_>>>()
        };
        let (visible, near) = (parse(report.visible)?, parse(report.near)?);
        let Some(new) = self
            .lock()
            .report(device, report.generation, visible, near, Instant::now())
        else {
            return Ok(());
        };
        let catalog: Vec<Shown> = new
            .iter()
            .filter(|s| s.route == Route::Catalog && s.size > 0)
            .cloned()
            .collect();
        if !catalog.is_empty() {
            let me = self.clone();
            self.spawn(async move {
                for s in catalog {
                    me.want(&s.id, s.size).await;
                }
            });
        }
        let _ = self.0.shown.send(Arc::new(new));
        Ok(())
    }

    /// Pictures that came onto a screen, per report. After `Lagged`, read `on_screen`.
    pub fn shown(&self) -> broadcast::Receiver<Arc<Vec<Shown>>> {
        self.0.shown.subscribe()
    }

    /// Every picture on a current screen.
    pub fn on_screen(&self) -> Vec<Shown> {
        self.lock().on_screen()
    }

    /// `id`'s best place now, if any: for work outside this service that follows the screen.
    pub fn rank(&self, id: &str) -> Option<Rank> {
        self.lock().ranks.get(id).copied()
    }

    /// The playing song's album, then the next songs', each with the sizes to have ready, as
    /// `owner` (the crate that serves them) sees them: they go first, and replace what `owner`
    /// said played before.
    pub async fn set_playing(&self, owner: &'static str, albums: Vec<(String, Vec<u32>)>) {
        self.lock()
            .set_playing(owner, albums.iter().map(|(id, _)| id.clone()).collect());
        for (id, sizes) in albums {
            for size in sizes {
                self.want(&id, size).await;
            }
        }
    }

    /// Tells the pages a picture changed source (an artist's photo vetted): `art.ready` carries
    /// its new `image` URL, and the cached copies of the old one go.
    pub async fn replaced(&self, id: &str, image: Option<String>) {
        self.forget(id).await;
        if let Some(url) = image {
            self.0.ready.announce_image(id, url);
        }
    }

    /// The cache of `id` at `size`; a larger cached size is scaled down to it (and kept).
    pub async fn cached(&self, id: &str, size: u32) -> Option<Vec<u8>> {
        let path = self.file(id, size);
        if let Some(bytes) = read_fresh(&path, KEEP).await {
            return Some(bytes);
        }
        for larger in SIZES.into_iter().filter(|&s| s > size) {
            if let Some(bytes) = read_fresh(&self.file(id, larger), KEEP).await {
                let bytes = scale(bytes, size).await?;
                write(&path, &bytes).await;
                return Some(bytes);
            }
        }
        None
    }

    /// How much longer "the source has none" is believed for `id`, if at all.
    pub async fn none_left(&self, id: &str) -> Option<Duration> {
        let modified = tokio::fs::metadata(self.none_file(id)).await.ok()?.modified().ok()?;
        let age = SystemTime::now().duration_since(modified).unwrap_or_default();
        NONE_FOR.checked_sub(age).filter(|left| !left.is_zero())
    }

    /// Removes every cached size of `id`, and "none".
    pub async fn forget(&self, id: &str) {
        if !is_id(id) {
            return;
        }
        for size in SIZES {
            let _ = tokio::fs::remove_file(self.file(id, size)).await;
        }
        let _ = tokio::fs::remove_file(self.none_file(id)).await;
    }

    fn file(&self, id: &str, size: u32) -> PathBuf {
        self.0.dir.join(format!("{id}-{size}"))
    }

    /// "The source has none", one file for every size.
    fn none_file(&self, id: &str) -> PathBuf {
        self.0.dir.join(format!("{id}.none"))
    }

    async fn source(&self, id: &str) -> Result<Source> {
        match self.0.finder.get() {
            Some(finder) => finder.source(id).await,
            None => Ok(Source::Unknown),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Queue> {
        self.0.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn spawn(&self, task: impl std::future::Future<Output = ()> + Send + 'static) {
        let shutdown = self.0.shutdown.clone();
        tokio::spawn(async move {
            shutdown.run_until_cancelled(task).await;
        });
    }

    /// Joins or starts the job for `id`; the receiver turns `true` when it ends. A request holds
    /// the `Asker` while it waits.
    fn ask(&self, id: &str, size: u32, start: Source, asker: bool) -> (watch::Receiver<bool>, Option<Asker>) {
        let (done, lane) = self.lock().ask(id, size, start, asker);
        if let Some(lane) = lane {
            self.wake(lane);
        }
        let asker = asker.then(|| Asker {
            pictures: self.clone(),
            id: id.to_owned(),
        });
        (done, asker)
    }

    /// Wakes `lane`'s workers, starting them with its first job.
    fn wake(&self, lane: &'static str) {
        let (notify, new) = {
            let mut lanes = self.0.lanes.lock().unwrap_or_else(PoisonError::into_inner);
            match lanes.get(lane) {
                Some(n) => (n.clone(), false),
                None => {
                    let n = Arc::new(Notify::new());
                    lanes.insert(lane, n.clone());
                    (n, true)
                }
            }
        };
        if new {
            let workers = if lane == FETCH { FETCH_WORKERS } else { 1 };
            for _ in 0..workers {
                let (me, notify) = (self.clone(), notify.clone());
                self.spawn(async move { me.work(lane, notify).await });
            }
        }
        notify.notify_waiters();
    }

    /// One worker of `lane`: the best-ranked job whose time has come, for as long as the hub runs.
    async fn work(&self, lane: &'static str, notify: Arc<Notify>) {
        loop {
            // Made before looking, so a job pushed in between still wakes this worker.
            let woken = notify.notified();
            let picked = self.lock().pick(lane, Instant::now());
            match picked {
                Pick::Job(id, task) => self.run(id, task).await,
                Pick::Wait(until) => match until {
                    Some(at) => {
                        tokio::select! {
                            () = woken => {}
                            () = tokio::time::sleep_until(at) => {}
                        }
                    }
                    None => woken.await,
                },
            }
        }
    }

    /// One step of a job: a fetch, or one of the finder's steps.
    async fn run(&self, id: String, task: Task) {
        let outcome = match (&task.source, self.0.finder.get()) {
            (Some(url), _) => self.fetch_into(&id, url, task.want).await,
            (None, Some(finder)) => finder.find(&id, task.step).await.map(Step::Found),
            (None, None) => Ok(Step::Found(Found::Missing)),
        };
        match &outcome {
            Ok(Step::Fetched(_)) => self.0.ready.announce(&id),
            Ok(Step::Found(Found::Missing)) => write(&self.none_file(&id), &[]).await,
            Ok(_) => {}
            Err(e) => tracing::info!(id, error = %e, "picture not fetched; tried again later"),
        }
        let next = {
            let mut q = self.lock();
            match outcome {
                Ok(Step::Fetched(size)) => q.finish(&id, Some(size)),
                Ok(Step::Found(Found::Url(url))) => q.again(&id, FETCH, 0, Some(url), None),
                Ok(Step::Found(Found::Next { lane, step })) => q.again(&id, lane, step, None, None),
                Ok(Step::Found(Found::Missing)) => q.finish(&id, None),
                Err(_) => q.retry(&id, Instant::now()),
            }
        };
        if let Some(lane) = next {
            self.wake(lane);
        }
    }

    /// The source's picture at its best for `want`, scaled to it and cached, with every smaller
    /// served size scaled from it. `Found(Missing)` when the source has none.
    async fn fetch_into(&self, id: &str, url: &str, want: u32) -> Result<Step> {
        if self.cached(id, want).await.is_some() {
            return Ok(Step::Fetched(want));
        }
        let at = sized(url, want);
        let mut fetched = fetch(&self.0.http, &at).await?;
        if fetched.is_none()
            && let Some(original) = archive_original(&at)
        {
            fetched = fetch(&self.0.http, &original).await?;
        }
        let Some(bytes) = fetched else {
            return Ok(Step::Found(Found::Missing));
        };
        for size in SIZES.into_iter().filter(|&s| s <= want).rev() {
            let Some(scaled) = scale(bytes.clone(), size).await else {
                continue;
            };
            write(&self.file(id, size), &scaled).await;
        }
        Ok(Step::Fetched(want))
    }
}

/// A request waiting for a picture; while one is, the job keeps the `Asked` place.
struct Asker {
    pictures: Pictures,
    id: String,
}

impl Drop for Asker {
    fn drop(&mut self) {
        self.pictures.lock().leave(&self.id);
    }
}

enum Step {
    /// Cached at this size and every smaller one.
    Fetched(u32),
    Found(Found),
}

/// One job per picture: where it stands and who waits for it.
struct Job {
    lane: &'static str,
    step: u8,
    /// Known: the next step is a fetch.
    source: Option<String>,
    /// The largest size wanted.
    want: u32,
    /// Requests waiting on it.
    askers: usize,
    /// When the latest request asked (`Queue::seq`), newest first among asked ones.
    asked: u64,
    tries: u32,
    not_before: Option<Instant>,
    done: watch::Sender<bool>,
}

/// What a worker needs of a job; the job itself stays in `Queue::running`.
struct Task {
    source: Option<String>,
    step: u8,
    want: u32,
}

enum Pick {
    Job(String, Task),
    /// Nothing to run now; look again then (or when woken).
    Wait(Option<Instant>),
}

/// Screens, ranks and jobs: the ordering, apart from the I/O.
#[derive(Default)]
struct Queue {
    seq: u64,
    screens: HashMap<String, Screen>,
    /// Per crate, the albums it serves for what plays now and next.
    playing: HashMap<&'static str, Vec<String>>,
    /// The best place of every picture on a current screen or playing.
    ranks: HashMap<String, Rank>,
    waiting: HashMap<String, Job>,
    running: HashMap<String, Job>,
}

struct Screen {
    generation: u64,
    seq: u64,
    at: Instant,
    visible: Vec<Shown>,
    near: Vec<Shown>,
}

impl Queue {
    fn next_seq(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    /// Replaces `device`'s screen unless `gen` is older; the pictures new to it, or `None`.
    fn report(
        &mut self,
        device: &str,
        generation: u64,
        visible: Vec<Shown>,
        near: Vec<Shown>,
        now: Instant,
    ) -> Option<Vec<Shown>> {
        let before: HashSet<&Shown> = match self.screens.get(device) {
            Some(s) if s.generation >= generation => return None,
            Some(s) => s.visible.iter().chain(&s.near).collect(),
            None => HashSet::new(),
        };
        let new: Vec<Shown> = visible
            .iter()
            .chain(&near)
            .filter(|s| !before.contains(s))
            .cloned()
            .collect();
        let seq = self.next_seq();
        self.screens.insert(
            device.to_owned(),
            Screen {
                generation,
                seq,
                at: now,
                visible,
                near,
            },
        );
        self.rerank(now);
        Some(new)
    }

    fn set_playing(&mut self, owner: &'static str, albums: Vec<String>) {
        self.playing.insert(owner, albums);
        self.rerank(Instant::now());
    }

    fn on_screen(&self) -> Vec<Shown> {
        let now = Instant::now();
        let mut seen = HashSet::new();
        self.screens
            .values()
            .filter(|s| now.duration_since(s.at) < SCREEN_KEEP)
            .flat_map(|s| s.visible.iter().chain(&s.near))
            .filter(|s| seen.insert(*s))
            .cloned()
            .collect()
    }

    /// Every picture's best place, then the waiting jobs nothing wants any more are dropped.
    fn rerank(&mut self, now: Instant) {
        self.screens.retain(|_, s| now.duration_since(s.at) < SCREEN_KEEP);
        let mut ranks: HashMap<String, Rank> = HashMap::new();
        let mut put = |id: &str, rank: Rank| {
            ranks
                .entry(id.to_owned())
                .and_modify(|r| *r = (*r).min(rank))
                .or_insert(rank);
        };
        for (pos, id) in self.playing.values().flat_map(|l| l.iter().enumerate()) {
            put(
                id,
                Rank {
                    place: Place::Playing,
                    newest: Reverse(u64::MAX),
                    pos,
                },
            );
        }
        for s in self.screens.values() {
            for (place, list) in [(Place::Visible, &s.visible), (Place::Near, &s.near)] {
                for (pos, shown) in list.iter().enumerate() {
                    put(
                        &shown.id,
                        Rank {
                            place,
                            newest: Reverse(s.seq),
                            pos,
                        },
                    );
                }
            }
        }
        self.ranks = ranks;
        let unwanted: Vec<String> = self
            .waiting
            .iter()
            .filter(|(id, job)| self.rank_of(id, job).is_none())
            .map(|(id, _)| id.clone())
            .collect();
        for id in unwanted {
            if let Some(job) = self.waiting.remove(&id) {
                job.done.send_replace(true);
            }
        }
    }

    fn rank_of(&self, id: &str, job: &Job) -> Option<Rank> {
        let asked = (job.askers > 0).then_some(Rank {
            place: Place::Asked,
            newest: Reverse(job.asked),
            pos: 0,
        });
        self.ranks.get(id).copied().into_iter().chain(asked).min()
    }

    /// Joins `id`'s job (raising the size it fetches), or starts one at `start`. The lane to wake
    /// comes back when the job is new.
    fn ask(
        &mut self,
        id: &str,
        size: u32,
        start: Source,
        asker: bool,
    ) -> (watch::Receiver<bool>, Option<&'static str>) {
        let seq = self.next_seq();
        let join = |job: &mut Job| {
            job.want = job.want.max(size);
            if asker {
                job.askers += 1;
                job.asked = seq;
            }
            job.done.subscribe()
        };
        if let Some(job) = self.running.get_mut(id) {
            return (join(job), None);
        }
        if let Some(job) = self.waiting.get_mut(id) {
            return (join(job), None);
        }
        let (lane, step, source) = match start {
            Source::Url(url) => (FETCH, 0, Some(url)),
            Source::Find { lane, step } => (lane, step, None),
            Source::Unknown => return (watch::channel(true).1, None),
        };
        let mut job = Job {
            lane,
            step,
            source,
            want: size,
            askers: 0,
            asked: 0,
            tries: 0,
            not_before: None,
            done: watch::channel(false).0,
        };
        let done = join(&mut job);
        self.waiting.insert(id.to_owned(), job);
        (done, Some(lane))
    }

    /// A request stopped waiting.
    fn leave(&mut self, id: &str) {
        if let Some(job) = self.running.get_mut(id).or_else(|| self.waiting.get_mut(id)) {
            job.askers = job.askers.saturating_sub(1);
        }
    }

    /// The best-ranked job of `lane` whose time has come. Jobs nothing wants are dropped here.
    fn pick(&mut self, lane: &'static str, now: Instant) -> Pick {
        let mut best: Option<(Rank, &String)> = None;
        let mut soonest: Option<Instant> = None;
        let mut unwanted = Vec::new();
        for (id, job) in self.waiting.iter().filter(|(_, j)| j.lane == lane) {
            let Some(rank) = self.rank_of(id, job) else {
                unwanted.push(id.clone());
                continue;
            };
            if let Some(at) = job.not_before.filter(|at| *at > now) {
                soonest = Some(soonest.map_or(at, |s| s.min(at)));
                continue;
            }
            if best.is_none_or(|(b, _)| rank < b) {
                best = Some((rank, id));
            }
        }
        let chosen = best.map(|(_, id)| id.clone());
        for id in unwanted {
            if let Some(job) = self.waiting.remove(&id) {
                job.done.send_replace(true);
            }
        }
        match chosen.and_then(|id| self.waiting.remove(&id).map(|job| (id, job))) {
            Some((id, job)) => {
                let task = Task {
                    source: job.source.clone(),
                    step: job.step,
                    want: job.want,
                };
                self.running.insert(id.clone(), job);
                Pick::Job(id, task)
            }
            None => Pick::Wait(soonest),
        }
    }

    /// The job goes on in `lane` (with its source, when found). The lane to wake comes back.
    fn again(
        &mut self,
        id: &str,
        lane: &'static str,
        step: u8,
        source: Option<String>,
        not_before: Option<Instant>,
    ) -> Option<&'static str> {
        let mut job = self.running.remove(id)?;
        job.lane = lane;
        job.step = step;
        if source.is_some() {
            job.source = source;
            job.tries = 0;
        }
        job.not_before = not_before;
        self.waiting.insert(id.to_owned(), job);
        Some(lane)
    }

    /// A failed step waits and is tried again, twice at most; its place is still the screen's.
    fn retry(&mut self, id: &str, now: Instant) -> Option<&'static str> {
        let job = self.running.get_mut(id)?;
        job.tries += 1;
        if job.tries > RETRIES {
            return self.finish(id, None);
        }
        let (lane, step, source, tries) = (job.lane, job.step, job.source.clone(), job.tries);
        self.again(id, lane, step, source, Some(now + RETRY_AFTER * tries))
    }

    /// The job ended. A larger size asked while it fetched is fetched next.
    fn finish(&mut self, id: &str, fetched: Option<u32>) -> Option<&'static str> {
        let job = self.running.get(id)?;
        if let Some(size) = fetched
            && job.want > size
        {
            let source = job.source.clone();
            return self.again(id, FETCH, 0, source, None);
        }
        let job = self.running.remove(id)?;
        job.done.send_replace(true);
        None
    }
}

/// The source URL at its best for `size`, scaled down after. The Cover Art Archive and Last.fm
/// have their own sizes: the smallest that covers the request (250 counts for 256), else the
/// original. Deezer is asked for at most `DEEZER_LARGEST`; iTunes and Wikimedia Commons serve
/// any size up to their original and never more.
pub fn sized(source: &str, size: u32) -> String {
    if source.starts_with("https://coverartarchive.org/") {
        return match art::archive_size(size) {
            Some(thumbnail) => source.replace("{size}", &thumbnail.to_string()),
            None => source.replace("-{size}", ""),
        };
    }
    if lastfm_picture(source).is_some() {
        // Checked 2026-11-04: any other size answers the original.
        return match size {
            0..=174 => source.replace("{size}", "174s"),
            175..=300 => source.replace("{size}", "300x300"),
            _ => source.replace("{size}/", ""),
        };
    }
    let size = if source.contains(".dzcdn.net/") {
        size.min(DEEZER_LARGEST)
    } else {
        size
    };
    source.replace("{size}", &size.to_string())
}

/// A Last.fm picture's address split where its size goes: `https://<host>/i/u/` and the rest
/// (`<size>/<file>`, or `<file>` for the original). Its host is one of Last.fm's on Fastly, whichever
/// it answers with: `lastfm.freetls.fastly.net` until 2026-11, `lastfm-img.freetls.fastly.net`
/// since. A source keeps the host it was given (`sized`).
pub fn lastfm_picture(url: &str) -> Option<(&str, &str)> {
    let after = url.strip_prefix("https://")?;
    let (host, path) = after.split_once('/')?;
    let rest = path.strip_prefix("i/u/")?;
    let lastfm = host.starts_with("lastfm") && host.ends_with(".freetls.fastly.net");
    lastfm.then(|| url.split_at(url.len() - rest.len()))
}

/// The largest Deezer picture asked for. Its CDN serves up to 1900 px whatever the master is,
/// and 1400 px is the smallest master it takes from labels, so this size is never blown up.
const DEEZER_LARGEST: u32 = 1400;

/// The archive's original for one of its thumbnails (`…/front-1200` → `…/front`).
fn archive_original(url: &str) -> Option<String> {
    if !url.starts_with("https://coverartarchive.org/") {
        return None;
    }
    let (front, size) = url.rsplit_once('-')?;
    size.parse::<u32>().ok().map(|_| front.to_owned())
}

/// A picture's bytes, `None` when the source answers 404. At most `http::PER_HOST` at once per host.
pub async fn fetch(http: &Http, url: &str) -> Result<Option<Vec<u8>>> {
    let service = if url.starts_with("https://coverartarchive.org/") {
        "Cover Art Archive"
    } else if url.contains(".mzstatic.com/") {
        "iTunes"
    } else if url.starts_with("https://commons.wikimedia.org/") {
        "Wikimedia Commons"
    } else if url.contains(".dzcdn.net/") {
        "Deezer"
    } else if lastfm_picture(url).is_some() {
        "Last.fm"
    } else {
        "picture host"
    };
    let _place = http.host_place(url).await;
    let resp = http
        .client()
        .get(url)
        .timeout(FETCH_TIMEOUT)
        .send()
        .await
        .map_err(|e| Error::unavailable(service, e.without_url().to_string()))?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let resp = crate::http::ok(service, resp)?;
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| Error::upstream(service, format!("picture cut short: {e}")))?;
    Ok(Some(bytes.to_vec()))
}

async fn scale(bytes: Vec<u8>, size: u32) -> Option<Vec<u8>> {
    tokio::task::spawn_blocking(move || art::fit(bytes, size)).await.ok()
}

async fn read_fresh(path: &Path, keep: Duration) -> Option<Vec<u8>> {
    let modified = tokio::fs::metadata(path).await.ok()?.modified().ok()?;
    let age = SystemTime::now().duration_since(modified).unwrap_or_default();
    if age > keep {
        return None;
    }
    tokio::fs::read(path).await.ok()
}

/// Best effort: a picture that can't be cached is fetched again next time.
async fn write(path: &Path, bytes: &[u8]) {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".part");
    let tmp = PathBuf::from(tmp);
    let written = async {
        if let Some(dir) = path.parent() {
            tokio::fs::create_dir_all(dir).await?;
        }
        tokio::fs::write(&tmp, bytes).await?;
        tokio::fs::rename(&tmp, path).await
    };
    if let Err(e) = written.await {
        tracing::warn!(error = %e, path = %path.display(), "could not cache a picture");
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn on(ids: &[&str]) -> Vec<Shown> {
        ids.iter()
            .map(|id| Shown {
                route: Route::Catalog,
                id: (*id).to_owned(),
                size: 256,
            })
            .collect()
    }

    fn fetch_of(q: &mut Queue, id: &str, asker: bool) -> watch::Receiver<bool> {
        q.ask(
            id,
            256,
            Source::Url(format!("https://cdn.example/{id}/{{size}}")),
            asker,
        )
        .0
    }

    /// The order `lane` runs its jobs in, each finished as it is picked.
    fn order(q: &mut Queue, now: Instant) -> Vec<String> {
        let mut ran = Vec::new();
        while let Pick::Job(id, _) = q.pick(FETCH, now) {
            q.finish(&id, None);
            ran.push(id);
        }
        ran
    }

    #[test]
    fn the_screen_orders_the_work_playing_then_visible_then_near_newest_screen_first() {
        // Live, a 5,000-song list scrolled fast filled its covers in the order the answers named
        // the albums, so the screen the listener stopped on filled last.
        let mut q = Queue::default();
        let now = Instant::now();
        q.report("phone", 1, on(&["v-old"]), on(&["n-old"]), now);
        q.report("laptop", 1, on(&["v1", "v2"]), on(&["n1"]), now);
        q.set_playing("discovery", vec!["p".into()]);
        for id in ["n-old", "q", "v2", "n1", "v-old", "p", "v1"] {
            fetch_of(&mut q, id, id == "q");
        }
        assert_eq!(order(&mut q, now), ["p", "v1", "v2", "v-old", "n1", "n-old", "q"]);
    }

    #[test]
    fn work_that_left_the_screen_is_dropped_before_it_starts() {
        let mut q = Queue::default();
        let now = Instant::now();
        q.report("laptop", 1, on(&["a", "b"]), vec![], now);
        let a = fetch_of(&mut q, "a", false);
        fetch_of(&mut q, "b", false);
        fetch_of(&mut q, "c", true);
        q.report("laptop", 2, on(&["b"]), vec![], now);
        assert!(*a.borrow(), "a's job ended: nothing wants it");
        assert_eq!(
            q.report("laptop", 1, on(&["a"]), vec![], now),
            None,
            "a report that arrives late changes nothing"
        );
        assert_eq!(order(&mut q, now), ["b", "c"], "a request still waits for c");
        fetch_of(&mut q, "d", true);
        q.leave("d");
        assert_eq!(order(&mut q, now), Vec::<String>::new(), "nobody waits for d any more");
    }

    #[test]
    fn a_retry_waits_and_keeps_the_screens_place() {
        let mut q = Queue::default();
        let now = Instant::now();
        q.report("laptop", 1, on(&["a", "b", "c"]), vec![], now);
        for id in ["a", "b", "c"] {
            fetch_of(&mut q, id, false);
        }
        let Pick::Job(first, _) = q.pick(FETCH, now) else {
            unreachable!("a is first on screen");
        };
        assert_eq!(first, "a");
        assert_eq!(q.retry("a", now), Some(FETCH), "a failed: it waits");
        let later = now + RETRY_AFTER;
        assert_eq!(order(&mut q, now), ["b", "c"], "a waits out its pause");
        // Meanwhile the listener scrolled: a is near now, d visible.
        q.report("laptop", 2, on(&["d"]), on(&["a"]), now);
        fetch_of(&mut q, "d", false);
        assert_eq!(
            order(&mut q, later),
            ["d", "a"],
            "the retry takes the screen's new place"
        );
    }

    #[test]
    fn each_source_is_asked_for_its_best_at_the_size() {
        let archive = "https://coverartarchive.org/release-group/0f3c0c64-5b0c/front-{size}";
        assert!(sized(archive, 96).ends_with("front-250"));
        assert!(sized(archive, 256).ends_with("front-250"));
        assert!(sized(archive, 512).ends_with("front-500"));
        assert!(sized(archive, 1024).ends_with("front-1200"));
        assert!(sized(archive, 2048).ends_with("/0f3c0c64-5b0c/front"), "the original");
        let deezer = "https://cdn-images.dzcdn.net/images/cover/d06e/{size}x{size}-000000-80-0-0.jpg";
        assert!(sized(deezer, 1024).contains("/1024x1024-"));
        // Live, its CDN answered 403 above 1900 px and may blow up a small master below that.
        assert!(sized(deezer, 2048).contains("/1400x1400-"));
        let itunes = "https://is1-ssl.mzstatic.com/image/thumb/x/1.png/{size}x{size}bb.jpg";
        // Live, iTunes answered 2048 at 2048 px and 5000 at its 4000 px original.
        assert!(sized(itunes, 2048).ends_with("/2048x2048bb.jpg"));
        // Live (2026-11-08), both of Last.fm's hosts served 174s, 300x300 and the original.
        for host in ["lastfm.freetls.fastly.net", "lastfm-img.freetls.fastly.net"] {
            let lastfm = format!("https://{host}/i/u/{{size}}/0a1b.jpg");
            assert_eq!(sized(&lastfm, 96), format!("https://{host}/i/u/174s/0a1b.jpg"));
            assert_eq!(sized(&lastfm, 256), format!("https://{host}/i/u/300x300/0a1b.jpg"));
            assert_eq!(
                sized(&lastfm, 512),
                format!("https://{host}/i/u/0a1b.jpg"),
                "the original"
            );
        }
    }

    #[test]
    fn a_lastfm_picture_is_known_by_its_host_and_path_on_any_of_its_hosts() {
        assert_eq!(
            lastfm_picture("https://lastfm-img.freetls.fastly.net/i/u/300x300/0a1b.jpg"),
            Some(("https://lastfm-img.freetls.fastly.net/i/u/", "300x300/0a1b.jpg"))
        );
        assert_eq!(
            lastfm_picture("https://lastfm.freetls.fastly.net/i/u/0a1b.jpg"),
            Some(("https://lastfm.freetls.fastly.net/i/u/", "0a1b.jpg"))
        );
        assert_eq!(
            lastfm_picture("https://other.freetls.fastly.net/i/u/0a1b.jpg"),
            None,
            "another site"
        );
        assert_eq!(lastfm_picture("https://lastfm.freetls.fastly.net/other/0a1b.jpg"), None);
        assert_eq!(lastfm_picture("http://lastfm.freetls.fastly.net/i/u/0a1b.jpg"), None);
    }

    #[test]
    fn an_archive_thumbnail_falls_back_to_the_original() {
        assert_eq!(
            archive_original("https://coverartarchive.org/release-group/0f3c0c64-5b0c/front-1200").as_deref(),
            Some("https://coverartarchive.org/release-group/0f3c0c64-5b0c/front")
        );
        assert_eq!(
            archive_original("https://coverartarchive.org/release-group/0f3c0c64-5b0c/front"),
            None
        );
        assert_eq!(archive_original("https://cdn.example/1200"), None);
    }

    #[test]
    fn a_reported_picture_is_checked_at_the_boundary() {
        assert!(Shown::parse("catalog/art/al-0a1f", 256).is_ok());
        assert!(
            Shown::parse("catalog/art/ar-0a1f", 0).is_ok(),
            "an artist without a photo"
        );
        assert!(Shown::parse("art/al-0a1f", 96).is_ok());
        for (pic, size) in [
            ("art/ar-0a1f", 96),
            ("catalog/art/al-0a1f", 0),
            ("catalog/art/al-0a1f", 300),
            ("catalog/art/al-../x", 96),
            ("catalog/art/tr-0a", 96),
            ("playlists/pl-1/cover", 96),
        ] {
            assert!(
                matches!(Shown::parse(pic, size), Err(Error::BadRequest(_))),
                "{pic} {size}"
            );
        }
    }

    /// A finder that serves `url` for every id and counts how often it is asked.
    struct Fixed {
        url: String,
        asked: AtomicUsize,
    }

    #[async_trait]
    impl Finder for Fixed {
        async fn source(&self, _id: &str) -> Result<Source> {
            self.asked.fetch_add(1, Ordering::SeqCst);
            Ok(Source::Url(self.url.clone()))
        }
        async fn find(&self, _id: &str, _step: u8) -> Result<Found> {
            Ok(Found::Missing)
        }
    }

    fn png(side: u32) -> Vec<u8> {
        let mut out = Vec::new();
        let image = image::RgbImage::from_pixel(side, side, image::Rgb([30, 140, 200]));
        let written = image.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png);
        assert!(written.is_ok());
        out
    }

    fn pictures(name: &str, url: &str) -> (Pictures, Arc<Fixed>) {
        let dir = std::env::temp_dir().join(format!("lull-pictures-{name}-{}", std::process::id()));
        let http = Http::new().unwrap_or_else(|e| unreachable!("http client: {e}"));
        let p = Pictures::new(dir, http, Bus::new(), CancellationToken::new());
        let finder = Arc::new(Fixed {
            url: url.to_owned(),
            asked: AtomicUsize::new(0),
        });
        p.set_finder(finder.clone());
        (p, finder)
    }

    #[tokio::test]
    async fn a_cached_picture_is_answered_at_once_and_never_queued() -> Result<()> {
        let (p, finder) = pictures("cached", "https://unreachable.invalid/{size}");
        write(&p.file("al-0a", 256), &png(256)).await;
        let Answer::Image(row) = p.get("al-0a", 96).await? else {
            unreachable!("the tile's picture makes the row's");
        };
        assert_eq!(art::dimensions(&row), Some((96, 96)));
        assert_eq!(finder.asked.load(Ordering::SeqCst), 0, "no source looked up");
        assert!(
            p.lock().waiting.is_empty() && p.lock().running.is_empty(),
            "nothing queued"
        );
        let _ = tokio::fs::remove_dir_all(&p.0.dir).await;
        Ok(())
    }

    #[tokio::test]
    async fn the_largest_size_wanted_is_fetched_once_and_scaled_for_the_rest() -> Result<()> {
        let served = Arc::new(AtomicUsize::new(0));
        let counter = served.clone();
        let app = axum::Router::new().route(
            "/pic/{size}",
            axum::routing::get(move || {
                counter.fetch_add(1, Ordering::SeqCst);
                async { png(1200) }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| Error::internal(e.to_string()))?;
        let port = listener
            .local_addr()
            .map_err(|e| Error::internal(e.to_string()))?
            .port();
        tokio::spawn(async move { axum::serve(listener, app).await });
        let (p, _) = pictures("largest", &format!("http://127.0.0.1:{port}/pic/{{size}}"));
        let Answer::Image(hero) = p.get("al-0b", 1024).await? else {
            unreachable!("the source answers at once");
        };
        assert_eq!(art::dimensions(&hero), Some((1024, 1024)));
        for size in [96, 256, 512] {
            let Answer::Image(smaller) = p.get("al-0b", size).await? else {
                unreachable!("scaled from the fetched picture");
            };
            assert_eq!(art::dimensions(&smaller), Some((size, size)));
        }
        assert_eq!(served.load(Ordering::SeqCst), 1, "one outside fetch for every size");
        let _ = tokio::fs::remove_dir_all(&p.0.dir).await;
        Ok(())
    }
}
