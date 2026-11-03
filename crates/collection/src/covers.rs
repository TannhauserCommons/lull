//! Playlist covers (docs/context/collection.md#covers): an image the listener uploads, or the one
//! a link import's source shows. Either is checked, cut to a centred square and stored on the hub
//! as a JPEG of at most 2048 px (`<data>/playlist-covers/pl-<n>.jpg`), so it keeps working whatever
//! the source does later, and is served at the sizes album art is (`lull_core::art`). A playlist
//! without one shows its songs' covers; that mosaic is the UI's.

use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Bytes;
use axum::extract::rejection::QueryRejection;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use image::imageops::FilterType;
use image::{ImageReader, Limits};
use lull_core::{Ctx, Error, Result, art};
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use serde_json::Value;

use crate::playlists::{self, seq_of};
use crate::{Service, bad_query};

/// The stored square's largest side, in pixels: the largest size served. A smaller picture is
/// kept at its own size, never blown up.
pub(crate) const SIZE: u32 = 2048;
/// The largest image accepted, uploaded or fetched.
const MAX_BYTES: usize = 10 * 1024 * 1024;
/// Larger pictures are refused before they are decoded (a small file can claim a huge image).
const MAX_SIDE: u32 = 8000;
const SERVICE: &str = "playlist cover";

/// Where a cover came from: a source's is replaced when the source's changes; an upload stays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Origin {
    Upload,
    Link,
}

impl Origin {
    fn as_str(self) -> &'static str {
        match self {
            Origin::Upload => "upload",
            Origin::Link => "link",
        }
    }
}

pub(crate) fn routes() -> Router<Arc<Service>> {
    Router::new()
        .route("/api/playlists/{id}/cover", get(serve).put(upload).delete(remove))
        .layer(DefaultBodyLimit::max(MAX_BYTES))
}

/// The cover's URL for the playlist's JSON; it changes with each new cover, so it can be cached.
pub(crate) fn url(seq: i64, cover_at: Option<i64>) -> Option<String> {
    cover_at.map(|v| format!("/api/playlists/pl-{seq}/cover?v={v}"))
}

fn file(ctx: &Ctx, seq: i64) -> PathBuf {
    ctx.config
        .data_dir
        .join("playlist-covers")
        .join(format!("pl-{seq}.jpg"))
}

#[derive(Deserialize)]
struct SizeQuery {
    size: Option<u32>,
}

/// `GET /api/playlists/{id}/cover?size=`: the JPEG, scaled down to fit `size` when asked.
async fn serve(
    State(svc): State<Arc<Service>>,
    Path(id): Path<String>,
    query: std::result::Result<Query<SizeQuery>, QueryRejection>,
) -> Result<Response> {
    let Query(q) = query.map_err(bad_query)?;
    response(&svc, seq_of(&id)?, q.size)
        .await?
        .ok_or_else(|| Error::NotFound(format!("Playlist {id} has no cover of its own.")))
}

/// The cover as a response, `None` when the playlist has none (also the `Collection` seam's).
pub(crate) async fn response(svc: &Service, seq: i64, size: Option<u32>) -> Result<Option<Response>> {
    let has: Option<Option<i64>> = svc
        .ctx
        .db
        .call(move |c| {
            Ok(
                c.query_row("SELECT cover_at FROM collection_playlists WHERE seq = ?1", [seq], |r| {
                    r.get(0)
                })
                .optional()?,
            )
        })
        .await?;
    if !matches!(has, Some(Some(_))) {
        return Ok(None);
    }
    jpeg_response(&file(&svc.ctx, seq), size).await
}

