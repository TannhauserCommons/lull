//! Playlists: create, rename, reorder, delete, M3U import preview and M3U8 export. Shape:
//! `MOCK.playlists` (`{ id, name, desc, created, updated, items: [{ trackId, added }] }`).

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use lull_core::ids::TrackId;
use lull_core::model::PlaylistEdit;
use lull_core::{Error, Result};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{Page, Service, bad_json, bad_query, entities, import, m3u};

pub(crate) fn routes() -> Router<Arc<Service>> {
    Router::new()
        .route("/api/playlists", get(list).post(create))
        .route("/api/playlists/import", post(import_preview))
        // `{id}.m3u8` too: the router has no suffix patterns, so `get_one` tells them apart.
        .route("/api/playlists/{id}", get(get_one).patch(edit).delete(remove))
        .route("/api/playlists/{id}/tracks", put(set_tracks))
}

const MAX_NAME: usize = 100;
const MAX_DESC: usize = 1000;
const MAX_ITEMS: usize = 10_000;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Playlist {
    id: String,
    name: String,
    desc: String,
    created: i64,
    updated: i64,
    items: Vec<Item>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Item {
    track_id: TrackId,
    added: i64,
}

async fn list(State(svc): State<Arc<Service>>, page: Result<Query<Page>, QueryRejection>) -> Result<Json<Value>> {
    let (limit, offset) = page.map_err(bad_query)?.bounds(200)?;
    svc.ctx
        .db
        .call(move |conn| {
            let total: i64 = conn.query_row("SELECT COUNT(*) FROM collection_playlists", [], |r| r.get(0))?;
            let seqs = conn
                .prepare("SELECT seq FROM collection_playlists ORDER BY updated_at DESC, seq DESC LIMIT ?1 OFFSET ?2")?
                .query_map(params![limit, offset], |r| r.get(0))?
                .collect::<rusqlite::Result<Vec<i64>>>()?;
            let mut items = Vec::with_capacity(seqs.len());
            for seq in seqs {
                items.extend(load(conn, seq)?);
            }
            let entities = entities(conn, items.iter().flat_map(|p| p.items.iter().map(|i| &i.track_id)))?;
            Ok(Json(json!({ "items": items, "total": total, "entities": entities })))
        })
        .await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateBody {
    name: String,
    #[serde(default)]
    desc: String,
    #[serde(default)]
    track_ids: Vec<TrackId>,
    /// Set when the playlist comes from an M3U import preview, so the import shows in Activity.
    imported: Option<Imported>,
}

/// The preview's file name and counts (`POST /api/playlists/import`).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Imported {
    file: String,
    library: u32,
    will_search: u32,
    not_found: u32,
}

async fn create(
    State(svc): State<Arc<Service>>,
    body: Result<Json<CreateBody>, JsonRejection>,
) -> Result<(StatusCode, Json<Value>)> {
    let Json(body) = body.map_err(bad_json)?;
    let now = lull_core::now();
    let (seq, playlist) = svc
        .write(move |tx, _| {
            let seq = make(tx, &body.name, &body.desc, &body.track_ids, now)?;
            Ok((seq, respond(tx, seq)?))
        })
        .await?;
    if let Some(i) = body.imported {
        // `MOCK.activity`'s import row; library's /api/jobs lists the latest event of each job.
        svc.ctx.bus.publish(
            "job",
            json!({
                "jobId": format!("job-import-{seq}"), "kind": "import", "name": i.file, "playlistId": format!("pl-{seq}"),
                "matched": i.library, "search": i.will_search, "notFound": i.not_found, "state": "done", "started": now,
            }),
        );
    }
    Ok((StatusCode::CREATED, playlist))
}

/// `GET /api/playlists/{id}` (JSON) or `GET /api/playlists/{id}.m3u8` (export).
async fn get_one(State(svc): State<Arc<Service>>, Path(id): Path<String>, headers: HeaderMap) -> Result<Response> {
    if let Some(id) = id.strip_suffix(".m3u8") {
        return export(&svc, seq_of(id)?, &headers).await;
    }
    let seq = seq_of(&id)?;
    svc.ctx
        .db
        .call(move |conn| Ok(respond(conn, seq)?.into_response()))
        .await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EditBody {
    name: Option<String>,
    desc: Option<String>,
}

async fn edit(
    State(svc): State<Arc<Service>>,
    Path(id): Path<String>,
    body: Result<Json<EditBody>, JsonRejection>,
) -> Result<Json<Value>> {
    let seq = seq_of(&id)?;
    let Json(body) = body.map_err(bad_json)?;
    svc.write(move |tx, _| {
        rename(tx, seq, body.name.as_deref(), body.desc.as_deref())?;
        respond(tx, seq)
    })
    .await
}

async fn remove(State(svc): State<Arc<Service>>, Path(id): Path<String>) -> Result<StatusCode> {
    let seq = seq_of(&id)?;
    svc.write(move |tx, _| match delete(tx, seq)? {
        true => Ok(StatusCode::NO_CONTENT),
        false => Err(not_found(seq)),
    })
    .await
}

async fn set_tracks(
    State(svc): State<Arc<Service>>,
    Path(id): Path<String>,
    body: Result<Json<Vec<TrackId>>, JsonRejection>,
) -> Result<Json<Value>> {
    let seq = seq_of(&id)?;
    let Json(ids) = body.map_err(bad_json)?;
    svc.write(move |tx, _| {
        reorder(tx, seq, ids)?;
        respond(tx, seq)
    })
    .await
}

/// Creates a playlist at `at` and returns its seq.
pub(crate) fn make(conn: &Connection, name: &str, desc: &str, ids: &[TrackId], at: i64) -> Result<i64> {
    let (name, desc) = (valid_name(name)?, valid_desc(desc)?);
    check_tracks(conn, ids)?;
    conn.execute(
        "INSERT INTO collection_playlists (name, description, created_at, updated_at) VALUES (?1, ?2, ?3, ?3)",
        params![name, desc, at],
    )?;
    let seq = conn.last_insert_rowid();
    write_items(conn, seq, &ids.iter().map(|id| (id.clone(), at)).collect::<Vec<_>>())?;
    Ok(seq)
}

/// Changes the name and/or description; `None` keeps what is there.
pub(crate) fn rename(conn: &Connection, seq: i64, name: Option<&str>, desc: Option<&str>) -> Result<()> {
    let name = name.map(valid_name).transpose()?;
    let desc = desc.map(valid_desc).transpose()?;
    let changed = conn.execute(
        "UPDATE collection_playlists SET name = COALESCE(?2, name), description = COALESCE(?3, description),
         updated_at = ?4 WHERE seq = ?1",
        params![seq, name, desc, lull_core::now()],
    )?;
    if changed == 0 {
        return Err(not_found(seq));
    }
    Ok(())
}

/// Deletes a playlist; false when there was none.
pub(crate) fn delete(conn: &Connection, seq: i64) -> Result<bool> {
    if conn.execute("DELETE FROM collection_playlists WHERE seq = ?1", [seq])? == 0 {
        return Ok(false);
    }
    conn.execute("DELETE FROM collection_playlist_items WHERE playlist_seq = ?1", [seq])?;
    Ok(true)
}

/// The whole new order: adds, removes and moves in one call. A song that stays keeps its
/// `added` date (the n-th copy of a song keeps the n-th copy's date).
pub(crate) fn reorder(conn: &Connection, seq: i64, ids: Vec<TrackId>) -> Result<()> {
    let Some(old) = items_of(conn, seq)? else {
        return Err(not_found(seq));
    };
    check_tracks(conn, &ids)?;
    let now = lull_core::now();
    let mut old: Vec<Option<Item>> = old.into_iter().map(Some).collect();
    let items: Vec<(TrackId, i64)> = ids
        .into_iter()
        .map(|id| {
            let kept = old
                .iter_mut()
                .find(|o| o.as_ref().is_some_and(|o| o.track_id == id))
                .and_then(Option::take);
            (id, kept.map_or(now, |o| o.added))
        })
        .collect();
    replace_items(conn, seq, &items, now)
}

/// The `Collection` seam's edit: removals are positions before the change, applied first;
/// additions go to the end.
pub(crate) fn edit_positions(conn: &Connection, seq: i64, edit: PlaylistEdit) -> Result<()> {
    if edit.name.is_some() || edit.description.is_some() {
        rename(conn, seq, edit.name.as_deref(), edit.description.as_deref())?;
    }
    let Some(old) = items_of(conn, seq)? else {
        return Err(not_found(seq));
    };
    if let Some(p) = edit.remove_positions.iter().find(|p| **p >= old.len()) {
        return Err(Error::BadRequest(format!("Playlist pl-{seq} has no position {p}.")));
    }
    check_tracks(conn, &edit.add)?;
    let now = lull_core::now();
    let items: Vec<(TrackId, i64)> = old
        .into_iter()
        .enumerate()
        .filter(|(pos, _)| !edit.remove_positions.contains(pos))
        .map(|(_, i)| (i.track_id, i.added))
        .chain(edit.add.into_iter().map(|id| (id, now)))
        .collect();
    if items.len() > MAX_ITEMS {
        return Err(too_many());
    }
    replace_items(conn, seq, &items, now)
}

/// Every playlist for the `Collection` seam, most recently changed first.
pub(crate) fn all(conn: &Connection) -> Result<Vec<lull_core::model::Playlist>> {
    let seqs = conn
        .prepare("SELECT seq FROM collection_playlists ORDER BY updated_at DESC, seq DESC")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<i64>>>()?;
    let mut out = Vec::with_capacity(seqs.len());
    for seq in seqs {
        out.extend(one(conn, seq)?);
    }
    Ok(out)
}

/// One playlist in the `Collection` seam's shape.
pub(crate) fn one(conn: &Connection, seq: i64) -> Result<Option<lull_core::model::Playlist>> {
    Ok(load(conn, seq)?.map(|p| lull_core::model::Playlist {
        id: p.id,
        name: p.name,
        description: p.desc,
        track_ids: p.items.into_iter().map(|i| i.track_id).collect(),
        created_at: p.created,
        updated_at: p.updated,
    }))
}

/// The M3U file is the request body; the answer previews each line (see `import`).
async fn import_preview(State(svc): State<Arc<Service>>, body: Bytes) -> Result<Json<Value>> {
    import::preview(&svc, &body).await.map(Json)
}

/// M3U8 with a stream link per song: the library's for saved songs, Soulseek streaming otherwise.
/// Links are absolute so the file works outside lull: `LULL_PUBLIC_URL`, else the request's host.
async fn export(svc: &Service, seq: i64, headers: &HeaderMap) -> Result<Response> {
    let base = match &svc.ctx.config.public_url {
        Some(url) => url.clone(),
        None => {
            let host = headers
                .get(header::HOST)
                .and_then(|h| h.to_str().ok())
                .ok_or_else(|| Error::BadRequest("Set LULL_PUBLIC_URL to export playlists.".into()))?;
            format!("http://{host}")
        }
    };
    let (name, text) = svc
        .ctx
        .db
        .call(move |conn| {
            let playlist = load(conn, seq)?.ok_or_else(|| not_found(seq))?;
            let tracks = playlist
                .items
                .iter()
                .filter_map(|i| lull_core::catalog::track(conn, &i.track_id).transpose())
                .collect::<Result<Vec<_>>>()?;
            let text = m3u::export(
                &playlist.name,
                tracks.iter().map(|t| {
                    let path = if t.saved { "api/library/stream" } else { "api/stream" };
                    (t, format!("{base}/{path}/{}", t.id))
                }),
            );
            Ok((playlist.name, text))
        })
        .await?;
    let file: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || " -_.()".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    Ok((
        [
            (header::CONTENT_TYPE, "audio/x-mpegurl; charset=utf-8".to_owned()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{}.m3u8\"", file.trim()),
            ),
        ],
        text,
    )
        .into_response())
}

fn respond(conn: &Connection, seq: i64) -> Result<Json<Value>> {
    let playlist = load(conn, seq)?.ok_or_else(|| not_found(seq))?;
    let entities = entities(conn, playlist.items.iter().map(|i| &i.track_id))?;
    let mut body = serde_json::to_value(playlist).map_err(|e| Error::internal(format!("playlist json: {e}")))?;
    body["entities"] = json!(entities);
    Ok(Json(body))
}

fn load(conn: &Connection, seq: i64) -> Result<Option<Playlist>> {
    let head = conn
        .query_row(
            "SELECT name, description, created_at, updated_at FROM collection_playlists WHERE seq = ?1",
            [seq],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let Some((name, desc, created, updated)) = head else {
        return Ok(None);
    };
    let items = items_of(conn, seq)?.unwrap_or_default();
    Ok(Some(Playlist {
        id: format!("pl-{seq}"),
        name,
        desc,
        created,
        updated,
        items,
    }))
}

/// The items in order, or `None` when there is no such playlist.
fn items_of(conn: &Connection, seq: i64) -> Result<Option<Vec<Item>>> {
    let exists: Option<i64> = conn
        .query_row("SELECT seq FROM collection_playlists WHERE seq = ?1", [seq], |r| {
            r.get(0)
        })
        .optional()?;
    if exists.is_none() {
        return Ok(None);
    }
    let items = conn
        .prepare_cached(
            "SELECT track_id, added_at FROM collection_playlist_items WHERE playlist_seq = ?1 ORDER BY pos",
        )?
        .query_map([seq], |r| {
            Ok(Item {
                track_id: TrackId(r.get(0)?),
                added: r.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Some(items))
}

fn replace_items(conn: &Connection, seq: i64, items: &[(TrackId, i64)], now: i64) -> Result<()> {
    conn.execute("DELETE FROM collection_playlist_items WHERE playlist_seq = ?1", [seq])?;
    write_items(conn, seq, items)?;
    conn.execute(
        "UPDATE collection_playlists SET updated_at = ?2 WHERE seq = ?1",
        params![seq, now],
    )?;
    Ok(())
}

fn write_items(conn: &Connection, seq: i64, items: &[(TrackId, i64)]) -> Result<()> {
    let mut stmt = conn.prepare_cached(
        "INSERT INTO collection_playlist_items (playlist_seq, pos, track_id, added_at) VALUES (?1, ?2, ?3, ?4)",
    )?;
    for (pos, (id, added)) in items.iter().enumerate() {
        stmt.execute(params![seq, pos as i64, id.as_str(), added])?;
    }
    Ok(())
}

/// Every id must resolve, so a playlist never holds a song the UI can't show.
fn check_tracks(conn: &Connection, ids: &[TrackId]) -> Result<()> {
    if ids.len() > MAX_ITEMS {
        return Err(too_many());
    }
    for id in ids {
        if lull_core::catalog::track(conn, id)?.is_none() {
            return Err(Error::BadRequest(format!("No track {id} is known to lull.")));
        }
    }
    Ok(())
}

fn too_many() -> Error {
    Error::BadRequest(format!("A playlist holds at most {MAX_ITEMS} songs."))
}

pub(crate) fn seq_of(id: &str) -> Result<i64> {
    id.strip_prefix("pl-")
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| Error::NotFound(format!("No playlist {id}.")))
}

fn not_found(seq: i64) -> Error {
    Error::NotFound(format!("No playlist pl-{seq}."))
}

fn valid_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME {
        return Err(Error::BadRequest(format!(
            "A playlist name is 1 to {MAX_NAME} characters."
        )));
    }
    Ok(name.to_owned())
}

fn valid_desc(desc: &str) -> Result<String> {
    let desc = desc.trim();
    if desc.chars().count() > MAX_DESC {
        return Err(Error::BadRequest(format!(
            "A description is at most {MAX_DESC} characters."
        )));
    }
    Ok(desc.to_owned())
}
