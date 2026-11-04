//! The sizes covers and artist photos are served at (`/api/art`, `/api/catalog/art`), and the
//! rules every picture route keeps: a picture is scaled down to fit the size asked for and never
//! up, and the answer says how large it really is (`X-Image-Size`), so a client that asked for
//! more than the source has stops asking for larger sizes.
//!
//! A picture the hub doesn't have yet answers 404 "not yet" (never cached), and `art.ready`
//! tells the pages once it has it, so a page asks for a picture once and waits instead of
//! polling. A picture no source has answers 404 "none", cached for as long as that is believed.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Cursor;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::Json;
use axum::http::{HeaderName, StatusCode, header};
use axum::response::{IntoResponse, Response};

use tokio_util::sync::CancellationToken;

use crate::events::Bus;
use crate::{Error, Result};

/// The sizes, in pixels, a picture fits into (both sides). 96 to 512 for rows, tiles and heroes;
/// 1024 and 2048 for large views on high-density screens.
pub const SIZES: [u32; 5] = [96, 256, 512, 1024, 2048];

/// `X-Image-Size: <width>x<height>`, the picture's real size in pixels. Smaller than the size
/// asked for when the source has no more.
pub const IMAGE_SIZE: HeaderName = HeaderName::from_static("x-image-size");

/// The Cover Art Archive's own sizes (its thumbnails); its original is larger still.
const ARCHIVE_SIZES: [u32; 3] = [250, 500, 1200];

/// 400 unless `size` is one of `SIZES`.
pub fn check(size: u32) -> Result<()> {
    if SIZES.contains(&size) {
        Ok(())
    } else {
        Err(Error::BadRequest("size must be 96, 256, 512, 1024 or 2048".into()))
    }
}

/// The Cover Art Archive thumbnail to ask for `size`: the smallest that covers it (250 counts for
/// 256), `None` for the original when none does.
pub fn archive_size(size: u32) -> Option<u32> {
    ARCHIVE_SIZES.into_iter().find(|s| s * 100 >= size * 95)
}

/// The picture scaled down to fit `size` × `size` (JPEG), or as it is when it already fits (never
/// scaled up) or can't be read.
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

/// Width and height from the picture's header, without decoding it.
pub fn dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()
}

/// A picture as an answer: its type, a day's private caching, an `ETag` (the hub answers
/// `If-None-Match` with a 304) and `X-Image-Size` when its header can be read.
pub fn response(bytes: Vec<u8>) -> Response {
    let mut headers = vec![
        (header::CONTENT_TYPE, content_type(&bytes).to_owned()),
        (header::CACHE_CONTROL, "private, max-age=86400".to_owned()),
        (header::ETAG, crate::etag::of(&bytes)),
    ];
    if let Some((w, h)) = dimensions(&bytes) {
        headers.push((IMAGE_SIZE, format!("{w}x{h}")));
    }
    let mut response = bytes.into_response();
    for (name, value) in headers {
        if let Ok(value) = value.parse() {
            response.headers_mut().insert(name, value);
        }
    }
    response
}

/// 404 while the picture is on its way: never cached, so the request after `art.ready` gets it.
pub fn not_yet(key: &str) -> Response {
    missing("no-store".into(), format!("The picture for {key} is still on its way."))
}

/// 404 for a picture no source has, cached by the browser for as long as the hub believes it.
pub fn none(key: &str, believed_for: Duration) -> Response {
    let cache = format!("private, max-age={}", believed_for.as_secs());
    missing(cache, format!("There is no picture for {key}."))
}

fn missing(cache: String, message: String) -> Response {
    let mut response = (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": "not_found", "message": message })),
    )
        .into_response();
    if let Ok(cache) = cache.parse() {
        response.headers_mut().insert(header::CACHE_CONTROL, cache);
    }
    response
}

/// How long `art.ready` collects keys before it goes out: a page of covers is one event.
const READY_BATCH: Duration = Duration::from_millis(250);

/// Publishes `art.ready { keys, images? }` (album and artist ids whose picture the hub now has,
/// at some size; `images`: an artist's new photo URL, for a page that drew it without one),
/// batched so a page of covers coming in is one event, not fifty.
pub struct Ready {
    bus: Bus,
    shutdown: CancellationToken,
    batch: Mutex<Batch>,
}

#[derive(Default)]
struct Batch {
    keys: BTreeSet<String>,
    images: BTreeMap<String, String>,
}

