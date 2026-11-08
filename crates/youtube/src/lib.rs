//! YouTube as a second source, through yt-dlp (docs/context/youtube.md). It finds the upload that
//! is the song, and downloads its native audio stream into a file soulseek's stream cache holds,
//! through the `Youtube` seam: soulseek decides when YouTube plays (the listener's mode), this
//! crate decides which upload and which stream. It also keeps yt-dlp current, and backs off when
//! YouTube asks for a sign-in check or says lull asked too often.

mod api;
mod check;
mod cut;
mod formats;
mod lanes;
mod matching;
mod playlist;
mod search;
mod sound;
mod update;
mod ytdlp;

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::Router;
use lull_core::db::Migrations;
use lull_core::ids::TrackId;
use lull_core::model::{SourceRef, Track};
use lull_core::seams::{Copies, Library, Previews, Youtube};
use lull_core::status::State;
use lull_core::youtube::{Copy, Found, FoundPlaylist, FoundSong, Listing, Mode, Settings, Want};
use lull_core::{Ctx, Error, Result};
use serde_json::{Value, json};
use std::sync::atomic::Ordering;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Semaphore;

use crate::ytdlp::{Problem, Ytdlp};

pub const MIGRATIONS: Migrations = Migrations {
    owner: "youtube",
    steps: &[
        r#"
        -- The upload matched to a song under one strictness; video_id NULL: nothing matched.
        CREATE TABLE youtube_matches (
            track_id    TEXT NOT NULL,
            strictness  TEXT NOT NULL,
            video_id    TEXT,
            official    INTEGER NOT NULL DEFAULT 0,
            checked_at  INTEGER NOT NULL,          -- unix seconds
            PRIMARY KEY (track_id, strictness)
        );
        CREATE INDEX youtube_matches_video ON youtube_matches(video_id);
    "#,
        r#"
        -- Matching reads a fan's "Title - Artist" and translated uploads now: misses are looked for again.
        DELETE FROM youtube_matches WHERE video_id IS NULL;
    "#,
        r#"
        -- An upload's sound against the track's references (ADR-0016): same, near (another master
        -- or mix), other (another recording), or unchecked (no reference, unreadable audio).
        CREATE TABLE youtube_checks (
            track_id    TEXT NOT NULL,
            video_id    TEXT NOT NULL,
            verdict     TEXT NOT NULL,
            score       REAL,                      -- bit error rate at the best offset
            reference   TEXT,                      -- "apple:<id>", "deezer:<id>", "acoustid"
            checked_at  INTEGER NOT NULL,          -- unix seconds
            PRIMARY KEY (track_id, video_id)
        );
        -- The catalog's previews of a track, fingerprinted: [{ source, id, print }]; [] = none.
        CREATE TABLE youtube_references (
            track_id    TEXT PRIMARY KEY,
            refs        TEXT NOT NULL,
            checked_at  INTEGER NOT NULL
        );
        -- Fan uploads matched before sound checks are found again, checked before they play;
        -- official ones are checked the next time they play.
        DELETE FROM youtube_matches WHERE official = 0;
    "#,
        r#"
        -- Fingerprints of the song's own copies, references for its others (ADR-0016): keys
        -- "listener:<track>:<copy>", "library:<track>", "soulseek:<peer>\n<path>",
        -- "youtube:<track>:<video>" (an official upload that played).
        CREATE TABLE youtube_prints (
            key         TEXT PRIMARY KEY,
            print       TEXT NOT NULL,             -- base64 of little-endian fingerprint items
            checked_at  INTEGER NOT NULL
        );
    "#,
        r#"
        -- Whether an upload is the song under one strictness, for shelves' availability marks
        -- (search::available): from one flat video search judged by playback's rule, without
        -- the extraction playback's own match makes. Found kept 30 days, not found 1.
        CREATE TABLE youtube_available (
            track_id    TEXT NOT NULL,
            strictness  TEXT NOT NULL,
            available   INTEGER NOT NULL,
            checked_at  INTEGER NOT NULL,          -- unix seconds
            PRIMARY KEY (track_id, strictness)
        );
    "#,
        r#"
        -- A song cut from a full-album upload: where it lies in the upload (ms; ADR-0016).
        ALTER TABLE youtube_matches ADD COLUMN section_from INTEGER;
        ALTER TABLE youtube_matches ADD COLUMN section_to INTEGER;
    "#,
    ],
};

