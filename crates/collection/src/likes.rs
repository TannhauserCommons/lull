//! Liked songs (core `likes` table). Newest first. A `like` event tells the taste model and the
//! listener's other devices.

use std::sync::Arc;

use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, put};
use axum::{Json, Router};
use lull_core::Result;
use lull_core::ids::TrackId;
use rusqlite::params;
use serde_json::{Value, json};

use crate::{Page, Service, bad_query, entities, known_track};

pub(crate) fn routes() -> Router<Arc<Service>> {
    Router::new()
        .route("/api/likes", get(list))
        .route("/api/likes/{track_id}", put(like).delete(unlike))
}

async fn list(State(svc): State<Arc<Service>>, page: Result<Query<Page>, QueryRejection>) -> Result<Json<Value>> {
    let (limit, offset) = page.map_err(bad_query)?.bounds(100)?;
    svc.ctx
        .db
        .call(move |conn| {
            let total: i64 = conn.query_row("SELECT COUNT(*) FROM likes", [], |r| r.get(0))?;
            let ids = conn
                .prepare("SELECT track_id FROM likes ORDER BY liked_at DESC, rowid DESC LIMIT ?1 OFFSET ?2")?
                .query_map(params![limit, offset], |r| r.get(0).map(TrackId))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let entities = entities(conn, &ids)?;
            Ok(Json(json!({ "items": ids, "total": total, "entities": entities })))
        })
        .await
}

/// Idempotent: liking a liked song keeps its original date.
async fn like(State(svc): State<Arc<Service>>, Path(id): Path<TrackId>) -> Result<StatusCode> {
    let track_id = id.clone();
    svc.ctx
        .db
        .call(move |conn| {
            let t = known_track(conn, &track_id)?;
            conn.execute(
                "INSERT INTO likes (track_id, artist, title, album, liked_at) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(track_id) DO NOTHING",
                params![t.id.as_str(), t.artist, t.title, t.album, lull_core::now()],
            )?;
            Ok(())
        })
        .await?;
    svc.ctx.bus.publish("like", json!({ "trackId": id, "liked": true }));
    Ok(StatusCode::NO_CONTENT)
}

async fn unlike(State(svc): State<Arc<Service>>, Path(id): Path<TrackId>) -> Result<StatusCode> {
    let track_id = id.clone();
    svc.ctx
        .db
        .call(move |conn| {
            conn.execute("DELETE FROM likes WHERE track_id = ?1", [track_id.as_str()])?;
            Ok(())
        })
        .await?;
    svc.ctx.bus.publish("like", json!({ "trackId": id, "liked": false }));
    Ok(StatusCode::NO_CONTENT)
}
