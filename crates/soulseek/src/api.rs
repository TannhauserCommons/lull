//! HTTP handlers; shapes are in docs/context/api.md (Soulseek section).

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use axum::Extension;
use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequestParts, Path, Query, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use lull_core::devices::Device;
use lull_core::ids::TrackId;
use lull_core::model::{SourceRef, Track};
use lull_core::quality::StreamProfile;
use lull_core::youtube::{self, Mode, Want};
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
        .route("/api/tracks/{id}/verify", post(verify))
        .route("/api/stream/{id}/output-error", post(output_error))
        .route("/api/stream/prefetch", put(prefetch))
        .route("/api/stream/{id}", get(stream).delete(stop))
        .route("/api/stream/{id}/copy", get(copy))
        .route("/api/stream/{id}/switch", post(switch))
        .route("/api/stream/{id}/not-switched", post(not_switched))
        .route("/api/soulseek/diagnostics", get(diagnostics))
        .route("/api/transfers", get(transfers))
        .route("/api/transfers/{job}", delete(cancel))
        .route("/api/transfers/{job}/retry", post(retry))
        .route("/api/cache/clear", post(clear_cache))
}

/// The streaming profile of the asking device's source preset (ADR-0017): what everything
/// fetched or ordered for the request streams by.
struct Asking(StreamProfile);

impl FromRequestParts<Arc<Service>> for Asking {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, svc: &Arc<Service>) -> std::result::Result<Self, Self::Rejection> {
        let device = parts.extensions.get::<Device>().map(|d| d.id.as_str());
        Ok(Asking(svc.rules.get().profile_for(device, &parts.headers)))
    }
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
async fn sources(
    State(svc): State<Arc<Service>>,
    Path(id): Path<String>,
    Query(q): Query<For>,
    Asking(profile): Asking,
) -> Result<Json<Value>> {
    let purpose = match q.purpose.as_deref() {
        None | Some("stream") => Purpose::Stream,
        Some("save") => Purpose::Save,
        Some(other) => return Err(Error::BadRequest(format!("for={other}: stream or save"))),
    };
    let track = svc.track(&TrackId(id)).await?;
    let rules = svc.rules.get().with_stream(&profile);
    let ranked = svc.sources.ordered(&track, &rules, purpose);
    let mut searching = svc.searches.searching_for(&track);
    // Copies other searches came across (an album's folder, another query) don't stand for a
    // search for the song: live, two soundtrack folders' copies kept six more peers' unseen.
    if !svc.sources.searched(&track.id) && !searching && svc.online.load(Ordering::Relaxed) {
        let (searches, track) = (svc.searches.clone(), track.clone());
        tokio::spawn(async move {
            if let Err(e) = searches.resolve(&track).await {
                tracing::warn!(error = %e, "searching for a song's sources");
            }
        });
        searching = true;
    }
    let youtube = match purpose {
        Purpose::Stream => youtube_row(&svc, &track, &rules).await,
        Purpose::Save => YoutubeRow::Off,
    };
    let (youtube, finding) = match youtube {
        YoutubeRow::Off => (Value::Null, false),
        YoutubeRow::Finding => (Value::Null, true),
        YoutubeRow::Found(row) => (row, false),
    };
    let done = !searching && !finding;
    // The hub's sentence for an empty answer: the web words no fact of its own.
    let why = (done && ranked.is_empty() && youtube.is_null()).then_some(NO_PEER);
    Ok(Json(
        json!({ "sources": ranked, "youtube": youtube, "done": done, "why": why }),
    ))
}

/// Why the sources answer is empty once the search for the song has ended.
const NO_PEER: &str = "No Soulseek peer shares this song right now.";

/// How long the picker's answer waits for YouTube's match; past it the match goes on and the
/// next ask gets it (`done: false` until then).
const YOUTUBE_WAIT: Duration = Duration::from_millis(400);

enum YoutubeRow {
    /// YouTube is off, or nothing matches.
    Off,
    Finding,
    Found(Value),
}

