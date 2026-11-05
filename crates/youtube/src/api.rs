//! HTTP handlers; shapes are in docs/context/api.md (YouTube section).

use std::sync::Arc;

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use lull_core::{Error, Result};
use serde_json::{Value, json};

use crate::Service;

pub fn router(svc: Arc<Service>) -> Router {
    Router::new()
        .route("/api/youtube", get(read))
        .route("/api/youtube/update", post(update_now))
        .with_state(svc)
}

/// The `youtube` status and the yt-dlp in use, for Settings › Sources.
async fn read(State(svc): State<Arc<Service>>) -> Json<Value> {
    Json(body(&svc))
}

/// Checks for a newer yt-dlp now ("Check now"); only while Keep yt-dlp up to date is on.
async fn update_now(State(svc): State<Arc<Service>>) -> Result<Json<Value>> {
    if !svc.settings().auto_update {
        return Err(Error::Conflict(
            "Keep yt-dlp up to date is off. Turn it on to check for a newer one.".into(),
        ));
    }
    crate::update::check(&svc).await;
    Ok(Json(body(&svc)))
}

fn body(svc: &Service) -> Value {
    let (state, detail) = svc.state();
    let bin = svc.ytdlp.current();
    let checked = svc.updates.checked();
    json!({
        "state": state,
        "detail": detail,
        "mode": svc.settings().mode,
        "ytdlp": {
            "version": bin.version,
            "updated": bin.updated,
            "pinnedVersion": svc.ytdlp.pinned_version(),
            "latest": checked.latest,
            "checkedAt": checked.checked_at,
            "error": checked.error,
        },
        "backoffUntil": svc.backoff_until(),
        "soundChecks": {
            "counts": svc.checks.counts(),
            "idle": svc.checks.counts().idle(),
            "acoustid": svc.ctx.config.youtube.acoustid_key.is_some(),
        },
    })
}
