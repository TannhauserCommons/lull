//! HTTP handlers: the Library section of docs/context/api.md. Input is checked here; the
//! modules behind them trust it.

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::{Path, Query, State};
use axum::http::HeaderValue;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use lull_core::config::LibraryMode;
use lull_core::ids::{AlbumId, TrackId};
use lull_core::model::{Entities, SourceRef};
use lull_core::{Error, Result, catalog};
use serde::Deserialize;
use serde_json::{Value, json};

use lull_core::quality::Quality;

use crate::items::Location;
use crate::jobs::{Duplicate, RescanJob, SaveJob, Status};
use crate::{Service, art, connect, folder, save, switch, sync};

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
        .route("/api/library/source", get(source).put(set_source))
        .route("/api/jellyfin/discover", get(discover))
        .route("/api/jellyfin/quickconnect", post(quick_connect))
        .route("/api/jellyfin/quickconnect/{secret}", get(quick_connect_state))
        .route("/api/jellyfin/signin", post(sign_in))
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

/// `{ items, total, entities, stale }`: `stale` while Jellyfin is down (Jellyfin mode) and the
/// list is the last one read.
fn list(svc: &Service, ids: Vec<Value>, total: usize, entities: Entities) -> Result<Json<Value>> {
    let stale = svc.mode == LibraryMode::Jellyfin && !svc.online();
    Ok(Json(
        json!({ "items": ids, "total": total, "entities": entities, "stale": stale }),
    ))
}

/// Songs also carry `songs: { trackId: { addedAt, plays } }`, the list's own columns. Plays are
/// per-listener state, so they stay out of the track entities.
async fn songs(State(svc): Svc, Query(q): Query<ListQuery>) -> Result<Json<Value>> {
    let (page, total, entities, added, ids) = {
        let index = svc.index();
        let (page, total) = q.page(index.songs(&q.sort, &q.filter)?)?;
        let mut entities = Entities::default();
        for t in &page {
            entities.tracks.insert(t.id.clone(), (*t).clone());
        }
        let added: Vec<String> = page
            .iter()
            .map(|t| index.saved(&t.id).map(|s| s.added.clone()).unwrap_or_default())
            .collect();
        let ids = played_as(index.aliases(), page.iter().map(|t| &t.id));
        (
            page.iter().map(|t| json!(t.id)).collect::<Vec<_>>(),
            total,
            entities,
            added,
            ids,
        )
    };
    let plays = svc.ctx.db.call(move |c| play_counts(c, &ids)).await?;
    let columns: serde_json::Map<String, Value> = page
        .iter()
        .zip(added)
        .zip(plays)
        .filter_map(|((id, added_at), plays)| {
            Some((id.as_str()?.to_owned(), json!({ "addedAt": added_at, "plays": plays })))
        })
        .collect();
    let Json(mut body) = list(&svc, page, total, entities)?;
    body["songs"] = Value::Object(columns);
    Ok(Json(body))
}

/// Each song's id plus the old ids that alias it: plays recorded before a source switch still
/// count for the song.
fn played_as<'a>(aliases: &HashMap<TrackId, TrackId>, songs: impl Iterator<Item = &'a TrackId>) -> Vec<Vec<TrackId>> {
    let mut ids: Vec<Vec<TrackId>> = songs.map(|id| vec![id.clone()]).collect();
    let mut wanted: HashMap<TrackId, &mut Vec<TrackId>> = HashMap::new();
    for song in &mut ids {
        if let Some(own) = song.first().cloned() {
            wanted.insert(own, song);
        }
    }
    for (old, saved) in aliases {
        if let Some(song) = wanted.get_mut(saved) {
            song.push(old.clone());
        }
    }
    ids
}

