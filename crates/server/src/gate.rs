//! The pairing gate (ADR-0002). Every `/api` call needs a paired device's token, as the HttpOnly
//! cookie `lull_device` (browsers: EventSource and <audio> can't send headers) or
//! `Authorization: Bearer` (the app). Open: everything outside `/api` (the static UI, `/health`,
//! and `/rest`, which checks Subsonic app passwords itself), plus the calls that pair a device.
//! Loopback is never trusted: `tailscale serve` proxies to the hub from 127.0.0.1.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use lull_core::config::Config;
use lull_core::devices::{self, Kind};

use crate::admin;
use crate::hub::Hub;

pub const COOKIE: &str = "lull_device";

/// Browsers cap a cookie's life at 400 days, so `/api/ping` renews it (`require`).
const COOKIE_MAX_AGE_S: u32 = 400 * 24 * 60 * 60;

pub async fn require(State(hub): State<Arc<Hub>>, mut req: Request, next: Next) -> Response {
    if is_open(req.method(), req.uri().path()) {
        return next.run(req).await;
    }
    // lull's own commands, over the hub's admin socket (admin.rs).
    if req.extensions().get::<admin::Admin>().is_some() && admin::may(req.method(), req.uri().path()) {
        return next.run(req).await;
    }
    let Some((token, from_cookie)) = presented(req.headers()) else {
        return lull_core::Error::Unauthorized.into_response();
    };
    let token = token.to_owned();
    let device = match devices::verify(&hub.db, &token).await {
        // An app password opens `/rest` only.
        Ok(Some(d)) if d.kind != Kind::Subsonic => d,
        Ok(_) => return lull_core::Error::Unauthorized.into_response(),
        Err(e) => return e.into_response(),
    };
    let renewal = if from_cookie && req.uri().path() == "/api/ping" {
        cookie(&token, https(req.headers(), &hub.ctx().await.config))
    } else {
        None
    };
    req.extensions_mut().insert(device);
    let mut resp = next.run(req).await;
    if let Some(cookie) = renewal {
        resp.headers_mut().append(header::SET_COOKIE, cookie);
    }
    resp
}

fn is_open(method: &Method, path: &str) -> bool {
    let api = path == "/api" || path.starts_with("/api/");
    !api || matches!(
        (method, path),
        (&Method::GET, "/api/setup") | (&Method::POST, "/api/setup/claim" | "/api/pair")
    )
}

/// The token a request carries, and whether it came in the cookie. The header wins.
fn presented(headers: &HeaderMap) -> Option<(&str, bool)> {
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    if let Some(token) = bearer {
        return Some((token.trim(), false));
    }
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .find_map(|c| c.trim().strip_prefix(COOKIE)?.strip_prefix('='))
        .map(|token| (token, true))
}

/// The cookie that pairs a browser. Lax, not Strict: Last.fm's sign-in returns to
/// `/api/lastfm/callback` by a cross-site top-level GET, which must still carry it.
/// Cross-site POSTs never do.
pub fn cookie(token: &str, secure: bool) -> Option<HeaderValue> {
    let secure = if secure { "; Secure" } else { "" };
    HeaderValue::from_str(&format!(
        "{COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={COOKIE_MAX_AGE_S}{secure}"
    ))
    .ok()
}

/// Whether the browser reached the hub over HTTPS: the public URL says so, or the proxy in front
/// (`tailscale serve`, a reverse proxy) does.
pub fn https(headers: &HeaderMap, config: &Config) -> bool {
    config.public_url.as_deref().is_some_and(|u| u.starts_with("https://"))
        || headers
            .get("x-forwarded-proto")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|p| p.eq_ignore_ascii_case("https"))
}
