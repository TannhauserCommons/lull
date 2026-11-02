//! The web UI (`web/`), compiled into release builds and read from disk in debug builds, so
//! `mise run dev` picks up UI edits without a rebuild. Served with ETags; nothing is cached
//! without revalidation, because file names are not content-hashed.
//!
//! Release images embed `web/` as `scripts/web-dist` leaves it: minified, with a `.br` and a
//! `.gz` next to each text file that compresses. The variant the browser accepts is served as
//! is, so nothing is compressed per request. A checkout has no variants and serves plain files.

use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../../web/"]
#[exclude = "node_modules/*"]
#[exclude = "package.json"]
#[exclude = "package-lock.json"]
struct Web;

/// What a page may load (ADR-0004): everything from the hub, so a shared theme can't fetch from or
/// report to anywhere else. Inline scripts are the pre-paint ones in index.html and the desktop
/// wrapper; inline styles are style="" attributes and the layered `<style>@import` lines; images
/// also come from the Winamp Skin Museum's CDN (skin screenshots); Theme Studio frames the app itself.
/// Inside the lull app (ADR-0006), downloaded covers come over Tauri's asset protocol and the page
/// talks to the app over its IPC; in a browser those `*.localhost` sources reach nothing.
const CSP: &str = "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; \
    img-src 'self' data: blob: https://r2.webampskins.org https://asset.localhost; font-src 'self' data:; media-src 'self' blob:; \
    connect-src 'self' ipc: http://ipc.localhost; worker-src 'self'; manifest-src 'self'; frame-src 'self'; frame-ancestors 'self'; \
    object-src 'none'; base-uri 'self'; form-action 'self'";

/// `Content-Encoding` and the suffix of its pre-compressed file, in the order lull prefers them.
const ENCODINGS: [(&str, &str); 2] = [("br", ".br"), ("gzip", ".gz")];

pub async fn serve(uri: Uri, headers: HeaderMap) -> Response {
    let path = uri.path().trim_start_matches('/');
    if path == "api" || path.starts_with("api/") {
        return lull_core::Error::NotFound(format!("no such endpoint: /{path}")).into_response();
    }
    let path = if path.is_empty() { "index.html" } else { path };
    let Some(file) = Web::get(path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mimetype = file.metadata.mimetype().to_owned();
    let (file, encoding) = ENCODINGS
        .iter()
        .filter(|(coding, _)| accepts(&headers, coding))
        .find_map(|(coding, suffix)| Web::get(&format!("{path}{suffix}")).map(|f| (f, Some(*coding))))
        .unwrap_or((file, None));

    // Each variant has its own bytes, so its own ETag; caches keep them apart by Vary.
    let etag = format!("\"{}\"", hex(&file.metadata.sha256_hash()[..16]));
    let common = [
        (header::ETAG, etag.clone()),
        (header::CACHE_CONTROL, "no-cache".to_owned()),
        (header::VARY, "Accept-Encoding".to_owned()),
        (header::CONTENT_SECURITY_POLICY, CSP.to_owned()),
    ];
    if matches(&headers, &etag) {
        return (StatusCode::NOT_MODIFIED, common).into_response();
    }
    let mut response = (common, [(header::CONTENT_TYPE, mimetype)], file.data).into_response();
    if let Some(coding) = encoding {
        response
            .headers_mut()
            .insert(header::CONTENT_ENCODING, header::HeaderValue::from_static(coding));
    }
    response
}

/// Whether `Accept-Encoding` allows `coding`: listed, and not with `q=0`.
fn accepts(headers: &HeaderMap, coding: &str) -> bool {
    headers
        .get_all(header::ACCEPT_ENCODING)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .any(|item| {
            let mut parts = item.split(';');
            let name = parts.next().unwrap_or_default().trim();
            let q = parts
                .find_map(|p| p.trim().strip_prefix("q="))
                .map_or(Some(1.0), |q| q.trim().parse::<f32>().ok());
            name.eq_ignore_ascii_case(coding) && q.is_some_and(|q| q > 0.0)
        })
}

fn matches(headers: &HeaderMap, etag: &str) -> bool {
    headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|tag| tag.trim() == etag))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accept_encoding(value: &'static str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(header::ACCEPT_ENCODING, header::HeaderValue::from_static(value));
        h
    }

    #[test]
    fn a_coding_refused_with_q_zero_is_never_served() {
        let h = accept_encoding("br;q=0, gzip;q=0.5, identity");
        assert!(!accepts(&h, "br"));
        assert!(accepts(&h, "gzip"));
        assert!(accepts(&accept_encoding("gzip, deflate, BR"), "br"));
        assert!(!accepts(&accept_encoding("gzip"), "br"));
        assert!(!accepts(&HeaderMap::new(), "gzip"));
    }
}
