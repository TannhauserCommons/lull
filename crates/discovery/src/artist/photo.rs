//! Artist photos, and only photos. Deezer shows an album cover for an artist without a photo
//! (live, BIJI's "photo" was the Cyberpunk: Edgerunners 2 cover), and several artists share a
//! name, so a picture is taken only when it is surely this artist's:
//! 1. Wikimedia Commons, by way of Wikidata's item for the artist's MBID;
//! 2. else Deezer's picture of the Deezer artist MusicBrainz or Wikidata links to (or the only
//!    one of that name), when it is none of the artist's covers (`same_picture` on small
//!    copies of the picture and of the covers lull knows: the discography's, Deezer radio's).
//!
//! Without either, the artist has no photo and each design draws its own fallback. The verdict
//! is kept in `discovery_artist_photos` (a photo a month, "none" a week) and wins over a
//! picture search recorded in the catalog.

use std::path::Path as FsPath;
use std::sync::Arc;

use lull_core::Result;
use lull_core::catalog;
use lull_core::ids::ArtistId;
use lull_core::model::Artist;
use rusqlite::{Connection, OptionalExtension, params};
use tokio::task::JoinSet;

use crate::Service;
use crate::sources::cache::DAY;
use crate::sources::wikidata::WikiArtist;

const KEEP_PHOTO_S: i64 = 30 * DAY;
const KEEP_NONE_S: i64 = 7 * DAY;
/// Bits of 64 two copies of one picture may differ by (scaling, JPEG); unrelated pictures
/// differ by about half.
const SAME_BITS: u32 = 10;
/// Covers compared at most, newest first.
const COVERS: usize = 8;
/// The size pictures are compared at.
const PROBE: u32 = 96;

/// An artist whose photo is wanted, with what the page already knows: the Deezer artist
/// MusicBrainz links it to, and its covers.
pub struct Ask {
    pub artist: Artist,
    pub deezer: Option<u64>,
    pub covers: Vec<String>,
}

/// The photo the catalog serves for an artist: the vetted one (or none), else what search
/// recorded.
pub fn image_of(c: &Connection, id: &ArtistId) -> Result<Option<String>> {
    match vetted(c, id)? {
        Some((image, _)) => Ok(image),
        None => catalog::artist_image(c, id),
    }
}

/// The route's version for a vetted photo, so a browser that kept the old picture (a cover)
/// asks again when it changes.
pub fn version(source: &str) -> String {
    // FNV-1a: stable across builds, unlike the std hasher.
    let h = source
        .bytes()
        .fold(0x811c_9dc5_u32, |h, b| (h ^ u32::from(b)).wrapping_mul(0x0100_0193));
    format!("{h:08x}")
}

