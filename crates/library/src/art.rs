//! `GET /api/art/{albumId}`: the library's cover for saved albums (Jellyfin's image, or the
//! music folder's image file or embedded picture), else the Cover Art Archive's. Every image is
//! cached on disk under `<data_dir>/art`, and so is "the archive has none" (for a week), so a
//! cover costs at most one outside call per album and size.
//!
//! The outside lookup (MusicBrainz, then the archive) never holds a request for long: a page asks
//! for many covers at once, and browsers open only six connections per host. A request waits
//! `WAIT` for it, then answers 404 while the lookup finishes in the background and caches its
//! answer for the next request.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use lull_core::ids::AlbumId;
use lull_core::{Ctx, Error, Result, catalog};
use tokio::sync::watch;

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

pub struct Image {
    pub bytes: Vec<u8>,
    pub content_type: &'static str,
}

pub async fn cover(svc: &Service, album: &AlbumId, size: u32) -> Result<Image> {
    if ![96, 256, 512].contains(&size) {
        return Err(Error::BadRequest("size must be 96, 256 or 512".into()));
    }
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
                return Ok(image);
            }
            match svc.jellyfin()?.image(item, size).await {
                Ok(Some(bytes)) => return store(&path, bytes).await,
                Ok(None) => {}
                // Jellyfin down: the archive may still have it.
                Err(e) => tracing::debug!(%album, error = %e, "Jellyfin cover unavailable"),
            }
        }
        Some(Art::File { path, embedded, stamp }) => {
            let cache = svc.art_dir.join(format!("{album}-{size}-file-{stamp}"));
            if let Some(image) = cached(&cache).await {
                return Ok(image);
            }
            match file_cover(svc.folder.file(path), *embedded, size).await {
                Ok(Some(bytes)) => return store(&cache, bytes).await,
                Ok(None) => {}
                // Changed since it was indexed: the watcher catches up; the archive may have it.
                Err(e) => tracing::debug!(%album, error = e, "folder cover unreadable"),
            }
        }
        None => {}
    }
    let path = svc.art_dir.join(format!("{album}-{size}-caa"));
    if let Some(image) = cached(&path).await {
        return Ok(image);
    }
    let none = svc.art_dir.join(format!("{album}-nocover"));
    if recently_written(&none).await {
        return Err(Error::NotFound(format!("no cover for {album}")));
    }
    if svc.lookups.down() {
        return Err(Error::NotFound(format!(
            "no cover for {album} yet: cover lookups failed a moment ago"
        )));
    }
    let known = saved.map(|(_, release_group, release)| Known { release_group, release });
    let (mut done, start) = svc.lookups.join(&path);
    if let Some(tx) = start {
        let (ctx, lookups, album) = (svc.ctx.clone(), svc.lookups.clone(), album.clone());
        let (path, none) = (path.clone(), none.clone());
        svc.ctx.spawn(async move {
            look_up(&ctx, &lookups, &album, known, size, &path, &none).await;
            tx.send_replace(true);
            lookups.end(&path);
        });
    }
    if tokio::time::timeout(WAIT, done.wait_for(|ended| *ended)).await.is_ok()
        && let Some(image) = cached(&path).await
    {
        return Ok(image);
    }
    Err(Error::NotFound(format!("no cover for {album} yet")))
}

/// Finds a cover outside and caches the answer: the image, or "none" when the archive has
/// none. A failure caches nothing and pauses lookups for `DOWN_FOR`.
async fn look_up(
    ctx: &Ctx,
    lookups: &Lookups,
    album: &AlbumId,
    known: Option<Known>,
    size: u32,
    path: &Path,
    none: &Path,
) {
    // Queued behind a lookup that has since failed.
    if lookups.down() {
        return;
    }
    let archive_size = if size <= 256 { 250 } else { 500 };
    match find(ctx, album, known, archive_size).await {
        Ok(Some(Some(bytes))) => match scaled(bytes, size).await {
            Ok(bytes) => {
                if let Err(e) = write_atomic(path, bytes).await {
                    tracing::warn!(path = %path.display(), error = %e, "could not cache a cover");
                }
            }
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
}

/// The archive's cover (`Some(None)`: it has none), or `None` when lull doesn't know the album.
async fn find(ctx: &Ctx, album: &AlbumId, known: Option<Known>, size: u32) -> Result<Option<Option<Vec<u8>>>> {
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
        Ok(bytes.map(|b| fit(b, size)))
    })
    .await
    .map_err(|e| format!("cover task: {e}"))?
}

/// An archive cover at the size asked for: the archive's smallest thumbnail is 250 px, and a 96 px
/// row cover kept at 250 is 6.8 times the pixels for every client to decode and hold.
async fn scaled(bytes: Vec<u8>, size: u32) -> Result<Vec<u8>, String> {
    tokio::task::spawn_blocking(move || fit(bytes, size))
        .await
        .map_err(|e| format!("cover task: {e}"))
}

/// Scales an image down to fit `size` × `size` (JPEG), so a 96 px thumbnail isn't a 3 MB
/// scan. Images already small enough, and formats lull doesn't decode, are kept as they are.
fn fit(bytes: Vec<u8>, size: u32) -> Vec<u8> {
    let Ok(image) = image::load_from_memory(&bytes) else {
        return bytes;
    };
    if image.width() <= size && image.height() <= size {
        return bytes;
    }
    let mut out = Vec::new();
    let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 88);
    match image.thumbnail(size, size).to_rgb8().write_with_encoder(encoder) {
        Ok(()) => out,
        Err(e) => {
            tracing::debug!(error = %e, "cover could not be scaled; serving it whole");
            bytes
        }
    }
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

async fn cached(path: &Path) -> Option<Image> {
    let bytes = tokio::fs::read(path).await.ok()?;
    Some(Image {
        content_type: content_type(&bytes),
        bytes,
    })
}

async fn store(path: &Path, bytes: Vec<u8>) -> Result<Image> {
    if let Err(e) = write_atomic(path, bytes.clone()).await {
        tracing::warn!(path = %path.display(), error = %e, "could not cache a cover");
    }
    Ok(Image {
        content_type: content_type(&bytes),
        bytes,
    })
}

async fn write_atomic(path: &Path, bytes: Vec<u8>) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    let temp = PathBuf::from(format!("{}.part", path.display()));
    tokio::fs::write(&temp, bytes).await?;
    tokio::fs::rename(&temp, path).await
}

async fn recently_written(path: &Path) -> bool {
    let modified = tokio::fs::metadata(path).await.and_then(|m| m.modified());
    modified.is_ok_and(|t| SystemTime::now().duration_since(t).is_ok_and(|age| age < NONE_FOR))
}

fn content_type(bytes: &[u8]) -> &'static str {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => "image/png",
        [b'G', b'I', b'F', ..] => "image/gif",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => "image/webp",
        _ => "image/jpeg",
    }
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
