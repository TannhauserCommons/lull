//! YouTube as a second source, through yt-dlp (docs/context/youtube.md). It finds the upload that
//! is the song, and downloads its native audio stream into a file soulseek's stream cache holds,
//! through the `Youtube` seam: soulseek decides when YouTube plays (the listener's mode), this
//! crate decides which upload and which stream. It also keeps yt-dlp current, and backs off when
//! YouTube asks for a sign-in check or says lull asked too often.

mod api;
mod formats;
mod matching;
mod search;
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
use lull_core::seams::Youtube;
use lull_core::status::State;
use lull_core::youtube::{Copy, Found, Mode, Settings, Want};
use lull_core::{Ctx, Error, Result};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Semaphore;

use crate::ytdlp::{Problem, Ytdlp};

pub const MIGRATIONS: Migrations = Migrations {
    owner: "youtube",
    steps: &[r#"
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
    "#],
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

pub struct Service {
    ctx: Ctx,
    /// `<data>/youtube`.
    dir: PathBuf,
    ytdlp: Ytdlp,
    settings: RwLock<Settings>,
    backoff: Mutex<Option<Backoff>>,
    lookups: Semaphore,
    downloads: Semaphore,
    /// One search per song at a time: a preload and the player asking together search once.
    finding: Mutex<std::collections::HashMap<TrackId, Arc<tokio::sync::Mutex<()>>>>,
    updates: update::State,
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
    for sub in ["info", "cache", "tmp", "deno", "bin"] {
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
        finding: Mutex::default(),
        updates: update::State::default(),
    });
    svc.report();
    // Subscribed before collection starts, so its start-up `settings` arrives (ADR-0050).
    ctx.spawn(follow_settings(svc.clone()));
    ctx.spawn(update::run(svc.clone()));
    ctx.spawn(housekeeping(svc.clone()));
    Ok(svc)
}

/// `GET /api/youtube`, `POST /api/youtube/update`.
pub fn router(svc: Arc<Service>) -> Router {
    api::router(svc)
}

#[async_trait]
impl Youtube for Service {
    async fn find(&self, track: &Track, want: Want) -> Result<Option<Found>> {
        let settings = self.settings();
        if settings.mode == Mode::Off {
            return Ok(None);
        }
        self.backed_off()?;
        let matched = self.matched(track, &settings).await?;
        let Some(m) = matched else { return Ok(None) };
        let (info, _) = search::info(self, &m.video, false).await.map_err(|p| self.failed(p))?;
        let Some(chosen) = formats::choose(&info, want, settings.format) else {
            tracing::info!(video = %m.video, "no audio stream lull can stream from this upload");
            return Ok(None);
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
}

impl Service {
    pub fn settings(&self) -> Settings {
        self.settings.read().map(|s| s.clone()).unwrap_or_default()
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
        let _turn = self
            .lookups
            .acquire()
            .await
            .map_err(|_| Problem::Other("youtube lookups closed".into()))?;
        self.ctx.http.pace("youtube", PACE).await;
        match self.ytdlp.output(args, limit).await {
            Err(Problem::Broken(why)) if self.ytdlp.current().updated => {
                tracing::warn!(why, "the updated yt-dlp failed; going back to the pinned one");
                self.ytdlp.fall_back().await;
                self.report();
                self.ytdlp.output(args, limit).await
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
        (State::Ok, format!("Ready, with yt-dlp {version} ({how})"))
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