/// The song's YouTube copy as the picker lists it, when Settings › Sources › YouTube is on: the
/// copy the player's YouTube path would play (the streaming cap's), with its chip ("YouTube ·
/// Opus 160"). Lossy, it has no score: the picker lists it after the peers. `meets`: it passes
/// the streaming minimum (`policy::passes`), as a peer's copy does.
async fn youtube_row(svc: &Service, track: &Track, rules: &crate::rules::Rules) -> YoutubeRow {
    if rules.youtube.mode == Mode::Off {
        return YoutubeRow::Off;
    }
    let (yt, track, want) = (
        svc.fetcher.youtube.clone(),
        track.clone(),
        Want::Stream {
            max_kbps: rules.stream.max_kbps,
        },
    );
    let (duration, profile) = (track.duration, rules.stream.clone());
    // Its own task: a match still running when the wait ends goes on, and is remembered.
    let find = tokio::spawn(async move { yt.find(&track, want).await });
    match tokio::time::timeout(YOUTUBE_WAIT, find).await {
        Err(_) => YoutubeRow::Finding,
        Ok(Ok(Ok(Some(found)))) => match youtube::Copy::of(&found.source) {
            Some(copy) => YoutubeRow::Found(json!({
                "peer": found.source.user,
                "path": found.source.path,
                "sizeBytes": found.source.size,
                "format": copy.facts(found.source.size, duration).format,
                "official": found.official,
                "meets": crate::fetch::bare_source(&found.source, duration)
                    .is_some_and(|s| crate::policy::passes(&profile, &s, duration)),
                // A song cut from a full-album upload: where in the album it is, in seconds.
                "section": copy.section.map(|(from, to)| [from / 1000, to.div_ceil(1000)]),
            })),
            None => YoutubeRow::Off,
        },
        Ok(_) => YoutubeRow::Off,
    }
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
    Asking(profile): Asking,
    headers: HeaderMap,
) -> Result<Response> {
    let track = svc.track(&TrackId(id)).await?;
    let pick = match (pick.peer, pick.path) {
        (Some(peer), Some(path)) => Some(known_source(&svc, &track, peer, path, &profile).await?),
        (None, None) => None,
        _ => return Err(Error::BadRequest("peer and path go together".into())),
    };
    let fetch = svc.fetcher.stream(track, pick, &profile).await?;
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
    Asking(profile): Asking,
    headers: HeaderMap,
) -> Result<Response> {
    let fetch = svc.fetcher.copy(&TrackId(id), &profile, &copy.source_ref())?;
    crate::stream::serve(fetch, &headers).await
}

