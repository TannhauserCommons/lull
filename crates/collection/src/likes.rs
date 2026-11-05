//! Liked songs (core `likes` table). Newest first. A `like` event tells the taste model and the
//! listener's other devices.
//!
//! With `library.autoSaveLikes` on, a song liked on its own is saved too (ADR-0099): every like,
//! from any client, Subsonic or an offline sync, passes `set`, which asks library
//! (`saves.request`) as Save would. One action liking many songs at once is a batch: it saves
//! nothing by itself, so it never starts many downloads unasked. The web asks "Save these too?";
//! the rest wait as "liked, not saved" (`/api/likes/unsaved`) until the listener saves them.

use std::collections::HashSet;
use std::sync::Arc;

use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use lull_core::Result;
use lull_core::ids::TrackId;
use rusqlite::{Connection, params};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{Events, Page, Service, bad_json, bad_query, entities, known_track, settings};

pub(crate) fn routes() -> Router<Arc<Service>> {
    Router::new()
        .route("/api/likes", get(list))
        .route("/api/likes/unsaved", get(unsaved))
        .route("/api/likes/save", post(save))
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

/// How a like reached the hub: one song liked on its own, or one action liking many at once
/// (multi-select, a Subsonic `star` of several ids, an import). Only the first saves by itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum How {
    One,
    Batch,
}

impl How {
    pub(crate) fn of(batch: bool) -> How {
        if batch { How::Batch } else { How::One }
    }
}

#[derive(Deserialize)]
struct LikeQuery {
    /// Part of one action liking many songs.
    #[serde(default)]
    batch: bool,
}

