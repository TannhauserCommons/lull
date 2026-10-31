//! HTTP handlers; shapes are in docs/context/api.md (Soulseek section).

use std::sync::Arc;
use std::sync::atomic::Ordering;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use lull_core::ids::TrackId;
use lull_core::model::SourceRef;
use lull_core::{Error, Result};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Service;
use crate::search::Started;

pub fn router(svc: Arc<Service>) -> Router {
    Router::new()
        .route("/api/search", get(search))
        .route("/api/tracks/{id}/sources", get(sources))
        .route("/api/stream/{id}", get(stream).delete(stop))
        .route("/api/transfers", get(transfers))
        .route("/api/transfers/{job}", delete(cancel))
        .route("/api/transfers/{job}/retry", post(retry))
        .with_state(svc)
}

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
}

async fn search(State(svc): State<Arc<Service>>, Query(q): Query<SearchQuery>) -> Result<Json<Started>> {
    let online = svc.online.load(Ordering::Relaxed);
    Ok(Json(svc.searches.start(&q.q, online).await?))
}

/// Ranked sources. A song nothing is known about yet starts a search for it; the answer then
/// says `done: false` and the picker asks again.
async fn sources(State(svc): State<Arc<Service>>, Path(id): Path<String>) -> Result<Json<Value>> {
    let track = svc.track(&TrackId(id)).await?;
    let ranked = svc.sources.ranked(&track.id);
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

/// A picked source must be one a search reported: its size comes from there.
fn known_source(svc: &Service, track: &TrackId, peer: String, path: String) -> Result<SourceRef> {
    svc.sources
        .ranked(track)
        .into_iter()
        .find(|r| r.peer == peer && r.path == path)
        .map(|r| r.source.source_ref())
        .ok_or_else(|| Error::NotFound(format!("{peer} is not a known source for this song; search again")))
}

async fn stop(State(svc): State<Arc<Service>>, Path(id): Path<String>) -> StatusCode {
    svc.fetcher.stop_streaming(&TrackId(id));
    StatusCode::NO_CONTENT
}

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
