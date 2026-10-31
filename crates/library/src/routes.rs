//! HTTP handlers: the Library section of docs/context/api.md. Input is checked here; the
//! modules behind them trust it.

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use lull_core::ids::{AlbumId, TrackId};
use lull_core::model::{Entities, SourceRef};
use lull_core::status::State as ServiceState;
use lull_core::{Error, Result, catalog};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::items::Quality;
use crate::jobs::{Duplicate, RescanJob, SaveJob, Status};
use crate::{Service, art, save, sync};

type Svc = State<Arc<Service>>;

pub fn router(svc: Arc<Service>) -> Router {
    Router::new()
        .route("/api/tracks/{id}", get(track))
        .route("/api/library/albums", get(albums))
        .route("/api/library/artists", get(artists))
        .route("/api/library/songs", get(songs))
        .route("/api/library/genres", get(genres))
        .route("/api/library/stream/{id}", get(stream))
        .route("/api/library/rescan", post(rescan))
        .route("/api/library/duplicates", post(duplicates))
        .route("/api/art/{id}", get(cover))
        .route("/api/saves", post(start_saves))
        .route("/api/saves/{id}/releases", get(releases))
        .route("/api/saves/{id}/release", post(choose_release))
        .route("/api/saves/{id}/retry", post(retry))
        .route("/api/jobs", get(jobs).delete(clear_jobs))
        .with_state(svc)
}

// ---------- tracks and lists ----------

async fn track(State(svc): Svc, Path(id): Path<String>) -> Result<Json<Value>> {
    let id = track_id(id)?;
    let saved = svc.index().track(&id);
    let track = match saved {
        Some(t) => t,
        None => {
            let wanted = id.clone();
            let known = svc.ctx.db.call(move |c| catalog::track(c, &wanted)).await?;
            // The catalog may remember a song as saved that has since left the library.
            let mut t = known.ok_or_else(|| Error::NotFound(format!("no track {id}")))?;
            t.saved = false;
            t
        }
    };
    let mut entities = Entities::default();
    if let Some(album) = svc.index().album(&track.album_id) {
        entities.albums.insert(album.album.id.clone(), album.album.clone());
    }
    entities.tracks.insert(track.id.clone(), track.clone());
    Ok(Json(json!({ "track": track, "entities": entities })))
}

#[derive(Deserialize)]
struct ListQuery {
    #[serde(default)]
    sort: String,
    #[serde(default)]
    filter: String,
    limit: Option<usize>,
    #[serde(default)]
    offset: usize,
}

const DEFAULT_LIMIT: usize = 200;
const MAX_LIMIT: usize = 5000;

impl ListQuery {
    fn page<T>(&self, items: Vec<T>) -> Result<(Vec<T>, usize)> {
        let limit = self.limit.unwrap_or(DEFAULT_LIMIT);
        if limit == 0 || limit > MAX_LIMIT {
            return Err(Error::BadRequest(format!("limit must be 1 to {MAX_LIMIT}")));
        }
        let total = items.len();
        Ok((items.into_iter().skip(self.offset).take(limit).collect(), total))
    }
}

/// `{ items, total, entities, stale }`: `stale` while Jellyfin is down and the list is the last
/// one read.
fn list(svc: &Service, ids: Vec<Value>, total: usize, entities: Entities) -> Result<Json<Value>> {
    svc.jellyfin()?;
    let stale = svc
        .ctx
        .status
        .snapshot()
        .get("jellyfin")
        .is_none_or(|s| s.state != ServiceState::Ok);
    Ok(Json(
        json!({ "items": ids, "total": total, "entities": entities, "stale": stale }),
    ))
}

async fn songs(State(svc): Svc, Query(q): Query<ListQuery>) -> Result<Json<Value>> {
    let (page, total, entities) = {
        let index = svc.index();
        let (page, total) = q.page(index.songs(&q.sort, &q.filter)?)?;
        let mut entities = Entities::default();
        for t in &page {
            entities.tracks.insert(t.id.clone(), (*t).clone());
        }
        (page.iter().map(|t| json!(t.id)).collect(), total, entities)
    };
    list(&svc, page, total, entities)
}

async fn albums(State(svc): Svc, Query(q): Query<ListQuery>) -> Result<Json<Value>> {
    let (page, total, entities) = {
        let index = svc.index();
        let (page, total) = q.page(index.albums(&q.sort, &q.filter)?)?;
        let mut entities = Entities::default();
        for a in &page {
            entities.albums.insert(a.album.id.clone(), a.album.clone());
        }
        (page.iter().map(|a| json!(a.album.id)).collect(), total, entities)
    };
    list(&svc, page, total, entities)
}

