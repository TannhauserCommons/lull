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

use crate::ranges::Ranges;

/// Free space kept on the transfers disk beyond the file being fetched, so slskd never writes
/// the disk to the last byte.
const DISK_MARGIN: u64 = 200_000_000;

/// One cached file: where it is on disk and which source it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub track: TrackId,
    pub source: SourceRef,
    pub path: PathBuf,
    pub complete: bool,
    pub used_at: i64,
    /// What of an unfinished copy has arrived; `None` for a file written front to back.
    pub ranges: Option<Ranges>,
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

    fn is_pinned(&self, track: &TrackId) -> bool {
        self.pins.lock().is_ok_and(|p| p.contains_key(track))
    }

    /// Records (or moves) a file lull asked for; it now counts against the cap.
    pub async fn record(&self, e: Entry) -> Result<()> {
        self.db
            .call(move |c| {
                let ranges = e
                    .ranges
                    .as_ref()
                    .map(serde_json::to_string)
                    .transpose()
                    .map_err(|err| Error::internal(format!("ranges as JSON: {err}")))?;
                c.execute(
                    "INSERT INTO soulseek_files (track_id, user, remote_path, size, local_path, complete, used_at, ranges)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                     ON CONFLICT(user, remote_path) DO UPDATE SET track_id = excluded.track_id,
                       local_path = excluded.local_path, complete = excluded.complete, used_at = excluded.used_at,
                       ranges = excluded.ranges",
                    params![
                        e.track.as_str(),
                        e.source.user,
                        e.source.path,
                        e.source.size as i64,
                        e.path.to_string_lossy(),
                        e.complete,
                        e.used_at,
                        ranges
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

    /// The unfinished copy of this source lull has on disk, with what of it has arrived: a later
    /// fetch of the same copy fetches only what is missing.
    pub async fn partial(&self, track: &TrackId, source: &SourceRef) -> Result<Option<Entry>> {
        let mut found = self.entries_for(track).await?;
        found.retain(|e| !e.complete && e.source == *source && e.ranges.is_some() && e.path.is_file());
        Ok(found.pop())
    }

    /// Where lull keeps a Soulseek copy while it arrives (ADR-0014): its own file under `lull/`,
    /// beside slskd's folders, so legs from any byte land in one place that never moves.
    pub async fn copy_file(&self, source: &SourceRef) -> Result<PathBuf> {
        let path = crate::layout::beside(&self.root.join("lull"), &source.user, &source.path);
        if let Some(dir) = path.parent() {
            tokio::fs::create_dir_all(dir)
                .await
                .map_err(|e| Error::unavailable("cache", format!("can't create {}: {e}", dir.display())))?;
        }
        Ok(path)
    }

    /// Where a YouTube copy is written: a `youtube` folder beside slskd's, so it counts against
    /// the same cap and disk and is evicted the same way.
    pub async fn youtube_file(&self, copy: &youtube::Copy) -> Result<PathBuf> {
        let dir = self.root.join("youtube");
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| Error::unavailable("cache", format!("can't create {}: {e}", dir.display())))?;
        let video = match copy.section {
            Some((from, to)) => format!("{}~{from}-{to}", copy.video),
            None => copy.video.clone(),
        };
        Ok(dir.join(format!("{video}.{}.{}", copy.format, copy.ext)))
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
        self.report_usage(plan.used_after, held(&entries, &pinned), plan.disk_short);
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
        let entries = self.on_disk().await?;
        let used = entries.iter().map(|(_, bytes)| bytes).sum();
        self.report_usage(used, held(&entries, &self.pinned()), false);
        Ok(())
    }

    /// Empties the cache on the listener's word (`POST /api/cache/clear`): every file goes but
    /// those of a song playing, preloading or being saved, which stay until nothing holds them.
    pub async fn clear(&self) -> Result<Cleared> {
        let mut cleared = Cleared::default();
        for (e, bytes) in self.on_disk().await? {
            // Asked again for each file: a song may have started playing since the walk began.
            if self.is_pinned(&e.track) {
                cleared.kept_bytes += bytes;
                continue;
            }
            self.remove(&e).await?;
            cleared.freed_bytes += bytes;
            cleared.files += 1;
        }
        tracing::info!(
            files = cleared.files,
            bytes = cleared.freed_bytes,
            "stream cache cleared"
        );
        self.report_usage(cleared.kept_bytes, cleared.kept_bytes, false);
        Ok(cleared)
    }

    fn report_usage(&self, used: u64, held: u64, disk_short: bool) {
        let (state, detail) = usage(used, held, self.cap, disk_short);
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
                // A copy arriving in legs has holes: what arrived is what it holds.
                Ok(m) => {
                    let bytes = e.ranges.as_ref().map_or(m.len(), Ranges::total);
                    present.push((e, bytes));
                }
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

/// What `Cache::clear` removed, and what it kept for songs still playing or being saved.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Cleared {
    pub files: u64,
    pub freed_bytes: u64,
    pub kept_bytes: u64,
}

/// The `cache` status for `used` bytes, `held` of them by pinned files. A cache at its cap is
/// working: eviction makes room for each new file, so `ok`. Only pinned files can hold it over
/// the cap (`warn`), and only a full disk stops streaming (`down`).
fn usage(used: u64, held: u64, cap: u64, disk_short: bool) -> (State, String) {
    if disk_short {
        return (
            State::Down,
            "The transfers disk is full: nothing new can stream until there is room".to_owned(),
        );
    }
    if held > cap {
        return (
            State::Warn,
            format!(
                "{} of {} used: music that is playing or being saved holds it over its size until it ends",
                size(used),
                size(cap)
            ),
        );
    }
    (
        State::Ok,
        format!("{} of {} used, oldest audio evicted first", size(used), size(cap)),
    )
}

/// The bytes of the files no eviction may take: those of pinned tracks.
fn held(entries: &[(Entry, u64)], pinned: &[TrackId]) -> u64 {
    entries
        .iter()
        .filter(|(e, _)| pinned.contains(&e.track))
        .map(|(_, bytes)| bytes)
        .sum()
}

/// "640 MB", "3.1 GB": what the status tray shows.
fn size(bytes: u64) -> String {
    match bytes {
        b if b < 1_000_000_000 => format!("{} MB", b / 1_000_000),
        b => format!("{:.1} GB", b as f64 / 1e9),
    }
}

const SELECT: &str =
    "SELECT track_id, user, remote_path, size, local_path, complete, used_at, ranges FROM soulseek_files";

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
        // Unreadable ranges trust nothing: the copy is fetched again.
        ranges: r
            .get::<_, Option<String>>(7)?
            .map(|j| serde_json::from_str(&j).unwrap_or_default()),
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
            ranges: None,
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
            section: None,
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
                    ranges: None,
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

    /// A cache of `cap` bytes in a folder of its own, and a way to record a finished file of
    /// `bytes` bytes in it for track `t-<n>`.
    struct Fixture {
        dir: PathBuf,
        cache: Cache,
    }

    impl Fixture {
        fn new(name: &str, cap: u64) -> Result<Fixture> {
            let dir = std::env::temp_dir().join(format!("lull-cache-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).map_err(|e| Error::internal(e.to_string()))?;
            let db = lull_core::db::Db::memory()?;
            db.migrate(&crate::MIGRATIONS)?;
            let cache = Cache::new(db, Status::new(lull_core::events::Bus::new()), dir.clone(), cap);
            Ok(Fixture { dir, cache })
        }

        async fn file(&self, n: u8, bytes: usize) -> Result<PathBuf> {
            let mut e = entry(n, i64::from(n));
            e.path = self.dir.join(&e.source.path);
            std::fs::write(&e.path, vec![0u8; bytes]).map_err(|err| Error::internal(err.to_string()))?;
            self.cache.record(e.clone()).await?;
            Ok(e.path)
        }

        fn state(&self) -> Option<State> {
            self.cache.status.snapshot().get("cache").map(|s| s.state)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[tokio::test]
    async fn a_cache_at_its_size_with_nothing_held_is_working() -> Result<()> {
        // Eviction makes room for each new file only as it comes, so a cache in use sits just
        // under its size. Before, anything over 95 % of it read "full, every item in use".
        let fx = Fixture::new("at-size", 100)?;
        fx.file(1, 49).await?;
        fx.file(2, 49).await?;
        fx.cache.report().await?;
        assert_eq!(fx.state(), Some(State::Ok), "98 of 100 bytes, nothing playing");
        fx.cache.make_room(0).await?;
        assert_eq!(fx.state(), Some(State::Ok), "after the minute's housekeeping too");
        Ok(())
    }

    #[tokio::test]
    async fn only_files_held_by_what_plays_can_keep_the_cache_over_its_size() -> Result<()> {
        let fx = Fixture::new("held", 100)?;
        fx.file(1, 120).await?;
        let playing = fx.cache.pin(&TrackId("t-1".into()));
        fx.cache.make_room(0).await?;
        assert_eq!(
            fx.state(),
            Some(State::Warn),
            "a song playing holds 120 bytes of a 100-byte cache"
        );
        drop(playing);
        fx.cache.make_room(0).await?;
        assert_eq!(
            fx.state(),
            Some(State::Ok),
            "once it ends, it is evicted like any other"
        );
        Ok(())
    }

    #[tokio::test]
    async fn clearing_removes_every_file_but_those_of_songs_playing_or_being_saved() -> Result<()> {
        let fx = Fixture::new("clear", 1_000)?;
        let old = fx.file(1, 30).await?;
        let playing = fx.file(2, 20).await?;
        let _pin = fx.cache.pin(&TrackId("t-2".into()));
        let cleared = fx.cache.clear().await?;
        assert_eq!(
            cleared,
            Cleared {
                files: 1,
                freed_bytes: 30,
                kept_bytes: 20
            }
        );
        assert!(!old.exists() && playing.exists(), "the song playing keeps its file");
        assert!(
            fx.cache.complete(&TrackId("t-1".into())).await?.is_empty(),
            "and the index forgets the rest"
        );
        let used = fx
            .cache
            .status
            .snapshot()
            .get("cache")
            .map(|s| s.extra["usedBytes"].clone());
        assert_eq!(used, Some(serde_json::json!(20)), "the status shows what is left");
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
