//! Instant play from catalog search (ADR-0008). Discovery publishes `search.catalog` with the
//! top tracks of a query; their Soulseek copies are looked up in the background, one search at
//! a time and in the search budget's `Prewarm` lane, so pressing play finds sources waiting.
//! The same tracks tell search which Soulseek songs no catalog listed (`onlySoulseek`).

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use lull_core::catalog;
use lull_core::ids::TrackId;
use lull_core::model::Track;
use serde::Deserialize;
use tokio::sync::{Mutex, Notify};

use crate::Service;
use crate::budget::Lane;

/// Only the top of a query is pre-warmed: each lookup is a full network search.
const TOP: usize = 3;
/// Least time between two pre-warm searches, on top of search's own spacing: they share slskd,
/// which runs two searches at a time, with the listener's own.
const GAP: Duration = Duration::from_secs(5);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CatalogEvent {
    query: String,
    track_ids: Vec<TrackId>,
}

/// Follows `search.catalog`: records the catalog's tracks for the query and queues the top
/// ones to pre-warm. A newer query replaces what an older one left queued.
pub async fn follow_catalog(svc: Arc<Service>) {
    let queue: Arc<Mutex<VecDeque<Track>>> = Arc::default();
    let wake = Arc::new(Notify::new());
    svc.ctx.spawn(warm(svc.clone(), queue.clone(), wake.clone()));
    let mut events = svc.ctx.bus.subscribe();
    loop {
        let event = match events.recv().await {
            Ok(e) if e.topic == "search.catalog" => e,
            Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
        };
        let CatalogEvent { query, track_ids } = match serde_json::from_value(event.data) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "search.catalog can't be read");
                continue;
            }
        };
        let tracks = match svc.ctx.db.call(move |c| known(c, &track_ids)).await {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(error = %e, "reading the catalog's tracks for a search");
                continue;
            }
        };
        let top: VecDeque<Track> = tracks.iter().take(TOP).cloned().collect();
        svc.searches.catalog(&query, tracks);
        *queue.lock().await = top;
        wake.notify_one();
    }
}

fn known(c: &rusqlite::Connection, ids: &[TrackId]) -> lull_core::Result<Vec<Track>> {
    let mut tracks = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(t) = catalog::track(c, id)? {
            tracks.push(t);
        }
    }
    Ok(tracks)
}

/// One pre-warm search at a time, `GAP` apart; songs with known copies and saved songs are
/// skipped, and nothing runs while Soulseek is offline.
async fn warm(svc: Arc<Service>, queue: Arc<Mutex<VecDeque<Track>>>, wake: Arc<Notify>) {
    loop {
        let next = queue.lock().await.pop_front();
        let Some(track) = next else {
            wake.notified().await;
            continue;
        };
        let online = svc.online.load(std::sync::atomic::Ordering::Relaxed);
        if !online || track.saved || svc.sources.known(&track) {
            continue;
        }
        if let Err(e) = svc.searches.resolve_in(&track, Lane::Prewarm).await {
            tracing::debug!(error = %e, track = %track.id, "pre-warming a song's sources");
        }
        tokio::time::sleep(GAP).await;
    }
}