/// The player switched to the better copy on its way: it becomes the song's stream, and the
/// copy before it stops once its response ends. Answers the song's `stream` event as it is now
/// (its new job, naming the one it replaces), which the player follows from then on. 409 when
/// that copy is no longer on its way.
async fn switch(
    State(svc): State<Arc<Service>>,
    Path(id): Path<String>,
    Asking(profile): Asking,
    body: std::result::Result<Json<Copy>, JsonRejection>,
) -> Result<Json<Value>> {
    let Json(copy) = body.map_err(|e| Error::BadRequest(e.body_text()))?;
    let f = svc.fetcher.adopt(&TrackId(id), &profile, &copy.source_ref(), false)?;
    Ok(Json(svc.fetcher.event(&f)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NotSwitched {
    peer: String,
    path: String,
    why: crate::fetch::NotSwitched,
}

/// The player couldn't switch to the better copy on its way mid-song: the copy's job says why,
/// for Activity. 409 when that copy is no longer on its way.
async fn not_switched(
    State(svc): State<Arc<Service>>,
    Path(id): Path<String>,
    Asking(profile): Asking,
    body: std::result::Result<Json<NotSwitched>, JsonRejection>,
) -> Result<StatusCode> {
    let Json(b) = body.map_err(|e| Error::BadRequest(e.body_text()))?;
    let copy = SourceRef {
        user: b.peer,
        path: b.path,
        size: 0,
    };
    svc.fetcher.not_switched(&TrackId(id), &profile, &copy, b.why)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OutputError {
    #[serde(rename = "jobId", default)]
    job_id: Option<String>,
}

/// The player's audio failed while it followed a job: the job's own state answers, with its
/// reason; a copy that arrived but this device can't play is recorded on the job. 404 for a job
/// the hub no longer knows. Never starts or restarts a transfer.
async fn output_error(
    State(svc): State<Arc<Service>>,
    Path(id): Path<String>,
    device: Option<Extension<Device>>,
    body: std::result::Result<Json<OutputError>, JsonRejection>,
) -> Result<Json<Value>> {
    let Json(b) = body.map_err(|e| Error::BadRequest(e.body_text()))?;
    let device = device.map(|Extension(d)| d.name).unwrap_or_default();
    svc.fetcher
        .output_error(&TrackId(id), b.job_id.as_deref(), &device)
        .map(Json)
        .ok_or_else(|| Error::NotFound("the hub no longer knows that job".into()))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Answer {
    /// "yes" or "no".
    answer: String,
}

/// "Right song?" for the YouTube copy playing unverified (ADR-0016): yes makes it the song's
/// reference; no refuses it, and the player's next opening of the song takes the next copy.
async fn verify(
    State(svc): State<Arc<Service>>,
    Path(id): Path<String>,
    Asking(profile): Asking,
    body: std::result::Result<Json<Answer>, JsonRejection>,
) -> Result<StatusCode> {
    let Json(a) = body.map_err(|e| Error::BadRequest(e.body_text()))?;
    let yes = match a.answer.as_str() {
        "yes" => true,
        "no" => false,
        _ => return Err(Error::BadRequest("answer is yes or no".into())),
    };
    let track = svc.track(&TrackId(id)).await?;
    let copy = svc
        .fetcher
        .playing_copy(&track.id, &profile)
        .filter(youtube::is_youtube)
        .ok_or_else(|| Error::Conflict("no YouTube copy of this song is playing".into()))?;
    let file = svc
        .cache
        .complete(&track.id)
        .await?
        .into_iter()
        .find(|e| e.source.user == copy.user && e.source.path == copy.path);
    let copy = file.as_ref().map_or(copy, |e| e.source.clone());
    svc.fetcher
        .youtube
        .confirm(&track, &copy, file.as_ref().map(|e| e.path.as_path()), yes)
        .await?;
    if !yes {
        svc.fetcher.refused(&track.id, &copy).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// A picked source must be one a search reported: its size comes from there. A YouTube pick is
/// the song's YouTube match (remembered from the picker's ask).
async fn known_source(
    svc: &Service,
    track: &Track,
    peer: String,
    path: String,
    profile: &StreamProfile,
) -> Result<SourceRef> {
    let wanted = SourceRef {
        user: peer,
        path,
        size: 0,
    };
    if youtube::is_youtube(&wanted) {
        let want = Want::Stream {
            max_kbps: profile.max_kbps,
        };
        return match svc.fetcher.youtube.find(track, want).await? {
            Some(found) if found.source.path == wanted.path => Ok(found.source),
            _ => Err(Error::NotFound(
                "YouTube no longer offers that copy of this song; pick again".into(),
            )),
        };
    }
    svc.fetcher.picked(track, &wanted).await?.ok_or_else(|| {
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
    Asking(profile): Asking,
    body: std::result::Result<Json<Prefetch>, JsonRejection>,
) -> Result<StatusCode> {
    let Json(body) = body.map_err(|e| Error::BadRequest(e.body_text()))?;
    let device = device.map(|Extension(d)| d.id).unwrap_or_default();
    crate::prefetch::listed(&svc, device, profile, body.tracks)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn stop(State(svc): State<Arc<Service>>, Path(id): Path<String>, Asking(profile): Asking) -> StatusCode {
    svc.fetcher.stop_streaming(&TrackId(id), &profile);
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

async fn retry(
    State(svc): State<Arc<Service>>,
    Path(job): Path<String>,
    Asking(profile): Asking,
) -> Result<Json<Value>> {
    let job = svc.fetcher.retry(&job, &profile)?;
    Ok(Json(json!({ "jobId": job })))
}

async fn clear_cache(State(svc): State<Arc<Service>>) -> Result<Json<Value>> {
    let c = svc.cache.clear().await?;
    Ok(Json(
        json!({ "files": c.files, "freedBytes": c.freed_bytes, "keptBytes": c.kept_bytes }),
    ))
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_preload_route_sits_beside_the_stream_route() {
        // axum refuses conflicting routes by panicking when they're added, at hub start-up.
        let _ = super::routes();
    }
}
