//! HTTP handlers; shapes are in docs/context/api.md (Soulseek section).

use std::sync::Arc;
use std::sync::atomic::Ordering;

use axum::Extension;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use lull_core::devices::Device;
use lull_core::ids::TrackId;
use lull_core::model::SourceRef;
use lull_core::{Error, Result};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Service;
use crate::rules::Purpose;
use crate::search::Started;

pub fn router(svc: Arc<Service>) -> Router {
    routes().with_state(svc)
}

fn routes() -> Router<Arc<Service>> {
    Router::new()
        .route("/api/soulseek/search", get(search))
        .route("/api/tracks/{id}/sources", get(sources))
        .route("/api/stream/prefetch", put(prefetch))
        .route("/api/stream/{id}", get(stream).delete(stop))
        .route("/api/stream/{id}/copy", get(copy))
        .route("/api/stream/{id}/switch", post(switch))
        .route("/api/soulseek/diagnostics", get(diagnostics))
        .route("/api/transfers", get(transfers))
        .route("/api/transfers/{job}", delete(cancel))
        .route("/api/transfers/{job}/retry", post(retry))
}

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
}

async fn search(State(svc): State<Arc<Service>>, Query(q): Query<SearchQuery>) -> Result<Json<Started>> {
    let online = svc.online.load(Ordering::Relaxed);
    Ok(Json(svc.searches.start(&q.q, online)?))
}

#[derive(Deserialize)]
struct For {
    #[serde(rename = "for")]
    purpose: Option<String>,
}

/// Sources in the order the streaming profile (or with `?for=save`, the saving one) takes
/// them, each marked whether it passes the minimum. A song nothing is known about yet starts
/// a search for it; the answer then says `done: false` and the picker asks again.
async fn sources(State(svc): State<Arc<Service>>, Path(id): Path<String>, Query(q): Query<For>) -> Result<Json<Value>> {
    let purpose = match q.purpose.as_deref() {
        None | Some("stream") => Purpose::Stream,
        Some("save") => Purpose::Save,
        Some(other) => return Err(Error::BadRequest(format!("for={other}: stream or save"))),
    };
    let track = svc.track(&TrackId(id)).await?;
    let ranked = svc
        .sources
        .ordered(&track.id, track.duration, &svc.rules.get(), purpose);
    let mut searching = svc.searches.searching_for(&track);
    if ranked.is_empty() && !searching && svc.online.load(Ordering::Relaxed) {
        let (searches, track) = (svc.searches.clone(), track.clone());
        tokio::spawn(async move {
            if let Err(e) = searches.resolve(&track).await {
                tracing::warn!(error = %e, "searching for a song's sources");
            }
        });
        searching = true;
    }
    Ok(Json(json!({ "sources": ranked, "done": !searching })))
}

#[derive(Deserialize)]
struct Pick {
    peer: Option<String>,
    path: Option<String>,
}

async fn stream(
    State(svc): State<Arc<Service>>,
    Path(id): Path<String>,
    Query(pick): Query<Pick>,
    headers: HeaderMap,
) -> Result<Response> {
    let track = svc.track(&TrackId(id)).await?;
    let pick = match (pick.peer, pick.path) {
        (Some(peer), Some(path)) => Some(known_source(&svc, &track.id, peer, path)?),
        (None, None) => None,
        _ => return Err(Error::BadRequest("peer and path go together".into())),
    };
    let fetch = svc.fetcher.stream(track, pick).await?;
    crate::stream::serve(fetch, &headers).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Copy {
    peer: String,
    path: String,
}

impl Copy {
    fn source_ref(self) -> SourceRef {
        SourceRef {
            user: self.peer,
            path: self.path,
            size: 0,
        }
    }
}

/// One copy's bytes while it plays or arrives beside the copy playing, changing nothing: the
/// player lines a better copy up with the song before it switches (`switch`).
async fn copy(
    State(svc): State<Arc<Service>>,
    Path(id): Path<String>,
    Query(copy): Query<Copy>,
    headers: HeaderMap,
) -> Result<Response> {
    let fetch = svc.fetcher.copy(&TrackId(id), &copy.source_ref())?;
    crate::stream::serve(fetch, &headers).await
}

/// The player switched to the better copy on its way: it becomes the song's stream, and the
/// copy before it stops once its response ends. 409 when that copy is no longer on its way.
async fn switch(
    State(svc): State<Arc<Service>>,
    Path(id): Path<String>,
    body: std::result::Result<Json<Copy>, JsonRejection>,
) -> Result<StatusCode> {
    let Json(copy) = body.map_err(|e| Error::BadRequest(e.body_text()))?;
    svc.fetcher.adopt(&TrackId(id), &copy.source_ref(), false)?;
    Ok(StatusCode::NO_CONTENT)
}

/// A picked source must be one a search reported: its size comes from there.
fn known_source(svc: &Service, track: &TrackId, peer: String, path: String) -> Result<SourceRef> {
    let wanted = SourceRef {
        user: peer,
        path,
        size: 0,
    };
    svc.sources.get(track, &wanted).map(|s| s.source_ref()).ok_or_else(|| {
        Error::NotFound(format!(
            "{} is not a known source for this song; search again",
            wanted.user
        ))
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Prefetch {
    tracks: Vec<TrackId>,
}

/// The calling device's songs to preload, in queue order (at most 3): replaces its list.
/// Requests without a device (the gate always names one) share a list.
async fn prefetch(
    State(svc): State<Arc<Service>>,
    device: Option<Extension<Device>>,
    body: std::result::Result<Json<Prefetch>, JsonRejection>,
) -> Result<StatusCode> {
    let Json(body) = body.map_err(|e| Error::BadRequest(e.body_text()))?;
    let device = device.map(|Extension(d)| d.id).unwrap_or_default();
    crate::prefetch::listed(&svc, device, body.tracks)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn stop(State(svc): State<Arc<Service>>, Path(id): Path<String>) -> StatusCode {
    svc.fetcher.stop_streaming(&TrackId(id));
    StatusCode::NO_CONTENT
}

/// What slskd itself sees, for when Soulseek misbehaves and slskd's own UI and logs are out of
/// reach: its connection and network state, how recent searches ended, and its latest log lines.
async fn diagnostics(State(svc): State<Arc<Service>>) -> Result<Json<Value>> {
    let slskd = svc.slskd.state().await?;
    let log = svc.slskd.recent_log().await?;
    let skip = log.len().saturating_sub(DIAGNOSTIC_LOG_LINES);
    Ok(Json(json!({
        "slskd": slskd,
        "searches": svc.searches.outcomes(),
        "log": log.into_iter().skip(skip).collect::<Vec<_>>(),
    })))
}

/// slskd keeps a few hundred lines; the latest ones tell the story.
const DIAGNOSTIC_LOG_LINES: usize = 80;

async fn transfers(State(svc): State<Arc<Service>>) -> Json<Value> {
    let items = svc.fetcher.rows();
    Json(json!({ "total": items.len(), "items": items }))
}

async fn cancel(State(svc): State<Arc<Service>>, Path(job): Path<String>) -> Result<StatusCode> {
    svc.fetcher.cancel(&job)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn retry(State(svc): State<Arc<Service>>, Path(job): Path<String>) -> Result<Json<Value>> {
    let job = svc.fetcher.retry(&job)?;
    Ok(Json(json!({ "jobId": job })))
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_preload_route_sits_beside_the_stream_route() {
        // axum refuses conflicting routes by panicking when they're added, at hub start-up.
        let _ = super::routes();
    }
}