/// Searches and extractions start at least this far apart, lull-wide.
const PACE: Duration = Duration::from_millis(1500);
/// At most this many searches or extractions, and this many downloads, at once.
const LOOKUPS_AT_ONCE: usize = 2;
const DOWNLOADS_AT_ONCE: usize = 3;
/// After a sign-in check or a 429, YouTube is left alone this long, doubling up to the cap.
const BACKOFF_FIRST: Duration = Duration::from_secs(15 * 60);
const BACKOFF_MAX: Duration = Duration::from_secs(6 * 3600);
const HOUSEKEEPING: Duration = Duration::from_secs(15 * 60);
/// Unpacked yt-dlp runs left behind by a cancelled download are removed after this long.
const LEFTOVERS: Duration = Duration::from_secs(30 * 60);
/// A full-album upload's audio and its songs' cuts are kept this long (`cut`).
const ALBUMS_FOR: Duration = Duration::from_secs(24 * 3600);

pub struct Service {
    ctx: Ctx,
    /// `<data>/youtube`.
    dir: PathBuf,
    ytdlp: Ytdlp,
    settings: RwLock<Settings>,
    backoff: Mutex<Option<Backoff>>,
    lookups: Semaphore,
    downloads: Semaphore,
    /// Who goes first for a lookup: the listener's own, then lookups for later (`find_later`).
    lanes: lanes::Lanes,
    /// One search per song at a time: a preload and the player asking together search once.
    finding: Mutex<std::collections::HashMap<TrackId, Arc<tokio::sync::Mutex<()>>>>,
    /// Availability checks (`available`): one at a time, so a lookup is always free for what
    /// the listener does...
    availability: Semaphore,
    /// ...and only while nothing else runs: yt-dlp runs in flight, and those of them checks.
    running: std::sync::atomic::AtomicUsize,
    checking: std::sync::atomic::AtomicUsize,
    updates: update::State,
    /// Sound checks (ADR-0016).
    checks: check::Checks,
    /// One fetch of a full-album upload's audio at a time, per upload.
    albums: Mutex<std::collections::HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

/// Counts one in flight for as long as it lives.
struct Counted<'a>(&'a std::sync::atomic::AtomicUsize);

impl<'a> Counted<'a> {
    fn on(n: &'a std::sync::atomic::AtomicUsize) -> Self {
        n.fetch_add(1, Ordering::SeqCst);
        Counted(n)
    }
}

impl Drop for Counted<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

#[derive(Debug, Clone)]
struct Backoff {
    until: Instant,
    /// Unix seconds, for the status and `/api/youtube`.
    until_unix: i64,
    problem: Problem,
    /// How many back-offs in a row: the next one is twice as long.
    times: u32,
}

pub async fn start(ctx: &Ctx) -> Result<Arc<Service>> {
    let dir = ctx.config.data_dir.join("youtube");
    for sub in ["info", "cache", "tmp", "deno", "bin", "check", "albums"] {
        tokio::fs::create_dir_all(dir.join(sub))
            .await
            .map_err(|e| Error::internal(format!("creating {}: {e}", dir.join(sub).display())))?;
    }
    let config = &ctx.config.youtube;
    let svc = Arc::new(Service {
        ctx: ctx.clone(),
        ytdlp: Ytdlp::new(config.ytdlp.clone(), config.deno.clone(), dir.clone()),
        dir,
        settings: RwLock::default(),
        backoff: Mutex::default(),
        lookups: Semaphore::new(LOOKUPS_AT_ONCE),
        downloads: Semaphore::new(DOWNLOADS_AT_ONCE),
        lanes: lanes::Lanes::default(),
        finding: Mutex::default(),
        availability: Semaphore::new(1),
        running: Default::default(),
        checking: Default::default(),
        updates: update::State::default(),
        checks: check::Checks::default(),
        albums: Mutex::default(),
    });
    svc.report();
    // Subscribed before collection starts, so its start-up `settings` arrives (ADR-0050).
    ctx.spawn(follow_settings(svc.clone()));
    ctx.spawn(update::run(svc.clone()));
    ctx.spawn(housekeeping(svc.clone()));
    Ok(svc)
}

