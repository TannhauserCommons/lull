//! Auto-upgrade (ADR-0008): songs lull saved below the saving minimum are searched for again
//! every `upgradeEveryH`, at most one search a minute, and replaced through the save pipeline's
//! "replace" path when a copy turns up that passes the minimum and clearly beats the saved one
//! (`Quality::beats`). The replacement keeps the saved release (tags, cover) and, at the same
//! path, the Jellyfin entry. Each one is a save job, so it shows in Activity
//! (docs/context/save-flow.md#auto-upgrade).

use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::{Duration, Instant};

use lull_core::db::Db;
use lull_core::ids::TrackId;
use lull_core::model::Track;
use lull_core::quality::{Quality, SaveProfile, extension};
use lull_core::{Result, now};
use rusqlite::OptionalExtension;
use serde_json::Value;

use crate::Service;
use crate::jobs::{Duplicate, Release, SaveJob};
use crate::save;

/// At most one upgrade search this often, for every song together: each is a network search.
const PACE: Duration = Duration::from_secs(60);
/// How often the watcher looks for a song that is due.
const TICK: Duration = Duration::from_secs(60);

/// The saving profile (from the `settings` event) and the shared search pacer.
pub struct Upgrades {
    profile: RwLock<SaveProfile>,
    pacer: Mutex<Pacer>,
}

impl Default for Upgrades {
    fn default() -> Self {
        Upgrades {
            profile: RwLock::new(SaveProfile::default()),
            pacer: Mutex::new(Pacer::default()),
        }
    }
}

impl Upgrades {
    pub fn profile(&self) -> SaveProfile {
        self.profile.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Waits for this search's turn: `PACE` after the one before it.
    pub async fn pace(&self) {
        let wait = self
            .pacer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .reserve(Instant::now());
        tokio::time::sleep(wait).await;
    }

    fn apply(&self, settings: &Value) {
        let Some(save) = settings.get("sources").and_then(|s| s.get("save")) else {
            return;
        };
        match serde_json::from_value::<SaveProfile>(save.clone()) {
            Ok(p) => *self.profile.write().unwrap_or_else(PoisonError::into_inner) = p,
            Err(e) => tracing::warn!(error = %e, "settings.sources.save can't be read; keeping the previous profile"),
        }
    }
}

/// Hands out search slots `PACE` apart, however many callers ask at once.
#[derive(Debug, Default)]
struct Pacer {
    next: Option<Instant>,
}

impl Pacer {
    /// How long a search asking at `now` waits; its slot is taken.
    fn reserve(&mut self, now: Instant) -> Duration {
        let slot = self.next.map_or(now, |next| next.max(now));
        self.next = Some(slot + PACE);
        slot - now
    }
}

/// Keeps the saving profile current. Subscribed in `start`, before collection publishes.
pub fn follow_settings(svc: &Arc<Service>) -> impl Future<Output = ()> + Send + 'static {
    let mut events = svc.ctx.bus.subscribe();
    let svc = svc.clone();
    async move {
        loop {
            match events.recv().await {
                Ok(e) if e.topic == "settings" => svc.upgrades.apply(&e.data),
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            }
        }
    }
}

/// A save finished: watch the song while its file is below the saving minimum, else stop.
/// Returns whether it is below.
pub async fn consider(svc: &Service, job: &SaveJob, saved: &Track) -> bool {
    let ext = job
        .dest
        .as_deref()
        .map(Path::to_string_lossy)
        .map(|p| extension(&p))
        .unwrap_or_default();
    let profile = svc.upgrades.profile();
    let below = !Quality::of(saved.file.as_ref()).meets(&ext, &profile.minimum);
    let watched = Watched {
        track: job.track_id.clone(),
        ext,
        release: job.release.clone(),
    };
    let every = every(&profile);
    let stored = svc
        .ctx
        .db
        .call(move |c| match below {
            true => watch(c, &watched, now() + every),
            false => forget(c, &watched.track),
        })
        .await;
    if let Err(e) = stored {
        tracing::warn!(error = %e, "could not record a song for auto-upgrade");
    }
    below
}

/// Every minute, the most overdue watched song gets its search, while auto-upgrade is on and
/// the library is readable.
pub async fn run(svc: Arc<Service>) {
    loop {
        tokio::time::sleep(TICK).await;
        let profile = svc.upgrades.profile();
        // Before the library is read (Jellyfin down, the folder loading) every song would look
        // unsaved, and be forgotten.
        if !profile.auto_upgrade || !svc.online() {
            continue;
        }
        let at = now();
        match svc.ctx.db.call(move |c| due(c, at)).await {
            Ok(Some(w)) => check(&svc, w, &profile).await,
            Ok(None) => {}
            Err(e) => tracing::warn!(error = %e, "reading songs to upgrade"),
        }
    }
}

