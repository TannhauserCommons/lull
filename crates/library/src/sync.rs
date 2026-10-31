//! Keeps the index in step with Jellyfin: a full read at start-up and every half hour, the
//! newest items while a save waits for its rescan, and the `jellyfin` service status.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError};
use std::time::{Duration, Instant};

use lull_core::db::Db;
use lull_core::ids::TrackId;
use lull_core::status::State;
use lull_core::{Result, catalog};
use serde_json::json;

use crate::Service;
use crate::items::{Saved, saved_from};
use crate::jellyfin::{MusicLibrary, Scan};
use crate::jobs::RescanJob;
use crate::save;

const FULL_EVERY: Duration = Duration::from_secs(30 * 60);
const IDLE_TICK: Duration = Duration::from_secs(60);
/// While a save waits for Jellyfin to show its file.
const WAITING_TICK: Duration = Duration::from_secs(5);
/// Newest items read per check while waiting: more than any one album.
const LATEST: usize = 200;

/// The persisted library and aliases, for start-up.
pub async fn load(db: &Db) -> Result<(Vec<Saved>, HashMap<TrackId, TrackId>)> {
    db.call(|c| {
        let mut stmt = c.prepare("SELECT json FROM library_items")?;
        let items = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .filter_map(|json| match json.map(|j| serde_json::from_str::<Saved>(&j)) {
                Ok(Ok(saved)) => Some(Ok(saved)),
                // An older shape is replaced by the next full read; it must not stop start-up.
                Ok(Err(e)) => {
                    tracing::warn!(error = %e, "skipping unreadable library row");
                    None
                }
                Err(e) => Some(Err(e)),
            })
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut stmt = c.prepare("SELECT from_id, to_id FROM library_aliases")?;
        let aliases = stmt
            .query_map([], |r| Ok((TrackId(r.get(0)?), TrackId(r.get(1)?))))?
            .collect::<rusqlite::Result<HashMap<_, _>>>()?;
        Ok((items, aliases))
    })
    .await
}

pub async fn run(svc: Arc<Service>) {
    let mut last_full: Option<Instant> = None;
    loop {
        match tick(&svc, &mut last_full).await {
            Ok(()) => {
                let songs = format!("Library online, {} songs", thousands(svc.index().len()));
                // Without a music folder over LULL_MUSIC_DIR, saved files never show up in Jellyfin.
                let (state, detail) = if svc.music().is_some() {
                    (State::Ok, songs)
                } else {
                    (
                        State::Warn,
                        format!(
                            "{songs}, but Jellyfin has no Music library over LULL_MUSIC_DIR, so saved songs won't \
                             show up: add one in Jellyfin's dashboard"
                        ),
                    )
                };
                svc.ctx
                    .status
                    .set("jellyfin", state, detail, json!({ "songs": svc.index().len() }));
            }
            Err(e) => {
                // Read everything again once it's back: anything may have changed meanwhile.
                last_full = None;
                svc.ctx.status.set(
                    "jellyfin",
                    State::Down,
                    format!("{e}; showing the last known library"),
                    json!({ "songs": svc.index().len() }),
                );
                save::jellyfin_down(&svc, &e.to_string()).await;
            }
        }
        let delay = if save::waiting(&svc) { WAITING_TICK } else { IDLE_TICK };
        tokio::select! {
            () = tokio::time::sleep(delay) => {}
            () = svc.wake.notified() => {}
        }
    }
}

async fn tick(svc: &Arc<Service>, last_full: &mut Option<Instant>) -> Result<()> {
    let jellyfin = svc.jellyfin()?;
    jellyfin.ping().await?;
    if last_full.is_none_or(|t| t.elapsed() >= FULL_EVERY) {
        full_read(svc).await?;
        *last_full = Some(Instant::now());
    } else if save::waiting(svc) {
        let latest = jellyfin.latest_audio(LATEST).await?;
        let changed = svc
            .index_mut()
            .merge(latest.into_iter().filter_map(saved_from).collect());
        persist(svc, changed, false).await?;
    }
    save::settle(svc).await;
    Ok(())
}

