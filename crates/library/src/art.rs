//! `GET /api/art/{albumId}`: the library's cover for saved albums (Jellyfin's image, or the
//! music folder's image file or embedded picture), else the Cover Art Archive's. Every image is
//! cached on disk under `<data_dir>/art`, and so is "the archive has none" (for a week), so a
//! cover costs at most one outside call per album and size. Sizes and scaling are
//! `lull_core::art`'s: scaled down to the size asked for, never up.
//!
//! The outside lookup (MusicBrainz, then the archive) never holds a request for long: a page asks
//! for many covers at once, and browsers open only six connections per host. A request waits
//! `WAIT` for it, then answers 404 "not yet" while the lookup finishes in the background; once it
//! has the cover, `art.ready` tells the pages, which ask again then (never on a timer).
//!
//! Covers are also fetched before anyone asks (`warm`, `lull_core::warm`): what devices play and
//! play next, the albums discovery hands out without a catalog picture, and the saved albums'
//! row and tile sizes while nothing else waits.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use lull_core::events::Event;
use lull_core::ids::{AlbumId, TrackId};
use lull_core::warm::Priority;
use lull_core::{Ctx, Error, Result, art, catalog};
use serde::Deserialize;
use tokio::sync::{broadcast, oneshot, watch};

use crate::items::Art;
use crate::musicbrainz::{self, CoverOf};
use crate::{Service, tags};

/// How long "no cover anywhere" is believed before asking again.
const NONE_FOR: Duration = Duration::from_secs(7 * 24 * 3600);
/// How long a request waits for an outside lookup before answering 404.
const WAIT: Duration = Duration::from_secs(3);
/// After a lookup fails (MusicBrainz or the archive unreachable), how long new lookups aren't
/// started: a page of covers then asks once, not once per album.
const DOWN_FOR: Duration = Duration::from_secs(60);

/// Outside lookups running or recently failed. Shared with the background tasks.
#[derive(Default)]
pub struct Lookups {
    /// Cache path → turns `true` when its lookup ends, so requests for the same cover share one.
    running: Mutex<HashMap<PathBuf, watch::Receiver<bool>>>,
    down_until: Mutex<Option<Instant>>,
}

impl Lookups {
    fn down(&self) -> bool {
        let until = self.down_until.lock().unwrap_or_else(PoisonError::into_inner);
        until.is_some_and(|t| Instant::now() < t)
    }

    fn failed(&self) {
        *self.down_until.lock().unwrap_or_else(PoisonError::into_inner) = Some(Instant::now() + DOWN_FOR);
    }

    /// Joins the lookup for `path`. The sender comes back when there was none: the caller starts it.
    fn join(&self, path: &Path) -> (watch::Receiver<bool>, Option<watch::Sender<bool>>) {
        let mut running = self.running.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(done) = running.get(path) {
            return (done.clone(), None);
        }
        let (tx, rx) = watch::channel(false);
        running.insert(path.to_owned(), rx.clone());
        (rx, Some(tx))
    }

    fn end(&self, path: &Path) {
        self.running.lock().unwrap_or_else(PoisonError::into_inner).remove(path);
    }
}

/// The MusicBrainz ids a saved album carries, if any.
struct Known {
    release_group: Option<String>,
    release: Option<String>,
}

/// What `/api/art` answers.
pub enum Cover {
    /// At the size asked, or smaller when its source is.
    Image(Vec<u8>),
    /// An outside lookup is on its way (or paused after a failure); `art.ready` follows it.
    NotYet,
    /// No source has one; believed for this much longer.
    None(Duration),
}

/// The archive's size fetched for rows and tiles: one 250 px thumbnail makes 96 and 256.
pub(crate) const SMALL: u32 = 256;

