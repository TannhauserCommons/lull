//! The stream cache: every file lull asked slskd (or YouTube) for, capped at `LULL_CACHE_MAX_GB`. The least
//! recently used files go first; a file being played or saved is pinned and never evicted.
//! Only files lull itself requested are ever deleted, so an existing slskd's other downloads
//! are safe (docs/context/cache.md).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use lull_core::db::Db;
use lull_core::ids::TrackId;
use lull_core::model::SourceRef;
use lull_core::status::{State, Status};
use lull_core::{Error, Result, youtube};
use rusqlite::params;
use serde_json::json;

/// Free space kept on the transfers disk beyond the file being fetched, so slskd never writes
/// the disk to the last byte.
const DISK_MARGIN: u64 = 200_000_000;
/// Above this share of the cap the cache reports `warn` (only pinned files can keep it there).
const WARN_FRACTION: f64 = 0.95;

/// One cached file: where it is on disk and which source it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub track: TrackId,
    pub source: SourceRef,
    pub path: PathBuf,
    pub complete: bool,
    pub used_at: i64,
}

#[derive(Clone)]
pub struct Cache {
    db: Db,
    status: Status,
    /// Where slskd's files are, as lull sees them.
    root: PathBuf,
    cap: u64,
    pins: Arc<Mutex<HashMap<TrackId, u32>>>,
}

/// Keeps a track's files from eviction while it lives (playing, or being saved).
pub struct Pin {
    pins: Arc<Mutex<HashMap<TrackId, u32>>>,
    track: TrackId,
}

impl Drop for Pin {
    fn drop(&mut self) {
        let Ok(mut pins) = self.pins.lock() else { return };
        if let Some(n) = pins.get_mut(&self.track) {
            *n -= 1;
            if *n == 0 {
                pins.remove(&self.track);
            }
        }
    }
}

impl Cache {
    pub fn new(db: Db, status: Status, root: PathBuf, cap: u64) -> Self {
        Cache {
            db,
            status,
            root,
            cap,
            pins: Arc::default(),
        }
    }

    pub fn pin(&self, track: &TrackId) -> Pin {
        if let Ok(mut pins) = self.pins.lock() {
            *pins.entry(track.clone()).or_default() += 1;
        }
        Pin {
            pins: self.pins.clone(),
            track: track.clone(),
        }
    }

