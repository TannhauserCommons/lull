//! Playlists: create, rename, reorder, delete, the order they are shown in, M3U import preview
//! and M3U8 export. Shape: `MOCK.playlists` (`{ id, name, desc, created, updated, sort,
//! items: [{ trackId, added }] }`).
//!
//! `added` orders "Date added", so songs added together never share a second: they take
//! distinct, increasing seconds in the order given, ending at the moment they were added. A
//! source that knows when each song was added (Exportify, Deezer) gives its own dates.

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

use crate::{Events, Page, Service, bad_json, bad_query, entities, import, m3u};

pub(crate) fn routes() -> Router<Arc<Service>> {
    Router::new()
        .route("/api/playlists", get(list).post(create))
        .route("/api/playlists/import", post(import_preview))
        // `{id}.m3u8` too: the router has no suffix patterns, so `get_one` tells them apart.
        .route("/api/playlists/{id}", get(get_one).patch(edit).delete(remove))
        .route("/api/playlists/{id}/tracks", put(set_tracks))
        .route("/api/playlists/{id}/sort", put(set_sort))
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
    /// The playlist's own cover (`/api/playlists/{id}/cover?v=`), `null` for its songs' covers.
    cover: Option<String>,
    sort: Sort,
    items: Vec<Item>,
}