/// A stored cover file as a response, scaled down to fit the smallest served size that covers
/// `size` when asked (never up), with its real size in `X-Image-Size`; `None` when there is no
/// file.
pub(crate) async fn jpeg_response(path: &std::path::Path, size: Option<u32>) -> Result<Option<Response>> {
    let jpeg = match tokio::fs::read(path).await {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(Error::internal(format!("reading {}: {e}", path.display()))),
    };
    let jpeg = match size.map(|s| art::SIZES.into_iter().find(|&b| b >= s).unwrap_or(SIZE)) {
        Some(s) => tokio::task::spawn_blocking(move || art::fit(jpeg, s))
            .await
            .map_err(|e| Error::internal(format!("scaling a cover: {e}")))?,
        None => jpeg,
    };
    let real = art::dimensions(&jpeg).map(|(w, h)| format!("{w}x{h}"));
    let mut response = (
        [
            (header::CONTENT_TYPE, "image/jpeg"),
            // The URL carries the cover's version (`?v=`), so a new cover is a new URL.
            (header::CACHE_CONTROL, "private, max-age=31536000, immutable"),
        ],
        jpeg,
    )
        .into_response();
    if let Some(Ok(real)) = real.map(|r| r.parse()) {
        response.headers_mut().insert(art::IMAGE_SIZE, real);
    }
    Ok(Some(response))
}

/// `PUT /api/playlists/{id}/cover` (body: a JPEG, PNG or WebP up to 10 MB): the playlist with it.
async fn upload(State(svc): State<Arc<Service>>, Path(id): Path<String>, body: Bytes) -> Result<Json<Value>> {
    let seq = seq_of(&id)?;
    let jpeg = tokio::task::spawn_blocking(move || square(&body))
        .await
        .map_err(|e| Error::internal(format!("reading a cover: {e}")))??;
    store(&svc, seq, jpeg, Origin::Upload, None).await?;
    svc.ctx.db.call(move |c| playlists::respond(c, seq)).await
}

/// `DELETE /api/playlists/{id}/cover`: back to its songs' covers; the playlist.
async fn remove(State(svc): State<Arc<Service>>, Path(id): Path<String>) -> Result<Json<Value>> {
    let seq = seq_of(&id)?;
    let body = svc
        .write(move |tx, events| {
            set_row(tx, seq, None, None, None)?;
            playlists::announce(events, seq, false);
            playlists::respond(tx, seq)
        })
        .await?;
    forget(&svc.ctx, seq).await;
    Ok(body)
}

/// Writes the cover's file, then records it on the playlist (404 if there is none).
async fn store(svc: &Service, seq: i64, jpeg: Vec<u8>, origin: Origin, url: Option<String>) -> Result<()> {
    let known: Option<i64> = svc
        .ctx
        .db
        .call(move |c| {
            Ok(
                c.query_row("SELECT seq FROM collection_playlists WHERE seq = ?1", [seq], |r| {
                    r.get(0)
                })
                .optional()?,
            )
        })
        .await?;
    if known.is_none() {
        return Err(Error::NotFound(format!("No playlist pl-{seq}.")));
    }
    let path = file(&svc.ctx, seq);
    let written = async {
        if let Some(dir) = path.parent() {
            tokio::fs::create_dir_all(dir).await?;
        }
        let part = path.with_extension("jpg.part");
        tokio::fs::write(&part, &jpeg).await?;
        tokio::fs::rename(&part, &path).await
    };
    written
        .await
        .map_err(|e| Error::internal(format!("storing pl-{seq}'s cover: {e}")))?;
    let at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64);
    svc.write(move |tx, events| {
        set_row(tx, seq, Some(at), Some(origin), url.as_deref())?;
        playlists::announce(events, seq, false);
        Ok(())
    })
    .await
}

fn set_row(
    conn: &rusqlite::Connection,
    seq: i64,
    at: Option<i64>,
    origin: Option<Origin>,
    url: Option<&str>,
) -> Result<()> {
    let changed = conn.execute(
        "UPDATE collection_playlists SET cover_at = ?2, cover_from = ?3, cover_url = ?4, updated_at = ?5 WHERE seq = ?1",
        params![seq, at, origin.map(Origin::as_str), url, lull_core::now()],
    )?;
    if changed == 0 {
        return Err(Error::NotFound(format!("No playlist pl-{seq}.")));
    }
    Ok(())
}

/// Removes a cover's file; a missing one is fine.
pub(crate) async fn forget(ctx: &Ctx, seq: i64) {
    if let Err(e) = tokio::fs::remove_file(file(ctx, seq)).await
        && e.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(error = %e, playlist = seq, "couldn't remove a playlist cover");
    }
}