/// The album's cover at `size` (one of `art::SIZES`).
pub async fn cover(svc: &Service, album: &AlbumId, size: u32) -> Result<Cover> {
    art::check(size)?;
    if !album.as_str().starts_with(AlbumId::PREFIX) || !album.as_str()[3..].chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(Error::NotFound(format!("no album {album}")));
    }
    let saved = {
        let index = svc.index();
        index.album(album).map(|a| {
            (
                a.art.clone(),
                a.mb_release_group.clone(),
                a.album.musicbrainz_id.clone(),
            )
        })
    };
    match saved.as_ref().and_then(|(art, ..)| art.as_ref()) {
        Some(Art::Jellyfin { item, tag }) => {
            let path = svc.art_dir.join(format!("{album}-{size}-jf-{tag}"));
            if let Some(image) = cached(&path).await {
                return Ok(Cover::Image(image));
            }
            match svc.jellyfin()?.image(item, size).await {
                Ok(Some(bytes)) => return Ok(Cover::Image(store(&path, bytes).await)),
                Ok(None) => {}
                // Jellyfin down: the archive may still have it.
                Err(e) => tracing::debug!(%album, error = %e, "Jellyfin cover unavailable"),
            }
        }
        Some(Art::File { path, embedded, stamp }) => {
            let cache = svc.art_dir.join(format!("{album}-{size}-file-{stamp}"));
            if let Some(image) = cached(&cache).await {
                return Ok(Cover::Image(image));
            }
            match file_cover(svc.folder.file(path), *embedded, size).await {
                Ok(Some(bytes)) => return Ok(Cover::Image(store(&cache, bytes).await)),
                Ok(None) => {}
                // Changed since it was indexed: the watcher catches up; the archive may have it.
                Err(e) => tracing::debug!(%album, error = e, "folder cover unreadable"),
            }
        }
        None => {}
    }
    let path = archive_file(svc, album, size);
    if let Some(image) = cached(&path).await {
        return Ok(Cover::Image(image));
    }
    // A row's size from the tile's thumbnail, without asking outside again.
    let tier = size.max(SMALL);
    if tier != size
        && let Some(small) = cached(&archive_file(svc, album, tier)).await
        && let Ok(bytes) = scaled(small, size).await
    {
        return Ok(Cover::Image(store(&path, bytes).await));
    }
    let none = svc.art_dir.join(format!("{album}-nocover"));
    if let Some(left) = none_left(&none).await {
        return Ok(Cover::None(left));
    }
    if svc.lookups.down() {
        return Ok(Cover::NotYet);
    }
    let known = saved.map(|(_, release_group, release)| Known { release_group, release });
    let mut done = start_look_up(svc, album, known, tier);
    if tokio::time::timeout(WAIT, done.wait_for(|ended| *ended)).await.is_ok() {
        if let Some(image) = cached(&path).await {
            return Ok(Cover::Image(image));
        }
        if let Some(left) = none_left(&none).await {
            return Ok(Cover::None(left));
        }
    }
    Ok(Cover::NotYet)
}

fn archive_file(svc: &Service, album: &AlbumId, size: u32) -> PathBuf {
    svc.art_dir.join(format!("{album}-{size}-caa"))
}

/// Joins the archive lookup for `album` at `tier`, starting it when none runs; the receiver
/// turns `true` when it ends. A cover found is announced (`art.ready`).
fn start_look_up(svc: &Service, album: &AlbumId, known: Option<Known>, tier: u32) -> watch::Receiver<bool> {
    let path = archive_file(svc, album, tier);
    let (done, start) = svc.lookups.join(&path);
    if let Some(tx) = start {
        let (ctx, lookups, ready, album) = (svc.ctx.clone(), svc.lookups.clone(), svc.ready.clone(), album.clone());
        let row = (tier == SMALL).then(|| archive_file(svc, &album, art::SIZES[0]));
        let none = svc.art_dir.join(format!("{album}-nocover"));
        svc.ctx.spawn(async move {
            if look_up(&ctx, &lookups, &album, known, tier, &path, &none).await {
                if let Some(row) = row
                    && let Some(small) = cached(&path).await
                    && let Ok(bytes) = scaled(small, art::SIZES[0]).await
                {
                    store(&row, bytes).await;
                }
                ready.announce(album.as_str());
            }
            tx.send_replace(true);
            lookups.end(&path);
        });
    }
    done
}

/// Fetches the album's cover at `size` in the background, before anyone asks.
pub fn warm(svc: &Service, album: &AlbumId, size: u32, priority: Priority) {
    let Some(me) = svc.me.upgrade() else { return };
    let album = album.clone();
    svc.warmer
        .push(format!("{album} {}", size.max(SMALL)), priority, async move {
            if let Err(e) = cover(&me, &album, size).await {
                tracing::debug!(%album, error = %e, "cover not warmed");
            }
        });
}

/// Songs played next, from `device.state`: covers the UI asks library for (no catalog picture),
/// at the player bar's and Now Playing's sizes, ahead of what pages asked for.
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
        let albums = svc
            .ctx
            .db
            .call(move |c| {
                let mut albums = Vec::new();
                for song in &songs {
                    if let Some(track) = catalog::track(c, song)?
                        && catalog::album_art(c, &track.album_id)?.is_none()
                        && !albums.contains(&track.album_id)
                    {
                        albums.push(track.album_id);
                    }
                }
                Ok(albums)
            })
            .await;
        let Ok(albums) = albums else { continue };
        svc.warmer.clear(Priority::Playing);
        for (i, album) in albums.iter().enumerate() {
            let sizes: &[u32] = if i == 0 { &[SMALL, 512, 1024] } else { &[SMALL, 512] };
            for &size in sizes {
                warm(&svc, album, size, Priority::Playing);
            }
        }
    }
}