/// The vetted verdict and when it was reached, if any.
pub fn vetted(c: &Connection, id: &ArtistId) -> Result<Option<(Option<String>, i64)>> {
    Ok(c.query_row(
        "SELECT image, checked_at FROM discovery_artist_photos WHERE artist_id = ?1",
        [id.as_str()],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()?)
}

/// Vets the photos of `asks` that have no fresh verdict, side by side.
pub async fn find(svc: &Arc<Service>, asks: Vec<Ask>) {
    let now = lull_core::now();
    let mut stale = Vec::new();
    for ask in asks {
        let id = ask.artist.id.clone();
        let verdict = svc.ctx.db.call(move |c| vetted(c, &id)).await;
        let fresh = match verdict {
            Ok(Some((Some(_), at))) => now - at < KEEP_PHOTO_S,
            Ok(Some((None, at))) => now - at < KEEP_NONE_S,
            Ok(None) => false,
            Err(e) => {
                tracing::warn!(error = %e, "artist photo verdict unreadable");
                true
            }
        };
        if !fresh {
            stale.push(ask);
        }
    }
    if stale.is_empty() {
        return;
    }
    let mbids: Vec<String> = stale.iter().filter_map(|a| a.artist.musicbrainz_id.clone()).collect();
    let wiki = if mbids.is_empty() {
        Default::default()
    } else {
        svc.wikidata.artists(&mbids).await.unwrap_or_else(|e| {
            tracing::info!(error = %e, "no artist photos from Wikidata this time");
            Default::default()
        })
    };
    let mut tasks = JoinSet::new();
    for ask in stale {
        let found = ask.artist.musicbrainz_id.as_ref().and_then(|m| wiki.get(m)).cloned();
        let svc = svc.clone();
        tasks.spawn(async move {
            let photo = photo(&svc, &ask, found).await;
            if let Some(photo) = photo {
                record(&svc, &ask.artist.id, photo).await;
            }
        });
    }
    while tasks.join_next().await.is_some() {}
}

/// The artist's photo (`Some(None)`: surely none), or `None` when a source failed and the
/// question stays open.
async fn photo(svc: &Service, ask: &Ask, wiki: Option<WikiArtist>) -> Option<Option<String>> {
    let wiki = wiki.unwrap_or_default();
    if let Some(image) = wiki.image {
        return Some(Some(image));
    }
    let deezer = match ask.deezer.or(wiki.deezer) {
        Some(id) => svc.deezer.artist(id).await,
        None => svc.deezer.artists_named(&ask.artist.name).await.map(|named| {
            // Two artists of one name and no link between them: neither picture is surely this one.
            let mut named = named.into_iter();
            match (named.next(), named.next()) {
                (Some(only), None) => Some(only),
                _ => None,
            }
        }),
    };
    let deezer = match deezer {
        Ok(Some(d)) => d,
        Ok(None) => return Some(None),
        Err(e) => {
            tracing::info!(artist = %ask.artist.name, error = %e, "no artist photo from Deezer this time");
            return None;
        }
    };
    let Some(picture) = deezer.picture else {
        return Some(None);
    };
    let mut covers = ask.covers.clone();
    match svc.deezer.artist_covers(deezer.id).await {
        Ok(more) => covers.extend(more),
        Err(e) => tracing::debug!(error = %e, "no Deezer radio covers"),
    }
    covers.dedup();
    covers.truncate(COVERS);
    if covers.is_empty() {
        // Nothing to tell a photo from a cover with.
        return Some(None);
    }
    let probe = hash_of(svc, &picture).await?;
    for cover in &covers {
        if let Some(h) = hash_of(svc, cover).await
            && same_picture(probe, h)
        {
            tracing::info!(artist = %ask.artist.name, "Deezer's artist picture is a cover; no photo");
            return Some(None);
        }
    }
    Some(Some(picture))
}

/// The verdict, kept; the pictures cached for the old source are dropped.
async fn record(svc: &Service, id: &ArtistId, image: Option<String>) {
    let (key, stored) = (id.clone(), image.clone());
    let changed = svc
        .ctx
        .db
        .call(move |c| {
            let before = image_of(c, &key)?;
            c.execute(
                "INSERT INTO discovery_artist_photos (artist_id, image, checked_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(artist_id) DO UPDATE SET image = excluded.image, checked_at = excluded.checked_at",
                params![key.as_str(), stored, lull_core::now()],
            )?;
            Ok(before != stored)
        })
        .await;
    match changed {
        Ok(true) => forget_files(&svc.art_dir, id.as_str()).await,
        Ok(false) => {}
        Err(e) => tracing::warn!(error = %e, "could not keep an artist photo verdict"),
    }
}

/// A small copy's difference hash, or `None` when it can't be fetched or read.
async fn hash_of(svc: &Service, source: &str) -> Option<u64> {
    let url = crate::art::sized(source, PROBE);
    match crate::art::fetch(svc, &url).await {
        Ok(Some(bytes)) => dhash(&bytes),
        Ok(None) => None,
        Err(e) => {
            tracing::debug!(url, error = %e, "picture not fetched for comparing");
            None
        }
    }
}

/// A 64-bit difference hash: whether each pixel of a 9×8 grey copy is brighter than its right
/// neighbour. Scaling and recompressing barely change it.
pub fn dhash(bytes: &[u8]) -> Option<u64> {
    let grey = image::load_from_memory(bytes).ok()?.to_luma8();
    let small = image::imageops::resize(&grey, 9, 8, image::imageops::FilterType::Triangle);
    let mut h = 0u64;
    for y in 0..8 {
        for x in 0..8 {
            h = (h << 1) | u64::from(small.get_pixel(x, y)[0] > small.get_pixel(x + 1, y)[0]);
        }
    }
    Some(h)
}

pub fn same_picture(a: u64, b: u64) -> bool {
    (a ^ b).count_ones() <= SAME_BITS
}

/// Removes every cached size of a picture.
pub async fn forget_files(dir: &FsPath, id: &str) {
    for size in crate::art::SIZES {
        for name in [format!("{id}-{size}"), format!("{id}-{size}.none")] {
            let _ = tokio::fs::remove_file(dir.join(name)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{dhash, same_picture};

    fn jpeg(img: image::RgbImage, size: u32) -> Vec<u8> {
        let img = image::imageops::resize(&img, size, size, image::imageops::FilterType::Triangle);
        let mut out = Vec::new();
        let encoded =
            image::DynamicImage::ImageRgb8(img).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Jpeg);
        assert!(encoded.is_ok());
        out
    }

    fn cover() -> image::RgbImage {
        image::RgbImage::from_fn(500, 500, |x, y| {
            let v = ((x * 7 + y * 3) % 256) as u8;
            image::Rgb([v, 255 - v, ((x / 50) * 25) as u8])
        })
    }

    fn photo() -> image::RgbImage {
        image::RgbImage::from_fn(500, 500, |x, y| {
            let d = ((x as i32 - 250).pow(2) + (y as i32 - 200).pow(2)) as u32;
            let v = if d < 120 * 120 { 220 } else { (y / 3) as u8 };
            image::Rgb([v, v / 2, 90])
        })
    }

    #[test]
    fn a_cover_served_as_an_artist_picture_is_told_from_a_photo() {
        // Deezer's "photo" of BIJI was the Edgerunners cover at another size and JPEG quality.
        let (a, b, other) = (
            dhash(&jpeg(cover(), 96)),
            dhash(&jpeg(cover(), 250)),
            dhash(&jpeg(photo(), 96)),
        );
        let (Some(a), Some(b), Some(other)) = (a, b, other) else {
            unreachable!("the test pictures decode");
        };
        assert!(same_picture(a, b), "the same cover at two sizes");
        assert!(!same_picture(a, other), "a photo is not the cover");
        assert_eq!(dhash(b"not a picture"), None);
    }
}