async fn artists(State(svc): Svc, Query(q): Query<ListQuery>) -> Result<Json<Value>> {
    let (page, total, entities) = {
        let index = svc.index();
        let (page, total) = q.page(index.artists(&q.sort, &q.filter)?)?;
        let mut entities = Entities::default();
        for a in &page {
            entities.artists.insert(a.id.clone(), (*a).clone());
        }
        (page.iter().map(|a| json!(a.id)).collect(), total, entities)
    };
    list(&svc, page, total, entities)
}

async fn genres(State(svc): Svc, Query(q): Query<ListQuery>) -> Result<Json<Value>> {
    let (page, total, entities) = {
        let index = svc.index();
        let (page, total) = q.page(index.genres(&q.sort, &q.filter)?)?;
        let mut entities = Entities::default();
        for id in page.iter().flat_map(|g| &g.track_ids) {
            if let Some(t) = index.track(id) {
                entities.tracks.insert(id.clone(), t);
            }
        }
        (page.iter().map(|g| json!(g)).collect(), total, entities)
    };
    list(&svc, page, total, entities)
}

// ---------- audio and art ----------

/// Saved-track audio from Jellyfin, Range passed through and the body streamed as it comes.
async fn stream(State(svc): Svc, Path(id): Path<String>, headers: HeaderMap) -> Result<Response> {
    let id = track_id(id)?;
    let item = svc
        .index()
        .saved(&id)
        .map(|s| s.item_id.clone())
        .ok_or_else(|| Error::NotFound(format!("{id} is not saved")))?;
    let upstream = svc
        .jellyfin()?
        .stream(&item, headers.get(header::RANGE).cloned())
        .await?;
    let status = upstream.status();
    if !(status.is_success() || status == StatusCode::RANGE_NOT_SATISFIABLE) {
        return Err(Error::upstream("jellyfin", format!("answered {status} for the audio")));
    }
    let mut response = Response::builder().status(status);
    for name in [
        header::CONTENT_TYPE,
        header::CONTENT_LENGTH,
        header::CONTENT_RANGE,
        header::ACCEPT_RANGES,
        header::LAST_MODIFIED,
        header::ETAG,
    ] {
        if let Some(value) = upstream.headers().get(&name) {
            response = response.header(name, value.clone());
        }
    }
    response
        .body(Body::from_stream(upstream.bytes_stream()))
        .map_err(|e| Error::internal(format!("stream response: {e}")))
}

#[derive(Deserialize)]
struct ArtQuery {
    size: Option<u32>,
}

async fn cover(State(svc): Svc, Path(id): Path<String>, Query(q): Query<ArtQuery>) -> Result<Response> {
    let image = art::cover(&svc, &AlbumId(id), q.size.unwrap_or(256)).await?;
    Ok((
        [
            (header::CONTENT_TYPE, image.content_type),
            (header::CACHE_CONTROL, "private, max-age=86400"),
        ],
        image.bytes,
    )
        .into_response())
}

// ---------- rescan and duplicates ----------

async fn rescan(State(svc): Svc) -> Result<(StatusCode, Json<Value>)> {
    svc.jellyfin()?;
    let job = RescanJob::new();
    svc.jobs.put_rescan(&job).await;
    let id = job.id.clone();
    tokio::spawn(sync::rescan(svc.clone(), job));
    Ok((StatusCode::ACCEPTED, Json(json!({ "jobId": id }))))
}

/// A Soulseek file as the UI names it (`GET /api/tracks/{id}/sources` rows).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SourceIn {
    peer: String,
    path: String,
    #[serde(alias = "sizeBytes")]
    size: u64,
}

impl SourceIn {
    fn into_ref(self) -> Result<SourceRef> {
        if self.peer.is_empty() || self.path.is_empty() || self.size == 0 {
            return Err(Error::BadRequest("a source needs peer, path and size".into()));
        }
        Ok(SourceRef {
            user: self.peer,
            path: self.path,
            size: self.size,
        })
    }
}

#[derive(Deserialize)]
struct DuplicatesBody {
    ids: Vec<String>,
    #[serde(default)]
    source: HashMap<String, SourceIn>,
}

/// For the duplicate dialog: which of `ids` are saved already, and whether the copy on offer
/// is clearly better than the saved one.
async fn duplicates(State(svc): Svc, Json(body): Json<DuplicatesBody>) -> Result<Json<Value>> {
    let index = svc.index();
    let mut items = Vec::new();
    for raw in body.ids {
        let id = track_id(raw)?;
        let Some(existing) = index.track(&id) else { continue };
        let better = body.source.get(id.as_str()).is_some_and(|s| {
            Quality::of_file(&s.path, s.size, existing.duration).beats(&Quality::of(existing.file.as_ref()))
        });
        items.push(json!({ "trackId": id, "existing": existing, "better": better }));
    }
    Ok(Json(json!({ "items": items })))
}