    fn pinned(&self) -> Vec<TrackId> {
        self.pins
            .lock()
            .map(|p| p.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// Records (or moves) a file lull asked for; it now counts against the cap.
    pub async fn record(&self, e: Entry) -> Result<()> {
        self.db
            .call(move |c| {
                c.execute(
                    "INSERT INTO soulseek_files (track_id, user, remote_path, size, local_path, complete, used_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                     ON CONFLICT(user, remote_path) DO UPDATE SET track_id = excluded.track_id,
                       local_path = excluded.local_path, complete = excluded.complete, used_at = excluded.used_at",
                    params![
                        e.track.as_str(),
                        e.source.user,
                        e.source.path,
                        e.source.size as i64,
                        e.path.to_string_lossy(),
                        e.complete,
                        e.used_at
                    ],
                )?;
                Ok(())
            })
            .await
    }

    /// Marks a track's files as just used (it's playing again).
    pub async fn touch(&self, track: &TrackId) -> Result<()> {
        let id = track.as_str().to_owned();
        self.db
            .call(move |c| {
                c.execute(
                    "UPDATE soulseek_files SET used_at = ?2 WHERE track_id = ?1",
                    params![id, lull_core::now()],
                )?;
                Ok(())
            })
            .await
    }

    /// The track's finished files still on disk; which one plays is the fetcher's choice
    /// (`Fetcher::best_cached`).
    pub async fn complete(&self, track: &TrackId) -> Result<Vec<Entry>> {
        let mut found = self.entries_for(track).await?;
        found.retain(|e| e.complete && e.path.is_file());
        Ok(found)
    }

    /// The finished file of exactly this copy, while it is on disk.
    pub async fn finished(&self, track: &TrackId, source: &SourceRef) -> Result<Option<Entry>> {
        Ok(self.complete(track).await?.into_iter().find(|e| e.source == *source))
    }

    /// Where a YouTube copy is written: a `youtube` folder beside slskd's, so it counts against
    /// the same cap and disk and is evicted the same way.
    pub async fn youtube_file(&self, copy: &youtube::Copy) -> Result<PathBuf> {
        let dir = self.root.join("youtube");
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| Error::unavailable("cache", format!("can't create {}: {e}", dir.display())))?;
        Ok(dir.join(format!("{}.{}.{}", copy.video, copy.format, copy.ext)))
    }

    async fn entries_for(&self, track: &TrackId) -> Result<Vec<Entry>> {
        let id = track.as_str().to_owned();
        self.db
            .call(move |c| {
                let mut stmt = c.prepare_cached(&format!("{SELECT} WHERE track_id = ?1"))?;
                let rows = stmt.query_map([id], row)?;
                Ok(rows.collect::<Result<Vec<_>, _>>()?)
            })
            .await
    }

    /// Makes room for a file of `incoming` bytes: evicts least recently used, unpinned files
    /// until the cache fits under its cap and the disk has room. Errs when the disk can't fit
    /// it even after evicting everything evictable.
    pub async fn make_room(&self, incoming: u64) -> Result<()> {
        let entries = self.on_disk().await?;
        let pinned = self.pinned();
        let free = disk_free(&self.root);
        let plan = plan(&entries, &pinned, self.cap, incoming, free);
        for victim in &plan.evict {
            self.remove(victim).await?;
        }
        if !plan.evict.is_empty() {
            tracing::info!(
                files = plan.evict.len(),
                "stream cache evicted least recently used audio"
            );
        }
        self.report_usage(plan.used_after, plan.disk_short).await;
        if plan.disk_short {
            return Err(Error::unavailable(
                "cache",
                "the transfers disk is full; free some space or lower LULL_CACHE_MAX_GB",
            ));
        }
        Ok(())
    }

    /// Recomputes usage and publishes the `cache` status.
    pub async fn report(&self) -> Result<()> {
        let used = self.on_disk().await?.iter().map(|(_, bytes)| bytes).sum();
        self.report_usage(used, false).await;
        Ok(())
    }

    async fn report_usage(&self, used: u64, disk_short: bool) {
        let (state, detail) = if disk_short {
            (
                State::Down,
                "The transfers disk is full: nothing new can stream until there is room".to_owned(),
            )
        } else if used as f64 > self.cap as f64 * WARN_FRACTION {
            (
                State::Warn,
                format!(
                    "{} of {} used, by music that is playing or being saved",
                    size(used),
                    size(self.cap)
                ),
            )
        } else {
            (
                State::Ok,
                format!("{} of {} used, oldest audio evicted first", size(used), size(self.cap)),
            )
        };
        self.status.set(
            "cache",
            state,
            detail,
            json!({ "usedBytes": used, "capBytes": self.cap }),
        );
    }

    /// Every recorded file with its bytes on disk. Rows whose file is gone (saved and moved
    /// away, or deleted by hand) are dropped here: the disk is the truth.
    async fn on_disk(&self) -> Result<Vec<(Entry, u64)>> {
        let all = self
            .db
            .call(|c| {
                let mut stmt = c.prepare_cached(SELECT)?;
                let rows = stmt.query_map([], row)?;
                Ok(rows.collect::<Result<Vec<_>, _>>()?)
            })
            .await?;
        let mut present = Vec::with_capacity(all.len());
        for e in all {
            match tokio::fs::metadata(&e.path).await {
                Ok(m) => present.push((e, m.len())),
                Err(_) => self.forget(&e.source).await?,
            }
        }
        Ok(present)
    }

    async fn remove(&self, e: &Entry) -> Result<()> {
        match tokio::fs::remove_file(&e.path).await {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(Error::internal(format!("evict {}: {err}", e.path.display()))),
        }
        self.forget(&e.source).await
    }

    pub async fn forget(&self, source: &SourceRef) -> Result<()> {
        let (user, path) = (source.user.clone(), source.path.clone());
        self.db
            .call(move |c| {
                c.execute(
                    "DELETE FROM soulseek_files WHERE user = ?1 AND remote_path = ?2",
                    params![user, path],
                )?;
                Ok(())
            })
            .await
    }
}

/// "640 MB", "3.1 GB": what the status tray shows.
fn size(bytes: u64) -> String {
    match bytes {
        b if b < 1_000_000_000 => format!("{} MB", b / 1_000_000),
        b => format!("{:.1} GB", b as f64 / 1e9),
    }
}

const SELECT: &str = "SELECT track_id, user, remote_path, size, local_path, complete, used_at FROM soulseek_files";

fn row(r: &rusqlite::Row) -> rusqlite::Result<Entry> {
    Ok(Entry {
        track: TrackId(r.get(0)?),
        source: SourceRef {
            user: r.get(1)?,
            path: r.get(2)?,
            size: r.get::<_, i64>(3)? as u64,
        },
        path: PathBuf::from(r.get::<_, String>(4)?),
        complete: r.get(5)?,
        used_at: r.get(6)?,
    })
}

/// Free bytes on the disk holding `root`; unknown counts as plenty, so a filesystem that
/// can't report never blocks streaming (the cap still bounds the cache).
fn disk_free(root: &Path) -> u64 {
    fs4::available_space(root).unwrap_or(u64::MAX)
}

#[derive(Debug, PartialEq)]
struct Plan {
    evict: Vec<Entry>,
    used_after: u64,
    /// Even with everything evictable gone, the disk can't hold the incoming file.
    disk_short: bool,
}

/// Chooses what to evict: least recently used first, never a pinned track, until
/// `used + incoming` fits the cap and the disk keeps `DISK_MARGIN` free.
fn plan(entries: &[(Entry, u64)], pinned: &[TrackId], cap: u64, incoming: u64, free: u64) -> Plan {
    let mut used: u64 = entries.iter().map(|(_, b)| b).sum();
    let mut free = free;
    let mut candidates: Vec<&(Entry, u64)> = entries.iter().filter(|(e, _)| !pinned.contains(&e.track)).collect();
    candidates.sort_by_key(|(e, _)| e.used_at);
    let fits =
        |used: u64, free: u64| used.saturating_add(incoming) <= cap && free >= incoming.saturating_add(DISK_MARGIN);
    let mut evict = Vec::new();
    for (e, bytes) in candidates {
        if fits(used, free) {
            break;
        }
        used -= bytes;
        free = free.saturating_add(*bytes);
        evict.push(e.clone());
    }
    Plan {
        evict,
        used_after: used,
        disk_short: free < incoming.saturating_add(DISK_MARGIN),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MB: u64 = 1_000_000;
    const PLENTY: u64 = u64::MAX / 2;

    fn entry(n: u8, used_at: i64) -> Entry {
        Entry {
            track: TrackId(format!("t-{n}")),
            source: SourceRef {
                user: "peer".into(),
                path: format!("song{n}.flac"),
                size: 30 * MB,
            },
            path: PathBuf::from(format!("song{n}.flac")),
            complete: true,
            used_at,
        }
    }

    fn tracks(p: &Plan) -> Vec<&str> {
        p.evict.iter().map(|e| e.track.as_str()).collect()
    }

    #[test]
    fn evicts_least_recently_used_until_the_new_file_fits_the_cap() {
        let files = [
            (entry(1, 300), 30 * MB),
            (entry(2, 100), 30 * MB),
            (entry(3, 200), 30 * MB),
        ];
        // 90 MB cached, cap 100 MB, 30 MB incoming: drop the oldest (t-2) only.
        let p = plan(&files, &[], 100 * MB, 30 * MB, PLENTY);
        assert_eq!(tracks(&p), ["t-2"]);
        assert_eq!(p.used_after, 60 * MB);
        assert!(!p.disk_short);
        // Already fits: nothing goes.
        assert!(plan(&files, &[], 200 * MB, 30 * MB, PLENTY).evict.is_empty());
    }

    #[test]
    fn never_evicts_what_is_playing_or_being_saved() {
        let files = [(entry(1, 100), 50 * MB), (entry(2, 200), 50 * MB)];
        let pinned = [TrackId("t-1".into())];
        let p = plan(&files, &pinned, 60 * MB, 30 * MB, PLENTY);
        // t-1 is oldest but pinned: t-2 goes, and the cache stays over its cap rather than
        // cutting off the song that's playing.
        assert_eq!(tracks(&p), ["t-2"]);
        assert_eq!(p.used_after, 50 * MB);
    }

    #[tokio::test]
    async fn the_cache_lists_finished_copies_and_finds_one_by_its_source() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("lull-cache-youtube-{}", std::process::id()));
        std::fs::create_dir_all(&dir).map_err(|e| Error::internal(e.to_string()))?;
        let db = lull_core::db::Db::memory()?;
        db.migrate(&crate::MIGRATIONS)?;
        let cache = Cache::new(db, Status::new(lull_core::events::Bus::new()), dir.clone(), PLENTY);
        let track = TrackId("t".into());
        let peer = SourceRef {
            user: "peer".into(),
            path: "Music\\A\\01 - Song.mp3".into(),
            size: 3,
        };
        let copy = youtube::Copy {
            video: "LUjGtyYEi90".into(),
            format: "251".into(),
            ext: "webm".into(),
        };
        let yt = copy.source_ref(3);
        for (source, path, used_at) in [
            (&peer, dir.join("song.mp3"), 100),
            (&yt, cache.youtube_file(&copy).await?, 200),
        ] {
            std::fs::write(&path, b"abc").map_err(|e| Error::internal(e.to_string()))?;
            cache
                .record(Entry {
                    track: track.clone(),
                    source: source.clone(),
                    path,
                    complete: true,
                    used_at,
                })
                .await?;
        }
        let all = cache.complete(&track).await?.len();
        let asked = cache.finished(&track, &yt).await?.map(|e| e.source);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(all, 2, "both finished copies");
        assert_eq!(asked, Some(yt), "exactly the copy asked for");
        Ok(())
    }

    #[test]
    fn a_full_disk_evicts_below_the_cap_and_reports_when_even_that_is_not_enough() {
        let files = [(entry(1, 100), 50 * MB), (entry(2, 200), 50 * MB)];
        // Well under the cap, but the disk has only 100 MB free: one file must go to keep the
        // margin for a 30 MB download.
        let p = plan(&files, &[], 10_000 * MB, 30 * MB, 200 * MB);
        assert_eq!(tracks(&p), ["t-1"]);
        assert!(!p.disk_short);
        // Everything pinned and the disk full: nothing to evict, the caller is told.
        let pinned = [TrackId("t-1".into()), TrackId("t-2".into())];
        let p = plan(&files, &pinned, 10_000 * MB, 30 * MB, 10 * MB);
        assert!(p.evict.is_empty() && p.disk_short);
    }
}