impl Service {
    /// The catalog's previews, once discovery runs: sound checks start then (ADR-0016).
    pub fn use_previews(self: &Arc<Self>, previews: Arc<dyn Previews>) {
        self.checks.bind(previews);
        self.report();
    }

    /// The saved library and soulseek's cached copies: the song's own copies are references
    /// for its others (ADR-0016).
    pub fn use_library(&self, library: Arc<dyn Library>) {
        self.checks.bind_library(library);
    }

    pub fn use_copies(&self, copies: Arc<dyn Copies>) {
        self.checks.bind_copies(copies);
    }
}

/// `GET /api/youtube`, `POST /api/youtube/update`.
pub fn router(svc: Arc<Service>) -> Router {
    api::router(svc)
}

#[async_trait]
impl Youtube for Service {
    async fn find_later(&self, track: &Track, want: Want) -> Result<Option<Found>> {
        self.lanes.later(&track.id, self.find(track, want)).await
    }

    async fn find(&self, track: &Track, want: Want) -> Result<Option<Found>> {
        self.lanes.wanted_now(&track.id);
        let settings = self.settings();
        if settings.mode == Mode::Off {
            return Ok(None);
        }
        self.backed_off()?;
        let matched = self.matched(track, &settings).await?;
        let Some(m) = matched else { return Ok(None) };
        if let Some(section) = m.section {
            let (copy, size) = self
                .cut_copy(&m.video, section, "m4a")
                .await
                .map_err(|p| self.failed(p))?;
            let (info, _) = search::info(self, &m.video, false).await.map_err(|p| self.failed(p))?;
            return Ok(Some(Found {
                source: copy.source_ref(size),
                title: info.title.clone(),
                channel: info.channel.clone().or(info.uploader.clone()).unwrap_or_default(),
                official: m.official,
            }));
        }
        let (info, _) = search::info(self, &m.video, false).await.map_err(|p| self.failed(p))?;
        let Some(chosen) = formats::choose(&info, want, settings.format) else {
            return Err(Error::upstream(
                "youtube",
                format!("the matching upload ({}) has no audio lull can stream", m.video),
            ));
        };
        Ok(Some(Found {
            source: chosen.copy.source_ref(chosen.size),
            title: info.title.clone(),
            channel: info.channel.clone().or(info.uploader.clone()).unwrap_or_default(),
            official: m.official,
        }))
    }

    async fn download(&self, source: &SourceRef, dest: &Path) -> Result<()> {
        let copy =
            Copy::of(source).ok_or_else(|| Error::BadRequest(format!("{} is not a YouTube copy", source.path)))?;
        self.backed_off()?;
        if let Some(section) = copy.section {
            self.cut_copy(&copy.video, section, &copy.ext)
                .await
                .map_err(|p| self.failed(p))?;
            let from = cut::cut_path(&self.dir, &copy.video, section.0, section.1, &copy.ext);
            tokio::fs::copy(&from, dest)
                .await
                .map_err(|e| Error::internal(format!("copying the song's cut: {e}")))?;
            return Ok(());
        }
        let _turn = self
            .downloads
            .acquire()
            .await
            .map_err(|_| Error::internal("youtube downloads closed"))?;
        // An extraction older than YouTube's stream URLs, or one that stopped working, is redone once.
        for fresh in [false, true] {
            let (_, json) = search::info(self, &copy.video, fresh)
                .await
                .map_err(|p| self.failed(p))?;
            match self.pipe(&json, &copy.format, dest, source.size).await {
                Ok(()) => {
                    self.succeeded();
                    return Ok(());
                }
                Err((0, Problem::Other(why))) if !fresh => {
                    tracing::info!(video = %copy.video, why, "extracting the upload again for its download");
                }
                Err((_, p @ Problem::Gone(_))) => {
                    search::forget(self, &copy.video).await?;
                    return Err(self.failed(p));
                }
                Err((_, p)) => return Err(self.failed(p)),
            }
        }
        Err(Error::upstream("youtube", "the download failed twice"))
    }

