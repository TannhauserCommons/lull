//! `GET /api/art/{albumId}`: a saved album's own cover (Jellyfin's image, or the music folder's
//! image file or embedded picture), cached on disk under `<data_dir>/art` and keyed by its source,
//! so a changed cover is read again. Sizes and scaling are `lull_core::art`'s: scaled down to the
//! size asked for, never up.
//!
//! An album without a cover of its own (not saved, or saved without a picture) gets its outside
//! cover from the hub's picture service (`lull_core::pictures`, ADR-0033), which looks it up by
//! name in the order the listener's screens show it. Library tells the catalog a saved album's
//! name and release group first, so the service's finder can look it up.
//!
//! Own covers are read before anyone asks (`warm`, a `lull_core::warm` queue): what devices play
//! and play next, the saved albums discovery hands out, and every saved album's row and tile
//! sizes once per start while nothing more urgent waits. Outside covers follow the screens that
//! show this route's pictures, and what plays.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use lull_core::events::Event;
use lull_core::ids::{AlbumId, TrackId};
use lull_core::pictures::{Answer, Route};
use lull_core::warm::Priority;
use lull_core::{Error, Result, art, catalog};
use serde::Deserialize;
use tokio::sync::{broadcast, oneshot};

use crate::items::Art;
use crate::{Service, tags};

/// The size read for rows and tiles before anyone asks.
pub(crate) const SMALL: u32 = 256;

/// The album's cover at `size` (one of `art::SIZES`): its own, else the outside one.
pub async fn cover(svc: &Service, album: &AlbumId, size: u32) -> Result<Answer> {
    art::check(size)?;
    if !album.as_str().starts_with(AlbumId::PREFIX) || !lull_core::pictures::is_id(album.as_str()) {
        return Err(Error::NotFound(format!("no album {album}")));
    }
    if let Some(bytes) = own(svc, album, size).await {
        return Ok(Answer::Image(bytes));
    }
    findable(svc, album).await?;
    svc.ctx.pictures.get(album.as_str(), size).await
}

/// The album's own cover at `size`, when it has one that can be read now.
async fn own(svc: &Service, album: &AlbumId, size: u32) -> Option<Vec<u8>> {
    let art = svc.index().album(album).and_then(|a| a.art.clone())?;
    match art {
        Art::Jellyfin { item, tag } => {
            let path = svc.art_dir.join(format!("{album}-{size}-jf-{tag}"));
            if let Some(image) = cached(&path).await {
                return Some(image);
            }
            match svc.jellyfin().ok()?.image(&item, size).await {
                Ok(Some(bytes)) => Some(store(&path, bytes).await),
                Ok(None) => None,
                // Jellyfin down: the outside cover may still do.
                Err(e) => {
                    tracing::debug!(%album, error = %e, "Jellyfin cover unavailable");
                    None
                }
            }
        }
        Art::File { path, embedded, stamp } => {
            let cache = svc.art_dir.join(format!("{album}-{size}-file-{stamp}"));
            if let Some(image) = cached(&cache).await {
                return Some(image);
            }
            match file_cover(svc.folder.file(&path), embedded, size).await {
                Ok(Some(bytes)) => Some(store(&cache, bytes).await),
                Ok(None) => None,
                // Changed since it was indexed: the watcher catches up; the outside cover may do.
                Err(e) => {
                    tracing::debug!(%album, error = e, "folder cover unreadable");
                    None
                }
            }
        }
    }
}

fn has_own(svc: &Service, album: &AlbumId) -> bool {
    svc.index().album(album).is_some_and(|a| a.art.is_some())
}

/// Tells the catalog a saved album's name and release group, unless it knows the album already,
/// so the picture service can look its cover up. An album that isn't saved is the catalog's.
async fn findable(svc: &Service, album: &AlbumId) -> Result<()> {
    let Some(mut saved) = svc.index().album(album).map(|a| {
        let mut record = a.album.clone();
        record.musicbrainz_id = a.mb_release_group.clone();
        record
    }) else {
        return Ok(());
    };
    saved.track_ids.clear();
    svc.ctx
        .db
        .call(move |c| {
            if catalog::album(c, &saved.id)?.is_none() {
                catalog::remember_album(c, &saved, None)?;
            }
            Ok(())
        })
        .await
}

/// Wants the outside cover of `album` at `size`, for a screen or what plays.
async fn want_outside(svc: &Service, album: &AlbumId, size: u32) {
    if let Err(e) = findable(svc, album).await {
        tracing::debug!(%album, error = %e, "album not told to the catalog");
    }
    svc.ctx.pictures.want(album.as_str(), size).await;
}

/// Reads the album's own cover at `size` in the background, before anyone asks.
pub fn warm(svc: &Service, album: &AlbumId, size: u32, priority: Priority) {
    let Some(me) = svc.me.upgrade() else { return };
    if !has_own(svc, album) {
        return;
    }
    let album = album.clone();
    svc.warmer.push(format!("{album} {size}"), priority, async move {
        own(&me, &album, size).await;
    });
}

