//! `GET /api/art/{albumId}`: Jellyfin's cover for saved albums, else the Cover Art Archive's.
//! Every image is cached on disk under `<data_dir>/art`, and so is "the archive has none" (for a
//! week), so a cover costs at most one outside call per album and size.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use lull_core::ids::AlbumId;
use lull_core::{Error, Result, catalog};

use crate::Service;
use crate::musicbrainz::{self, CoverOf};

/// How long "no cover anywhere" is believed before asking again.
const NONE_FOR: Duration = Duration::from_secs(7 * 24 * 3600);

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
    if let Some((Some((item, tag)), _, _)) = &saved {
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
    let path = svc.art_dir.join(format!("{album}-{size}-caa"));
    if let Some(image) = cached(&path).await {
        return Ok(image);
    }
    let none = svc.art_dir.join(format!("{album}-nocover"));
    if recently_written(&none).await {
        return Err(Error::NotFound(format!("no cover for {album}")));
    }
    let archive_size = if size <= 256 { 250 } else { 500 };
    let found = match saved {
        Some((_, Some(group), _)) => fetch(svc, CoverOf::ReleaseGroup(&group), archive_size).await?,
        Some((_, None, Some(release))) => fetch(svc, CoverOf::Release(&release), archive_size).await?,
        _ => {
            let Some(groups) = release_groups_by_name(svc, album).await? else {
                // Not "no cover": lull just doesn't know this album yet, so nothing is cached.
                return Err(Error::NotFound(format!(
                    "no cover for {album}: lull doesn't know this album"
                )));
            };
            let mut found = None;
            for group in &groups {
                found = fetch(svc, CoverOf::ReleaseGroup(group), archive_size).await?;
                if found.is_some() {
                    break;
                }
            }
            found
        }
    };
    match found {
        Some(bytes) => store(&path, bytes).await,
        None => {
            if let Err(e) = write_atomic(&none, Vec::new()).await {
                tracing::warn!(error = %e, "could not cache a missing cover");
            }
            Err(Error::NotFound(format!("no cover for {album}")))
        }
    }
}

async fn fetch(svc: &Service, of: CoverOf<'_>, size: u32) -> Result<Option<Vec<u8>>> {
    musicbrainz::cover(&svc.ctx.http, of, size).await
}

/// An album lull only knows by id (a search result, a recommendation): its artist and title
/// come from the catalog, then MusicBrainz names the release groups it may be. `None` when the
/// catalog has no song on it.
async fn release_groups_by_name(svc: &Service, album: &AlbumId) -> Result<Option<Vec<String>>> {
    let id = album.clone();
    let Some(track) = svc.ctx.db.call(move |c| catalog::track_on_album(c, &id)).await? else {
        return Ok(None);
    };
    if track.album.trim().is_empty() {
        return Ok(Some(vec![]));
    }
    musicbrainz::release_groups(&svc.ctx.http, &track.artist, &track.album)
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
