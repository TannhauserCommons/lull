//! The folder source (ADR-0003): lull indexes `LULL_MUSIC_DIR` itself. A walk reads every
//! audio file's tags (files unchanged since the last read are reused from `library_files`),
//! and a watcher re-reads whatever changes, so added, retagged and removed files reach the
//! index within seconds.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use lull_core::db::Db;
use lull_core::status::State;
use lull_core::{Error, Result, catalog, now};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde_json::json;
use tokio::sync::mpsc;

use crate::Service;
use crate::files;
use crate::items::{Art, Saved};
use crate::jobs::RescanJob;

/// Changes are collected this long before they're read, so a copied album is one refresh.
const SETTLE: Duration = Duration::from_secs(2);
/// Image files that are an album's cover, best first (compared without case).
const COVERS: &[&str] = &[
    "cover.jpg",
    "cover.jpeg",
    "cover.png",
    "folder.jpg",
    "folder.jpeg",
    "folder.png",
    "front.jpg",
    "front.png",
];

pub struct Folder {
    root: PathBuf,
    /// What each indexed file looked like when it was read.
    stamps: Mutex<HashMap<String, Stamp>>,
    /// Watching stops when this is dropped, with the service.
    watcher: Mutex<Option<RecommendedWatcher>>,
    /// Why the folder isn't watched, when it isn't.
    watch_error: Mutex<Option<String>>,
    /// Unix seconds of the last whole-folder walk.
    last_scan: Mutex<Option<i64>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    pub modified: i64,
    pub size: u64,
    /// The file embeds a picture (its cover when the folder has no image file).
    pub picture: bool,
}

/// One indexed file, as `library_files` keeps it.
#[derive(Debug, Clone)]
pub struct Row {
    pub stamp: Stamp,
    pub saved: Saved,
}

impl Row {
    fn path(&self) -> &str {
        self.saved.location.key()
    }
}

impl Folder {
    pub fn new(root: PathBuf) -> Self {
        Folder::with(root, &[])
    }

