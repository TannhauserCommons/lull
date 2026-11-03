//! Outside playlists (docs/context/collection.md#outside-playlists): someone's playlist on
//! YouTube or Deezer (any link lull imports from), opened from search. It previews like an album
//! page without becoming a lull playlist, and can be bookmarked, as following a playlist on
//! Spotify: the hub keeps only its link and what it showed.
//!
//! - **Read once an hour:** a listing is kept in `collection_outside` and read again from the
//!   source when opened after `FRESH_FOR`; if the source fails then, the kept one is shown.
//! - **Playable at once:** every song has a track id from the start. One the catalog matched
//!   (`matches`) plays as that song; one not matched yet, or not at all, plays as the link names
//!   it (searched on Soulseek, or YouTube, by artist and title), and says so.
//! - **Matched in the background:** songs not matched yet are looked up after the answer, a few
//!   at a time, and each batch goes out as an `outside` event the page applies.
//! - **Pictures** are fetched once a listing is read and kept on the hub, so the CSP stays
//!   same-origin and a bookmark's picture survives the source.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::get;
use axum::{Json, Router};
use futures_util::{StreamExt, stream};
use lull_core::ids::TrackId;
use lull_core::{Ctx, Error, Result, catalog};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::links::parse::{self, Link, Parsed};
use crate::links::{self, Song};
use crate::{Service, bad_query, covers, entities, import, matches};

/// A listing is read from its source again when opened after this long (seconds).
const FRESH_FOR: i64 = 3600;
/// A listing nobody bookmarked, and its picture, is forgotten after this long (seconds).
const KEEP_FOR: i64 = 30 * 86_400;
/// How long a preview waits for the picture before answering without it.
const PICTURE_WAIT: Duration = Duration::from_secs(4);
/// Songs looked up at once (the catalog paces itself), and how often a batch goes out.
const PARALLEL: usize = 4;
const REPORT_EVERY: Duration = Duration::from_millis(1000);

pub(crate) fn routes() -> Router<Arc<Service>> {
    Router::new()
        .route("/api/outside", get(preview))
        .route("/api/outside/cover", get(cover))
        .route("/api/bookmarks", get(list).put(save).delete(unsave))
}

#[derive(Deserialize)]
struct LinkQuery {
    /// A link key (`youtube:playlist:<id>`, as search gives) or a playlist's link.
    link: String,
    size: Option<u32>,
}

/// The playlist a key or a pasted link names.
fn link_of(text: &str) -> Result<Link> {
    if let Ok(link) = Link::from_key(text.trim()) {
        return Ok(link);
    }
    match parse::parse(text)? {
        Parsed::Link(link) => Ok(link),
        Parsed::Short(_) => Err(Error::BadRequest(
            "Open the short link once and use the playlist's own link.".into(),
        )),
    }
}

/// A listing as kept: what the source showed when it was last read.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Kept {
    name: String,
    owner: Option<String>,
    description: Option<String>,
    note: Option<String>,
    songs: Vec<Song>,
    fetched_at: i64,
    /// When its picture was stored (its URL's version); `None` without one.
    cover_at: Option<i64>,
}

// ---------------------------------------------------------------------------------------------
// Preview

