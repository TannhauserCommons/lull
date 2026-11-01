//! `POST /api/sync`: the offline outbox (ADR-0005). A device that was offline sends the changes
//! it recorded, in order, each with an id it made. Each op is applied in its own transaction
//! with its id, so a resent op is acknowledged and not applied twice, and publishes the same
//! events as the matching route.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::routing::post;
use axum::{Json, Router};
use lull_core::ids::TrackId;
use lull_core::{Error, Result};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::follows::{self, Kind};
use crate::playlists::{self, seq_of};
use crate::plays::{self, EndBody};
use crate::{Events, Service, bad_json, likes};

pub(crate) fn routes() -> Router<Arc<Service>> {
    Router::new().route("/api/sync", post(sync))
}

/// Ops per request; a longer outbox is sent in several requests.
const MAX_OPS: usize = 1000;
const MAX_ID: usize = 100;
/// How long an applied op id (and a `tempId`) is remembered. An outbox resends within minutes
/// of a failed request; this covers a device that lost the answer and stayed offline for weeks.
pub(crate) const RETENTION: i64 = 90 * 86_400;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Body {
    ops: Vec<Value>,
}

/// One change, after its envelope (`id`, `at`) is taken off.
#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all_fields = "camelCase", deny_unknown_fields)]
enum Op {
    #[serde(rename = "play")]
    Play {
        track_id: TrackId,
        started_at: i64,
        played_s: u32,
        completed: bool,
        skipped: bool,
    },
    #[serde(rename = "like")]
    Like { track_id: TrackId, liked: bool },
    #[serde(rename = "follow")]
    Follow {
        kind: Kind,
        id: String,
        name: Option<String>,
        followed: bool,
    },
    #[serde(rename = "playlist.create")]
    PlaylistCreate {
        temp_id: String,
        name: String,
        #[serde(default)]
        desc: String,
        #[serde(default)]
        track_ids: Vec<TrackId>,
    },
    #[serde(rename = "playlist.edit")]
    PlaylistEdit {
        playlist_id: Option<String>,
        temp_id: Option<String>,
        name: Option<String>,
        desc: Option<String>,
        track_ids: Option<Vec<TrackId>>,
    },
    #[serde(rename = "playlist.delete")]
    PlaylistDelete {
        playlist_id: Option<String>,
        temp_id: Option<String>,
    },
}

/// An op and its time, ready to apply, or why it can't be.
type Parsed = std::result::Result<(i64, Op), String>;

#[derive(Serialize)]
struct Rejected {
    id: String,
    message: String,
}

/// `{ applied: [id], rejected: [{ id, message }], playlists: { tempId: playlistId } }`.
/// `playlists` names the playlist made for each `tempId` the batch created or named, so the
/// device can swap its temporary ids.
async fn sync(State(svc): State<Arc<Service>>, body: Result<Json<Body>, JsonRejection>) -> Result<Json<Value>> {
    let Json(body) = body.map_err(bad_json)?;
    apply_all(&svc, body.ops).await.map(Json)
}

/// Applies a batch in order (see `sync`).
async fn apply_all(svc: &Service, ops: Vec<Value>) -> Result<Value> {
    if ops.len() > MAX_OPS {
        return Err(Error::BadRequest(format!("Send at most {MAX_OPS} changes at a time.")));
    }
    let ops = ops.into_iter().map(envelope).collect::<Result<Vec<_>>>()?;
    svc.ctx.db.call(forget_old).await?;

    let (mut applied, mut rejected, mut made) = (Vec::new(), Vec::new(), BTreeMap::new());
    for (id, op) in ops {
        let outcome = match op {
            Ok((at, op)) => {
                let op_id = id.clone();
                svc.write(move |tx, events| apply(tx, events, &op_id, at, op)).await
            }
            Err(message) => Err(Error::BadRequest(message)),
        };
        match outcome {
            Ok(temp) => {
                made.extend(temp);
                applied.push(id);
            }
            // The op itself is wrong: the device drops it. Anything else (the database failing)
            // fails the request; ops before it are applied and recorded, so a resend skips them.
            Err(Error::BadRequest(message) | Error::NotFound(message) | Error::Conflict(message)) => {
                rejected.push(Rejected { id, message });
            }
            Err(e) => return Err(e),
        }
    }
    Ok(json!({ "applied": applied, "rejected": rejected, "playlists": made }))
}

/// Takes `id` and `at` off an op. Without a usable `id` the op can't be acknowledged or
/// rejected, so the whole request is refused; any other fault rejects just that op.
fn envelope(raw: Value) -> Result<(String, Parsed)> {
    let Value::Object(mut fields) = raw else {
        return Err(Error::BadRequest("Every change is an object.".into()));
    };
    let id = match fields.remove("id") {
        Some(Value::String(id)) if !id.is_empty() && id.len() <= MAX_ID => id,
        _ => {
            return Err(Error::BadRequest(format!(
                "Every change needs an \"id\" of 1 to {MAX_ID} characters."
            )));
        }
    };
    Ok((id, parse(fields)))
}