    /// With the rows the last run indexed.
    pub fn with(root: PathBuf, rows: &[Row]) -> Self {
        Folder {
            root,
            stamps: Mutex::new(rows.iter().map(|r| (r.path().to_owned(), r.stamp)).collect()),
            watcher: Mutex::new(None),
            watch_error: Mutex::new(None),
            last_scan: Mutex::new(None),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// A file of the index on disk.
    pub fn file(&self, rel: &str) -> PathBuf {
        to_path(&self.root, rel)
    }

    pub fn last_scan(&self) -> Option<i64> {
        *lock(&self.last_scan)
    }

    fn stamps(&self) -> MutexGuard<'_, HashMap<String, Stamp>> {
        lock(&self.stamps)
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The index the last run kept, for start-up.
pub async fn load(db: &Db) -> Result<Vec<Row>> {
    db.call(|c| {
        let mut stmt = c.prepare("SELECT modified, size, picture, json FROM library_files")?;
        let rows = stmt
            .query_map([], |r| {
                let stamp = Stamp {
                    modified: r.get(0)?,
                    size: r.get::<_, i64>(1)? as u64,
                    picture: r.get(2)?,
                };
                Ok((stamp, r.get::<_, String>(3)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows
            .into_iter()
            .filter_map(|(stamp, json)| match serde_json::from_str(&json) {
                Ok(saved) => Some(Row { stamp, saved }),
                // An older shape is read again by the start-up walk; it must not stop start-up.
                Err(e) => {
                    tracing::warn!(error = %e, "skipping unreadable library file row");
                    None
                }
            })
            .collect())
    })
    .await
}

/// Folder mode's background work: walk the folder, then follow the watcher, until the hub stops
/// the crates (`Ctx::spawn`).
pub async fn run(svc: Arc<Service>) {
    let (tx, mut changes) = mpsc::unbounded_channel();
    watch(&svc, tx);
    if let Err(e) = refresh(&svc, "").await {
        tracing::warn!(error = %e, "music folder walk failed");
    }
    report(&svc);
    // The watcher holds the only sender: without one, there is nothing to follow.
    while let Some(first) = changes.recv().await {
        let mut paths = vec![first];
        let settle = tokio::time::sleep(SETTLE);
        tokio::pin!(settle);
        loop {
            tokio::select! {
                () = &mut settle => break,
                more = changes.recv() => match more {
                    Some(p) => paths.push(p),
                    None => break,
                },
            }
        }
        for under in to_refresh(&svc.folder.root, paths) {
            if let Err(e) = refresh(&svc, &under).await {
                tracing::warn!(path = under, error = %e, "music folder refresh failed");
            }
        }
        report(&svc);
    }
}

fn watch(svc: &Service, tx: mpsc::UnboundedSender<PathBuf>) {
    let started = notify::recommended_watcher(move |event: notify::Result<notify::Event>| match event {
        Ok(e) if !matches!(e.kind, EventKind::Access(_)) => {
            for p in e.paths {
                // Fails only once the loop has ended; nothing is left to tell.
                let _ = tx.send(p);
            }
        }
        Ok(_) => {}
        Err(e) => tracing::warn!(error = %e, "music folder watcher"),
    })
    .and_then(|mut w| w.watch(&svc.folder.root, RecursiveMode::Recursive).map(|()| w));
    match started {
        Ok(w) => *lock(&svc.folder.watcher) = Some(w),
        Err(e) => {
            tracing::warn!(error = %e, "can't watch the music folder");
            *lock(&svc.folder.watch_error) = Some(e.to_string());
        }
    }
}

/// The `library` status in folder mode.
fn report(svc: &Service) {
    let songs = svc.index().len();
    let extra = json!({ "songs": songs });
    let root = svc.folder.root.display();
    let (state, detail) = if !svc.folder.root.is_dir() {
        (
            State::Down,
            format!("The music folder {root} doesn't exist: set LULL_MUSIC_DIR"),
        )
    } else if let Some(why) = lock(&svc.folder.watch_error).clone() {
        (
            State::Warn,
            format!("Music folder, {songs} songs; not watching it for changes ({why}): rescan to pick up new files"),
        )
    } else {
        (State::Ok, format!("Music folder, {songs} songs"))
    };
    svc.ctx.status.set("library", state, detail, extra);
}

/// Which paths to read again for a batch of watcher events: changed audio files, the folder of
/// a changed cover image, and changed or vanished folders. Paths under another are covered by it.
fn to_refresh(root: &Path, paths: Vec<PathBuf>) -> Vec<String> {
    let mut wanted = BTreeSet::new();
    for path in paths {
        let Some(rel) = rel_of(root, &path) else { continue };
        let name = rel.rsplit('/').next().unwrap_or(&rel).to_ascii_lowercase();
        if files::is_audio(&path) {
            wanted.insert(rel);
        } else if COVERS.contains(&name.as_str()) {
            wanted.insert(rel.rsplit_once('/').map_or("", |(dir, _)| dir).to_owned());
        } else if path.is_dir() || (!path.exists() && path.extension().is_none()) {
            wanted.insert(rel);
        }
    }
    let mut out: Vec<String> = Vec::new();
    for rel in wanted {
        if !out.iter().any(|o| under(&rel, o)) {
            out.push(rel);
        }
    }
    out
}

/// `path` is `dir` or inside it ("" is the whole folder).
fn under(path: &str, dir: &str) -> bool {
    dir.is_empty() || path == dir || path.strip_prefix(dir).is_some_and(|r| r.starts_with('/'))
}

/// Reads everything at or under `rel` again and puts it in the index, the database and the
/// catalog; returns how many songs the index has afterwards.
pub async fn refresh(svc: &Arc<Service>, rel: &str) -> Result<usize> {
    let known: HashMap<String, Row> = {
        let stamps = svc.folder.stamps();
        let index = svc.index();
        stamps
            .iter()
            .filter(|(path, _)| under(path, rel))
            .filter_map(|(path, stamp)| {
                let saved = index.item(path)?.clone();
                Some((path.clone(), Row { stamp: *stamp, saved }))
            })
            .collect()
    };
    let root = svc.folder.root.clone();
    let under_rel = rel.to_owned();
    let rows = tokio::task::spawn_blocking(move || scan(&root, &under_rel, &known))
        .await
        .map_err(|e| Error::internal(format!("folder walk task: {e}")))?;
    apply(svc, rel, rows).await?;
    if rel.is_empty() {
        *lock(&svc.folder.last_scan) = Some(now());
    }
    Ok(svc.index().len())
}

/// Puts the rows read at or under `rel` in place of what was indexed there.
async fn apply(svc: &Service, rel: &str, rows: Vec<Row>) -> Result<()> {
    let gone: Vec<String> = {
        let mut stamps = svc.folder.stamps();
        let gone: Vec<String> = stamps
            .keys()
            .filter(|p| under(p, rel) && !rows.iter().any(|r| r.path() == p.as_str()))
            .cloned()
            .collect();
        for p in &gone {
            stamps.remove(p);
        }
        for r in &rows {
            stamps.insert(r.path().to_owned(), r.stamp);
        }
        gone
    };
    let changed = {
        let mut index = svc.index_mut();
        index.remove(&gone);
        index.merge(rows.iter().map(|r| r.saved.clone()).collect())
    };
    if gone.is_empty() && changed.is_empty() {
        return Ok(());
    }
    tracing::debug!(
        rel,
        gone = gone.len(),
        changed = changed.len(),
        "music folder refreshed"
    );
    persist(&svc.ctx.db, rel.to_owned(), rows, changed).await
}

/// The whole folder index, read for a switch to folder mode.
pub async fn store_all(db: &Db, rows: Vec<Row>) -> Result<()> {
    let all = rows.iter().map(|r| r.saved.clone()).collect();
    persist(db, String::new(), rows, all).await
}

/// `library_files` at or under `rel` becomes `rows`; the changed songs go to the catalog.
async fn persist(db: &Db, rel: String, rows: Vec<Row>, changed: Vec<Saved>) -> Result<()> {
    db.call(move |c| {
        let tx = c.transaction()?;
        tx.execute(
            "DELETE FROM library_files
             WHERE ?1 = '' OR path = ?1 OR substr(path, 1, length(?1) + 1) = ?1 || '/'",
            [&rel],
        )?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO library_files (path, modified, size, picture, json) VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for r in &rows {
                let json = serde_json::to_string(&r.saved).map_err(|e| Error::internal(format!("file json: {e}")))?;
                stmt.execute(rusqlite::params![
                    r.path(),
                    r.stamp.modified,
                    r.stamp.size as i64,
                    r.stamp.picture,
                    json
                ])?;
            }
        }
        let tracks: Vec<_> = changed.into_iter().map(|s| s.track).collect();
        catalog::remember(&tx, &tracks)?;
        tx.commit()?;
        Ok(())
    })
    .await
}

/// Walks `root/rel` (a file or a folder) and returns a row per audio file. Files whose size and
/// modified time match `known` aren't read again. Blocking.
pub fn scan(root: &Path, rel: &str, known: &HashMap<String, Row>) -> Vec<Row> {
    let mut covers: HashMap<String, Option<Art>> = HashMap::new();
    let mut rows = Vec::new();
    let start = to_path(root, rel);
    let walk = walkdir::WalkDir::new(&start)
        .follow_links(true)
        .into_iter()
        .filter_entry(|e| e.depth() == 0 || !hidden(e.file_name()));
    for entry in walk {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                // A missing start is a removed file or folder: nothing to index there.
                if e.path() != Some(start.as_path())
                    || e.io_error().is_none_or(|io| io.kind() != std::io::ErrorKind::NotFound)
                {
                    tracing::warn!(error = %e, "skipping part of the music folder");
                }
                continue;
            }
        };
        if !entry.file_type().is_file() || !files::is_audio(entry.path()) {
            continue;
        }
        let Some(path) = rel_of(root, entry.path()) else {
            tracing::warn!(path = %entry.path().display(), "skipping a file whose name isn't UTF-8");
            continue;
        };
        let Ok(meta) = entry.metadata() else { continue };
        let modified = meta.modified().map_or(0, unix);
        let mut row = match known.get(&path) {
            Some(r) if r.stamp.modified == modified && r.stamp.size == meta.len() => r.clone(),
            _ => match files::read(root, &path, modified, meta.len()) {
                Ok(read) => Row {
                    stamp: Stamp {
                        modified,
                        size: meta.len(),
                        picture: read.has_picture,
                    },
                    saved: read.saved,
                },
                Err(e) => {
                    tracing::info!(error = e, "skipping an unreadable audio file");
                    continue;
                }
            },
        };
        let dir = path.rsplit_once('/').map_or("", |(d, _)| d).to_owned();
        let cover = covers
            .entry(dir.clone())
            .or_insert_with(|| folder_cover(root, &dir))
            .clone();
        row.saved.art = cover.or_else(|| {
            row.stamp.picture.then(|| Art::File {
                path: path.clone(),
                embedded: true,
                stamp: format!("{}-{}", row.stamp.modified, row.stamp.size),
            })
        });
        rows.push(row);
    }
    rows
}

/// The cover image next to a folder's songs, if there is one.
fn folder_cover(root: &Path, dir: &str) -> Option<Art> {
    let entries = std::fs::read_dir(to_path(root, dir)).ok()?;
    let mut found: Vec<(usize, String, std::fs::Metadata)> = entries
        .filter_map(|e| {
            let e = e.ok()?;
            let name = e.file_name().into_string().ok()?;
            let rank = COVERS.iter().position(|c| c.eq_ignore_ascii_case(&name))?;
            Some((rank, name, e.metadata().ok()?))
        })
        .collect();
    found.sort_by_key(|(rank, ..)| *rank);
    let (_, name, meta) = found.into_iter().next()?;
    Some(Art::File {
        path: if dir.is_empty() { name } else { format!("{dir}/{name}") },
        embedded: false,
        stamp: format!("{}-{}", meta.modified().map_or(0, unix), meta.len()),
    })
}

/// A save in folder mode: its file joins the index now, without waiting for the watcher.
pub async fn pick_up(svc: &Arc<Service>, file: &Path) -> Result<()> {
    let rel = rel_of(&svc.folder.root, file)
        .ok_or_else(|| Error::internal(format!("{} is outside LULL_MUSIC_DIR", file.display())))?;
    refresh(svc, &rel).await?;
    report(svc);
    Ok(())
}

/// `POST /api/library/rescan` in folder mode: walks the whole folder again.
pub async fn rescan(svc: Arc<Service>, mut job: RescanJob) {
    let before = svc.index().len();
    match refresh(&svc, "").await {
        Ok(after) => {
            let found = after.saturating_sub(before);
            job.state = "done".into();
            job.pct = 100;
            job.detail = match found {
                0 => "No new songs in the music folder".into(),
                1 => "Found 1 new song in the music folder".into(),
                n => format!("Found {n} new songs in the music folder"),
            };
        }
        Err(e) => {
            job.state = "failed".into();
            job.detail = e.to_string();
        }
    }
    report(&svc);
    svc.jobs.put_rescan(&job).await;
}

/// `root/rel` for a `/`-separated `rel`.
fn to_path(root: &Path, rel: &str) -> PathBuf {
    rel.split('/')
        .filter(|p| !p.is_empty())
        .fold(root.to_owned(), |p, part| p.join(part))
}

/// `path` under `root` as `/`-separated text; `None` outside it, through a hidden folder or
/// file (a save's temporary file), or when a name isn't UTF-8.
pub fn rel_of(root: &Path, path: &Path) -> Option<String> {
    let rest = path.strip_prefix(root).ok()?;
    let parts = rest
        .components()
        .map(|c| c.as_os_str().to_str())
        .collect::<Option<Vec<&str>>>()?;
    if parts.iter().any(|p| p.starts_with('.')) {
        return None;
    }
    Some(parts.join("/"))
}

fn hidden(name: &std::ffi::OsStr) -> bool {
    name.to_str().is_some_and(|n| n.starts_with('.'))
}

fn unix(t: SystemTime) -> i64 {
    t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tags::tests::{tiny_flac, tiny_mp3};
    use crate::tags::{self, MbIds, TagSet};

    fn tagged(
        dir: &Path,
        rel: &str,
        bytes: Vec<u8>,
        title: &str,
        cover: Option<&[u8]>,
    ) -> std::result::Result<(), String> {
        let path = to_path(dir, rel);
        std::fs::create_dir_all(path.parent().ok_or("no parent")?).map_err(|e| e.to_string())?;
        std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
        let set = TagSet {
            title: title.into(),
            artist: "Glass Harbour".into(),
            album: "Lantern Season".into(),
            album_artist: "Glass Harbour".into(),
            track: Some(1),
            date: Some("2024".into()),
            genres: vec!["slowcore".into()],
            mb: MbIds::default(),
            ..TagSet::default()
        };
        tags::write(&path, &set, cover)
    }

    #[test]
    fn the_folder_index_reads_tags_covers_and_skips_what_is_not_music() -> std::result::Result<(), String> {
        let root = std::env::temp_dir().join(format!("lull-folder-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let jpeg = [0xFF, 0xD8, 0xFF, 0xE0, 9, 9];
        tagged(
            &root,
            "Glass Harbour/Lantern Season/01 - Rope.flac",
            tiny_flac(),
            "Rope & Tide",
            None,
        )?;
        tagged(
            &root,
            "Glass Harbour/Lantern Season/02 - Tide.mp3",
            tiny_mp3(),
            "Low Tide",
            None,
        )?;
        std::fs::write(root.join("Glass Harbour/Lantern Season/Cover.JPG"), jpeg).map_err(|e| e.to_string())?;
        tagged(&root, "Loose/single.flac", tiny_flac(), "Paper Moon", Some(&jpeg))?;
        // Not music, or hidden (a save's temporary file): never indexed.
        std::fs::write(root.join("Loose/notes.txt"), b"hi").map_err(|e| e.to_string())?;
        tagged(&root, "Loose/.lull-job-1.flac", tiny_flac(), "Half written", None)?;

        let rows = scan(&root, "", &HashMap::new());
        let mut titles: Vec<&str> = rows.iter().map(|r| r.saved.track.title.as_str()).collect();
        titles.sort_unstable();
        assert_eq!(titles, ["Low Tide", "Paper Moon", "Rope & Tide"]);

        let rope = rows
            .iter()
            .find(|r| r.saved.track.title == "Rope & Tide")
            .ok_or("no rope")?;
        assert_eq!(
            rope.saved.track.id,
            lull_core::ids::TrackId::of("Glass Harbour", "Lantern Season", "Rope & Tide")
        );
        assert_eq!(rope.saved.track.file.as_ref().map(|f| f.lossless), Some(true));
        assert_eq!(rope.saved.track.tags, ["slowcore"]);
        assert!(
            matches!(&rope.saved.art, Some(Art::File { path, embedded: false, .. }) if path.ends_with("Cover.JPG"))
        );
        let single = rows
            .iter()
            .find(|r| r.saved.track.title == "Paper Moon")
            .ok_or("no single")?;
        assert!(
            matches!(&single.saved.art, Some(Art::File { path, embedded: true, .. }) if path == "Loose/single.flac")
        );

        // An unchanged file is reused, not read again: a stale title in `known` survives.
        let mut known: HashMap<String, Row> = rows.iter().map(|r| (r.path().to_owned(), r.clone())).collect();
        if let Some(r) = known.get_mut("Loose/single.flac") {
            r.saved.track.title = "From the last run".into();
        }
        let again = scan(&root, "Loose", &known);
        assert_eq!(again.len(), 1);
        assert_eq!(again[0].saved.track.title, "From the last run");

        // A removed file: scanning its path finds nothing, so it leaves the index.
        std::fs::remove_file(root.join("Loose/single.flac")).map_err(|e| e.to_string())?;
        assert!(scan(&root, "Loose/single.flac", &known).is_empty());
        std::fs::remove_dir_all(&root).map_err(|e| e.to_string())
    }

    #[test]
    fn watcher_events_refresh_the_smallest_covering_paths() {
        let root = Path::new("/music");
        let got = to_refresh(
            root,
            vec![
                root.join("A/Album/01.flac"),
                root.join("A/Album/cover.jpg"),
                root.join("A/Album/.lull-job-2.flac"),
                root.join("A/Album/notes.txt"),
                root.join("B/Gone Album"),
                root.join("B/Gone Album/03.mp3"),
                PathBuf::from("/elsewhere/x.flac"),
            ],
        );
        assert_eq!(got, ["A/Album", "B/Gone Album"]);
    }
}