    async fn search_playlists(&self, query: &str, limit: usize) -> Result<Vec<FoundPlaylist>> {
        self.backed_off()?;
        let found = playlist::search(self, query, limit).await.map_err(|p| self.failed(p))?;
        self.succeeded();
        Ok(found)
    }

    async fn available(&self, track: &Track) -> Result<Option<bool>> {
        let Some(turn) = self.availability_turn().await? else {
            return Ok(None);
        };
        let found = search::available(self, track, turn.matching)
            .await
            .map_err(|p| self.failed(p))?;
        self.succeeded();
        Ok(Some(found))
    }

    async fn album_available(&self, artist: &str, album: &str) -> Result<Option<bool>> {
        let Some(turn) = self.availability_turn().await? else {
            return Ok(None);
        };
        let found = search::album_available(self, artist, album, turn.matching)
            .await
            .map_err(|p| self.failed(p))?;
        self.succeeded();
        Ok(Some(found))
    }

    async fn search_songs(&self, query: &str) -> Result<Vec<FoundSong>> {
        self.backed_off()?;
        let found = search::songs(self, query).await.map_err(|p| self.failed(p))?;
        self.succeeded();
        Ok(found)
    }

    async fn checked(&self, track: &Track, source: &SourceRef, path: &Path) -> Option<bool> {
        let copy = Copy::of(source)?;
        // A song cut from a full-album upload: the upload's own track list is the evidence.
        if copy.section.is_some() {
            return Some(true);
        }
        check::played(self, track, &copy.video, path).await
    }

    async fn confirm(&self, track: &Track, source: &SourceRef, path: Option<&Path>, yes: bool) -> Result<()> {
        let copy = Copy::of(source).ok_or_else(|| Error::BadRequest("only a YouTube copy is asked about".into()))?;
        check::confirm(self, track, &copy.video, path, yes).await;
        Ok(())
    }

    async fn check_file(&self, track: &Track, path: &Path) -> Option<bool> {
        check::file(self, track, path).await
    }

    async fn playlist(&self, url: &str, from: usize, count: usize) -> Result<Listing> {
        self.backed_off()?;
        let listing = playlist::page(self, url, from, count)
            .await
            .map_err(|p| self.failed(p))?;
        self.succeeded();
        Ok(listing)
    }
}

/// The right to run one availability check (`Service::availability_turn`), held while it runs.
struct Turn<'a> {
    _one: tokio::sync::SemaphorePermit<'a>,
    _counted: Counted<'a>,
    matching: lull_core::youtube::Match,
}

impl Service {
    pub fn settings(&self) -> Settings {
        self.settings.read().map(|s| s.clone()).unwrap_or_default()
    }

