//! Text answers go out compressed (brotli, else gzip) when the client accepts it: JSON shrinks
//! about six times, which is most of a page's wait on a phone over Tailscale. The web UI's own
//! files are compressed once at build time (`web.rs`) and pass through untouched, as does
//! anything that is not text.

use axum::http::{Extensions, HeaderMap, StatusCode, Version, header};
use tower_http::compression::predicate::{Predicate, SizeAbove};
use tower_http::compression::{CompressionLayer, CompressionLevel};

/// Smaller answers fit in one packet anyway.
const MIN_BYTES: u16 = 1024;
/// Brotli 5 and gzip 5: a 30 KB answer takes well under a millisecond, for nearly the size the
/// slowest levels give.
const LEVEL: i32 = 5;

pub fn layer() -> CompressionLayer<impl Predicate> {
    CompressionLayer::new()
        .quality(CompressionLevel::Precise(LEVEL))
        .compress_when(SizeAbove::new(MIN_BYTES).and(text))
}

/// Only text is worth it. Audio and pictures are compressed already, ranged audio must keep its
/// byte offsets, and the event stream must reach the browser event by event.
fn text(_: StatusCode, _: Version, headers: &HeaderMap, _: &Extensions) -> bool {
    let Some(value) = headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    let mime = value.split(';').next().unwrap_or_default().trim().to_ascii_lowercase();
    if mime == "text/event-stream" {
        return false;
    }
    mime.starts_with("text/")
        || mime.ends_with("+json")
        || mime.ends_with("+xml")
        || matches!(
            mime.as_str(),
            "application/json" | "application/xml" | "application/javascript"
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn of(content_type: &'static str) -> bool {
        let mut h = HeaderMap::new();
        h.insert(header::CONTENT_TYPE, header::HeaderValue::from_static(content_type));
        text(StatusCode::OK, Version::HTTP_11, &h, &Extensions::new())
    }

    #[test]
    fn only_text_is_compressed_never_audio_pictures_or_the_event_stream() {
        assert!(of("application/json"));
        assert!(of("application/xml; charset=utf-8"), "Subsonic");
        assert!(of("text/html; charset=utf-8"));
        assert!(of("image/svg+xml"));
        assert!(!of("text/event-stream"));
        assert!(!of("audio/flac"));
        assert!(!of("audio/x-mpegurl; charset=utf-8"));
        assert!(!of("image/jpeg"));
        assert!(!of("application/zip"));
        assert!(!of("application/octet-stream"));
    }
}