impl Ready {
    pub fn new(bus: Bus, shutdown: CancellationToken) -> Arc<Ready> {
        Arc::new(Ready {
            bus,
            shutdown,
            batch: Mutex::default(),
        })
    }

    pub fn announce(self: &Arc<Self>, key: impl Into<String>) {
        self.add(|b| {
            b.keys.insert(key.into());
        });
    }

    /// `key` has a new picture at `image` (a URL the pages use as the entity's `image`).
    pub fn announce_image(self: &Arc<Self>, key: impl Into<String>, image: String) {
        let key = key.into();
        self.add(|b| {
            b.keys.insert(key.clone());
            b.images.insert(key, image);
        });
    }

    fn add(self: &Arc<Self>, put: impl FnOnce(&mut Batch)) {
        let first = {
            let mut batch = self.batch.lock().unwrap_or_else(PoisonError::into_inner);
            let was_empty = batch.keys.is_empty();
            put(&mut batch);
            was_empty
        };
        if first {
            let me = self.clone();
            let shutdown = self.shutdown.clone();
            tokio::spawn(shutdown.run_until_cancelled_owned(async move {
                tokio::time::sleep(READY_BATCH).await;
                let batch = std::mem::take(&mut *me.batch.lock().unwrap_or_else(PoisonError::into_inner));
                let mut data = serde_json::json!({ "keys": batch.keys });
                if !batch.images.is_empty() {
                    data["images"] = serde_json::json!(batch.images);
                }
                me.bus.publish("art.ready", data);
            }));
        }
    }
}

pub fn content_type(bytes: &[u8]) -> &'static str {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => "image/png",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => "image/webp",
        [b'G', b'I', b'F', ..] => "image/gif",
        _ => "image/jpeg",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jpeg(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        let image = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(w, h, image::Rgb([200, 120, 40])));
        let written = image.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Jpeg);
        assert!(written.is_ok());
        out
    }

    #[test]
    fn a_picture_is_scaled_down_to_fit_and_never_up() {
        // Live, size=96 came back as the archive's 188x250 original.
        assert_eq!(dimensions(&fit(jpeg(188, 250), 96)), Some((72, 96)));
        // A 600 px source asked for at 2048 stays 600 px: the client learns that from the header.
        let small = jpeg(600, 600);
        assert_eq!(fit(small.clone(), 2048), small);
        let unreadable = vec![1, 2, 3];
        assert_eq!(
            fit(unreadable.clone(), 96),
            unreadable,
            "what can't be read is served as it is"
        );
    }

    #[test]
    fn the_answer_says_the_pictures_real_size() {
        let answer = response(jpeg(600, 400));
        assert_eq!(
            answer.headers().get(IMAGE_SIZE).and_then(|v| v.to_str().ok()),
            Some("600x400")
        );
        assert!(answer.headers().contains_key(header::ETAG));
    }

    #[test]
    fn the_archive_is_asked_for_the_smallest_size_that_covers_the_request() {
        assert_eq!(archive_size(96), Some(250));
        assert_eq!(archive_size(256), Some(250));
        assert_eq!(archive_size(512), Some(500));
        assert_eq!(archive_size(1024), Some(1200));
        assert_eq!(archive_size(2048), None, "the original");
    }

    #[tokio::test]
    async fn covers_coming_in_together_are_one_art_ready_event() -> Result<()> {
        let bus = Bus::new();
        let mut rx = bus.subscribe();
        let ready = Ready::new(bus.clone(), CancellationToken::new());
        for key in ["al-0b", "al-0a", "al-0b", "ar-0c"] {
            ready.announce(key);
        }
        ready.announce_image("ar-0d", "api/catalog/art/ar-0d?v=1".into());
        let event = rx.recv().await.map_err(|e| Error::internal(e.to_string()))?;
        assert_eq!(event.topic, "art.ready");
        assert_eq!(
            event.data,
            serde_json::json!({
                "keys": ["al-0a", "al-0b", "ar-0c", "ar-0d"],
                "images": { "ar-0d": "api/catalog/art/ar-0d?v=1" },
            })
        );
        assert!(rx.try_recv().is_err(), "one event for the batch");
        Ok(())
    }

    #[test]
    fn only_the_served_sizes_are_accepted() {
        for size in SIZES {
            assert!(check(size).is_ok());
        }
        assert!(matches!(check(600), Err(Error::BadRequest(_))));
    }
}