/// `GET /api/outside?link=`: the playlist as an album page shows it, each song with its status
/// (`matched`, `pending`, `unmatched`) and a playable track id, plus `bookmarked`.
async fn preview(
    State(svc): State<Arc<Service>>,
    query: std::result::Result<Query<LinkQuery>, QueryRejection>,
) -> Result<Json<Value>> {
    let Query(q) = query.map_err(bad_query)?;
    let link = link_of(&q.link)?;
    let kept = listing(&svc, &link).await?;
    let keys: Vec<String> = kept.songs.iter().map(|s| s.key.clone()).collect();
    let known = matches::known(&svc, keys).await?;
    let pending: Vec<(usize, Song)> = kept
        .songs
        .iter()
        .enumerate()
        .filter(|(_, s)| !known.contains_key(&s.key))
        .map(|(i, s)| (i, s.clone()))
        .collect();
    let key = link.key();
    if !pending.is_empty() {
        start_matching(&svc, key.clone(), pending);
    }
    let matching = svc.previewing.lock().is_ok_and(|p| p.contains(&key));
    let body = svc
        .ctx
        .db
        .call(move |c| {
            let rows: Vec<Row> = kept
                .songs
                .iter()
                .enumerate()
                .map(|(pos, s)| row(c, pos, s, known.get(&s.key).cloned()))
                .collect::<Result<_>>()?;
            let bookmarked = bookmarked(c, &key)?;
            let entities = entities(c, rows.iter().map(|r| &r.track_id))?;
            let duration: u32 = kept.songs.iter().filter_map(|s| s.duration).sum();
            Ok(json!({
                "id": key, "url": link.url(), "site": link.site.name(), "name": kept.name,
                "owner": kept.owner, "description": kept.description, "note": kept.note,
                "cover": cover_url(&key, kept.cover_at), "total": rows.len(), "duration": duration,
                "fetchedAt": kept.fetched_at, "bookmarked": bookmarked, "matching": matching,
                "songs": rows, "entities": entities,
            }))
        })
        .await?;
    Ok(Json(body))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum Status {
    /// The catalog's song.
    Matched,
    /// Not looked up yet: plays as the link names it meanwhile.
    Pending,
    /// The catalog has no match: plays as the link names it.
    Unmatched,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Row {
    pos: usize,
    artist: String,
    title: String,
    status: Status,
    /// Always set: the match, else the song as the link names it.
    track_id: TrackId,
}

/// A song's row; one without a match is recorded in the catalog as the link names it, so its id
/// resolves and plays search Soulseek by artist and title (as an M3U line does).
fn row(c: &Connection, pos: usize, s: &Song, known: Option<Option<TrackId>>) -> Result<Row> {
    let (status, matched) = match known {
        Some(Some(id)) => (Status::Matched, Some(id)),
        Some(None) => (Status::Unmatched, None),
        None => (Status::Pending, None),
    };
    let track_id = match matched {
        Some(id) => id,
        None => named(c, s)?,
    };
    Ok(Row {
        pos,
        artist: s.artist.clone(),
        title: s.title.clone(),
        status,
        track_id,
    })
}

fn named(c: &Connection, s: &Song) -> Result<TrackId> {
    let track = import::unsaved_track(&s.artist, s.album.as_deref().unwrap_or(""), &s.title, s.duration);
    let id = track.id.clone();
    if catalog::track(c, &id)?.is_none() {
        catalog::remember(c, &[track])?;
    }
    Ok(id)
}

/// The kept listing, read from the source first when it is older than `FRESH_FOR` (or there is
/// none). A source that fails then leaves the kept one, if any.
async fn listing(svc: &Service, link: &Link) -> Result<Kept> {
    let key = link.key();
    let kept = load(svc, key.clone()).await?;
    let now = lull_core::now();
    if let Some(k) = &kept
        && now - k.fetched_at < FRESH_FOR
    {
        return Ok(k.clone());
    }
    let read = match links::fetch(svc, link).await {
        Ok(read) => read,
        Err(e) => {
            return match kept {
                Some(k) => {
                    tracing::info!(error = %e, link = %key, "an outside playlist is shown as kept");
                    Ok(k)
                }
                None => Err(e),
            };
        }
    };
    if read.songs.is_empty() {
        return Err(Error::BadRequest("That playlist has no songs lull can read.".into()));
    }
    let picture = tokio::time::timeout(PICTURE_WAIT, covers::first_picture(svc, &read.covers)).await;
    let cover_at = match picture {
        Ok(Ok((jpeg, _))) => match write_picture(&svc.ctx, &key, &jpeg).await {
            Ok(()) => Some(now),
            Err(e) => {
                tracing::warn!(error = %e, "an outside playlist's picture wasn't kept");
                kept.as_ref().and_then(|k| k.cover_at)
            }
        },
        _ => kept.as_ref().and_then(|k| k.cover_at),
    };
    let fresh = Kept {
        name: read.name,
        owner: read.owner,
        description: read.description,
        note: read.note,
        songs: read.songs,
        fetched_at: now,
        cover_at,
    };
    store(svc, key, fresh.clone()).await?;
    Ok(fresh)
}

async fn load(svc: &Service, key: String) -> Result<Option<Kept>> {
    svc.ctx
        .db
        .call(move |c| {
            let json: Option<String> = c
                .query_row("SELECT kept FROM collection_outside WHERE key = ?1", [key], |r| {
                    r.get(0)
                })
                .optional()?;
            // A kept listing this version can't read is read from the source again.
            Ok(json.and_then(|j| serde_json::from_str(&j).ok()))
        })
        .await
}

/// Keeps a listing (and refreshes a bookmark's name and size), then forgets listings nobody
/// bookmarked and nobody opened for `KEEP_FOR`, with their pictures.
async fn store(svc: &Service, key: String, kept: Kept) -> Result<()> {
    let json = serde_json::to_string(&kept).map_err(|e| Error::internal(format!("kept listing: {e}")))?;
    let old: Vec<String> = svc
        .ctx
        .db
        .call(move |c| {
            c.execute(
                "INSERT INTO collection_outside (key, kept, fetched_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(key) DO UPDATE SET kept = excluded.kept, fetched_at = excluded.fetched_at",
                params![key, json, kept.fetched_at],
            )?;
            c.execute(
                "UPDATE collection_bookmarks SET name = ?2, owner = ?3, total = ?4 WHERE key = ?1",
                params![key, kept.name, kept.owner, kept.songs.len() as i64],
            )?;
            let cutoff = lull_core::now() - KEEP_FOR;
            let old: Vec<String> = c
                .prepare(
                    "SELECT key FROM collection_outside WHERE fetched_at < ?1
                     AND key NOT IN (SELECT key FROM collection_bookmarks)",
                )?
                .query_map([cutoff], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            for k in &old {
                c.execute("DELETE FROM collection_outside WHERE key = ?1", [k])?;
            }
            Ok(old)
        })
        .await?;
    for k in old {
        if let Err(e) = tokio::fs::remove_file(picture_file(&svc.ctx, &k)).await
            && e.kind() != std::io::ErrorKind::NotFound
        {
            tracing::info!(error = %e, "an old outside playlist's picture wasn't removed");
        }
    }
    Ok(())
}

fn picture_file(ctx: &Ctx, key: &str) -> PathBuf {
    let hex: String = key.bytes().map(|b| format!("{b:02x}")).collect();
    ctx.config.data_dir.join("outside-covers").join(format!("{hex}.jpg"))
}

async fn write_picture(ctx: &Ctx, key: &str, jpeg: &[u8]) -> std::io::Result<()> {
    let path = picture_file(ctx, key);
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    let part = path.with_extension("jpg.part");
    tokio::fs::write(&part, jpeg).await?;
    tokio::fs::rename(&part, &path).await
}

fn cover_url(key: &str, cover_at: Option<i64>) -> Option<String> {
    cover_at.map(|v| {
        format!(
            "/api/outside/cover?link={}&v={v}",
            utf8_percent_encode(key, NON_ALPHANUMERIC)
        )
    })
}

/// `GET /api/outside/cover?link=&size=`: the kept picture, a square JPEG.
async fn cover(
    State(svc): State<Arc<Service>>,
    query: std::result::Result<Query<LinkQuery>, QueryRejection>,
) -> Result<Response> {
    let Query(q) = query.map_err(bad_query)?;
    let key = link_of(&q.link)?.key();
    covers::jpeg_response(&picture_file(&svc.ctx, &key), q.size)
        .await?
        .ok_or_else(|| Error::NotFound(format!("lull keeps no picture for {key}.")))
}

/// Looks the songs up in the background, one run per playlist at a time, and sends each batch
/// as an `outside` event: `{ id, songs: [{ pos, status, trackId }], entities, done }`.
fn start_matching(svc: &Arc<Service>, key: String, songs: Vec<(usize, Song)>) {
    {
        let Ok(mut running) = svc.previewing.lock() else { return };
        if !running.insert(key.clone()) {
            return;
        }
    }
    let svc = svc.clone();
    svc.ctx.clone().spawn(async move {
        let mut lookups = stream::iter(songs)
            .map(|(pos, song)| {
                let svc = svc.clone();
                async move { (pos, song.clone(), matches::find(&svc, &song).await) }
            })
            .buffered(PARALLEL);
        let (mut batch, mut last) = (Vec::new(), Instant::now());
        while let Some((pos, song, found)) = lookups.next().await {
            match found {
                Ok(found) => batch.push((pos, song, found)),
                Err(e) => tracing::info!(error = %e, "an outside playlist's song wasn't looked up"),
            }
            if last.elapsed() >= REPORT_EVERY && !batch.is_empty() {
                announce(&svc, &key, std::mem::take(&mut batch), false).await;
                last = Instant::now();
            }
        }
        if let Ok(mut running) = svc.previewing.lock() {
            running.remove(&key);
        }
        announce(&svc, &key, batch, true).await;
    });
}

async fn announce(svc: &Service, key: &str, batch: Vec<(usize, Song, Option<TrackId>)>, done: bool) {
    let key = key.to_owned();
    let made = svc
        .ctx
        .db
        .call(move |c| {
            let rows: Vec<Row> = batch
                .iter()
                .map(|(pos, s, found)| row(c, *pos, s, Some(found.clone())))
                .collect::<Result<_>>()?;
            let entities = entities(c, rows.iter().map(|r| &r.track_id))?;
            let songs: Vec<Value> = rows
                .iter()
                .map(|r| json!({ "pos": r.pos, "status": r.status, "trackId": r.track_id }))
                .collect();
            Ok(json!({ "id": key, "songs": songs, "entities": entities, "done": done }))
        })
        .await;
    match made {
        Ok(event) => svc.ctx.bus.publish("outside", event),
        Err(e) => tracing::warn!(error = %e, "an outside playlist's matches weren't announced"),
    }
}

// ---------------------------------------------------------------------------------------------
// Bookmarks

fn bookmarked(c: &Connection, key: &str) -> Result<bool> {
    Ok(
        c.query_row("SELECT 1 FROM collection_bookmarks WHERE key = ?1", [key], |_| Ok(()))
            .optional()?
            .is_some(),
    )
}

/// `GET /api/bookmarks`: `{ items: [{ id, url, site, name, owner, total, cover, savedAt }] }`,
/// newest first.
async fn list(State(svc): State<Arc<Service>>) -> Result<Json<Value>> {
    svc.ctx
        .db
        .call(|c| {
            let items = c
                .prepare(
                    "SELECT b.key, b.name, b.owner, b.total, b.saved_at, o.kept FROM collection_bookmarks b
                     LEFT JOIN collection_outside o ON o.key = b.key ORDER BY b.saved_at DESC, b.seq DESC",
                )?
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, i64>(4)?,
                        r.get::<_, Option<String>>(5)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?
                .into_iter()
                .filter_map(|(key, name, owner, total, at, kept)| {
                    let link = Link::from_key(&key).ok()?;
                    let cover_at = kept
                        .and_then(|k| serde_json::from_str::<Kept>(&k).ok())
                        .and_then(|k| k.cover_at);
                    Some(json!({
                        "id": key, "url": link.url(), "site": link.site.name(), "name": name,
                        "owner": owner, "total": total, "cover": cover_url(&key, cover_at), "savedAt": at,
                    }))
                })
                .collect::<Vec<_>>();
            Ok(Json(json!({ "items": items })))
        })
        .await
}

/// `PUT /api/bookmarks?link=`: bookmarks the playlist (reading it if lull hasn't); 200 with the
/// bookmark. Publishes `bookmark { id, saved: true }`.
async fn save(
    State(svc): State<Arc<Service>>,
    query: std::result::Result<Query<LinkQuery>, QueryRejection>,
) -> Result<Json<Value>> {
    let Query(q) = query.map_err(bad_query)?;
    let link = link_of(&q.link)?;
    let kept = listing(&svc, &link).await?;
    let key = link.key();
    let now = lull_core::now();
    let (k, name, owner, total) = (
        key.clone(),
        kept.name.clone(),
        kept.owner.clone(),
        kept.songs.len() as i64,
    );
    svc.ctx
        .db
        .call(move |c| {
            c.execute(
                "INSERT INTO collection_bookmarks (key, name, owner, total, saved_at) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(key) DO UPDATE SET name = excluded.name, owner = excluded.owner, total = excluded.total",
                params![k, name, owner, total, now],
            )?;
            Ok(())
        })
        .await?;
    svc.ctx.bus.publish("bookmark", json!({ "id": key, "saved": true }));
    Ok(Json(json!({
        "id": key, "url": link.url(), "site": link.site.name(), "name": kept.name, "owner": kept.owner,
        "total": kept.songs.len(), "cover": cover_url(&key, kept.cover_at), "savedAt": now,
    })))
}

/// `DELETE /api/bookmarks?link=`: 204, also when it wasn't bookmarked. Publishes
/// `bookmark { id, saved: false }`.
async fn unsave(
    State(svc): State<Arc<Service>>,
    query: std::result::Result<Query<LinkQuery>, QueryRejection>,
) -> Result<StatusCode> {
    let Query(q) = query.map_err(bad_query)?;
    let key = link_of(&q.link)?.key();
    let k = key.clone();
    svc.ctx
        .db
        .call(move |c| {
            c.execute("DELETE FROM collection_bookmarks WHERE key = ?1", [k])?;
            Ok(())
        })
        .await?;
    svc.ctx.bus.publish("bookmark", json!({ "id": key, "saved": false }));
    Ok(StatusCode::NO_CONTENT)
}

/// The keys being matched now, for `matching`.
pub(crate) type Previewing = std::sync::Mutex<HashSet<String>>;

#[cfg(test)]
mod tests;