/// Keeps a link's cover on its playlist: the first of the source's pictures (best first) that
/// loads, fetched when the source shows a picture lull hasn't kept yet, and never over one the
/// listener uploaded. Three are tried at most; a failure leaves the cover as it was.
pub(crate) async fn from_link(svc: &Service, seq: i64, pictures: &[String]) -> Result<()> {
    let pictures: Vec<String> = pictures
        .iter()
        .filter(|u| u.starts_with("https://"))
        .take(3)
        .cloned()
        .collect();
    if pictures.is_empty() {
        return Ok(());
    }
    let had: Option<(Option<String>, Option<String>)> = svc
        .ctx
        .db
        .call(move |c| {
            Ok(c.query_row(
                "SELECT cover_from, cover_url FROM collection_playlists WHERE seq = ?1",
                [seq],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
        })
        .await?;
    match had {
        None => return Ok(()),
        Some((Some(from), _)) if from == Origin::Upload.as_str() => return Ok(()),
        Some((_, Some(kept))) if pictures.contains(&kept) && file(&svc.ctx, seq).exists() => return Ok(()),
        _ => {}
    }
    let (jpeg, url) = first_picture(svc, &pictures).await?;
    store(svc, seq, jpeg, Origin::Link, Some(url)).await
}

/// The first of a source's pictures (best first, up to three) that loads, as a stored cover's
/// JPEG, with its URL.
pub(crate) async fn first_picture(svc: &Service, pictures: &[String]) -> Result<(Vec<u8>, String)> {
    let mut failed = None;
    for url in pictures.iter().filter(|u| u.starts_with("https://")).take(3) {
        match fetch(svc, url).await {
            Ok(jpeg) => return Ok((jpeg, url.clone())),
            Err(e) => failed = Some(e),
        }
    }
    Err(failed.unwrap_or_else(|| Error::NotFound("the source shows no picture".into())))
}

/// A source's picture as the stored cover.
async fn fetch(svc: &Service, url: &str) -> Result<Vec<u8>> {
    let resp = lull_core::http::ok(SERVICE, svc.ctx.http.send(SERVICE, |c| c.get(url)).await?)?;
    if resp.content_length().is_some_and(|n| n > MAX_BYTES as u64) {
        return Err(Error::upstream(SERVICE, "the source's picture is larger than 10 MB"));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| Error::upstream(SERVICE, format!("reading the source's picture: {e}")))?;
    tokio::task::spawn_blocking(move || square(&bytes))
        .await
        .map_err(|e| Error::internal(format!("reading a cover: {e}")))?
}

/// A picture as the stored cover: a JPEG, PNG or WebP of sane size, cut to its centred square,
/// at most `SIZE` px, as JPEG.
pub(crate) fn square(bytes: &[u8]) -> Result<Vec<u8>> {
    let refuse = |why: String| Error::BadRequest(format!("That picture can't be a cover: {why}."));
    if bytes.len() > MAX_BYTES {
        return Err(refuse("it is larger than 10 MB".into()));
    }
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| refuse(e.to_string()))?;
    if !matches!(
        reader.format(),
        Some(image::ImageFormat::Jpeg | image::ImageFormat::Png | image::ImageFormat::WebP)
    ) {
        return Err(refuse("it isn't a JPEG, PNG or WebP image".into()));
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    reader.limits(limits);
    let image = reader.decode().map_err(|e| refuse(e.to_string()))?;
    let side = image.width().min(image.height());
    if side == 0 {
        return Err(refuse("it is empty".into()));
    }
    let cut = image.crop_imm((image.width() - side) / 2, (image.height() - side) / 2, side, side);
    let out = side.min(SIZE);
    encode(&cut.resize_exact(out, out, FilterType::Lanczos3))
}

fn encode(image: &image::DynamicImage) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 88)
        .encode_image(&image.to_rgb8())
        .map_err(|e| Error::internal(format!("encoding a cover: {e}")))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(w: u32, h: u32, format: image::ImageFormat) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(w, h, image::Rgb([200, 120, 40])))
            .write_to(&mut Cursor::new(&mut out), format)
            .map_err(|e| Error::internal(e.to_string()))?;
        Ok(out)
    }

    fn side_of(jpeg: &[u8]) -> Result<(u32, u32)> {
        let image = image::load_from_memory(jpeg).map_err(|e| Error::internal(e.to_string()))?;
        Ok((image.width(), image.height()))
    }

    #[tokio::test]
    async fn an_uploaded_cover_shows_on_the_playlist_scales_and_can_be_removed() -> Result<()> {
        let (svc, _rx) = crate::testing::service().await?;
        let seq = svc
            .write(|tx, events| playlists::make(tx, events, "Porch", "", &[], 0))
            .await?;
        let id = format!("pl-{seq}");
        let png = Bytes::from(picture(900, 700, image::ImageFormat::Png)?);
        let Json(body) = upload(State(svc.clone()), Path(id.clone()), png).await?;
        let cover = body["cover"].as_str().unwrap_or_default().to_owned();
        assert!(cover.starts_with(&format!("/api/playlists/{id}/cover?v=")), "{cover}");
        assert!(file(&svc.ctx, seq).exists());

        let small = response(&svc, seq, Some(96))
            .await?
            .ok_or_else(|| Error::internal("no cover"))?;
        let bytes = axum::body::to_bytes(small.into_body(), MAX_BYTES)
            .await
            .map_err(|e| Error::internal(e.to_string()))?;
        assert_eq!(side_of(&bytes)?, (96, 96));

        let Json(body) = remove(State(svc.clone()), Path(id.clone())).await?;
        assert!(body["cover"].is_null());
        assert!(!file(&svc.ctx, seq).exists());
        assert!(response(&svc, seq, None).await?.is_none());

        let nothing = Bytes::from(picture(10, 10, image::ImageFormat::Png)?);
        assert!(matches!(
            upload(State(svc.clone()), Path("pl-999".into()), nothing).await,
            Err(Error::NotFound(_))
        ));
        assert!(
            !file(&svc.ctx, 999).exists(),
            "nothing is written for a playlist that isn't there"
        );
        Ok(())
    }

    #[test]
    fn a_wide_picture_becomes_its_centred_square_of_at_most_2048_px() -> Result<()> {
        assert_eq!(
            side_of(&square(&picture(1280, 720, image::ImageFormat::Png)?)?)?,
            (720, 720)
        );
        assert_eq!(
            side_of(&square(&picture(3000, 2400, image::ImageFormat::Jpeg)?)?)?,
            (2048, 2048)
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_stored_cover_is_served_at_most_at_its_own_size() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("lull-cover-{}", std::process::id()));
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| Error::internal(e.to_string()))?;
        let path = dir.join("pl-1.jpg");
        tokio::fs::write(&path, square(&picture(600, 600, image::ImageFormat::Jpeg)?)?)
            .await
            .map_err(|e| Error::internal(e.to_string()))?;
        for (asked, real) in [(Some(1024), "600x600"), (Some(200), "256x256"), (None, "600x600")] {
            let answer = jpeg_response(&path, asked)
                .await?
                .ok_or_else(|| Error::internal("no cover"))?;
            assert_eq!(
                answer.headers().get(art::IMAGE_SIZE).and_then(|v| v.to_str().ok()),
                Some(real),
                "asked {asked:?}"
            );
        }
        let _ = tokio::fs::remove_dir_all(&dir).await;
        Ok(())
    }

    #[test]
    fn a_small_picture_is_cut_square_but_not_blown_up() -> Result<()> {
        assert_eq!(
            side_of(&square(&picture(300, 200, image::ImageFormat::Jpeg)?)?)?,
            (200, 200)
        );
        Ok(())
    }

    #[test]
    fn anything_but_a_picture_is_refused_in_plain_words() {
        for bytes in [&b"GIF89a not really"[..], b"hello", b""] {
            assert!(matches!(square(bytes), Err(Error::BadRequest(_))));
        }
    }

    #[test]
    fn a_picture_claiming_a_huge_size_is_refused_before_decoding() -> Result<()> {
        // A PNG header for 9000 x 9000 pixels, with no pixels behind it.
        let mut png = picture(1, 1, image::ImageFormat::Png)?;
        png[16..20].copy_from_slice(&9000u32.to_be_bytes());
        png[20..24].copy_from_slice(&9000u32.to_be_bytes());
        assert!(matches!(square(&png), Err(Error::BadRequest(_))));
        Ok(())
    }
}