/// One watched song: still saved, still below, not saving now? Then search, and replace it
/// when a passing copy that beats it turns up. Either way it is next due `upgradeEveryH` later.
async fn check(svc: &Arc<Service>, w: Watched, profile: &SaveProfile) {
    let Some(saved) = svc.index().track(&w.track) else {
        return forget_logged(&svc.ctx.db, w.track).await;
    };
    let current = Quality::of(saved.file.as_ref());
    if current.meets(&w.ext, &profile.minimum) {
        return forget_logged(&svc.ctx.db, w.track).await;
    }
    let next = now() + every(profile);
    let (db, again) = (svc.ctx.db.clone(), w.clone());
    if let Err(e) = db.call(move |c| watch(c, &again, next)).await {
        tracing::warn!(error = %e, "rescheduling an upgrade search");
    }
    if svc.jobs.active_for(&w.track).is_some() {
        return;
    }
    svc.upgrades.pace().await;
    let found = match svc.transfers.save_candidate(&w.track, true).await {
        Ok(found) => found,
        Err(e) => return tracing::info!(track = %w.track, error = %e, "upgrade search failed; trying again later"),
    };
    let Some(better) = found.filter(|c| c.meets && c.quality.beats(&current)) else {
        return;
    };
    tracing::info!(track = %w.track, peer = better.source.user, "upgrading a saved song");
    let mut job = SaveJob::new(w.track, Some(better.source), Some(Duplicate::Replace));
    job.release = w.release;
    job.upgrade = true;
    svc.jobs.put(&job).await;
    save::spawn(svc, job);
}

fn every(profile: &SaveProfile) -> i64 {
    i64::from(profile.upgrade_every_h) * 3600
}

/// A song watched for an upgrade: its saved extension (for the format list) and release, so the
/// replacement is tagged the same way without matching again.
#[derive(Debug, Clone, PartialEq)]
struct Watched {
    track: TrackId,
    ext: String,
    release: Option<Release>,
}

fn watch(c: &rusqlite::Connection, w: &Watched, next_at: i64) -> Result<()> {
    let release = w
        .release
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|e| lull_core::Error::internal(format!("release json: {e}")))?;
    c.execute(
        "INSERT INTO library_upgrades (track_id, ext, release, next_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(track_id) DO UPDATE SET ext = excluded.ext, release = excluded.release,
            next_at = excluded.next_at",
        rusqlite::params![w.track.as_str(), w.ext, release, next_at],
    )?;
    Ok(())
}

fn forget(c: &rusqlite::Connection, track: &TrackId) -> Result<()> {
    c.execute("DELETE FROM library_upgrades WHERE track_id = ?1", [track.as_str()])?;
    Ok(())
}

async fn forget_logged(db: &Db, track: TrackId) {
    if let Err(e) = db.call(move |c| forget(c, &track)).await {
        tracing::warn!(error = %e, "could not stop watching a song for upgrades");
    }
}

/// The most overdue watched song at `at` (unix seconds).
fn due(c: &rusqlite::Connection, at: i64) -> Result<Option<Watched>> {
    let row: Option<(String, String, Option<String>)> = c
        .query_row(
            "SELECT track_id, ext, release FROM library_upgrades WHERE next_at <= ?1
             ORDER BY next_at LIMIT 1",
            [at],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    Ok(row.map(|(track, ext, release)| Watched {
        track: TrackId(track),
        ext,
        // A release this build can't read only means the upgrade matches again.
        release: release.and_then(|r| serde_json::from_str(&r).ok()),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upgrade_searches_are_at_least_a_minute_apart_however_many_ask() {
        let mut pacer = Pacer::default();
        let t0 = Instant::now();
        assert_eq!(pacer.reserve(t0), Duration::ZERO, "the first goes at once");
        assert_eq!(pacer.reserve(t0 + Duration::from_secs(10)), Duration::from_secs(50));
        assert_eq!(
            pacer.reserve(t0 + Duration::from_secs(10)),
            Duration::from_secs(110),
            "a third caller queues behind the second"
        );
        assert_eq!(
            pacer.reserve(t0 + Duration::from_secs(500)),
            Duration::ZERO,
            "idle long enough"
        );
    }

    #[tokio::test]
    async fn a_watched_song_is_due_again_only_after_its_interval() -> Result<()> {
        let db = Db::memory()?;
        db.migrate(&crate::MIGRATIONS)?;
        let w = Watched {
            track: TrackId("t-1".into()),
            ext: "mp3".into(),
            release: Some(Release::Match {
                mbid: "rel".into(),
                recording: "rec".into(),
            }),
        };
        let early = Watched {
            track: TrackId("t-0".into()),
            ..w.clone()
        };
        let (a, b) = (w.clone(), early.clone());
        db.call(move |c| {
            watch(c, &a, 1_000)?;
            watch(c, &b, 500)
        })
        .await?;
        assert_eq!(
            db.call(|c| due(c, 999)).await?,
            Some(early.clone()),
            "most overdue first"
        );
        let every_day = 24 * 3600;
        let b = early.clone();
        db.call(move |c| watch(c, &b, 999 + every_day)).await?;
        assert_eq!(
            db.call(|c| due(c, 999)).await?,
            None,
            "t-1 isn't due yet, t-0 was just searched"
        );
        assert_eq!(db.call(|c| due(c, 1_000)).await?, Some(w));
        db.call(|c| forget(c, &TrackId("t-1".into()))).await?;
        assert_eq!(db.call(move |c| due(c, 999 + every_day)).await?, Some(early));
        Ok(())
    }
}