    /// Waits for a turn at an availability check, which is one at a time and behind everything
    /// the listener waits on: it starts only once no other yt-dlp run is in flight (a play, a
    /// preload, a search), however long that takes. `None` when YouTube is off or backed off.
    async fn availability_turn(&self) -> Result<Option<Turn<'_>>> {
        let settings = self.settings();
        if settings.mode == Mode::Off || self.backed_off().is_err() {
            return Ok(None);
        }
        let one = self
            .availability
            .acquire()
            .await
            .map_err(|_| Error::internal("youtube availability checks closed"))?;
        while self.running.load(Ordering::SeqCst) > self.checking.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        Ok(Some(Turn {
            _one: one,
            _counted: Counted::on(&self.checking),
            matching: settings.matching,
        }))
    }

    /// A song cut from a full-album upload (`cut`): the upload's AAC audio fetched once for all its
    /// songs (kept in `<data>/youtube/albums` a day), the song's frames copied into a file of its
    /// own: an M4A (`ext` "m4a"; an earlier lull's cut, "aac", is raw AAC). Its copy and size.
    async fn cut_copy(&self, video: &str, section: (u32, u32), ext: &str) -> Result<(Copy, u64), Problem> {
        let copy = Copy {
            video: video.to_owned(),
            format: "140".into(),
            ext: ext.to_owned(),
            section: Some(section),
        };
        let path = cut::cut_path(&self.dir, video, section.0, section.1, ext);
        if let Ok(m) = tokio::fs::metadata(&path).await
            && m.len() > 0
        {
            return Ok((copy, m.len()));
        }
        let lock = {
            let mut albums = self.albums.lock().map_err(|_| Problem::Other("youtube lock".into()))?;
            albums.entry(video.to_owned()).or_default().clone()
        };
        let _one = lock.lock().await;
        let album = cut::album_path(&self.dir, video);
        if !album.is_file() {
            let (info, json) = search::info(self, video, false).await?;
            let chosen = formats::choose(&info, Want::Save, lull_core::youtube::Format::Aac)
                .ok_or_else(|| Problem::Other("the album upload has no AAC audio".into()))?;
            let part = album.with_extension("part");
            {
                let _turn = self
                    .downloads
                    .acquire()
                    .await
                    .map_err(|_| Problem::Other("youtube downloads closed".into()))?;
                self.pipe(&json, &chosen.copy.format, &part, chosen.size)
                    .await
                    .map_err(|(_, p)| p)?;
            }
            tokio::fs::rename(&part, &album)
                .await
                .map_err(|e| Problem::Other(format!("keeping the album's audio: {e}")))?;
        }
        let (from, to) = (album, path);
        let size = tokio::task::spawn_blocking(move || cut::cut(&from, section.0, section.1, &to))
            .await
            .map_err(|e| Problem::Other(format!("cutting the song: {e}")))?
            .map_err(Problem::Other)?;
        Ok((copy, size))
    }

    /// The remembered match, else a search (one per song at a time).
    async fn matched(&self, track: &Track, settings: &Settings) -> Result<Option<search::Matched>> {
        if let Some(known) = search::remembered(self, track, settings.matching).await? {
            return Ok(known);
        }
        let lock = {
            let mut finding = self.finding.lock().map_err(|_| Error::internal("youtube lock"))?;
            finding.entry(track.id.clone()).or_default().clone()
        };
        let _one = lock.lock().await;
        if let Some(known) = search::remembered(self, track, settings.matching).await? {
            return Ok(known);
        }
        let found = search::search(self, track, settings.matching).await;
        if let Ok(mut finding) = self.finding.lock() {
            finding.remove(&track.id);
        }
        let found = found.map_err(|p| self.failed(p))?;
        self.succeeded();
        search::remember(self, track, settings.matching, found.as_ref()).await?;
        match &found {
            Some(m) => tracing::info!(track = %track.id, video = %m.video, official = m.official, "matched on YouTube"),
            None => tracing::info!(track = %track.id, "no YouTube upload matches"),
        }
        Ok(found)
    }

    /// One yt-dlp lookup (search or extraction), paced and bounded lull-wide. An updated yt-dlp
    /// that won't run is dropped for the pinned one, and the run tried again with it.
    async fn run(&self, args: &[&str], limit: Duration) -> Result<Vec<u8>, Problem> {
        self.run_with(args, limit, false).await
    }

    /// `run`, keeping what a `-i` run printed before it failed (`Ytdlp::output`).
    async fn run_with(&self, args: &[&str], limit: Duration, partial: bool) -> Result<Vec<u8>, Problem> {
        let _lane = self.lanes.lane().await;
        let _turn = self
            .lookups
            .acquire()
            .await
            .map_err(|_| Problem::Other("youtube lookups closed".into()))?;
        let _counted = Counted::on(&self.running);
        self.ctx.http.pace("youtube", PACE).await;
        match self.ytdlp.output(args, limit, partial).await {
            Err(Problem::Broken(why)) if self.ytdlp.current().updated => {
                tracing::warn!(why, "the updated yt-dlp failed; going back to the pinned one");
                self.ytdlp.fall_back().await;
                self.report();
                self.ytdlp.output(args, limit, partial).await
            }
            other => other,
        }
    }

    /// `yt-dlp --load-info-json … -o -` into `dest`, chunk by chunk, so the stream cache can
    /// serve it while it arrives. On failure, how many bytes were written and why.
    async fn pipe(&self, info: &Path, format: &str, dest: &Path, size: u64) -> Result<(), (u64, Problem)> {
        let info = info.to_string_lossy();
        let mut cmd = self.ytdlp.command(&[
            "--load-info-json",
            &info,
            "-f",
            format,
            "-o",
            "-",
            "--no-part",
            "--quiet",
            "--no-progress",
        ]);
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(Stdio::null());
        let mut child = cmd
            .spawn()
            .map_err(|e| (0, Problem::Broken(format!("yt-dlp won't start: {e}"))))?;
        let (Some(mut out), Some(mut err)) = (child.stdout.take(), child.stderr.take()) else {
            return Err((0, Problem::Other("yt-dlp's output isn't readable".into())));
        };
        let mut file = tokio::fs::File::create(dest)
            .await
            .map_err(|e| (0, Problem::Other(format!("can't write {}: {e}", dest.display()))))?;
        let copy = async {
            let mut written = 0u64;
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                let n = out.read(&mut buf).await.map_err(|e| (written, e))?;
                if n == 0 {
                    return Ok(written);
                }
                file.write_all(&buf[..n]).await.map_err(|e| (written, e))?;
                // Readers judge what has arrived by the file's length on disk.
                file.flush().await.map_err(|e| (written, e))?;
                written += n as u64;
            }
        };
        let mut said = String::new();
        let (copied, _) = tokio::join!(copy, err.read_to_string(&mut said));
        drop(out);
        if copied.is_err()
            && let Err(e) = child.kill().await
        {
            tracing::debug!(error = %e, "stopping yt-dlp after a failed write");
        }
        let status = child
            .wait()
            .await
            .map_err(|e| (0, Problem::Other(format!("yt-dlp: {e}"))))?;
        let written = copied.map_err(|(n, e)| (n, Problem::Other(format!("writing the download: {e}"))))?;
        if !status.success() {
            return Err((written, ytdlp::classify(&said)));
        }
        if written != size {
            return Err((
                written,
                Problem::Other(format!("YouTube sent {written} bytes where it said {size}")),
            ));
        }
        Ok(())
    }

    /// Refuses at once while YouTube is backed off.
    fn backed_off(&self) -> Result<()> {
        let b = self.backoff.lock().ok().and_then(|b| b.clone());
        match b {
            Some(b) if b.until > Instant::now() => Err(Error::unavailable("youtube", b.problem.plain())),
            _ => Ok(()),
        }
    }

    /// A failed run as an error; a sign-in check or a 429 starts (or lengthens) the back-off.
    fn failed(&self, p: Problem) -> Error {
        if matches!(p, Problem::BotCheck | Problem::RateLimited) {
            if let Ok(mut b) = self.backoff.lock() {
                let times = b.as_ref().map_or(0, |b| b.times + 1);
                let wait = BACKOFF_FIRST
                    .saturating_mul(2u32.saturating_pow(times))
                    .min(BACKOFF_MAX);
                tracing::warn!(problem = p.plain(), minutes = wait.as_secs() / 60, "YouTube backs off");
                *b = Some(Backoff {
                    until: Instant::now() + wait,
                    until_unix: lull_core::now() + wait.as_secs() as i64,
                    problem: p.clone(),
                    times,
                });
            }
            self.report();
            return Error::unavailable("youtube", p.plain());
        }
        if matches!(p, Problem::Broken(_)) {
            self.report();
            return Error::unavailable("youtube", p.plain());
        }
        Error::upstream("youtube", p.plain())
    }

    /// YouTube answered: a back-off that has run out is over.
    fn succeeded(&self) {
        let ended = self.backoff.lock().is_ok_and(|mut b| match b.as_ref() {
            Some(x) if x.until <= Instant::now() => {
                *b = None;
                true
            }
            _ => false,
        });
        if ended {
            self.report();
        }
    }

    /// The `youtube` status: off, ready (with the yt-dlp in use), backed off, or yt-dlp missing.
    pub fn report(&self) {
        let (state, detail) = self.state();
        let bin = self.ytdlp.current();
        self.ctx.status.set(
            "youtube",
            state,
            detail,
            json!({
                "version": bin.version,
                "pinnedVersion": self.ytdlp.pinned_version(),
                "updated": bin.updated,
                "backoffUntil": self.backoff_until(),
                "soundChecks": self.checks.counts(),
            }),
        );
    }

    fn state(&self) -> (State, String) {
        let bin = self.ytdlp.current();
        if self.settings().mode == Mode::Off {
            return (State::Off, "Off: turn it on in Settings › Sources".into());
        }
        let Some(version) = bin.version else {
            return (
                State::Down,
                "yt-dlp isn't installed or won't start, so YouTube can't play".into(),
            );
        };
        if let Some(b) = self.backoff.lock().ok().and_then(|b| b.clone())
            && b.until > Instant::now()
        {
            return (
                State::Warn,
                format!("{} (lull asks again at {} UTC)", b.problem.plain(), clock(b.until_unix)),
            );
        }
        let how = if bin.updated { "kept up to date" } else { "built in" };
        let checks = if self.checks.counts().idle() {
            ". Sound checks are idle: the catalog's store here has no previews of the songs looked up, so copies are matched by their titles and lengths"
        } else {
            ""
        };
        (State::Ok, format!("Ready, with yt-dlp {version} ({how}){checks}"))
    }

    fn backoff_until(&self) -> Option<i64> {
        self.backoff
            .lock()
            .ok()
            .and_then(|b| b.as_ref().filter(|b| b.until > Instant::now()).map(|b| b.until_unix))
    }

    fn apply(&self, settings: &Value) {
        let Some(youtube) = settings.get("sources").and_then(|s| s.get("youtube")) else {
            return;
        };
        let next: Settings = match serde_json::from_value(youtube.clone()) {
            Ok(s) => s,
            // Collection validates before publishing, so this is a contract break: say so.
            Err(e) => return tracing::warn!(error = %e, "settings.sources.youtube can't be read"),
        };
        let before = self.settings();
        if let Ok(mut s) = self.settings.write() {
            *s = next.clone();
        }
        if before.auto_update != next.auto_update {
            self.updates.wake();
        }
        self.report();
    }
}