/// How the playlist is shown, kept on the hub so every device shows it the same way: by its own
/// order (`custom`; descending is that order reversed) or by a column. Showing it sorted never
/// changes the playlist's own order.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Sort {
    key: SortKey,
    dir: SortDir,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum SortKey {
    Custom,
    Added,
    Title,
    Artist,
    Album,
    Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum SortDir {
    Asc,
    Desc,
}

impl SortKey {
    const ALL: [(SortKey, &str); 6] = [
        (SortKey::Custom, "custom"),
        (SortKey::Added, "added"),
        (SortKey::Title, "title"),
        (SortKey::Artist, "artist"),
        (SortKey::Album, "album"),
        (SortKey::Duration, "duration"),
    ];
    fn name(self) -> &'static str {
        Self::ALL.iter().find(|(k, _)| *k == self).map_or("custom", |(_, n)| n)
    }
    /// A stored key this build doesn't know shows the playlist's own order.
    fn stored(name: &str) -> SortKey {
        Self::ALL
            .iter()
            .find(|(_, n)| *n == name)
            .map_or(SortKey::Custom, |(k, _)| *k)
    }
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
    /// When each song was added (unix seconds, `null` = not known), one per `trackIds` entry: an
    /// import preview's `addedAt`.
    added: Option<Vec<Option<i64>>>,
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
    let items: Vec<Dated> = match body.added {
        Some(added) if added.len() != body.track_ids.len() => {
            return Err(Error::BadRequest("\"added\" needs one entry per track id.".into()));
        }
        Some(added) => body.track_ids.into_iter().zip(added).collect(),
        None => undated(&body.track_ids),
    };
    let (name, desc) = (body.name, body.desc);
    let (seq, playlist) = svc
        .write(move |tx, events| {
            let seq = make_dated(tx, events, &name, &desc, &items, now)?;
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
    svc.write(move |tx, events| {
        change(tx, events, seq, body.name.as_deref(), body.desc.as_deref(), None)?;
        respond(tx, seq)
    })
    .await
}

async fn remove(State(svc): State<Arc<Service>>, Path(id): Path<String>) -> Result<StatusCode> {
    let seq = seq_of(&id)?;
    svc.write(move |tx, events| match delete(tx, events, seq)? {
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
    svc.write(move |tx, events| {
        change(tx, events, seq, None, None, Some(ids))?;
        respond(tx, seq)
    })
    .await
}

/// `PUT /api/playlists/{id}/sort` `{ key, dir }`: how this playlist is shown, on every device.
/// A view setting, not a change of the playlist: its order and `updated` stay.
async fn set_sort(
    State(svc): State<Arc<Service>>,
    Path(id): Path<String>,
    body: Result<Json<Sort>, JsonRejection>,
) -> Result<Json<Value>> {
    let seq = seq_of(&id)?;
    let Json(sort) = body.map_err(bad_json)?;
    svc.ctx
        .db
        .call(move |conn| {
            store_sort(conn, seq, sort)?;
            respond(conn, seq)
        })
        .await
}

pub(crate) fn store_sort(conn: &Connection, seq: i64, sort: Sort) -> Result<()> {
    let changed = conn.execute(
        "UPDATE collection_playlists SET sort_key = ?2, sort_desc = ?3 WHERE seq = ?1",
        params![seq, sort.key.name(), sort.dir == SortDir::Desc],
    )?;
    if changed == 0 {
        return Err(not_found(seq));
    }
    Ok(())
}

/// A song for a playlist and when it was added, when its source says.
pub(crate) type Dated = (TrackId, Option<i64>);

pub(crate) fn undated(ids: &[TrackId]) -> Vec<Dated> {
    ids.iter().map(|id| (id.clone(), None)).collect()
}

/// The `added` of songs added together at `at`: a date the source gives (never after `at`),
/// else distinct, increasing seconds in the given order, the last one at `at`.
pub(crate) fn dates(items: &[Dated], at: i64) -> Vec<(TrackId, i64)> {
    let last = items.len().saturating_sub(1) as i64;
    items
        .iter()
        .enumerate()
        .map(|(i, (id, added))| (id.clone(), added.map_or(at - (last - i as i64), |d| d.clamp(0, at))))
        .collect()
}

/// Creates a playlist of songs added together at `at` and returns its seq.
pub(crate) fn make(
    conn: &Connection,
    events: &mut Events,
    name: &str,
    desc: &str,
    ids: &[TrackId],
    at: i64,
) -> Result<i64> {
    make_dated(conn, events, name, desc, &undated(ids), at)
}

/// `make`, with the dates a source gives (see `dates`).
pub(crate) fn make_dated(
    conn: &Connection,
    events: &mut Events,
    name: &str,
    desc: &str,
    items: &[Dated],
    at: i64,
) -> Result<i64> {
    let (name, desc) = (valid_name(name)?, valid_desc(desc)?);
    let ids: Vec<TrackId> = items.iter().map(|(id, _)| id.clone()).collect();
    check_tracks(conn, &ids)?;
    conn.execute(
        "INSERT INTO collection_playlists (name, description, created_at, updated_at) VALUES (?1, ?2, ?3, ?3)",
        params![name, desc, at],
    )?;
    let seq = conn.last_insert_rowid();
    write_items(conn, seq, &dates(items, at))?;
    announce(events, seq, false);
    Ok(seq)
}

/// Sets every song of a playlist with its date, as a source lists them (a link's re-import):
/// unlike `change`, nothing keeps the date it had.
pub(crate) fn refill(conn: &Connection, events: &mut Events, seq: i64, items: &[Dated], at: i64) -> Result<()> {
    if items_of(conn, seq)?.is_none() {
        return Err(not_found(seq));
    }
    let ids: Vec<TrackId> = items.iter().map(|(id, _)| id.clone()).collect();
    check_tracks(conn, &ids)?;
    replace_items(conn, seq, &dates(items, at), lull_core::now())?;
    announce(events, seq, false);
    Ok(())
}

/// Every change of a playlist publishes `playlist { playlistId, deleted }`, once per operation:
/// library mirrors playlists into Jellyfin from it.
pub(crate) fn announce(events: &mut Events, seq: i64, deleted: bool) {
    events.push(
        "playlist",
        json!({ "playlistId": format!("pl-{seq}"), "deleted": deleted }),
    );
}

/// Changes the name, the description and/or the songs (the whole new order, see `reorder_rows`);
/// `None` keeps what is there.
pub(crate) fn change(
    conn: &Connection,
    events: &mut Events,
    seq: i64,
    name: Option<&str>,
    desc: Option<&str>,
    ids: Option<Vec<TrackId>>,
) -> Result<()> {
    rename_row(conn, seq, name, desc)?;
    if let Some(ids) = ids {
        reorder_rows(conn, seq, ids)?;
    }
    announce(events, seq, false);
    Ok(())
}

/// Sets the name and/or description; also checks that the playlist exists.
fn rename_row(conn: &Connection, seq: i64, name: Option<&str>, desc: Option<&str>) -> Result<()> {
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
pub(crate) fn delete(conn: &Connection, events: &mut Events, seq: i64) -> Result<bool> {
    if conn.execute("DELETE FROM collection_playlists WHERE seq = ?1", [seq])? == 0 {
        return Ok(false);
    }
    conn.execute("DELETE FROM collection_playlist_items WHERE playlist_seq = ?1", [seq])?;
    announce(events, seq, true);
    Ok(true)
}

/// The whole new order: adds, removes and moves in one call. A song that stays keeps its
/// `added` date (the n-th copy of a song keeps the n-th copy's date); the new ones are added now,
/// in the order given.
fn reorder_rows(conn: &Connection, seq: i64, ids: Vec<TrackId>) -> Result<()> {
    let Some(old) = items_of(conn, seq)? else {
        return Err(not_found(seq));
    };
    check_tracks(conn, &ids)?;
    let now = lull_core::now();
    let mut old: Vec<Option<Item>> = old.into_iter().map(Some).collect();
    let kept: Vec<Option<i64>> = ids
        .iter()
        .map(|id| {
            old.iter_mut()
                .find(|o| o.as_ref().is_some_and(|o| o.track_id == *id))
                .and_then(Option::take)
                .map(|o| o.added)
        })
        .collect();
    // The new songs are added together, spread as `dates` spreads them.
    let mut left = kept.iter().filter(|k| k.is_none()).count() as i64;
    let items: Vec<(TrackId, i64)> = ids
        .into_iter()
        .zip(kept)
        .map(|(id, kept)| match kept {
            Some(added) => (id, added),
            None => {
                left -= 1;
                (id, now - left)
            }
        })
        .collect();
    replace_items(conn, seq, &items, now)
}

/// The `Collection` seam's edit: removals are positions before the change, applied first;
/// additions go to the end.
pub(crate) fn edit_positions(conn: &Connection, events: &mut Events, seq: i64, edit: PlaylistEdit) -> Result<()> {
    if edit.name.is_some() || edit.description.is_some() {
        rename_row(conn, seq, edit.name.as_deref(), edit.description.as_deref())?;
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
        .chain(dates(&undated(&edit.add), now))
        .collect();
    if items.len() > MAX_ITEMS {
        return Err(too_many());
    }
    replace_items(conn, seq, &items, now)?;
    announce(events, seq, false);
    Ok(())
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

pub(crate) fn respond(conn: &Connection, seq: i64) -> Result<Json<Value>> {
    let playlist = load(conn, seq)?.ok_or_else(|| not_found(seq))?;
    let entities = entities(conn, playlist.items.iter().map(|i| &i.track_id))?;
    let mut body = serde_json::to_value(playlist).map_err(|e| Error::internal(format!("playlist json: {e}")))?;
    body["entities"] = json!(entities);
    Ok(Json(body))
}

fn load(conn: &Connection, seq: i64) -> Result<Option<Playlist>> {
    let head = conn
        .query_row(
            "SELECT name, description, created_at, updated_at, cover_at, sort_key, sort_desc
             FROM collection_playlists WHERE seq = ?1",
            [seq],
            |r| {
                let sort = Sort {
                    key: SortKey::stored(&r.get::<_, String>(5)?),
                    dir: if r.get(6)? { SortDir::Desc } else { SortDir::Asc },
                };
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, sort))
            },
        )
        .optional()?;
    let Some((name, desc, created, updated, cover_at, sort)) = head else {
        return Ok(None);
    };
    let items = items_of(conn, seq)?.unwrap_or_default();
    Ok(Some(Playlist {
        id: format!("pl-{seq}"),
        name,
        desc,
        created,
        updated,
        cover: crate::covers::url(seq, cover_at),
        sort,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{known_song, service};

    async fn three_songs(svc: &Service) -> Result<Vec<TrackId>> {
        Ok(vec![
            known_song(svc, "Velvet Orchard", "Paper Moon Static").await?,
            known_song(svc, "Low Tide Radio", "Harbour Lights").await?,
            known_song(svc, "Moth & Lantern", "Porchlight").await?,
        ])
    }

    fn added(body: &Value) -> Vec<i64> {
        body["items"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|i| i["added"].as_i64())
            .collect()
    }

    #[tokio::test]
    async fn songs_added_together_get_distinct_increasing_dates_in_their_order() -> Result<()> {
        let (svc, _rx) = service().await?;
        let ids = three_songs(&svc).await?;
        let seq = svc
            .write(move |tx, events| make(tx, events, "Porch", "", &ids, 1_000))
            .await?;
        let Json(body) = svc.ctx.db.call(move |c| respond(c, seq)).await?;
        assert_eq!(
            added(&body),
            [998, 999, 1_000],
            "so \"Date added\" keeps the order they came in"
        );
        Ok(())
    }

    #[tokio::test]
    async fn an_imports_own_dates_are_kept_and_never_in_the_future() -> Result<()> {
        let (svc, _rx) = service().await?;
        let ids = three_songs(&svc).await?;
        let now = lull_core::now();
        let body = CreateBody {
            name: "From Spotify".into(),
            desc: String::new(),
            track_ids: ids,
            added: Some(vec![Some(1_614_883_160), None, Some(now + 86_400)]),
            imported: None,
        };
        let (_, Json(body)) = create(State(svc.clone()), Ok(Json(body))).await?;
        let dates = added(&body);
        assert_eq!(dates[0], 1_614_883_160, "Exportify's Added At");
        assert!(
            dates[1] >= now - 1 && dates[1] < dates[2],
            "an unknown date is the import's: {dates:?}"
        );
        assert!(
            dates[2] <= lull_core::now(),
            "a date ahead of the clock is clamped: {dates:?}"
        );

        let mismatched = CreateBody {
            name: "Odd".into(),
            desc: String::new(),
            track_ids: three_songs(&svc).await?,
            added: Some(vec![None]),
            imported: None,
        };
        assert!(matches!(
            create(State(svc.clone()), Ok(Json(mismatched))).await,
            Err(Error::BadRequest(_))
        ));
        Ok(())
    }

    #[tokio::test]
    async fn a_playlists_sort_is_kept_without_changing_its_order_or_when_it_changed() -> Result<()> {
        let (svc, _rx) = service().await?;
        let ids = three_songs(&svc).await?;
        let seq = svc
            .write(move |tx, events| make(tx, events, "Porch", "", &ids, 1_000))
            .await?;
        let id = format!("pl-{seq}");
        let Json(before) = svc.ctx.db.call(move |c| respond(c, seq)).await?;
        assert_eq!(
            before["sort"],
            json!({ "key": "custom", "dir": "asc" }),
            "its own order by default"
        );

        let sort: Sort = serde_json::from_value(json!({ "key": "added", "dir": "desc" }))
            .map_err(|e| Error::internal(e.to_string()))?;
        let Json(after) = set_sort(State(svc.clone()), Path(id.clone()), Ok(Json(sort))).await?;
        assert_eq!(after["sort"], json!({ "key": "added", "dir": "desc" }));
        assert_eq!(after["items"], before["items"], "the playlist's own order stays");
        assert_eq!(
            after["updated"], before["updated"],
            "a view setting is not a change of the playlist"
        );
        let Json(again) = svc.ctx.db.call(move |c| respond(c, seq)).await?;
        assert_eq!(again["sort"], after["sort"], "kept on the hub for every device");

        for wrong in [
            json!({ "key": "plays", "dir": "asc" }),
            json!({ "key": "title", "dir": "up" }),
            json!({ "key": "title" }),
        ] {
            assert!(serde_json::from_value::<Sort>(wrong.clone()).is_err(), "{wrong}");
        }
        assert!(matches!(
            set_sort(State(svc.clone()), Path("pl-999".into()), Ok(Json(sort))).await,
            Err(Error::NotFound(_))
        ));
        Ok(())
    }

    #[tokio::test]
    async fn songs_an_older_build_dated_alike_take_distinct_seconds_in_playlist_order() -> Result<()> {
        let db = lull_core::db::Db::memory()?;
        let steps = crate::MIGRATIONS.steps;
        db.migrate(&lull_core::db::Migrations {
            owner: crate::MIGRATIONS.owner,
            steps: &steps[..steps.len() - 1],
        })?;
        db.call(|c| {
            c.execute_batch(
                "INSERT INTO collection_playlists (name, description, created_at, updated_at) VALUES ('Old import', '', 500, 500);
                 INSERT INTO collection_playlist_items VALUES (1, 0, 't-a', 500), (1, 1, 't-b', 500), (1, 2, 't-c', 500), (1, 3, 't-d', 900);",
            )?;
            Ok(())
        })
        .await?;
        db.migrate(&crate::MIGRATIONS)?;
        let dates: Vec<i64> = db
            .call(|c| {
                let mut q = c.prepare("SELECT added_at FROM collection_playlist_items ORDER BY pos")?;
                let dates = q.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
                Ok(dates)
            })
            .await?;
        assert_eq!(
            dates,
            [498, 499, 500, 900],
            "an import's songs keep its order under \"Date added\""
        );
        Ok(())
    }

    #[tokio::test]
    async fn new_songs_in_a_reorder_are_dated_now_and_kept_songs_keep_their_dates() -> Result<()> {
        let (svc, _rx) = service().await?;
        let ids = three_songs(&svc).await?;
        let first = vec![ids[0].clone()];
        let seq = svc
            .write(move |tx, events| make(tx, events, "Porch", "", &first, 1_000))
            .await?;
        let order = vec![ids[1].clone(), ids[0].clone(), ids[2].clone()];
        let Json(body) = svc
            .write(move |tx, events| {
                change(tx, events, seq, None, None, Some(order))?;
                respond(tx, seq)
            })
            .await?;
        let dates = added(&body);
        assert_eq!(dates[1], 1_000, "the song that stayed keeps its date");
        assert!(
            dates[0] < dates[2] && dates[2] <= lull_core::now(),
            "added together, in order: {dates:?}"
        );
        Ok(())
    }
}
