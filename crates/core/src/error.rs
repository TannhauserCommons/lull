//! The one error type crates return from services and handlers. It maps to an HTTP status and
//! a JSON body `{ "error": kind, "message": text }` that the UI shows as it is.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    Conflict(String),
    /// The access token is missing or wrong.
    #[error("access token required")]
    Unauthorized,
    /// A dependency is down or not configured (Jellyfin, slskd, Last.fm...).
    #[error("{service} is unavailable: {reason}")]
    Unavailable { service: &'static str, reason: String },
    /// A dependency answered, but with something unusable.
    #[error("{service} failed: {reason}")]
    Upstream { service: &'static str, reason: String },
    #[error("{0}")]
    Internal(String),
}

impl Error {
    pub fn internal(msg: impl Into<String>) -> Self {
        Error::Internal(msg.into())
    }
    pub fn unavailable(service: &'static str, reason: impl Into<String>) -> Self {
        Error::Unavailable {
            service,
            reason: reason.into(),
        }
    }
    pub fn upstream(service: &'static str, reason: impl Into<String>) -> Self {
        Error::Upstream {
            service,
            reason: reason.into(),
        }
    }

    fn parts(&self) -> (StatusCode, &'static str) {
        match self {
            Error::NotFound(_) => (StatusCode::NOT_FOUND, "not_found"),
            Error::BadRequest(_) => (StatusCode::BAD_REQUEST, "bad_request"),
            Error::Conflict(_) => (StatusCode::CONFLICT, "conflict"),
            Error::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
            Error::Unavailable { .. } => (StatusCode::SERVICE_UNAVAILABLE, "unavailable"),
            Error::Upstream { .. } => (StatusCode::BAD_GATEWAY, "upstream"),
            Error::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal"),
        }
    }
}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Error::Internal(format!("database: {e}"))
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let (status, kind) = self.parts();
        if status.is_server_error() {
            tracing::warn!(error = %self, kind, "request failed");
        }
        (status, Json(json!({ "error": kind, "message": self.to_string() }))).into_response()
    }
}