/// "14:05": the time of day in UTC, for status lines.
fn clock(unix: i64) -> String {
    let secs = unix.rem_euclid(86_400);
    format!("{:02}:{:02}", secs / 3600, secs % 3600 / 60)
}

async fn follow_settings(svc: Arc<Service>) {
    let mut events = svc.ctx.bus.subscribe();
    loop {
        match events.recv().await {
            Ok(e) if e.topic == "settings" => svc.apply(&e.data),
            Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
        }
    }
}

/// Drops extractions whose stream URLs have expired, and what cancelled yt-dlp runs left in the
/// temporary folder (the standalone binary unpacks itself there on every run).
async fn housekeeping(svc: Arc<Service>) {
    loop {
        tokio::time::sleep(HOUSEKEEPING).await;
        remove_older(&svc.dir.join("info"), search::INFO_FOR).await;
        remove_older(&svc.dir.join("tmp"), LEFTOVERS).await;
        remove_older(&svc.dir.join("albums"), ALBUMS_FOR).await;
    }
}

async fn remove_older(dir: &Path, age: Duration) {
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
        return;
    };
    while let Ok(Some(e)) = entries.next_entry().await {
        let old = e
            .metadata()
            .await
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|m| m.elapsed().ok())
            .is_some_and(|a| a > age);
        if !old {
            continue;
        }
        let path = e.path();
        let removed = match e.file_type().await {
            Ok(t) if t.is_dir() => tokio::fs::remove_dir_all(&path).await,
            _ => tokio::fs::remove_file(&path).await,
        };
        if let Err(err) = removed {
            tracing::debug!(error = %err, path = %path.display(), "removing an old YouTube file");
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn status_times_read_as_a_utc_clock() {
        assert_eq!(super::clock(1_762_092_300), "14:05");
    }
}