/// Plays per song from core's `plays` table, in `ids` order.
fn play_counts(c: &rusqlite::Connection, ids: &[Vec<TrackId>]) -> Result<Vec<i64>> {
    let mut stmt = c.prepare_cached("SELECT COUNT(*) FROM plays WHERE track_id = ?1")?;
    let mut counts = Vec::with_capacity(ids.len());
    for song in ids {
        let mut n = 0;
        for id in song {
            n += stmt.query_row([&id.0], |r| r.get::<_, i64>(0))?;
        }
        counts.push(n);
    }
    Ok(counts)
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

async fn stream(State(svc): Svc, Path(id): Path<String>, headers: HeaderMap) -> Result<Response> {
    stream_saved(&svc, &track_id(id)?, headers.get(header::RANGE).cloned()).await
}

/// A saved track's audio with Range support: the file itself in folder mode (`ServeFile`), or
/// Jellyfin's original file with Range passed through and the body streamed as it comes.
pub async fn stream_saved(svc: &Service, id: &TrackId, range: Option<HeaderValue>) -> Result<Response> {
    let location = svc
        .index()
        .saved(id)
        .map(|s| s.location.clone())
        .ok_or_else(|| Error::NotFound(format!("{id} is not saved")))?;
    match location {
        Location::File { path } => serve_file(svc.folder.file(&path), range).await,
        Location::Jellyfin { item, .. } => proxy_jellyfin(svc, &item, range).await,
    }
}

/// tower-http's file serving: 200, 206 for one range, 416 past the end; ETag and Last-Modified.
pub async fn serve_file(file: std::path::PathBuf, range: Option<HeaderValue>) -> Result<Response> {
    use tower::ServiceExt;
    if !tokio::fs::try_exists(&file).await.unwrap_or(false) {
        return Err(Error::NotFound(format!(
            "{} is gone from the music folder",
            file.display()
        )));
    }
    let mut request = axum::http::Request::new(Body::empty());
    if let Some(range) = range {
        request.headers_mut().insert(header::RANGE, range);
    }
    let response = match tower_http::services::ServeFile::new(file).oneshot(request).await {
        Ok(r) => r,
        Err(never) => match never {},
    };
    Ok(response.map(Body::new))
}

async fn proxy_jellyfin(svc: &Service, item: &str, range: Option<HeaderValue>) -> Result<Response> {
    let upstream = svc.jellyfin()?.stream(item, range).await?;
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
    cover_response(&svc, &AlbumId(id), q.size.unwrap_or(256)).await
}

pub async fn cover_response(svc: &Service, id: &AlbumId, size: u32) -> Result<Response> {
    let image = art::cover(svc, id, size).await?;
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
    let job = RescanJob::new(match svc.mode {
        LibraryMode::Jellyfin => "Asking Jellyfin to scan",
        LibraryMode::Folder => "Reading the music folder",
    });
    svc.jobs.put_rescan(&job).await;
    let id = job.id.clone();
    match svc.mode {
        LibraryMode::Jellyfin => svc.ctx.spawn(sync::rescan(svc.clone(), job)),
        LibraryMode::Folder => svc.ctx.spawn(folder::rescan(svc.clone(), job)),
    }
    Ok((StatusCode::ACCEPTED, Json(json!({ "jobId": id }))))
}

// ---------- source and Jellyfin connect ----------

async fn source(State(svc): Svc) -> Json<Value> {
    Json(switch::describe(&svc))
}

#[derive(Deserialize)]
struct SourceBody {
    mode: String,
}

async fn set_source(State(svc): Svc, Json(body): Json<SourceBody>) -> Result<(StatusCode, Json<Value>)> {
    let mode = match body.mode.as_str() {
        "folder" => LibraryMode::Folder,
        "jellyfin" => LibraryMode::Jellyfin,
        other => return Err(Error::BadRequest(format!("mode {other:?} is not folder or jellyfin"))),
    };
    switch::to_mode(&svc, mode)?;
    Ok((StatusCode::ACCEPTED, Json(json!({}))))
}

async fn discover(State(svc): Svc) -> Result<Json<Value>> {
    Ok(Json(json!({ "items": connect::discover(&svc.ctx.http).await? })))
}

#[derive(Deserialize)]
struct UrlBody {
    url: String,
}

async fn quick_connect(State(svc): Svc, Json(body): Json<UrlBody>) -> Result<Json<Value>> {
    let url = connect::server_url(&body.url)?;
    let started = connect::initiate(&svc.ctx.http, &url).await?;
    svc.connects.add(&started.secret, url);
    Ok(Json(json!({ "code": started.code, "secret": started.secret })))
}

async fn quick_connect_state(State(svc): Svc, Path(secret): Path<String>) -> Result<Json<Value>> {
    let state = connect::poll(&svc, &secret).await?;
    Ok(Json(json!({ "state": state })))
}

#[derive(Deserialize)]
struct SignInBody {
    url: String,
    username: String,
    password: String,
}

/// The fallback when Quick Connect is off: the password is sent to Jellyfin once and dropped.
async fn sign_in(State(svc): Svc, Json(body): Json<SignInBody>) -> Result<StatusCode> {
    let url = connect::server_url(&body.url)?;
    let token = connect::sign_in(&svc.ctx.http, &url, &body.username, &body.password).await?;
    switch::to_jellyfin(&svc, url, token)?;
    Ok(StatusCode::ACCEPTED)
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

#[cfg(test)]
mod tests {
    use super::*;

    async fn get(file: &std::path::Path, range: Option<&'static str>) -> Result<(StatusCode, HeaderMap, Bytes)> {
        let response = serve_file(file.to_owned(), range.map(HeaderValue::from_static)).await?;
        let (parts, body) = response.into_parts();
        let bytes = axum::body::to_bytes(body, usize::MAX)
            .await
            .map_err(|e| Error::internal(e.to_string()))?;
        Ok((parts.status, parts.headers, bytes))
    }

    #[test]
    fn song_plays_include_plays_under_aliased_ids() -> Result<()> {
        let c = rusqlite::Connection::open_in_memory()?;
        c.execute_batch(
            "CREATE TABLE plays (id INTEGER PRIMARY KEY, track_id TEXT NOT NULL);
             INSERT INTO plays (track_id) VALUES ('new'), ('old'), ('old'), ('other');",
        )?;
        let id = |s: &str| TrackId(s.to_owned());
        let aliases = HashMap::from([(id("old"), id("new")), (id("gone"), id("elsewhere"))]);
        let songs = [id("new"), id("unplayed")];
        assert_eq!(play_counts(&c, &played_as(&aliases, songs.iter()))?, vec![3, 0]);
        Ok(())
    }

    #[tokio::test]
    async fn folder_files_are_served_with_ranges_for_seeking() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("lull-serve-{}", std::process::id()));
        std::fs::create_dir_all(&dir).map_err(|e| Error::internal(e.to_string()))?;
        let file = dir.join("song.flac");
        let audio: Vec<u8> = (0..1000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&file, &audio).map_err(|e| Error::internal(e.to_string()))?;

        let (status, headers, body) = get(&file, None).await?;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.len(), 1000);
        assert_eq!(
            headers.get(header::ACCEPT_RANGES).map(|v| v.as_bytes()),
            Some(&b"bytes"[..])
        );

        let (status, headers, body) = get(&file, Some("bytes=100-199")).await?;
        assert_eq!(status, StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            headers.get(header::CONTENT_RANGE).map(|v| v.as_bytes()),
            Some(&b"bytes 100-199/1000"[..])
        );
        assert_eq!(&body[..], &audio[100..200]);

        let (status, _, body) = get(&file, Some("bytes=900-")).await?;
        assert_eq!((status, &body[..]), (StatusCode::PARTIAL_CONTENT, &audio[900..]));

        let (status, ..) = get(&file, Some("bytes=5000-")).await?;
        assert_eq!(status, StatusCode::RANGE_NOT_SATISFIABLE);

        std::fs::remove_dir_all(&dir).map_err(|e| Error::internal(e.to_string()))?;
        assert!(
            matches!(get(&file, None).await, Err(Error::NotFound(_))),
            "a vanished file is 404"
        );
        Ok(())
    }
}