/// Reads the whole library, replaces the index and works out where the music folder is.
pub async fn full_read(svc: &Arc<Service>) -> Result<()> {
    let jellyfin = svc.jellyfin()?;
    let items: Vec<Saved> = jellyfin.all_audio().await?.into_iter().filter_map(saved_from).collect();
    svc.index_mut().replace(items.clone());
    let libraries = jellyfin.music_libraries().await?;
    let music = music_folder(libraries, &items, &svc.ctx.config.library.music_dir);
    *svc.music.write().unwrap_or_else(PoisonError::into_inner) = music;
    persist(svc, items, true).await
}

/// `POST /api/library/rescan`: runs Jellyfin's library scan, following its progress, then reads
/// the library again and reports how many songs it found.
pub async fn rescan(svc: Arc<Service>, mut job: RescanJob) {
    let before = svc.index().len();
    match scan(&svc, &mut job).await {
        Ok(()) => match full_read(&svc).await {
            Ok(()) => {
                let found = svc.index().len().saturating_sub(before);
                job.state = "done".into();
                job.pct = 100;
                job.detail = match found {
                    0 => "Jellyfin found no new songs".into(),
                    1 => "Jellyfin found 1 new song".into(),
                    n => format!("Jellyfin found {} new songs", thousands(n)),
                };
            }
            Err(e) => fail(&mut job, &e),
        },
        Err(e) => fail(&mut job, &e),
    }
    svc.jobs.put_rescan(&job).await;
    svc.wake.notify_one();
}

const SCAN_POLL: Duration = Duration::from_secs(3);
const SCAN_LIMIT: Duration = Duration::from_secs(30 * 60);
/// A small library can finish before the first poll; after this, idle means done.
const SCAN_START: Duration = Duration::from_secs(10);

async fn scan(svc: &Service, job: &mut RescanJob) -> Result<()> {
    let jellyfin = svc.jellyfin()?;
    jellyfin.scan_library().await?;
    let started = Instant::now();
    let mut seen_running = false;
    while started.elapsed() < SCAN_LIMIT {
        tokio::time::sleep(SCAN_POLL).await;
        match jellyfin.scan_state().await? {
            Scan::Running { pct } => {
                seen_running = true;
                if pct != job.pct {
                    job.pct = pct;
                    job.detail = "Jellyfin is scanning the library".into();
                    svc.jobs.put_rescan(job).await;
                }
            }
            Scan::Idle if seen_running || started.elapsed() >= SCAN_START => return Ok(()),
            Scan::Idle => {}
        }
    }
    Err(lull_core::Error::upstream(
        "jellyfin",
        "its library scan didn't finish within 30 minutes",
    ))
}

fn fail(job: &mut RescanJob, e: &lull_core::Error) {
    job.state = "failed".into();
    job.detail = e.to_string();
}

/// Stores items (all of them, or the changed ones) and records them in the catalog.
async fn persist(svc: &Service, items: Vec<Saved>, all: bool) -> Result<()> {
    if items.is_empty() && !all {
        return Ok(());
    }
    svc.ctx
        .db
        .call(move |c| {
            let tx = c.transaction()?;
            if all {
                tx.execute("DELETE FROM library_items", [])?;
            }
            {
                let mut stmt = tx.prepare_cached(
                    "INSERT INTO library_items (item_id, json) VALUES (?1, ?2)
                     ON CONFLICT(item_id) DO UPDATE SET json = excluded.json",
                )?;
                for s in &items {
                    let json =
                        serde_json::to_string(s).map_err(|e| lull_core::Error::internal(format!("item json: {e}")))?;
                    stmt.execute(rusqlite::params![s.item_id, json])?;
                }
            }
            let tracks: Vec<_> = items.into_iter().map(|s| s.track).collect();
            catalog::remember(&tx, &tracks)?;
            tx.commit()?;
            Ok(())
        })
        .await
}

/// `LULL_MUSIC_DIR` as Jellyfin knows it: one of its music libraries' folders.
#[derive(Debug, Clone, PartialEq)]
pub struct MusicFolder {
    /// The folder's path as Jellyfin sees it.
    pub root: String,
    /// The library's item, refreshed after a save.
    pub library: String,
}