fn parse(mut fields: Map<String, Value>) -> Parsed {
    let at = fields
        .remove("at")
        .and_then(|at| at.as_i64())
        .filter(|at| *at > 0)
        .ok_or("\"at\" must be a unix time in seconds.")?;
    let op = serde_json::from_value(Value::Object(fields)).map_err(|e| format!("Not a change lull knows: {e}."))?;
    // A device clock a little ahead must not date a change in the future.
    Ok((at.min(lull_core::now()), op))
}

/// Applies one op unless its id was applied before, and records the id. Returns the `tempId`
/// this op created or named, with the playlist id it stands for.
fn apply(conn: &Connection, events: &mut Events, id: &str, at: i64, op: Op) -> Result<Option<(String, String)>> {
    let seen: Option<i64> = conn
        .query_row("SELECT 1 FROM collection_sync_ops WHERE id = ?1", [id], |r| r.get(0))
        .optional()?;
    if seen.is_some() {
        // A resend: answer as the first time, including a playlist it made.
        return match op {
            Op::PlaylistCreate { temp_id, .. } => Ok(temp_playlist(conn, &temp_id)?.map(|seq| named(temp_id, seq))),
            _ => Ok(None),
        };
    }
    let made = run(conn, events, at, op)?;
    conn.execute(
        "INSERT INTO collection_sync_ops (id, applied_at) VALUES (?1, ?2)",
        params![id, lull_core::now()],
    )?;
    Ok(made)
}

fn run(conn: &Connection, events: &mut Events, at: i64, op: Op) -> Result<Option<(String, String)>> {
    match op {
        Op::Play {
            track_id,
            started_at,
            played_s,
            completed,
            skipped,
        } => {
            let end = EndBody {
                played_s,
                completed,
                skipped,
            };
            plays::record(conn, events, &track_id, started_at, &end)?;
        }
        Op::Like { track_id, liked } => likes::set(conn, events, &track_id, liked, at)?,
        Op::Follow {
            kind,
            id,
            name,
            followed,
        } => follows::set(conn, events, kind, &id, name.as_deref(), followed, at)?,
        Op::PlaylistCreate {
            temp_id,
            name,
            desc,
            track_ids,
        } => {
            if let Some(seq) = temp_playlist(conn, &temp_id)? {
                return Err(Error::Conflict(format!("tempId {temp_id} already made pl-{seq}.")));
            }
            let seq = playlists::make(conn, &name, &desc, &track_ids, at)?;
            conn.execute(
                "INSERT INTO collection_sync_playlists (temp_id, playlist_seq, created_at) VALUES (?1, ?2, ?3)",
                params![temp_id, seq, lull_core::now()],
            )?;
            return Ok(Some(named(temp_id, seq)));
        }
        Op::PlaylistEdit {
            playlist_id,
            temp_id,
            name,
            desc,
            track_ids,
        } => {
            if name.is_none() && desc.is_none() && track_ids.is_none() {
                return Err(Error::BadRequest(
                    "A playlist edit changes name, desc or trackIds.".into(),
                ));
            }
            let (seq, made) = target(conn, playlist_id, temp_id)?;
            if name.is_some() || desc.is_some() {
                playlists::rename(conn, seq, name.as_deref(), desc.as_deref())?;
            }
            if let Some(ids) = track_ids {
                playlists::reorder(conn, seq, ids)?;
            }
            return Ok(made);
        }
        Op::PlaylistDelete { playlist_id, temp_id } => {
            let (seq, made) = target(conn, playlist_id, temp_id)?;
            // Already gone (deleted on another device) is what the listener wanted anyway.
            playlists::delete(conn, seq)?;
            return Ok(made);
        }
    }
    Ok(None)
}

/// The playlist an edit or delete names: a real id, or the `tempId` of an earlier create.
fn target(
    conn: &Connection,
    playlist_id: Option<String>,
    temp_id: Option<String>,
) -> Result<(i64, Option<(String, String)>)> {
    match (playlist_id, temp_id) {
        (Some(id), None) => Ok((seq_of(&id)?, None)),
        (None, Some(temp_id)) => match temp_playlist(conn, &temp_id)? {
            Some(seq) => Ok((seq, Some(named(temp_id, seq)))),
            None => Err(Error::NotFound(format!("No playlist was made for tempId {temp_id}."))),
        },
        _ => Err(Error::BadRequest("Name the playlist with playlistId or tempId.".into())),
    }
}

