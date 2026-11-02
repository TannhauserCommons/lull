//! Conditional GETs for every route: an answer that carries an `ETag` the client already holds
//! (`If-None-Match`) goes out as a bodyless 304 with the same caching headers. Routes only set
//! the tag (`lull_core::etag`); a cover revalidated after its day of caching then costs a few
//! hundred bytes instead of up to 70 KB.

use axum::extract::Request;
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

pub async fn not_modified(req: Request, next: Next) -> Response {
    let held = req.headers().get(header::IF_NONE_MATCH).cloned();
    let response = next.run(req).await;
    let Some(held) = held.as_ref().and_then(|v| v.to_str().ok()) else {
        return response;
    };
    if response.status() != StatusCode::OK || !matches(held, response.headers()) {
        return response;
    }
    let mut kept = HeaderMap::new();
    for name in [
        header::ETAG,
        header::CACHE_CONTROL,
        header::VARY,
        header::CONTENT_LOCATION,
    ] {
        if let Some(v) = response.headers().get(&name) {
            kept.insert(name, v.clone());
        }
    }
    (StatusCode::NOT_MODIFIED, kept).into_response()
}

/// Whether one of the tags in `If-None-Match` (or `*`) is the answer's `ETag`.
fn matches(held: &str, headers: &HeaderMap) -> bool {
    let Some(etag) = headers.get(header::ETAG).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    held.split(',')
        .map(str::trim)
        .any(|tag| tag == "*" || tag == etag || tag.strip_prefix("W/") == Some(etag))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_held_tag_matches_among_several_and_weak_forms() {
        let mut h = HeaderMap::new();
        h.insert(header::ETAG, header::HeaderValue::from_static("\"ab-10\""));
        assert!(matches("\"zz\", \"ab-10\"", &h));
        assert!(matches("W/\"ab-10\"", &h));
        assert!(!matches("\"ab-11\"", &h));
        assert!(!matches("\"ab-10\"", &HeaderMap::new()), "no tag, nothing to match");
    }
}
