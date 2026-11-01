//! `GET /api/catalog/art/{id}?size=96|256|512`: covers and artist photos discovery found
//! (Deezer, iTunes, the Cover Art Archive), fetched once per size, scaled to it and served from
//! lull, so the UI never loads an outside image and the CSP stays same-origin. Where each
//! picture comes from is `lull_core::catalog`'s; an album with a MusicBrainz release group and
//! no other source gets the Cover Art Archive's. Entities point at this route as `art` (albums)
//! and `image` (artists) whenever a source is known.

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
const DEFAULT_SIZE: u32 = 256;
/// One try per picture: a slow source isn't worth a long wait for a thumbnail.
const FETCH_TIMEOUT: Duration = Duration::from_secs(6);
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
    svc.ctx.db.call(move |c| picture(c, e)).await
}

/// `pictured` on a connection. An album with a release group and no known source gets the Cover
/// Art Archive's, recorded so the route serves it.
pub fn picture(c: &rusqlite::Connection, e: Entities) -> Result<Pictured> {
    let mut out = Pictured {
        tracks: e.tracks,
        ..Pictured::default()
    };
    for (id, album) in e.albums {
        let mut source = catalog::album_art(c, &id)?;
        if source.is_none()
            && let Some(group) = &album.musicbrainz_id
        {
            let archive = cover_archive(group);
            catalog::remember_album(c, &album, Some(&archive))?;
            source = Some(archive);
        }
        let art = source.map(|_| url(id.as_str()));
        out.albums.insert(id, PicturedAlbum { album, art });
    }
    for (id, artist) in e.artists {
        let image = catalog::artist_image(c, &id)?.map(|_| url(id.as_str()));
        out.artists.insert(id, PicturedArtist { artist, image });
    }
    Ok(out)
}

/// The Cover Art Archive's front cover of a release group, as a picture source.
pub fn cover_archive(release_group: &str) -> String {
    format!("https://coverartarchive.org/release-group/{release_group}/front-{{size}}")
}

/// The route for an album's or artist's picture; the UI adds `?size=`. Relative to the page, as
/// the UI's own requests are, so lull also works behind a reverse-proxy sub-path.
pub fn url(id: &str) -> String {
    format!("api/catalog/art/{id}")
}

#[derive(Deserialize)]
pub struct Size {
    #[serde(default = "default_size")]
    size: u32,
}

fn default_size() -> u32 {
    DEFAULT_SIZE
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
                let size = q.size;
                let bytes = tokio::task::spawn_blocking(move || fit(bytes, size))
                    .await
                    .map_err(|e| Error::internal(format!("picture scaling: {e}")))?;
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

/// The source URL at `size`. The Cover Art Archive only has its own sizes: the smallest that
/// covers the request (250 counts for 256), scaled down after.
pub fn sized(source: &str, size: u32) -> String {
    let size = if source.starts_with("https://coverartarchive.org/") {
        ARCHIVE_SIZES.into_iter().find(|s| s * 100 >= size * 95).unwrap_or(1200)
    } else {
        size
    };
    source.replace("{size}", &size.to_string())
}

/// The picture scaled to fit `size` pixels (JPEG), or as it is when it already fits or can't
/// be read.
pub fn fit(bytes: Vec<u8>, size: u32) -> Vec<u8> {
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
            tracing::debug!(error = %e, "picture could not be scaled; serving it whole");
            bytes
        }
    }
}

async fn fetch(svc: &Service, url: &str) -> Result<Option<Vec<u8>>> {
    let service = if url.starts_with("https://coverartarchive.org/") {
        "Cover Art Archive"
    } else if url.contains(".mzstatic.com/") {
        "iTunes"
    } else {
        "Deezer"
    };
    let resp = svc
        .ctx
        .http
        .client()
        .get(url)
        .timeout(FETCH_TIMEOUT)
        .send()
        .await
        .map_err(|e| Error::unavailable(service, e.without_url().to_string()))?;
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

#[cfg(test)]
mod tests {
    use lull_core::db::Db;
    use lull_core::ids::{AlbumId, ArtistId};
    use lull_core::model::{Album, Entities, Saved};
    use lull_core::{Result, catalog};

    use super::{fit, picture, sized};

    fn album(title: &str, group: Option<&str>) -> Album {
        Album {
            id: AlbumId::of("Velvet Orchard", title),
            title: title.into(),
            artist_id: ArtistId::of("Velvet Orchard"),
            artist: "Velvet Orchard".into(),
            year: None,
            kind: Some("album".into()),
            released: None,
            label: None,
            tags: vec![],
            saved: Saved::None,
            track_ids: vec![],
            duration: None,
            musicbrainz_id: group.map(str::to_owned),
        }
    }

    #[tokio::test]
    async fn a_new_release_with_a_release_group_gets_the_archives_cover() -> Result<()> {
        // Live, Home's new releases had no `art`, and both picture routes answered 404.
        let db = Db::memory()?;
        db.migrate(&catalog::MIGRATIONS)?;
        let known = album("Satellite Porch", Some("0f3c0c64-5b0c-4d0b-9c0c-9a3a2c5b1e11"));
        let unknown = album("Paper Moon Static", None);
        let (known_id, unknown_id) = (known.id.clone(), unknown.id.clone());
        let e = Entities {
            albums: [(known.id.clone(), known), (unknown.id.clone(), unknown)].into(),
            ..Entities::default()
        };
        let asked = known_id.clone();
        let (out, source) = db
            .call(move |c| Ok((picture(c, e)?, catalog::album_art(c, &asked)?)))
            .await?;
        assert!(out.albums[&known_id].art.is_some());
        assert!(out.albums[&unknown_id].art.is_none());
        assert_eq!(
            source.as_deref(),
            Some("https://coverartarchive.org/release-group/0f3c0c64-5b0c-4d0b-9c0c-9a3a2c5b1e11/front-{size}")
        );
        Ok(())
    }

    #[test]
    fn the_archive_is_asked_for_the_smallest_size_that_covers_the_request() {
        let archive = "https://coverartarchive.org/release-group/x/front-{size}";
        assert!(sized(archive, 96).ends_with("front-250"));
        assert!(sized(archive, 256).ends_with("front-250"));
        assert!(sized(archive, 512).ends_with("front-500"));
        assert!(sized("https://cdn.example/{size}x{size}.jpg", 96).ends_with("/96x96.jpg"));
    }

    #[test]
    fn a_picture_is_scaled_to_the_size_asked_for() {
        // Live, size=96 came back as the archive's 188x250 original.
        let big = image::RgbImage::from_pixel(188, 250, image::Rgb([200, 120, 40]));
        let mut jpeg = Vec::new();
        let encoded = image::DynamicImage::ImageRgb8(big)
            .write_to(&mut std::io::Cursor::new(&mut jpeg), image::ImageFormat::Jpeg);
        assert!(encoded.is_ok());
        let scaled = image::load_from_memory(&fit(jpeg, 96))
            .map(|i| (i.width(), i.height()))
            .ok();
        assert_eq!(scaled, Some((72, 96)));
        let small = vec![1, 2, 3];
        assert_eq!(fit(small.clone(), 96), small, "what can't be read is served as it is");
    }
}