// ---------- saves ----------

#[derive(Deserialize)]
struct SavesBody {
    ids: Vec<String>,
    #[serde(default)]
    source: HashMap<String, SourceIn>,
    /// The duplicate dialog's answer per already-saved id; without one, a saved id is skipped.
    #[serde(default)]
    duplicate: HashMap<String, Duplicate>,
}

async fn start_saves(State(svc): Svc, Json(mut body): Json<SavesBody>) -> Result<Json<Value>> {
    svc.jellyfin()?;
    if body.ids.is_empty() {
        return Err(Error::BadRequest("ids is empty".into()));
    }
    let mut jobs = Vec::new();
    let mut skipped = Vec::new();
    for raw in body.ids {
        let id = track_id(raw)?;
        let source = body.source.remove(id.as_str()).map(SourceIn::into_ref).transpose()?;
        if let Some(running) = svc.jobs.active_for(&id) {
            jobs.push(running.id);
            continue;
        }
        let saved = svc.index().saved(&id).is_some();
        let duplicate = body.duplicate.get(id.as_str()).copied().filter(|_| saved);
        if saved && duplicate.is_none() {
            skipped.push(id);
            continue;
        }
        for old in svc.jobs.saves().into_iter().filter(|j| j.track_id == id) {
            svc.jobs.remove(&old.id).await;
        }
        let job = SaveJob::new(id, source, duplicate);
        jobs.push(job.id.clone());
        svc.jobs.put(&job).await;
        save::spawn(&svc, job);
    }
    Ok(Json(json!({ "jobs": jobs, "skipped": skipped })))
}

fn save_job(svc: &Service, id: &str) -> Result<SaveJob> {
    svc.jobs
        .get(id)
        .ok_or_else(|| Error::NotFound(format!("no save job {id}")))
}

/// The release picker: candidates best first, and what lull knows about the file.
async fn releases(State(svc): Svc, Path(id): Path<String>) -> Result<Json<Value>> {
    let job = save_job(&svc, &id)?;
    let wanted = job.track_id.clone();
    let track = svc.ctx.db.call(move |c| catalog::track(c, &wanted)).await?;
    Ok(Json(json!({ "items": job.candidates, "track": track })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReleaseBody {
    mbid: Option<String>,
    #[serde(default)]
    keep_tags: bool,
}

async fn choose_release(State(svc): Svc, Path(id): Path<String>, Json(body): Json<ReleaseBody>) -> Result<Json<Value>> {
    let job = save_job(&svc, &id)?;
    let mbid = match (body.mbid, body.keep_tags) {
        (Some(mbid), false) => Some(mbid),
        (None, true) => None,
        _ => return Err(Error::BadRequest("send either mbid or keepTags: true".into())),
    };
    let job = save::choose(&svc, job, mbid).await?;
    Ok(Json(json!({ "jobId": job.id })))
}

#[derive(Deserialize)]
struct RetryBody {
    source: Option<SourceIn>,
}

/// Retries from the failed step. A body `{ source }` changes the Soulseek source first.
async fn retry(State(svc): Svc, Path(id): Path<String>, body: Bytes) -> Result<Json<Value>> {
    let mut job = save_job(&svc, &id)?;
    if !body.is_empty() {
        let body: RetryBody =
            serde_json::from_slice(&body).map_err(|e| Error::BadRequest(format!("retry body: {e}")))?;
        if let Some(source) = body.source {
            job.source = Some(source.into_ref()?);
        }
    }
    let job = save::retry(&svc, job).await?;
    Ok(Json(
        json!({ "jobId": job.id, "step": job.step, "status": Status::Running }),
    ))
}

// ---------- jobs ----------

async fn jobs(State(svc): Svc) -> Json<Value> {
    Json(json!({ "items": svc.jobs.rows() }))
}

async fn clear_jobs(State(svc): Svc) -> StatusCode {
    svc.jobs.clear_finished().await;
    StatusCode::NO_CONTENT
}

fn track_id(raw: String) -> Result<TrackId> {
    if raw.starts_with(TrackId::PREFIX) && raw.len() > TrackId::PREFIX.len() {
        Ok(TrackId(raw))
    } else {
        Err(Error::BadRequest(format!("{raw:?} is not a track id")))
    }
}