/// Which music library folder is `LULL_MUSIC_DIR`: the only one there is, or the one whose
/// files lull can see on its own disk.
fn music_folder(libraries: Vec<MusicLibrary>, items: &[Saved], music_dir: &Path) -> Option<MusicFolder> {
    let folders: Vec<MusicFolder> = libraries
        .into_iter()
        .flat_map(|lib| {
            lib.locations.into_iter().map(move |root| MusicFolder {
                root,
                library: lib.item_id.clone(),
            })
        })
        .collect();
    if let [only] = folders.as_slice() {
        return Some(only.clone());
    }
    folders.into_iter().find(|f| {
        items
            .iter()
            .filter_map(|s| s.path.as_deref())
            .find(|p| p.starts_with(f.root.as_str()))
            .and_then(|p| to_local(&f.root, music_dir, p))
            .is_some_and(|local| local.exists())
    })
}

/// A Jellyfin path as lull sees it.
pub fn to_local(root: &str, music_dir: &Path, jellyfin_path: &str) -> Option<PathBuf> {
    let rel = jellyfin_path
        .strip_prefix(root.trim_end_matches('/'))?
        .strip_prefix('/')?;
    Some(rel.split('/').fold(music_dir.to_owned(), |p, part| p.join(part)))
}

fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use lull_core::ids::{AlbumId, ArtistId, TrackId};
    use lull_core::model::Track;

    use super::*;

    fn library(id: &str, locations: &[&str]) -> MusicLibrary {
        MusicLibrary {
            item_id: id.into(),
            locations: locations.iter().map(|l| (*l).to_owned()).collect(),
        }
    }

    fn saved_at(path: &str) -> Saved {
        Saved {
            item_id: "i-1".into(),
            track: Track {
                id: TrackId::of("Glass Harbour", "Lantern Season", "Rope & Tide"),
                title: "Rope & Tide".into(),
                artist_id: ArtistId::of("Glass Harbour"),
                artist: "Glass Harbour".into(),
                album_id: AlbumId::of("Glass Harbour", "Lantern Season"),
                album: "Lantern Season".into(),
                year: None,
                no: None,
                disc: None,
                duration: None,
                tags: vec![],
                saved: true,
                file: None,
            },
            album_artist: "Glass Harbour".into(),
            path: Some(path.into()),
            added: String::new(),
            art: None,
            mb_release: None,
            mb_release_group: None,
            mb_artist: None,
        }
    }

    #[test]
    fn saves_scan_the_music_library_lull_writes_into() -> std::io::Result<()> {
        let music_dir = std::env::temp_dir().join(format!("lull-sync-{}", std::process::id()));
        let album = music_dir.join("Glass Harbour").join("Lantern Season");
        std::fs::create_dir_all(&album)?;
        std::fs::write(album.join("03 - Rope & Tide.flac"), b"x")?;

        // One music folder: it is LULL_MUSIC_DIR, even while empty (a fresh library).
        let only = music_folder(vec![library("lib-music", &["/media/music"])], &[], &music_dir);
        assert_eq!(only.map(|m| m.library), Some("lib-music".into()));

        // Several: the one whose files lull can see.
        let libraries = vec![
            library("lib-old", &["/media/old-music"]),
            library("lib-music", &["/media/vinyl", "/media/internal/music"]),
        ];
        let items = [
            saved_at("/media/old-music/Someone/Else/01 - Other.flac"),
            saved_at("/media/internal/music/Glass Harbour/Lantern Season/03 - Rope & Tide.flac"),
        ];
        let found = music_folder(libraries.clone(), &items, &music_dir);
        assert_eq!(
            found,
            Some(MusicFolder {
                root: "/media/internal/music".into(),
                library: "lib-music".into(),
            })
        );

        // None of them: unknown, so saves fall back to a full scan (and the status warns).
        assert_eq!(music_folder(libraries, &items[..1], &music_dir), None);
        assert_eq!(music_folder(vec![], &items, &music_dir), None);
        std::fs::remove_dir_all(&music_dir)
    }
}