/// The saved albums' row and tile covers, one at a time while nothing more urgent waits, so the
/// library's lists show their covers at once. Once per start.
pub async fn warm_library(svc: Arc<Service>) {
    let albums: Vec<AlbumId> = svc
        .index()
        .albums("recent", "")
        .map_or_else(|_| Vec::new(), |l| l.iter().map(|e| e.album.id.clone()).collect());
    for album in albums {
        for size in [art::SIZES[0], SMALL] {
            let (tx, rx) = oneshot::channel();
            let me = svc.clone();
            let id = album.clone();
            svc.warmer
                .push(format!("{album} {size} idle"), Priority::Idle, async move {
                    let _ = cover(&me, &id, size).await;
                    let _ = tx.send(());
                });
            // Dropped (the queue full of more urgent work) or done: either way, the next one.
            let _ = rx.await;
        }
    }
}

/// Finds a cover outside and caches the answer: the image, or "none" when the archive has
/// none. A failure caches nothing and pauses lookups for `DOWN_FOR`. `true` when it found one.
async fn look_up(
    ctx: &Ctx,
    lookups: &Lookups,
    album: &AlbumId,
    known: Option<Known>,
    size: u32,
    path: &Path,
    none: &Path,
) -> bool {
    // Queued behind a lookup that has since failed.
    if lookups.down() {
        return false;
    }
    match find(ctx, album, known, art::archive_size(size)).await {
        Ok(Some(Some(bytes))) => match scaled(bytes, size).await {
            Ok(bytes) => match write_atomic(path, bytes).await {
                Ok(()) => return true,
                Err(e) => tracing::warn!(path = %path.display(), error = %e, "could not cache a cover"),
            },
            Err(e) => tracing::warn!(error = %e, "could not scale a cover"),
        },
        Ok(Some(None)) => {
            if let Err(e) = write_atomic(none, Vec::new()).await {
                tracing::warn!(error = %e, "could not cache a missing cover");
            }
        }
        // Not "no cover": lull just doesn't know this album yet, so nothing is cached.
        Ok(None) => {}
        Err(e) => {
            lookups.failed();
            tracing::info!(%album, error = %e, "cover lookup failed; pausing cover lookups for a minute");
        }
    }
    false
}

/// The archive's cover at its thumbnail `size` (`None`: the original), `Some(None)` when it has
/// none, or `None` when lull doesn't know the album.
async fn find(ctx: &Ctx, album: &AlbumId, known: Option<Known>, size: Option<u32>) -> Result<Option<Option<Vec<u8>>>> {
    let http = &ctx.http;
    let found = match known {
        Some(Known {
            release_group: Some(group),
            ..
        }) => musicbrainz::cover(http, CoverOf::ReleaseGroup(&group), size).await?,
        Some(Known {
            release: Some(release), ..
        }) => musicbrainz::cover(http, CoverOf::Release(&release), size).await?,
        _ => {
            let Some(groups) = release_groups_by_name(ctx, album).await? else {
                return Ok(None);
            };
            let mut found = None;
            for group in &groups {
                found = musicbrainz::cover(http, CoverOf::ReleaseGroup(group), size).await?;
                if found.is_some() {
                    break;
                }
            }
            found
        }
    };
    Ok(Some(found))
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

/// An archive cover at the size asked for: the archive's smallest thumbnail is 250 px, and a 96 px
/// row cover kept at 250 is 6.8 times the pixels for every client to decode and hold.
async fn scaled(bytes: Vec<u8>, size: u32) -> Result<Vec<u8>, String> {
    tokio::task::spawn_blocking(move || art::fit(bytes, size))
        .await
        .map_err(|e| format!("cover task: {e}"))
}

/// An album lull only knows by id (a search result, a recommendation): its artist and title
/// come from the catalog, then MusicBrainz names the release groups it may be. `None` when the
/// catalog has no song on it.
async fn release_groups_by_name(ctx: &Ctx, album: &AlbumId) -> Result<Option<Vec<String>>> {
    let id = album.clone();
    let Some(track) = ctx.db.call(move |c| catalog::track_on_album(c, &id)).await? else {
        return Ok(None);
    };
    if track.album.trim().is_empty() {
        return Ok(Some(vec![]));
    }
    musicbrainz::release_groups(&ctx.http, &track.artist, &track.album)
        .await
        .map(Some)
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

/// How much longer "no cover" at `path` is believed, if at all.
async fn none_left(path: &Path) -> Option<Duration> {
    let modified = tokio::fs::metadata(path).await.and_then(|m| m.modified()).ok()?;
    let age = SystemTime::now().duration_since(modified).unwrap_or_default();
    NONE_FOR.checked_sub(age).filter(|left| !left.is_zero())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn an_archive_cover_is_kept_at_the_size_asked_for() -> Result<(), String> {
        // The archive answers a 96 px ask with its 250 px thumbnail (look_up's archive_size).
        let mut png = Vec::new();
        image::RgbImage::new(250, 250)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .map_err(|e| e.to_string())?;
        let kept = image::load_from_memory(&scaled(png, 96).await?).map_err(|e| e.to_string())?;
        assert_eq!((kept.width(), kept.height()), (96, 96));
        Ok(())
    }
}
