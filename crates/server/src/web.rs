//! The web UI (`web/`), compiled into release builds and read from disk in debug builds, so
//! `mise run dev` picks up UI edits without a rebuild. Served with ETags; nothing is cached
//! without revalidation, because file names are not content-hashed.

use axum::http::{StatusCode, Uri, header};
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

pub async fn serve(uri: Uri, headers: axum::http::HeaderMap) -> Response {
    let path = uri.path().trim_start_matches('/');
    if path == "api" || path.starts_with("api/") {
        return lull_core::Error::NotFound(format!("no such endpoint: /{path}")).into_response();
    }
    let path = if path.is_empty() { "index.html" } else { path };
    let Some(file) = Web::get(path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let etag = format!("\"{}\"", hex(&file.metadata.sha256_hash()[..16]));
    if headers.get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok()) == Some(etag.as_str()) {
        return StatusCode::NOT_MODIFIED.into_response();
    }
    (
        [
            (header::CONTENT_TYPE, file.metadata.mimetype().to_owned()),
            (header::ETAG, etag),
            (header::CACHE_CONTROL, "no-cache".to_owned()),
            (header::CONTENT_SECURITY_POLICY, CSP.to_owned()),
        ],
        file.data,
    )
        .into_response()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
