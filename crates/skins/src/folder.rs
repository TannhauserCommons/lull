//! The listener's own skins folder (`LULL_SKINS_DIR`, docs/context/skins.md): every `.wsz` under
//! it, checked as a downloaded skin is, named by the md5 of its bytes so the same skin in two
//! places counts once. A file that isn't a skin is listed as skipped with why, never an error.
//! The folder is read again when it changes (a watcher) or when asked; what each file was is kept
//! in `skins_folder`, so a rescan reads only files whose size or time changed.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::{Duration, UNIX_EPOCH};

use lull_core::{Ctx, Error, Result};
use md5::{Digest, Md5};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use rusqlite::params;
use serde::Serialize;
use serde_json::json;
use tokio::sync::{mpsc, watch};

use crate::{MAX_SKIN_BYTES, display_name, wsz};

/// Changes arrive in bursts (a folder copied in): the folder is read once they settle.
const SETTLE: Duration = Duration::from_secs(2);

/// What `GET /api/skins/mine` answers once the folder has been read.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Listing {
    /// Why the folder can't be read at all (missing, not a folder), else `None`.
    pub problem: Option<String>,
    /// One per skin, by path; a skin found in several places is listed at its first.
    pub items: Vec<Mine>,
    pub skipped: Vec<Skipped>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Mine {
    pub md5: String,
    pub name: String,
    pub file: String,
    /// Under the folder, `/`-separated.
    pub path: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Skipped {
    pub path: String,
    pub why: String,
}

pub(crate) struct Folder {
    ctx: Ctx,
    root: PathBuf,
    listing: RwLock<Arc<Listing>>,
    /// md5 -> the file it was read from.
    files: RwLock<HashMap<String, PathBuf>>,
    /// One read at a time: the watcher's and a listener's.
    reading: tokio::sync::Mutex<()>,
    /// `true` once the first read has finished: the list waits for it.
    ready: watch::Sender<bool>,
    /// Asks the background loop to read the folder again.
    nudge: mpsc::UnboundedSender<()>,
    watcher: Mutex<Option<RecommendedWatcher>>,
    /// Why the folder isn't watched, when it isn't.
    watch_error: Mutex<Option<String>>,
}

impl Folder {
    /// Starts watching and reading `root` in the background, until the hub stops the crates.
    pub(crate) fn start(ctx: &Ctx, root: PathBuf) -> Arc<Folder> {
        let (nudge, nudges) = mpsc::unbounded_channel();
        let folder = Arc::new(Folder {
            ctx: ctx.clone(),
            root,
            listing: RwLock::default(),
            files: RwLock::default(),
            reading: tokio::sync::Mutex::new(()),
            ready: watch::Sender::new(false),
            nudge,
            watcher: Mutex::new(None),
            watch_error: Mutex::new(None),
        });
        folder.watch();
        ctx.spawn(run(folder.clone(), nudges));
        folder
    }

    /// The folder as last read; waits for the first read.
    pub(crate) async fn listing(&self) -> Arc<Listing> {
        // Fails only if the sender is gone, which `self` holds.
        let _ = self.ready.subscribe().wait_for(|done| *done).await;
        lock(self.listing.read()).clone()
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// Whether added and removed files show without asking (some network mounts send no events).
    pub(crate) fn watching(&self) -> bool {
        lock(self.watch_error.lock()).is_none() && lock(self.watcher.lock()).is_some()
    }

    /// The skin's bytes when the folder holds it. A file that changed since it was read is not
    /// served under its old md5 (the answer is cached for good): the folder is read again instead.
    pub(crate) async fn file(&self, md5: &str) -> Option<Vec<u8>> {
        let path = lock(self.files.read()).get(md5).cloned()?;
        let read = tokio::task::spawn_blocking(move || read_capped(&path)).await;
        match read {
            Ok(Ok(bytes)) if md5_hex(&bytes) == md5 => return Some(bytes),
            Ok(Ok(_)) => tracing::info!(md5, "a skin in the skins folder changed; reading the folder again"),
            Ok(Err(e)) => tracing::info!(
                md5,
                error = e,
                "a skin in the skins folder is gone; reading the folder again"
            ),
            Err(e) => tracing::warn!(error = %e, "reading a skin from the skins folder"),
        }
        let _ = self.nudge.send(());
        None
    }

    /// Reads the folder now and returns what it holds.
    pub(crate) async fn rescan(&self) -> Result<Arc<Listing>> {
        self.read().await?;
        Ok(lock(self.listing.read()).clone())
    }

    fn watch(&self) {
        let nudge = self.nudge.clone();
        let started = notify::recommended_watcher(move |event: notify::Result<notify::Event>| match event {
            // Fails only once the loop has ended; nothing is left to tell.
            Ok(e) if !matches!(e.kind, EventKind::Access(_)) => drop(nudge.send(())),
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "skins folder watcher"),
        })
        .and_then(|mut w| w.watch(&self.root, RecursiveMode::Recursive).map(|()| w));
        match started {
            Ok(w) => *lock(self.watcher.lock()) = Some(w),
            Err(e) => {
                tracing::warn!(folder = %self.root.display(), error = %e, "can't watch the skins folder");
                *lock(self.watch_error.lock()) = Some(e.to_string());
            }
        }
    }

    /// One read of the folder: unchanged files from `skins_folder`, the rest from disk.
    async fn read(&self) -> Result<()> {
        let _one = self.reading.lock().await;
        let root_key = self.root.to_string_lossy().into_owned();
        let key = root_key.clone();
        let known = self.ctx.db.call(move |conn| known_files(conn, &key)).await?;
        let root = self.root.clone();
        let read = tokio::task::spawn_blocking(move || read_folder(&root, &known))
            .await
            .map_err(|e| Error::internal(format!("skins folder read: {e}")))?;
        let listing = match read {
            Err(problem) => Listing {
                problem: Some(problem),
                ..Listing::default()
            },
            Ok(entries) => {
                for e in entries.iter().filter(|e| e.fresh) {
                    if let Err(why) = &e.verdict {
                        tracing::warn!(path = e.path, why, "skipping a file in the skins folder");
                    }
                }
                *lock(self.files.write()) = entries
                    .iter()
                    .filter_map(|e| Some((e.verdict.as_ref().ok()?.clone(), e.file.clone())))
                    .rev() // the first path of a duplicate wins
                    .collect();
                let listing = listing(&entries);
                self.ctx
                    .db
                    .call(move |conn| remember(conn, &root_key, &entries))
                    .await?;
                listing
            }
        };
        let changed = *lock(self.listing.read()).as_ref() != listing;
        *lock(self.listing.write()) = Arc::new(listing);
        if changed && *self.ready.borrow() {
            self.ctx.bus.publish("skins.mine", json!({}));
        }
        Ok(())
    }
}

/// Reads the folder, then again after each burst of changes, until the hub stops the crates.
async fn run(folder: Arc<Folder>, mut nudges: mpsc::UnboundedReceiver<()>) {
    if let Err(e) = folder.read().await {
        tracing::warn!(error = %e, "reading the skins folder failed");
    }
    folder.ready.send_replace(true);
    while nudges.recv().await.is_some() {
        let settle = tokio::time::sleep(SETTLE);
        tokio::pin!(settle);
        loop {
            tokio::select! {
                () = &mut settle => break,
                more = nudges.recv() => if more.is_none() { break },
            }
        }
        if let Err(e) = folder.read().await {
            tracing::warn!(error = %e, "reading the skins folder failed");
        }
    }
}

/// A `.wsz` as last read: its md5, or why it isn't a skin.
struct Known {
    size: u64,
    modified: i64,
    verdict: std::result::Result<String, String>,
}

struct Entry {
    file: PathBuf,
    path: String,
    size: u64,
    modified: i64,
    verdict: std::result::Result<String, String>,
    /// Read from disk this time (not remembered).
    fresh: bool,
}

fn known_files(conn: &rusqlite::Connection, root: &str) -> Result<HashMap<String, Known>> {
    Ok(conn
        .prepare("SELECT path, size, modified, md5, problem FROM skins_folder WHERE root = ?1")?
        .query_map([root], |r| {
            let md5: Option<String> = r.get(3)?;
            let problem: Option<String> = r.get(4)?;
            Ok((
                r.get::<_, String>(0)?,
                Known {
                    // SQLite integers are signed; a file is never 2^63 bytes.
                    size: u64::try_from(r.get::<_, i64>(1)?).unwrap_or(u64::MAX),
                    modified: r.get(2)?,
                    verdict: md5.ok_or_else(|| problem.unwrap_or_default()),
                },
            ))
        })?
        .collect::<rusqlite::Result<_>>()?)
}

/// Stores this read, and forgets files that are gone and folders no longer configured.
fn remember(conn: &mut rusqlite::Connection, root: &str, entries: &[Entry]) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM skins_folder WHERE root != ?1", [root])?;
    let seen: HashSet<&str> = entries.iter().map(|e| e.path.as_str()).collect();
    let gone: Vec<String> = tx
        .prepare("SELECT path FROM skins_folder WHERE root = ?1")?
        .query_map([root], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?
        .into_iter()
        .filter(|p| !seen.contains(p.as_str()))
        .collect();
    for path in gone {
        tx.execute(
            "DELETE FROM skins_folder WHERE root = ?1 AND path = ?2",
            params![root, path],
        )?;
    }
    for e in entries.iter().filter(|e| e.fresh) {
        tx.execute(
            "INSERT INTO skins_folder (root, path, size, modified, md5, problem) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(root, path) DO UPDATE SET size = excluded.size, modified = excluded.modified,
                 md5 = excluded.md5, problem = excluded.problem",
            params![
                root,
                e.path,
                i64::try_from(e.size).unwrap_or(i64::MAX),
                e.modified,
                e.verdict.as_ref().ok(),
                e.verdict.as_ref().err()
            ],
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// Every `.wsz` under `root` (following links, skipping hidden folders), in path order; `known`
/// answers for files whose size and time haven't changed. `Err` when the folder itself can't be read.
fn read_folder(root: &Path, known: &HashMap<String, Known>) -> std::result::Result<Vec<Entry>, String> {
    if !root.is_dir() {
        return Err(format!(
            "The skins folder {} doesn't exist on the hub, or isn't a folder.",
            root.display()
        ));
    }
    let mut entries = Vec::new();
    let walk = walkdir::WalkDir::new(root)
        .follow_links(true)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|e| e.depth() == 0 || !e.file_name().to_string_lossy().starts_with('.'));
    for item in walk {
        let item = match item {
            Ok(item) => item,
            Err(e) => {
                let path = e.path().map_or_else(String::new, |p| relative(root, p));
                tracing::warn!(path, error = %e, "can't read part of the skins folder");
                continue;
            }
        };
        let is_wsz = Path::new(item.file_name())
            .extension()
            .is_some_and(|x| x.eq_ignore_ascii_case("wsz"));
        if !item.file_type().is_file() || !is_wsz {
            continue;
        }
        let path = relative(root, item.path());
        let (size, modified) = match item.metadata() {
            Ok(m) => (m.len(), modified_ns(&m)),
            Err(e) => {
                tracing::warn!(path, error = %e, "can't read a skin file's details");
                (0, -1)
            }
        };
        let file = item.into_path();
        let entry = match known.get(&path) {
            Some(k) if k.size == size && k.modified == modified && modified >= 0 => Entry {
                file,
                path,
                size,
                modified,
                verdict: k.verdict.clone(),
                fresh: false,
            },
            _ => Entry {
                verdict: verdict(&file, size),
                file,
                path,
                size,
                modified,
                fresh: true,
            },
        };
        entries.push(entry);
    }
    Ok(entries)
}

/// The file's md5 when it is a classic skin, else why not, in a sentence for the listener.
fn verdict(file: &Path, size: u64) -> std::result::Result<String, String> {
    if size > MAX_SKIN_BYTES as u64 {
        return Err("it is over 16 MB, which no classic skin is".into());
    }
    let bytes = read_capped(file).map_err(|e| format!("it can't be read ({e})"))?;
    wsz::validate(&bytes).map_err(|why| why.to_string())?;
    Ok(md5_hex(&bytes))
}

/// The file, refusing one larger than a skin can be (it may have grown since it was listed).
fn read_capped(file: &Path) -> std::result::Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    std::fs::File::open(file)
        .and_then(|f| f.take(MAX_SKIN_BYTES as u64 + 1).read_to_end(&mut bytes))
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_SKIN_BYTES {
        return Err("it is over 16 MB".into());
    }
    Ok(bytes)
}

/// The skins, one per md5 at its first path, and the files that aren't skins.
fn listing(entries: &[Entry]) -> Listing {
    let mut seen = HashSet::new();
    let mut listing = Listing::default();
    for e in entries {
        match &e.verdict {
            Ok(md5) if seen.insert(md5.clone()) => {
                let file = e.path.rsplit('/').next().unwrap_or(&e.path).to_owned();
                listing.items.push(Mine {
                    md5: md5.clone(),
                    name: display_name(&file),
                    file,
                    path: e.path.clone(),
                    size: e.size,
                });
            }
            Ok(_) => {}
            Err(why) => listing.skipped.push(Skipped {
                path: e.path.clone(),
                why: why.clone(),
            }),
        }
    }
    listing
}

fn md5_hex(bytes: &[u8]) -> String {
    Md5::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// `path` under `root`, `/`-separated.
fn relative(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Nanoseconds since 1970, or -1 when the file system keeps no time (the file is then always read).
fn modified_ns(m: &std::fs::Metadata) -> i64 {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .and_then(|d| i64::try_from(d.as_nanos()).ok())
        .unwrap_or(-1)
}

/// A lock whose holder panicked still guards consistent data here (every write is one assignment).
fn lock<G>(r: std::result::Result<G, PoisonError<G>>) -> G {
    r.unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wsz::zip;

    /// A fresh, empty folder under the system's temp folder.
    fn temp_folder(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("lull-skins-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::create_dir_all(&root);
        root
    }

    fn put(root: &Path, path: &str, bytes: &[u8]) {
        let file = root.join(path);
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(file, bytes);
    }

    #[test]
    fn a_scan_lists_each_skin_once_and_says_why_other_files_are_skipped() {
        let root = temp_folder("scan");
        let plum = zip(&[("Plum/MAIN.BMP", b"BM plum"), ("Plum/eq.bmp", b"BM")]);
        put(&root, "plum_cassette.wsz", &plum);
        // The same bytes under another name, deeper down: one skin.
        put(&root, "zz old/Plum Cassette (copy).WSZ", &plum);
        put(&root, "modern/winamp5.wsz", &zip(&[("skin.xml", b"<skin/>")]));
        put(&root, "notes.txt", b"not a skin, not even named like one");
        put(&root, ".trash/gone.wsz", &zip(&[("main.bmp", b"BM gone")]));
        // Larger than any classic skin: refused by its size, never read.
        let big = root.join("huge.wsz");
        let _ = std::fs::File::create(&big).and_then(|f| f.set_len(MAX_SKIN_BYTES as u64 + 1));

        let entries = read_folder(&root, &HashMap::new());
        let _ = std::fs::remove_dir_all(&root);

        let listing = listing(&entries.unwrap_or_default());
        assert_eq!(
            listing.items,
            vec![Mine {
                md5: md5_hex(&plum),
                name: "plum cassette".into(),
                file: "plum_cassette.wsz".into(),
                path: "plum_cassette.wsz".into(),
                size: plum.len() as u64,
            }]
        );
        let skipped: Vec<(&str, &str)> = listing
            .skipped
            .iter()
            .map(|s| (s.path.as_str(), s.why.as_str()))
            .collect();
        assert_eq!(
            skipped,
            vec![
                ("huge.wsz", "it is over 16 MB, which no classic skin is"),
                (
                    "modern/winamp5.wsz",
                    "the archive has no main.bmp, so it is not a classic Winamp skin"
                ),
            ]
        );
        assert_eq!(listing.problem, None);
    }

    #[test]
    fn a_missing_folder_is_a_problem_not_an_empty_list() {
        let root = std::env::temp_dir().join("lull-skins-nowhere-at-all");
        assert!(read_folder(&root, &HashMap::new()).is_err_and(|why| why.contains("doesn't exist")));
    }

    #[test]
    fn an_unchanged_file_is_not_read_again() {
        let root = temp_folder("known");
        put(&root, "a.wsz", &zip(&[("main.bmp", b"BM")]));
        let first = read_folder(&root, &HashMap::new()).unwrap_or_default();
        let known: HashMap<String, Known> = first
            .iter()
            .map(|e| {
                // What the database remembers, with an md5 that only a remembered answer can give.
                let k = Known {
                    size: e.size,
                    modified: e.modified,
                    verdict: Ok("0".repeat(32)),
                };
                (e.path.clone(), k)
            })
            .collect();
        let again = read_folder(&root, &known).unwrap_or_default();
        put(&root, "a.wsz", &zip(&[("main.bmp", b"BM, changed")]));
        let changed = read_folder(&root, &known).unwrap_or_default();
        let _ = std::fs::remove_dir_all(&root);

        assert!(first.iter().all(|e| e.fresh));
        assert_eq!(
            again.iter().map(|e| (e.fresh, e.verdict.clone())).collect::<Vec<_>>(),
            vec![(false, Ok("0".repeat(32)))]
        );
        assert!(
            changed
                .iter()
                .all(|e| e.fresh && e.verdict.as_ref().is_ok_and(|m| *m != "0".repeat(32)))
        );
    }
}