/// The pictures screens show from this route (ADR-0033): an album without a cover of its own
/// gets its outside one, in the order the screens show it.
pub async fn follow_screens(svc: Arc<Service>) {
    let mut rx = svc.ctx.pictures.shown();
    loop {
        let shown = match rx.recv().await {
            Ok(shown) => shown.to_vec(),
            Err(broadcast::error::RecvError::Lagged(_)) => svc.ctx.pictures.on_screen(),
            Err(broadcast::error::RecvError::Closed) => return,
        };
        for s in shown.into_iter().filter(|s| s.route == Route::Library) {
            let album = AlbumId(s.id);
            if !has_own(&svc, &album) {
                want_outside(&svc, &album, s.size).await;
            }
        }
    }
}

/// Saved songs played next, from `device.state`: their albums' covers at the player bar's and
/// Now Playing's sizes, ahead of what pages asked for; an album without its own goes to the
/// picture service as what plays.
pub async fn follow_playing(svc: Arc<Service>, mut rx: broadcast::Receiver<Event>) {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Playing {
        track_id: Option<TrackId>,
        #[serde(default)]
        queue: Vec<TrackId>,
        queue_index: Option<usize>,
    }
    let mut last = Vec::new();
    loop {
        let event = match rx.recv().await {
            Ok(e) => e,
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => return,
        };
        if event.topic != "device.state" {
            continue;
        }
        let Ok(p) = serde_json::from_value::<Playing>(event.data) else {
            continue;
        };
        let next = p.queue_index.map_or(0, |i| i + 1);
        let songs: Vec<TrackId> = p
            .track_id
            .into_iter()
            .chain(p.queue.into_iter().skip(next).take(3))
            .collect();
        if songs == last {
            continue;
        }
        last.clone_from(&songs);
        let albums: Vec<AlbumId> = {
            let index = svc.index();
            let mut albums = Vec::new();
            for album in songs.iter().filter_map(|s| index.track(s)).map(|t| t.album_id) {
                if !albums.contains(&album) {
                    albums.push(album);
                }
            }
            albums
        };
        svc.warmer.clear(Priority::Playing);
        let mut outside = Vec::new();
        for (i, album) in albums.iter().enumerate() {
            let sizes = if i == 0 {
                vec![SMALL, 512, 1024]
            } else {
                vec![SMALL, 512]
            };
            if has_own(&svc, album) {
                for &size in &sizes {
                    warm(&svc, album, size, Priority::Playing);
                }
            } else {
                if let Err(e) = findable(&svc, album).await {
                    tracing::debug!(%album, error = %e, "album not told to the catalog");
                }
                outside.push((album.0.clone(), sizes));
            }
        }
        svc.ctx.pictures.set_playing("library", outside).await;
    }
}

/// The saved albums' own row and tile covers, one at a time while nothing more urgent waits, so
/// the library's lists show their covers at once. Once per start.
pub async fn warm_library(svc: Arc<Service>) {
    let albums: Vec<AlbumId> = svc.index().albums("recent", "").map_or_else(
        |_| Vec::new(),
        |l| {
            l.iter()
                .filter(|e| e.art.is_some())
                .map(|e| e.album.id.clone())
                .collect()
        },
    );
    for album in albums {
        for size in [art::SIZES[0], SMALL] {
            let (tx, rx) = oneshot::channel();
            let me = svc.clone();
            let id = album.clone();
            svc.warmer
                .push(format!("{album} {size} idle"), Priority::Idle, async move {
                    own(&me, &id, size).await;
                    let _ = tx.send(());
                });
            // Dropped (the queue full of more urgent work) or done: either way, the next one.
            let _ = rx.await;
        }
    }
}

/// A music-folder cover (an image file, or a song's embedded picture), scaled to fit `size`.
async fn file_cover(file: PathBuf, embedded: bool, size: u32) -> Result<Option<Vec<u8>>, String> {
    tokio::task::spawn_blocking(move || {
        let bytes = if embedded {
            tags::picture(&file)?
        } else {
            Some(std::fs::read(&file).map_err(|e| format!("{}: {e}", file.display()))?)
        };
        Ok(bytes.map(|b| art::fit(b, size)))
    })
    .await
    .map_err(|e| format!("cover task: {e}"))?
}

async fn cached(path: &Path) -> Option<Vec<u8>> {
    tokio::fs::read(path).await.ok()
}

async fn store(path: &Path, bytes: Vec<u8>) -> Vec<u8> {
    if let Err(e) = write_atomic(path, bytes.clone()).await {
        tracing::warn!(path = %path.display(), error = %e, "could not cache a cover");
    }
    bytes
}

async fn write_atomic(path: &Path, bytes: Vec<u8>) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    let temp = PathBuf::from(format!("{}.part", path.display()));
    tokio::fs::write(&temp, bytes).await?;
    tokio::fs::rename(&temp, path).await
}