fn temp_playlist(conn: &Connection, temp_id: &str) -> Result<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT playlist_seq FROM collection_sync_playlists WHERE temp_id = ?1",
            [temp_id],
            |r| r.get(0),
        )
        .optional()?)
}

fn named(temp_id: String, seq: i64) -> (String, String) {
    (temp_id, format!("pl-{seq}"))
}

/// Drops op ids and `tempId`s older than `RETENTION`.
fn forget_old(conn: &mut Connection) -> Result<()> {
    let before = lull_core::now() - RETENTION;
    conn.execute("DELETE FROM collection_sync_ops WHERE applied_at < ?1", [before])?;
    conn.execute("DELETE FROM collection_sync_playlists WHERE created_at < ?1", [before])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing;

    fn batch(ops: Value) -> Vec<Value> {
        match ops {
            Value::Array(ops) => ops,
            other => vec![other],
        }
    }

    async fn count(svc: &Service, sql: &'static str) -> Result<i64> {
        svc.ctx.db.call(move |c| Ok(c.query_row(sql, [], |r| r.get(0))?)).await
    }

    #[tokio::test]
    async fn a_resent_batch_is_acknowledged_without_applying_twice() -> Result<()> {
        let (svc, mut rx) = testing::service().await?;
        let song = testing::known_song(&svc, "Velvet Orchard", "Paper Moon Static").await?;
        let started_at = lull_core::now() - 7200;
        let ops = json!([
            { "id": "op-1", "at": started_at, "op": "play", "trackId": song, "startedAt": started_at,
              "playedS": 200, "completed": true, "skipped": false },
            { "id": "op-2", "at": started_at, "op": "playlist.create", "tempId": "tmp-a", "name": "Road", "trackIds": [song] },
        ]);
        let first = apply_all(&svc, batch(ops.clone())).await?;
        let first_events = testing::published(&mut rx).len();
        let again = apply_all(&svc, batch(ops)).await?;

        assert_eq!(first["applied"], json!(["op-1", "op-2"]));
        assert_eq!(
            again, first,
            "a resend answers as the first time, playlist ids included"
        );
        assert_eq!(first_events, 1, "one play.ended");
        assert!(testing::published(&mut rx).is_empty(), "a resend publishes nothing");
        assert_eq!(count(&svc, "SELECT COUNT(*) FROM plays").await?, 1);
        assert_eq!(count(&svc, "SELECT COUNT(*) FROM collection_playlists").await?, 1);
        Ok(())
    }

    #[tokio::test]
    async fn ops_apply_in_order_and_a_bad_one_does_not_stop_the_rest() -> Result<()> {
        let (svc, mut rx) = testing::service().await?;
        let song = testing::known_song(&svc, "Velvet Orchard", "Paper Moon Static").await?;
        let other = testing::known_song(&svc, "Low Tide Radio", "Harbour Lights").await?;
        let at = lull_core::now() - 60;
        let ops = json!([
            { "id": "a", "at": at, "op": "playlist.create", "tempId": "tmp-1", "name": "Offline", "trackIds": [song] },
            { "id": "b", "at": at, "op": "like", "trackId": song, "liked": true },
            { "id": "c", "at": at, "op": "like", "trackId": "t-unknown", "liked": true },
            { "id": "d", "at": at, "op": "playlist.edit", "tempId": "tmp-1", "trackIds": [other, song] },
            { "id": "e", "at": at, "op": "like", "trackId": song, "liked": false },
            { "id": "f", "at": at, "op": "like", "trackId": song, "liked": true, "extra": 1 },
        ]);
        let out = apply_all(&svc, batch(ops)).await?;

        assert_eq!(out["applied"], json!(["a", "b", "d", "e"]));
        let rejected: Vec<&str> = out["rejected"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|r| r["id"].as_str())
            .collect();
        assert_eq!(rejected, ["c", "f"]);
        assert_eq!(out["playlists"], json!({ "tmp-1": "pl-1" }));
        // The like then the unlike, in that order: the song ends up not liked.
        assert_eq!(count(&svc, "SELECT COUNT(*) FROM likes").await?, 0);
        let likes: Vec<Value> = testing::published(&mut rx)
            .into_iter()
            .filter(|e| e.topic == "like")
            .map(|e| e.data["liked"].clone())
            .collect();
        assert_eq!(likes, [json!(true), json!(false)]);
        let order = svc
            .ctx
            .db
            .call(|c| {
                Ok(
                    c.prepare("SELECT track_id FROM collection_playlist_items ORDER BY pos")?
                        .query_map([], |r| r.get(0))?
                        .collect::<rusqlite::Result<Vec<String>>>()?,
                )
            })
            .await?;
        assert_eq!(order, [other.0, song.0]);
        Ok(())
    }
}
