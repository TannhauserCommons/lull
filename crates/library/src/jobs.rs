//! Save and rescan jobs: kept in `library_jobs` so they survive a restart, and published as
//! `save` and `job` events. Other crates' jobs (transfers, imports) are learned from their `job`
//! events, so `GET /api/jobs` lists everything the Activity panel shows.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use lull_core::db::Db;
use lull_core::events::Bus;
use lull_core::ids::TrackId;
use lull_core::model::SourceRef;
use lull_core::{Result, now};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::matching::Candidate;

/// Finished jobs older than this are dropped at start-up.
const KEEP_FINISHED_S: i64 = 7 * 24 * 3600;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Running,
    Done,
    Failed,
    /// Written to disk; waiting for Jellyfin to pick it up.
    Pending,
    /// No copy passes the saving minimum and the profile says `wait`: searched again in the
    /// background until one does (ADR-0008).
    Waiting,
}

/// Which recovery the UI offers for a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FailKind {
    Transfer,
    Match,
    Write,
}

/// The duplicate dialog's answer for a song that is already saved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Duplicate {
    Separate,
    Replace,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Release {
    Match { mbid: String, recording: String },
    KeepTags,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveJob {
    pub id: String,
    pub track_id: TrackId,
    pub source: Option<SourceRef>,
    pub duplicate: Option<Duplicate>,
    /// 1 transfer, 2 match, 3 cover, 4 tag + write, 5 rescan.
    pub step: u8,
    pub status: Status,
    pub pct: u8,
    pub error: Option<String>,
    pub fail_kind: Option<FailKind>,
    pub nocover: bool,
    /// The finished transfer.
    pub file: Option<PathBuf>,
    pub release: Option<Release>,
    /// The release picker's rows when the match wasn't sure.
    pub candidates: Vec<Candidate>,
    /// Where the tagged file was written.
    pub dest: Option<PathBuf>,
    /// The id the written tags give (may differ from `track_id` after MusicBrainz tagging).
    pub saved_id: Option<TrackId>,
    pub rescan_asked: bool,
    pub waiting_since: Option<i64>,
    pub started: i64,
    /// Started by auto-upgrade, replacing a saved copy below the saving minimum.
    #[serde(default)]
    pub upgrade: bool,
}

impl SaveJob {
    pub fn new(track_id: TrackId, source: Option<SourceRef>, duplicate: Option<Duplicate>) -> Self {
        SaveJob {
            id: new_id(),
            track_id,
            source,
            duplicate,
            step: 1,
            status: Status::Running,
            pct: 0,
            error: None,
            fail_kind: None,
            nocover: false,
            file: None,
            release: None,
            candidates: vec![],
            dest: None,
            saved_id: None,
            rescan_asked: false,
            waiting_since: None,
            started: now(),
            upgrade: false,
        }
    }

    pub fn is_active(&self) -> bool {
        matches!(self.status, Status::Running | Status::Pending | Status::Waiting)
    }

    /// The `save` event (api.md#events).
    fn event(&self) -> Value {
        json!({
            "jobId": self.id, "trackId": self.track_id, "step": self.step, "status": self.status,
            "pct": self.pct, "error": self.error, "kind": self.fail_kind, "nocover": self.nocover,
            "upgrade": self.upgrade,
        })
    }

    /// The Activity row: the `job` event and `GET /api/jobs` item.
    fn row(&self) -> Value {
        json!({
            "jobId": self.id, "kind": "save", "trackId": self.track_id, "step": self.step,
            "stepState": self.status, "pct": self.pct, "error": self.error, "failKind": self.fail_kind,
            "nocover": self.nocover, "started": self.started, "upgrade": self.upgrade,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RescanJob {
    pub id: String,
    /// running, done or failed.
    pub state: String,
    pub pct: u8,
    pub detail: String,
    pub started: i64,
}

impl RescanJob {
    pub fn new(detail: &str) -> Self {
        RescanJob {
            id: new_id(),
            state: "running".into(),
            pct: 0,
            detail: detail.into(),
            started: now(),
        }
    }

    fn row(&self) -> Value {
        json!({
            "jobId": self.id, "kind": "rescan", "state": self.state, "pct": self.pct,
            "detail": self.detail, "started": self.started,
        })
    }
}

pub struct Jobs {
    db: Db,
    bus: Bus,
    saves: Mutex<BTreeMap<String, SaveJob>>,
    rescans: Mutex<BTreeMap<String, RescanJob>>,
    /// Other crates' jobs: the latest `job` event of each.
    others: Mutex<BTreeMap<String, Value>>,
}

impl Jobs {
    pub async fn load(db: Db, bus: Bus) -> Result<Arc<Self>> {
        let cutoff = now() - KEEP_FINISHED_S;
        let rows: Vec<(String, String)> = db
            .call(move |c| {
                c.execute(
                    "DELETE FROM library_jobs WHERE finished = 1 AND updated_at < ?1",
                    [cutoff],
                )?;
                let mut stmt = c.prepare("SELECT kind, json FROM library_jobs")?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
                Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
            })
            .await?;
        let mut saves = BTreeMap::new();
        let mut rescans = BTreeMap::new();
        for (kind, json) in rows {
            let parsed = match kind.as_str() {
                "save" => serde_json::from_str::<SaveJob>(&json).map(|j| {
                    saves.insert(j.id.clone(), j);
                }),
                _ => serde_json::from_str::<RescanJob>(&json).map(|j| {
                    rescans.insert(j.id.clone(), j);
                }),
            };
            // A row this build can't read is reported and skipped rather than blocking start-up.
            if let Err(e) = parsed {
                tracing::warn!(kind, error = %e, "skipping unreadable job row");
            }
        }
        let jobs = Arc::new(Jobs {
            db,
            bus,
            saves: Mutex::new(saves),
            rescans: Mutex::new(rescans),
            others: Mutex::default(),
        });
        // A restart stops a running rescan (`Ctx::spawn`); it would otherwise show as running forever.
        let interrupted: Vec<RescanJob> = lock(&jobs.rescans)
            .values()
            .filter(|j| j.state == "running")
            .cloned()
            .collect();
        for mut job in interrupted {
            job.state = "failed".into();
            job.detail = "Stopped by a restart: rescan again".into();
            jobs.put_rescan(&job).await;
        }
        Ok(jobs)
    }

    /// Learns other crates' jobs from their `job` events, until the hub stops the crates.
    pub fn follow(self: &Arc<Self>, ctx: &lull_core::Ctx) {
        ctx.spawn(learn_others(self.clone()));
    }

    pub fn get(&self, id: &str) -> Option<SaveJob> {
        lock(&self.saves).get(id).cloned()
    }

    pub fn saves(&self) -> Vec<SaveJob> {
        lock(&self.saves).values().cloned().collect()
    }

    /// The running or pending save of a song, if any.
    pub fn active_for(&self, track: &TrackId) -> Option<SaveJob> {
        lock(&self.saves)
            .values()
            .find(|j| &j.track_id == track && j.is_active())
            .cloned()
    }

    /// Records a save job's new state: persisted, then published.
    pub async fn put(&self, job: &SaveJob) {
        lock(&self.saves).insert(job.id.clone(), job.clone());
        self.persist(&job.id, "save", job, job.status == Status::Done).await;
        self.bus.publish("save", job.event());
        self.bus.publish("job", job.row());
    }

    /// Transfer progress during step 1: published, not persisted (it changes many times a second).
    pub fn progress(&self, id: &str, pct: u8) {
        let mut saves = lock(&self.saves);
        let Some(job) = saves.get_mut(id).filter(|j| j.step == 1 && j.pct != pct) else {
            return;
        };
        job.pct = pct;
        let (event, row) = (job.event(), job.row());
        drop(saves);
        self.bus.publish("save", event);
        self.bus.publish("job", row);
    }

    pub async fn remove(&self, id: &str) {
        lock(&self.saves).remove(id);
        let id = id.to_owned();
        if let Err(e) = self
            .db
            .call(move |c| Ok(c.execute("DELETE FROM library_jobs WHERE id = ?1", [id])?))
            .await
        {
            tracing::warn!(error = %e, "could not delete a job row");
        }
    }

    pub async fn put_rescan(&self, job: &RescanJob) {
        lock(&self.rescans).insert(job.id.clone(), job.clone());
        self.persist(&job.id, "rescan", job, job.state != "running").await;
        self.bus.publish("job", job.row());
    }

    /// Every job the Activity panel shows, newest first.
    pub fn rows(&self) -> Vec<Value> {
        let mut rows: Vec<Value> = lock(&self.saves).values().map(SaveJob::row).collect();
        rows.extend(lock(&self.rescans).values().map(RescanJob::row));
        rows.extend(lock(&self.others).values().cloned());
        rows.sort_by_key(|r| std::cmp::Reverse(r.get("started").and_then(Value::as_i64).unwrap_or(0)));
        rows
    }

    /// "Clear finished": done saves and finished rescans; failed saves stay until retried.
    pub async fn clear_finished(&self) {
        lock(&self.saves).retain(|_, j| j.status != Status::Done);
        lock(&self.rescans).retain(|_, j| j.state == "running");
        lock(&self.others).retain(|_, row| {
            let state = row
                .get("state")
                .or_else(|| row.get("stepState"))
                .and_then(Value::as_str);
            !matches!(state, Some("done" | "cancelled"))
        });
        if let Err(e) = self
            .db
            .call(|c| Ok(c.execute("DELETE FROM library_jobs WHERE finished = 1", [])?))
            .await
        {
            tracing::warn!(error = %e, "could not clear finished jobs");
        }
    }

    /// A failed write only loses the state across a restart; the job itself carries on.
    async fn persist(&self, id: &str, kind: &'static str, job: &impl Serialize, finished: bool) {
        let (id, json) = (id.to_owned(), serde_json::to_string(job));
        let result = match json {
            Ok(json) => {
                self.db
                    .call(move |c| {
                        c.execute(
                            "INSERT INTO library_jobs (id, kind, json, finished, updated_at)
                             VALUES (?1, ?2, ?3, ?4, ?5)
                             ON CONFLICT(id) DO UPDATE SET json = excluded.json,
                                finished = excluded.finished, updated_at = excluded.updated_at",
                            rusqlite::params![id, kind, json, finished, now()],
                        )?;
                        Ok(())
                    })
                    .await
            }
            Err(e) => Err(lull_core::Error::internal(format!("job json: {e}"))),
        };
        if let Err(e) = result {
            tracing::warn!(error = %e, "could not persist a job");
        }
    }
}

/// Keeps the latest `job` event of every job library doesn't own.
async fn learn_others(jobs: Arc<Jobs>) {
    let mut rx = jobs.bus.subscribe();
    loop {
        match rx.recv().await {
            Ok(event) if event.topic == "job" => {
                let Some(id) = event.data.get("jobId").and_then(Value::as_str) else {
                    continue;
                };
                let ours = lock(&jobs.saves).contains_key(id) || lock(&jobs.rescans).contains_key(id);
                if !ours {
                    lock(&jobs.others).insert(id.to_owned(), event.data.clone());
                }
            }
            Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
        }
    }
}

/// `job-` + time + a counter: unique within this database, sortable by creation.
fn new_id() -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("job-{nanos:x}{:04x}", COUNTER.fetch_add(1, Ordering::Relaxed) & 0xffff)
}

pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}
