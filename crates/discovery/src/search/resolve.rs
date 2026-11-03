//! `POST /api/catalog/resolve { trackIds }`: the songs the UI is about to show without a cover
//! (docs/context/discovery.md#bare-songs). Any crate may record a song bare, as its source names
//! it (`catalog::remember_named`); the UI asks here for each one it draws without a picture, so
//! every path is covered, today's and later ones, and no crate resolves songs itself.
//!
//! - **At once:** a song lull already has a picture for (a resolved one, a song whose album's
//!   cover is known, a source's own picture) comes back in the answer's `entities`, with its
//!   album pictured.
//! - **In the background:** a bare song due a lookup (never tried, or missed a day ago) is
//!   looked up as `Catalog::find_song` does (the library, Deezer by ISRC, the catalog by name),
//!   and on a match the bare record is enriched in place (`catalog::enrich`: the catalog's
//!   spelling, album and cover, the same id). Each batch goes out as `catalog.tracks
//!   { entities }`, which the UI absorbs and repaints.

use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use lull_core::ids::TrackId;
use lull_core::model::{Entities, SongQuery, Track};
use lull_core::seams::Catalog;
use lull_core::{Error, Result, catalog};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::Notify;

use crate::Service;
use crate::art;

/// Ids one request may name: a long playlist asks in a few requests.
const MAX_IDS: usize = 200;
/// A batch of resolved songs goes out at most this often.
const REPORT_EVERY: Duration = Duration::from_secs(1);

/// The bare songs waiting for a lookup, each once.
#[derive(Default)]
pub struct Queue {
    waiting: Mutex<(VecDeque<TrackId>, HashSet<TrackId>)>,
    arrived: Notify,
}

impl Queue {
    fn push(&self, ids: Vec<TrackId>) {
        let mut w = self.waiting.lock().unwrap_or_else(PoisonError::into_inner);
        for id in ids {
            if w.1.insert(id.clone()) {
                w.0.push_back(id);
            }
        }
        drop(w);
        self.arrived.notify_one();
    }

    fn pop(&self) -> Option<TrackId> {
        let mut w = self.waiting.lock().unwrap_or_else(PoisonError::into_inner);
        let id = w.0.pop_front()?;
        w.1.remove(&id);
        Some(id)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Body {
    track_ids: Vec<TrackId>,
}

/// `{ entities }` for the songs lull can picture now; the others are queued.
pub async fn post(
    State(svc): State<Arc<Service>>,
    body: std::result::Result<Json<Body>, JsonRejection>,
) -> Result<Json<Value>> {
    let Json(body) = body.map_err(|e| Error::BadRequest(e.body_text()))?;
    if body.track_ids.len() > MAX_IDS {
        return Err(Error::BadRequest(format!("Ask for at most {MAX_IDS} songs at once.")));
    }
    let now = lull_core::now();
    let (known, due) = svc
        .ctx
        .db
        .call(move |c| {
            let (mut known, mut due) = (Vec::new(), Vec::new());
            for id in body.track_ids {
                let Some(track) = catalog::track(c, &id)? else { continue };
                if !catalog::is_bare(c, &track)? {
                    known.push(track);
                } else if catalog::resolution(c, &id, now)?.1 {
                    due.push(id);
                }
            }
            Ok((known, due))
        })
        .await?;
    svc.resolver.push(due);
    Ok(Json(json!({ "entities": pictured(&svc, known).await? })))
}

/// Looks queued songs up, one at a time (the catalog paces itself), for as long as lull runs.
pub async fn work(svc: Arc<Service>) {
    let mut batch: Vec<Track> = Vec::new();
    let mut last = Instant::now();
    loop {
        let Some(id) = svc.resolver.pop() else {
            announce(&svc, std::mem::take(&mut batch)).await;
            svc.resolver.arrived.notified().await;
            continue;
        };
        match resolve(&svc, &id).await {
            Ok(Some(track)) => batch.push(track),
            Ok(None) => {}
            Err(e) => tracing::info!(error = %e, track = %id, "a bare song wasn't resolved; asked again when shown"),
        }
        if last.elapsed() >= REPORT_EVERY {
            announce(&svc, std::mem::take(&mut batch)).await;
            last = Instant::now();
        }
    }
}

/// The song as the catalog knows it, enriched in place; `None` when nothing matches (noted, so
/// it isn't looked up again for a day) or it no longer needs it.
async fn resolve(svc: &Service, id: &TrackId) -> Result<Option<Track>> {
    let (bare, isrc) = {
        let id = id.clone();
        let now = lull_core::now();
        let found = svc
            .ctx
            .db
            .call(move |c| {
                let Some(t) = catalog::track(c, &id)? else {
                    return Ok(None);
                };
                let (isrc, due) = catalog::resolution(c, &id, now)?;
                Ok((catalog::is_bare(c, &t)? && due).then_some((t, isrc)))
            })
            .await?;
        match found {
            Some(f) => f,
            None => return Ok(None),
        }
    };
    let query = SongQuery {
        artist: bare.artist.clone(),
        title: bare.title.clone(),
        album: Some(bare.album.clone()).filter(|a| !a.is_empty()),
        duration: bare.duration,
        isrc,
    };
    let found = svc.find_song(&query).await?;
    let id = id.clone();
    svc.ctx
        .db
        .call(move |c| {
            let now = lull_core::now();
            match found {
                // The catalog's song under the same id: its record and album are the catalog's now.
                Some(f) if f.id == id => Ok(catalog::track(c, &id)?.filter(|t| !t.saved)),
                Some(f) => catalog::enrich(c, &id, &f, now),
                None => {
                    catalog::missed(c, &id, now)?;
                    Ok(None)
                }
            }
        })
        .await
}

/// Sends a batch as `catalog.tracks { entities }`.
async fn announce(svc: &Service, tracks: Vec<Track>) {
    if tracks.is_empty() {
        return;
    }
    match pictured(svc, tracks).await {
        Ok(entities) => svc.ctx.bus.publish("catalog.tracks", json!({ "entities": entities })),
        Err(e) => tracing::warn!(error = %e, "resolved songs weren't announced"),
    }
}

/// The songs with their albums, pictured as search pictures them.
async fn pictured(svc: &Service, tracks: Vec<Track>) -> Result<art::Pictured> {
    let entities = svc
        .ctx
        .db
        .call(move |c| {
            let mut e = Entities::default();
            for t in tracks {
                if let Some(album) = catalog::album(c, &t.album_id)? {
                    e.albums.insert(album.id.clone(), album);
                }
                e.tracks.insert(t.id.clone(), t);
            }
            Ok(e)
        })
        .await?;
    art::pictured(svc, entities).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_song_waits_once_however_often_it_is_asked_for() {
        let q = Queue::default();
        let (a, b) = (TrackId("t-a".into()), TrackId("t-b".into()));
        q.push(vec![a.clone(), b.clone(), a.clone()]);
        q.push(vec![b.clone()]);
        assert_eq!((q.pop(), q.pop(), q.pop()), (Some(a.clone()), Some(b), None));
        q.push(vec![a.clone()]);
        assert_eq!(q.pop(), Some(a), "asked again once looked up");
    }
}