async fn like(
    State(svc): State<Arc<Service>>,
    Path(id): Path<TrackId>,
    query: Result<Query<LikeQuery>, QueryRejection>,
) -> Result<StatusCode> {
    let how = How::of(query.map_err(bad_query)?.batch);
    svc.write(move |tx, events| set(tx, events, &id, true, lull_core::now(), how))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn unlike(State(svc): State<Arc<Service>>, Path(id): Path<TrackId>) -> Result<StatusCode> {
    svc.write(move |tx, events| set(tx, events, &id, false, lull_core::now(), How::One))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Liked songs the library doesn't hold, newest liked first: `{ items, total }`. A song whose
/// save is still running counts until it is saved.
async fn unsaved(State(svc): State<Arc<Service>>) -> Result<Json<Value>> {
    let items = not_saved(&svc).await?;
    Ok(Json(json!({ "total": items.len(), "items": items })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SaveBody {
    /// The liked songs to save; every liked song not saved when left out.
    track_ids: Option<Vec<TrackId>>,
}

/// `{ trackIds? }` (`{}` for all): saves liked songs the library doesn't hold, on the listener's word (a song
/// not liked is left out). `{ asked }`: how many library was asked to save; it skips one saved
/// meanwhile and keeps a save already running.
async fn save(State(svc): State<Arc<Service>>, body: Result<Json<SaveBody>, JsonRejection>) -> Result<Json<Value>> {
    let Json(body) = body.map_err(bad_json)?;
    let mut ids = not_saved(&svc).await?;
    if let Some(wanted) = body.track_ids {
        let wanted: HashSet<TrackId> = wanted.into_iter().collect();
        ids.retain(|id| wanted.contains(id));
    }
    let asked = ids.len();
    if asked > 0 {
        svc.ctx.bus.publish("saves.request", json!({ "trackIds": ids }));
    }
    Ok(Json(json!({ "asked": asked })))
}

/// Liked songs, newest first, that the library doesn't hold.
async fn not_saved(svc: &Service) -> Result<Vec<TrackId>> {
    let liked: Vec<TrackId> = svc
        .ctx
        .db
        .call(|conn| {
            Ok(conn
                .prepare("SELECT track_id FROM likes ORDER BY liked_at DESC, rowid DESC")?
                .query_map([], |r| r.get(0).map(TrackId))?
                .collect::<rusqlite::Result<Vec<_>>>()?)
        })
        .await?;
    let saved: HashSet<TrackId> = svc.library.saved(&liked).await?.into_iter().map(|t| t.id).collect();
    Ok(liked.into_iter().filter(|id| !saved.contains(id)).collect())
}

/// Likes (dated `at`) or unlikes a song, idempotently: liking a liked song keeps its original
/// date, and changes nothing else. Only a song the catalog knows can be liked. A song newly
/// liked on its own is saved too when `library.autoSaveLikes` is on; library skips one already
/// saved. Unliking never touches a saved song or a running save.
pub(crate) fn set(conn: &Connection, events: &mut Events, id: &TrackId, liked: bool, at: i64, how: How) -> Result<()> {
    if liked {
        let t = known_track(conn, id)?;
        let new = conn.execute(
            "INSERT INTO likes (track_id, artist, title, album, liked_at) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(track_id) DO NOTHING",
            params![t.id.as_str(), t.artist, t.title, t.album, at],
        )? > 0;
        if new && how == How::One && settings::auto_save_likes(conn)? {
            events.push("saves.request", json!({ "trackIds": [id] }));
        }
    } else {
        conn.execute("DELETE FROM likes WHERE track_id = ?1", [id.as_str()])?;
    }
    events.push("like", json!({ "trackId": id, "liked": liked }));
    Ok(())
}

#[cfg(test)]
mod tests {
    use lull_core::seams::Collection;
    use tokio::sync::broadcast::Receiver;

    use super::*;
    use crate::testing;

    async fn auto_save(svc: &Service, on: bool) -> Result<()> {
        let json = json!({ "library": { "autoSaveLikes": on } }).to_string();
        svc.ctx
            .db
            .call(move |c| {
                c.execute(
                    "INSERT INTO collection_settings (id, json) VALUES (1, ?1)
                     ON CONFLICT(id) DO UPDATE SET json = excluded.json",
                    [json],
                )?;
                Ok(())
            })
            .await
    }

    async fn like_one(svc: &Arc<Service>, id: &TrackId, liked: bool, how: How) -> Result<()> {
        let id = id.clone();
        svc.write(move |tx, events| set(tx, events, &id, liked, lull_core::now(), how))
            .await
    }

    /// The songs each `saves.request` published so far asked for.
    fn asked(rx: &mut Receiver<lull_core::events::Event>) -> Vec<Vec<String>> {
        testing::published(rx)
            .into_iter()
            .filter(|e| e.topic == "saves.request")
            .map(|e| serde_json::from_value(e.data["trackIds"].clone()).unwrap_or_default())
            .collect()
    }

    #[tokio::test]
    async fn a_song_liked_on_its_own_is_saved_once_and_unliking_touches_nothing() -> Result<()> {
        let (svc, mut rx) = testing::service().await?;
        let song = testing::known_song(&svc, "Velvet Orchard", "Paper Moon").await?;
        auto_save(&svc, true).await?;

        like_one(&svc, &song, true, How::One).await?;
        assert_eq!(asked(&mut rx), [vec![song.0.clone()]]);
        like_one(&svc, &song, true, How::One).await?;
        assert!(asked(&mut rx).is_empty(), "liked already: nothing new to save");
        like_one(&svc, &song, false, How::One).await?;
        assert!(
            asked(&mut rx).is_empty(),
            "unliking asks library nothing, and deletes nothing"
        );
        Ok(())
    }

    #[tokio::test]
    async fn off_a_like_saves_nothing() -> Result<()> {
        let (svc, mut rx) = testing::service().await?;
        let song = testing::known_song(&svc, "Glow Tape", "Harbour Lights").await?;
        like_one(&svc, &song, true, How::One).await?;
        auto_save(&svc, false).await?;
        let other = testing::known_song(&svc, "Glow Tape", "Low Tide").await?;
        like_one(&svc, &other, true, How::One).await?;
        assert!(asked(&mut rx).is_empty(), "off is the default, and off when set");
        Ok(())
    }

    #[tokio::test]
    async fn many_liked_at_once_wait_as_liked_not_saved_until_the_listener_saves_them() -> Result<()> {
        // A Subsonic client starring 300 songs, a multi-select: no downloads unasked.
        let (svc, mut rx, _, _, library) = testing::service_with_library().await?;
        auto_save(&svc, true).await?;
        let mut songs = Vec::new();
        for title in ["One", "Two", "Three"] {
            songs.push(testing::known_song(&svc, "Pale Harbor", title).await?);
        }
        svc.set_liked(&songs, true).await?;
        assert!(asked(&mut rx).is_empty());

        library
            .saved
            .lock()
            .map_err(|_| lull_core::Error::internal("poisoned"))?
            .push(songs[0].clone());
        let waiting = not_saved(&svc).await?;
        assert_eq!(waiting.len(), 2, "the saved one isn't waiting");
        assert!(!waiting.contains(&songs[0]));

        let stranger = testing::known_song(&svc, "Quiet Ferns", "Not Liked").await?;
        let body = SaveBody {
            track_ids: Some(vec![songs[1].clone(), stranger]),
        };
        let Json(out) = save(State(svc.clone()), Ok(Json(body))).await?;
        assert_eq!(out["asked"], 1, "only liked songs not saved");
        assert_eq!(asked(&mut rx), [vec![songs[1].0.clone()]]);

        let Json(out) = save(State(svc.clone()), Ok(Json(SaveBody { track_ids: None }))).await?;
        assert_eq!(out["asked"], 2, "every liked song not saved");
        Ok(())
    }

    #[tokio::test]
    async fn one_song_from_a_client_that_cant_ask_is_a_single_like() -> Result<()> {
        // Subsonic's `star` of one id, the native Classic player: saved as any single like.
        let (svc, mut rx) = testing::service().await?;
        auto_save(&svc, true).await?;
        let song = testing::known_song(&svc, "Nuit Blanche", "Soir").await?;
        svc.set_liked(std::slice::from_ref(&song), true).await?;
        assert_eq!(asked(&mut rx), [vec![song.0]]);
        Ok(())
    }
}
