//! What the listener keeps: likes, follows, history (plays), playlists with M3U import and
//! export, stats and settings. Publishes `play` events that Last.fm and the taste model consume
//! (docs/context/collection.md).

mod follows;
mod import;
mod likes;
mod m3u;
mod playlists;
mod plays;
mod settings;
mod stats;

use std::sync::Arc;

use axum::Router;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use lull_core::db::Migrations;
use lull_core::events::Event;
use lull_core::ids::TrackId;
use lull_core::model::Entities;
use lull_core::seams::Library;
use lull_core::{Ctx, Error, Result, catalog};
use rusqlite::{Connection, params};
use serde::Deserialize;
use tokio::sync::broadcast::error::RecvError;

pub const MIGRATIONS: Migrations = Migrations {
    owner: "collection",
    steps: &[r#"
        -- Per-play state core's plays table has no column for.
        CREATE TABLE collection_plays (
            play_id      INTEGER PRIMARY KEY,         -- plays.id
            recommended  INTEGER NOT NULL DEFAULT 0,  -- started from a recommendation (feed, radio)
            ended_at     INTEGER,                     -- null while playing; play.ended goes out once
            scrobble     TEXT                         -- latest status from lastfm's scrobble event
        );
        -- Songs saved through lull, from library's save events (stats: saves, hit rate).
        CREATE TABLE collection_saves (
            job_id    TEXT PRIMARY KEY,
            track_id  TEXT NOT NULL,
            saved_at  INTEGER NOT NULL
        );
        CREATE INDEX collection_saves_at ON collection_saves(saved_at);
        CREATE TABLE collection_playlists (
            seq          INTEGER PRIMARY KEY AUTOINCREMENT,  -- id is "pl-<seq>"; never reused
            name         TEXT NOT NULL,
            description  TEXT NOT NULL,
            created_at   INTEGER NOT NULL,
            updated_at   INTEGER NOT NULL
        );
        CREATE TABLE collection_playlist_items (
            playlist_seq  INTEGER NOT NULL,
            pos           INTEGER NOT NULL,
            track_id      TEXT NOT NULL,
            added_at      INTEGER NOT NULL,
            PRIMARY KEY (playlist_seq, pos)
        );
        CREATE TABLE collection_settings (
            id    INTEGER PRIMARY KEY CHECK (id = 1),
            json  TEXT NOT NULL
        );
    "#],
};

pub struct Service {
    ctx: Ctx,
    library: Arc<dyn Library>,
}

pub async fn start(ctx: &Ctx, library: Arc<dyn Library>) -> Result<Arc<Service>> {
    // Subscribe before anything can publish, so no scrobble or save is missed.
    tokio::spawn(record_events(ctx.clone(), ctx.bus.subscribe()));
    settings::announce(ctx).await?;
    Ok(Arc::new(Service {
        ctx: ctx.clone(),
        library,
    }))
}

/// `/api/likes`, `/api/follows`, `/api/plays`, `/api/history`, `/api/playlists/...`, `/api/stats`, `/api/settings`.
pub fn router(svc: Arc<Service>) -> Router {
    Router::new()
        .merge(likes::routes())
        .merge(follows::routes())
        .merge(plays::routes())
        .merge(playlists::routes())
        .merge(stats::routes())
        .merge(settings::routes())
        .with_state(svc)
}

/// What other crates tell collection: scrobble status per play (lastfm) and finished saves (library).
async fn record_events(ctx: Ctx, mut rx: tokio::sync::broadcast::Receiver<Event>) {
    loop {
        let event = match rx.recv().await {
            Ok(event) => event,
            Err(RecvError::Lagged(n)) => {
                tracing::warn!(
                    missed = n,
                    "collection fell behind the event bus; some scrobble or save events were missed"
                );
                continue;
            }
            Err(RecvError::Closed) => return,
        };
        let outcome = match event.topic.as_str() {
            "scrobble" => record_scrobble(&ctx, event).await,
            "save" => record_save(&ctx, event).await,
            _ => Ok(()),
        };
        if let Err(e) = outcome {
            tracing::warn!(error = %e, "collection could not record an event");
        }
    }
}

async fn record_scrobble(ctx: &Ctx, event: Event) -> Result<()> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Scrobble {
        play_id: String,
        status: plays::ScrobbleStatus,
    }
    let s: Scrobble = serde_json::from_value(event.data)
        .map_err(|e| Error::internal(format!("scrobble event does not match api.md: {e}")))?;
    let play_id: i64 = s
        .play_id
        .parse()
        .map_err(|_| Error::internal(format!("scrobble event names play {:?}", s.play_id)))?;
    ctx.db
        .call(move |conn| {
            conn.execute(
                "UPDATE collection_plays SET scrobble = ?2 WHERE play_id = ?1",
                params![play_id, s.status.as_str()],
            )?;
            Ok(())
        })
        .await
}

/// A save counts once its file is in the library: step 5 done, or pending (written, rescan failed).
async fn record_save(ctx: &Ctx, event: Event) -> Result<()> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Save {
        job_id: String,
        track_id: TrackId,
        step: u8,
        status: String,
    }
    let s: Save = serde_json::from_value(event.data)
        .map_err(|e| Error::internal(format!("save event does not match api.md: {e}")))?;
    if s.step != 5 || !matches!(s.status.as_str(), "done" | "pending") {
        return Ok(());
    }
    ctx.db
        .call(move |conn| {
            conn.execute(
                "INSERT OR IGNORE INTO collection_saves (job_id, track_id, saved_at) VALUES (?1, ?2, ?3)",
                params![s.job_id, s.track_id.as_str(), lull_core::now()],
            )?;
            Ok(())
        })
        .await
}

/// The catalog's tracks for `ids`, for a response's `entities`. Ids the catalog doesn't know are left out.
fn entities<'a>(conn: &Connection, ids: impl IntoIterator<Item = &'a TrackId>) -> Result<Entities> {
    let mut entities = Entities::default();
    for id in ids {
        if entities.tracks.contains_key(id) {
            continue;
        }
        if let Some(track) = catalog::track(conn, id)? {
            entities.tracks.insert(id.clone(), track);
        }
    }
    Ok(entities)
}

/// A track the catalog knows, or 404: every id the UI holds came from a response, which recorded it.
fn known_track(conn: &Connection, id: &TrackId) -> Result<lull_core::model::Track> {
    catalog::track(conn, id)?.ok_or_else(|| Error::NotFound(format!("No track {id} is known to lull.")))
}

/// Axum's own rejections answer in plain text; the API answers `{ error, message }`.
fn bad_json(e: JsonRejection) -> Error {
    Error::BadRequest(e.body_text())
}

fn bad_query(e: QueryRejection) -> Error {
    Error::BadRequest(e.body_text())
}

/// `?limit=&offset=` for lists (api.md#conventions).
#[derive(Debug, Default, Deserialize)]
struct Page {
    limit: Option<u32>,
    offset: Option<u32>,
}

impl Page {
    const MAX: u32 = 500;

    fn bounds(&self, default_limit: u32) -> Result<(u32, u32)> {
        let limit = self.limit.unwrap_or(default_limit);
        if limit == 0 || limit > Self::MAX {
            return Err(Error::BadRequest(format!("limit must be between 1 and {}.", Self::MAX)));
        }
        Ok((limit, self.offset.unwrap_or(0)))
    }
}
