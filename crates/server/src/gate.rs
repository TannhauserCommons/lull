//! The optional shared access token (`LULL_ACCESS_TOKEN`). With it set, every `/api` call needs
//! the token as `Authorization: Bearer <token>` or the `lull_token` cookie that
//! `POST /api/session` sets (EventSource and <audio> can't send headers). `/health` and the
//! static UI stay open so the UI can show its token gate.

use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use lull_core::config::{Config, Secret};
use serde_json::Value;

const COOKIE: &str = "lull_token";

#[derive(Clone)]
pub struct Gate {
    token: Option<Secret>,
}

impl Gate {
    pub fn new(config: &Config) -> Self {
        Gate {
            token: config.access_token.clone(),
        }
    }

    fn accepts(&self, presented: &str) -> bool {
        self.token
            .as_ref()
            .is_none_or(|t| constant_eq(presented.as_bytes(), t.expose().as_bytes()))
    }

    fn allows(&self, headers: &HeaderMap) -> bool {
        self.token.is_none() || presented(headers).any(|t| self.accepts(t))
    }
}

pub async fn require(State(gate): State<Gate>, req: Request, next: Next) -> Response {
    if gate.allows(req.headers()) {
        next.run(req).await
    } else {
        lull_core::Error::Unauthorized.into_response()
    }
}

/// `POST /api/session { "token": "..." }`: checks the token and stores it as an HttpOnly cookie.
pub fn router(config: &Config) -> Router {
    let gate = Gate::new(config);
    let secure = config.public_url.as_deref().is_some_and(|u| u.starts_with("https://"));
    Router::new().route(
        "/api/session",
        post(move |Json(body): Json<Value>| async move {
            let token = body.get("token").and_then(Value::as_str).unwrap_or_default();
            if !gate.accepts(token) {
                return Err(lull_core::Error::Unauthorized);
            }
            let cookie = format!(
                "{COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age=31536000{}",
                if secure { "; Secure" } else { "" }
            );
            Ok(([(header::SET_COOKIE, cookie)], StatusCode::NO_CONTENT))
        }),
    )
}

fn presented(headers: &HeaderMap) -> impl Iterator<Item = &str> {
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    let cookies = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|c| c.trim().strip_prefix(COOKIE)?.strip_prefix('='));
    bearer.into_iter().chain(cookies)
}

fn constant_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}
