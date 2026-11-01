//! `GET /api/catalog/art/{id}?size=96|256|512`: covers and artist photos found by catalog
//! search (Deezer, the Cover Art Archive), fetched once per size and served from lull, so the
//! UI never loads an outside image and the CSP stays same-origin. Where each picture comes from
//! is `lull_core::catalog`'s; entities point at this route as `art` (albums) and `image`
//! (artists) when one is known.

use std::collections::BTreeMap;
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use axum::extract::{Path, Query, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use lull_core::ids::{AlbumId, ArtistId, TrackId};
use lull_core::model::{Album, Artist, Entities, Track};
use lull_core::{Error, Result, catalog};
use serde::{Deserialize, Serialize};

use crate::Service;

const SIZES: [u32; 3] = [96, 256, 512];
/// A fetched picture is used this long before it is fetched again.
const KEEP: Duration = Duration::from_secs(30 * 24 * 3600);
/// "The source has none" is believed this long.
const NONE_FOR: Duration = Duration::from_secs(24 * 3600);
/// The Cover Art Archive's own sizes; the next one up is served.
const ARCHIVE_SIZES: [u32; 3] = [250, 500, 1200];

/// An album as the UI gets it from discovery: with `art` when lull serves its cover.
#[derive(Debug, Clone, Serialize)]
pub struct PicturedAlbum {
    #[serde(flatten)]
    pub album: Album,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub art: Option<String>,
}

/// An artist with `image` when lull serves a photo.
#[derive(Debug, Clone, Serialize)]
pub struct PicturedArtist {
    #[serde(flatten)]
    pub artist: Artist,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
}

/// `Entities` with pictures (docs/context/api.md#conventions).
#[derive(Debug, Clone, Default, Serialize)]
pub struct Pictured {
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub tracks: BTreeMap<TrackId, Track>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub albums: BTreeMap<AlbumId, PicturedAlbum>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub artists: BTreeMap<ArtistId, PicturedArtist>,
}

/// Same-origin picture URLs for what the catalog has a picture source for.
pub async fn pictured(svc: &Service, e: Entities) -> Result<Pictured> {
    svc.ctx
        .db
        .call(move |c| {
            let mut out = Pictured {
                tracks: e.tracks,
                ..Pictured::default()
            };
            for (id, album) in e.albums {
                let art = catalog::album_art(c, &id)?.map(|_| url(id.as_str()));
                out.albums.insert(id, PicturedAlbum { album, art });
            }
            for (id, artist) in e.artists {
                let image = catalog::artist_image(c, &id)?.map(|_| url(id.as_str()));
                out.artists.insert(id, PicturedArtist { artist, image });
            }
            Ok(out)
        })
        .await
}

/// The route for an album's or artist's picture; the UI adds `?size=`. Relative to the page, as
/// the UI's own requests are, so lull also works behind a reverse-proxy sub-path.
pub fn url(id: &str) -> String {
    format!("api/catalog/art/{id}")
}

#[derive(Deserialize)]
pub struct Size {
    size: u32,
}

pub async fn get(State(svc): State<Arc<Service>>, Path(id): Path<String>, Query(q): Query<Size>) -> Result<Response> {
    if !SIZES.contains(&q.size) {
        return Err(Error::BadRequest("size must be 96, 256 or 512".into()));
    }
    let hex = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_hexdigit());
    let source = match id.as_str() {
        i if i.strip_prefix(AlbumId::PREFIX).is_some_and(hex) => {
            let id = AlbumId(id.clone());
            svc.ctx.db.call(move |c| catalog::album_art(c, &id)).await?
        }
        i if i.strip_prefix(ArtistId::PREFIX).is_some_and(hex) => {
            let id = ArtistId(id.clone());
            svc.ctx.db.call(move |c| catalog::artist_image(c, &id)).await?
        }
        _ => None,
    };
    let source = source.ok_or_else(|| Error::NotFound(format!("lull has no picture for {id}")))?;
    let path = svc.art_dir.join(format!("{id}-{}", q.size));
    let none = svc.art_dir.join(format!("{id}-{}.none", q.size));
    let bytes = match read_fresh(&path, KEEP).await {
        Some(bytes) => bytes,
        None if read_fresh(&none, NONE_FOR).await.is_some() => {
            return Err(Error::NotFound(format!("no picture for {id}")));
        }
        None => match fetch(&svc, &sized(&source, q.size)).await? {
            Some(bytes) => {
                write(&path, &bytes).await;
                bytes
            }
            None => {
                write(&none, &[]).await;
                return Err(Error::NotFound(format!("no picture for {id}")));
            }
        },
    };
    Ok((
        [
            (header::CONTENT_TYPE, content_type(&bytes)),
            (header::CACHE_CONTROL, "private, max-age=86400"),
        ],
        bytes,
    )
        .into_response())
}

/// The source URL at `size`; the Cover Art Archive only has its own sizes.
pub fn sized(source: &str, size: u32) -> String {
    let size = if source.starts_with("https://coverartarchive.org/") {
        ARCHIVE_SIZES.into_iter().find(|s| *s >= size).unwrap_or(1200)
    } else {
        size
    };
    source.replace("{size}", &size.to_string())
}

async fn fetch(svc: &Service, url: &str) -> Result<Option<Vec<u8>>> {
    let service = if url.starts_with("https://coverartarchive.org/") {
        "Cover Art Archive"
    } else {
        "Deezer"
    };
    let resp = svc.ctx.http.send(service, |c| c.get(url)).await?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let resp = lull_core::http::ok(service, resp)?;
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| Error::upstream(service, format!("picture cut short: {e}")))?;
    Ok(Some(bytes.to_vec()))
}

async fn read_fresh(path: &FsPath, keep: Duration) -> Option<Vec<u8>> {
    let modified = tokio::fs::metadata(path).await.ok()?.modified().ok()?;
    let age = SystemTime::now().duration_since(modified).unwrap_or_default();
    if age > keep {
        return None;
    }
    tokio::fs::read(path).await.ok()
}

/// Best effort: a picture that can't be cached is fetched again next time.
async fn write(path: &FsPath, bytes: &[u8]) {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".part");
    let tmp = PathBuf::from(tmp);
    let written = async {
        if let Some(dir) = path.parent() {
            tokio::fs::create_dir_all(dir).await?;
        }
        tokio::fs::write(&tmp, bytes).await?;
        tokio::fs::rename(&tmp, path).await
    };
    if let Err(e) = written.await {
        tracing::warn!(error = %e, path = %path.display(), "could not cache a picture");
    }
}

fn content_type(bytes: &[u8]) -> &'static str {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => "image/png",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => "image/webp",
        [b'G', b'I', b'F', ..] => "image/gif",
        _ => "image/jpeg",
    }
}
