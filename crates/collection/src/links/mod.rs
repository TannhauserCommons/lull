//! Playlist imports by link (docs/context/collection.md#playlist-links): a Spotify, Deezer,
//! YouTube, YouTube Music or SoundCloud link becomes a lull playlist whose songs play from
//! Soulseek (or YouTube) when played. A `job` row shows the progress in Activity.
//!
//! - **Matching** each song to the catalog is discovery's (`Catalog::find_song`): the library,
//!   then the catalog by ISRC or by artist, title and length.
//! - **Unmatched songs** stay out of the playlist until the listener fixes them (a song picked
//!   from search, or the song as the link names it) or skips them. Choices are kept by the
//!   source's id for the song, so they outlive re-imports.
//! - **Re-importing** the same link updates its playlist to the link's songs, in the link's
//!   order. A song matched before is not looked up again.

mod deezer;
mod parse;
mod spotify;
mod uploads;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use futures_util::{StreamExt, stream};
use lull_core::ids::TrackId;
use lull_core::model::SongQuery;
use lull_core::{Error, Result, catalog};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use self::parse::{Link, Parsed, Site};
use crate::{Events, Service, bad_json, entities, import, known_track, playlists};

/// The most songs one link imports.
pub(crate) const MAX_SONGS: usize = 2000;
/// Songs matched at once: the catalog paces itself, this keeps a few in flight.
const PARALLEL: usize = 4;
/// Progress goes out at most this often.
const REPORT_EVERY: Duration = Duration::from_millis(750);

pub(crate) fn routes() -> Router<Arc<Service>> {
    Router::new()
        .route("/api/import/link", post(start))
        .route("/api/import/links", get(list))
        .route("/api/import/links/{id}", get(one))
        .route("/api/import/links/{id}/songs/{pos}", put(choose))
}

/// A playlist as its link lists it.
#[derive(Debug, Default)]
pub(crate) struct Listing {
    pub(crate) name: String,
    pub(crate) songs: Vec<Song>,
    /// What the listener should know about the listing (Spotify's 100-song limit).
    pub(crate) note: Option<String>,
}

/// A song as the link lists it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Song {
    /// The source's id for the song (`spotify:track:<id>`, `youtube:<video>`): choices and
    /// earlier matches are kept by it.
    pub(crate) key: String,
    pub(crate) artist: String,
    pub(crate) title: String,
    pub(crate) album: Option<String>,
    /// Seconds, only when it is the song's own length (not a music video's).
    pub(crate) duration: Option<u32>,
    pub(crate) isrc: Option<String>,
}

// ---------------------------------------------------------------------------------------------
// Starting an import

#[derive(Deserialize)]
struct StartBody {
    url: String,
}

/// `{ url }`: 202 `{ jobId, id }`. A link lull can't read, an unknown or private playlist, or an
/// empty one is an answer here, not a failed job.
async fn start(
    State(svc): State<Arc<Service>>,
    body: Result<Json<StartBody>, JsonRejection>,
) -> Result<(StatusCode, Json<Value>)> {
    let Json(body) = body.map_err(bad_json)?;
    if body.url.len() > 2048 {
        return Err(Error::BadRequest("That link is too long.".into()));
    }
    let link = resolve(&svc, &body.url).await?;
    let claim = Claim::new(&svc, link.key())?;
    let (name, listing) = open(&svc, &link).await?;
    let (key, title) = (link.key(), name.clone());
    let (seq, now) = (svc.write(move |tx, _| begin(tx, &key, &title)).await?, lull_core::now());
    let job = Job::start(&svc, seq, name, now);
    let job_id = job.id.clone();
    svc.ctx.spawn(run(svc.clone(), link, listing, claim, job));
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "jobId": job_id, "id": format!("lk-{seq}") })),
    ))
}

/// The link a pasted text names, following a short link once.
async fn resolve(svc: &Service, text: &str) -> Result<Link> {
    match parse::parse(text)? {
        Parsed::Link(link) => Ok(link),
        Parsed::Short(url) => {
            let resp = svc.ctx.http.send("link", |c| c.get(url.as_str())).await?;
            let landed = resp.url().to_string();
            let page = resp.text().await.unwrap_or_default();
            parse::resolved(&landed, &page[..page.len().min(1 << 20)])
                .ok_or_else(|| Error::BadRequest("That short link doesn't lead to a playlist lull can read.".into()))
        }
    }
}

/// Checks the link answers before the 202: its name, and the whole listing when that is one
/// request anyway (Spotify).
async fn open(svc: &Service, link: &Link) -> Result<(String, Option<Listing>)> {
    let (name, listing) = match link.site {
        Site::Spotify => {
            let listing = spotify::read(svc, link).await?;
            (listing.name.clone(), Some(listing))
        }
        Site::Deezer => (deezer::name(svc, link).await?, None),
        Site::Youtube | Site::Soundcloud => {
            let page = svc.youtube.playlist(&link.url(), 0, 1).await?;
            if page.songs.is_empty() && page.total.unwrap_or(0) == 0 {
                return Err(empty());
            }
            (page.title, None)
        }
    };
    if listing.as_ref().is_some_and(|l| l.songs.is_empty()) {
        return Err(empty());
    }
    Ok((name, listing))
}

fn empty() -> Error {
    Error::BadRequest("That playlist has no songs lull can read.".into())
}

/// The import's row, made on the first import of a link: its id is known from the start.
fn begin(conn: &Connection, key: &str, name: &str) -> Result<i64> {
    conn.execute(
        "INSERT INTO collection_links (key, name, imported_at) VALUES (?1, ?2, 0) ON CONFLICT(key) DO NOTHING",
        params![key, name],
    )?;
    Ok(conn.query_row("SELECT seq FROM collection_links WHERE key = ?1", [key], |r| r.get(0))?)
}

/// Holds a link's import slot until dropped: one import of a link at a time.
struct Claim(Arc<Service>, String);

impl Claim {
    fn new(svc: &Arc<Service>, key: String) -> Result<Self> {
        let mut running = svc.linking.lock().map_err(|_| Error::internal("link imports lock"))?;
        if !running.insert(key.clone()) {
            return Err(Error::Conflict("That link is being imported already.".into()));
        }
        Ok(Claim(svc.clone(), key))
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        if let Ok(mut running) = self.0.linking.lock() {
            running.remove(&self.1);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The job

async fn run(svc: Arc<Service>, link: Link, listing: Option<Listing>, _claim: Claim, mut job: Job) {
    let result = import(&svc, &link, listing, &mut job).await;
    job.finish(result);
}

/// Reads the rest of the link, matches each song and fills the playlist.
async fn import(svc: &Arc<Service>, link: &Link, listing: Option<Listing>, job: &mut Job) -> Result<()> {
    let listing = match listing {
        Some(listing) => listing,
        None => read(svc, link, job).await?,
    };
    if listing.songs.is_empty() {
        return Err(empty());
    }
    job.name = listing.name.clone();
    let matches = match_songs(svc, link, &listing.songs, job).await?;
    let (key, url) = (link.key(), link.url());
    let done = svc
        .write(move |tx, events| store(tx, events, &key, &url, &listing, &matches))
        .await?;
    job.done = Some(done);
    Ok(())
}

async fn read(svc: &Service, link: &Link, job: &mut Job) -> Result<Listing> {
    match link.site {
        Site::Spotify => spotify::read(svc, link).await,
        Site::Deezer => deezer::read(svc, link).await,
        Site::Youtube | Site::Soundcloud => uploads::read(svc, link, |n, total| job.reading(n, total)).await,
    }
}

/// Each song's track id, `None` when the catalog has no match. A song this link matched
/// before keeps its match without a lookup. A song whose lookup fails counts as unmatched (a
/// re-import tries it again); the import fails only when every lookup does.
async fn match_songs(svc: &Arc<Service>, link: &Link, songs: &[Song], job: &mut Job) -> Result<Vec<Option<TrackId>>> {
    let earlier = earlier_matches(svc, link.key()).await?;
    let total = songs.len();
    let mut lookups = stream::iter(songs.iter().cloned())
        .map(|song| {
            let (svc, known) = (svc.clone(), earlier.get(&song.key).cloned());
            async move { match_one(&svc, &song, known).await }
        })
        .buffered(PARALLEL);
    let (mut matches, mut failures, mut first_error) = (Vec::with_capacity(total), 0, None);
    job.matching(0, total, 0);
    while let Some(found) = lookups.next().await {
        match found {
            Ok(id) => matches.push(id),
            Err(e) => {
                tracing::info!(error = %e, "a link import's song wasn't looked up");
                failures += 1;
                first_error.get_or_insert(e);
                matches.push(None);
            }
        }
        job.matching(matches.len(), total, matches.iter().filter(|m| m.is_some()).count());
    }
    match first_error {
        Some(e) if failures == total => Err(e),
        _ => Ok(matches),
    }
}

async fn match_one(svc: &Service, song: &Song, earlier: Option<TrackId>) -> Result<Option<TrackId>> {
    if let Some(id) = earlier {
        let known = svc
            .ctx
            .db
            .call(move |c| Ok(catalog::track(c, &id)?.map(|t| t.id)))
            .await?;
        if known.is_some() {
            return Ok(known);
        }
    }
    let query = SongQuery {
        artist: song.artist.clone(),
        title: song.title.clone(),
        album: song.album.clone(),
        duration: song.duration,
        isrc: song.isrc.clone(),
    };
    Ok(svc.catalog.find_song(&query).await?.map(|t| t.id))
}

/// The songs this link matched last time, by the source's id for them.
async fn earlier_matches(svc: &Service, key: String) -> Result<HashMap<String, TrackId>> {
    svc.ctx
        .db
        .call(move |c| {
            let mut stmt = c.prepare(
                "SELECT s.song, s.track_id FROM collection_link_songs s JOIN collection_links l ON l.seq = s.link_seq
                 WHERE l.key = ?1 AND s.track_id IS NOT NULL",
            )?;
            let rows = stmt.query_map([key], |r| Ok((r.get::<_, String>(0)?, TrackId(r.get(1)?))))?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        })
        .await
}

/// What a finished import tells Activity.
struct Done {
    playlist_seq: i64,
    counts: Counts,
}

/// Writes the listing and its matches, then makes or updates the link's playlist.
fn store(
    conn: &Connection,
    events: &mut Events,
    key: &str,
    url: &str,
    listing: &Listing,
    matches: &[Option<TrackId>],
) -> Result<Done> {
    let now = lull_core::now();
    conn.execute(
        "UPDATE collection_links SET name = ?2, note = ?3, imported_at = ?4 WHERE key = ?1",
        params![key, listing.name, listing.note, now],
    )?;
    let seq: i64 = conn.query_row("SELECT seq FROM collection_links WHERE key = ?1", [key], |r| r.get(0))?;
    conn.execute("DELETE FROM collection_link_songs WHERE link_seq = ?1", [seq])?;
    let mut insert = conn.prepare(
        "INSERT INTO collection_link_songs (link_seq, pos, song, artist, title, album, duration, isrc, track_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
    )?;
    for (pos, (song, id)) in listing.songs.iter().zip(matches).enumerate() {
        insert.execute(params![
            seq,
            pos as i64,
            song.key,
            song.artist,
            song.title,
            song.album,
            song.duration,
            song.isrc,
            id.as_ref().map(TrackId::as_str),
        ])?;
    }
    let playlist_seq = fill_playlist(conn, events, seq, &listing.name, url, now)?;
    Ok(Done {
        playlist_seq,
        counts: counts(&lines(conn, seq)?),
    })
}

/// Sets the link's playlist to its songs, making the playlist when it has none (or the
/// listener deleted it). The name is the link's only when it is made: a rename stays.
fn fill_playlist(conn: &Connection, events: &mut Events, seq: i64, name: &str, url: &str, now: i64) -> Result<i64> {
    let ids: Vec<TrackId> = lines(conn, seq)?.into_iter().filter_map(|l| l.track_id).collect();
    let existing: Option<i64> = conn
        .query_row(
            "SELECT p.seq FROM collection_links l JOIN collection_playlists p ON p.seq = l.playlist_seq WHERE l.seq = ?1",
            [seq],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(p) = existing {
        playlists::change(conn, events, p, None, None, Some(ids))?;
        return Ok(p);
    }
    let name: String = name.trim().chars().take(100).collect();
    let name = if name.is_empty() {
        "Imported playlist".to_owned()
    } else {
        name
    };
    let p = playlists::make(conn, events, &name, &format!("Imported from {url}."), &ids, now)?;
    conn.execute(
        "UPDATE collection_links SET playlist_seq = ?2 WHERE seq = ?1",
        params![seq, p],
    )?;
    Ok(p)
}

/// One import's `job` row (api.md#events): reading, matching, then done or failed.
struct Job {
    svc: Arc<Service>,
    id: String,
    /// `lk-<seq>`.
    link: String,
    name: String,
    started: i64,
    last: Option<Instant>,
    done: Option<Done>,
}

impl Job {
    fn start(svc: &Arc<Service>, seq: i64, name: String, now: i64) -> Self {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let job = Job {
            svc: svc.clone(),
            id: format!("job-link-{nanos}"),
            link: format!("lk-{seq}"),
            name,
            started: now,
            last: None,
            done: None,
        };
        job.publish("running", json!({ "pct": 0, "detail": "Reading the playlist" }));
        job
    }

    fn reading(&mut self, n: usize, total: Option<usize>) {
        if self.due() {
            let detail = match total {
                Some(t) => format!("Reading the playlist, {n} of {t} songs"),
                None => format!("Reading the playlist, {n} songs"),
            };
            self.publish("running", json!({ "pct": 0, "detail": detail }));
        }
    }

    fn matching(&mut self, n: usize, total: usize, matched: usize) {
        if n == 0 || n == total || self.due() {
            let pct = (n * 100 / total.max(1)).min(99);
            let detail = format!("Matching songs, {n} of {total}");
            self.publish("running", json!({ "pct": pct, "detail": detail, "matched": matched }));
        }
    }

    fn due(&mut self) -> bool {
        let due = self.last.is_none_or(|t| t.elapsed() >= REPORT_EVERY);
        if due {
            self.last = Some(Instant::now());
        }
        due
    }

    fn finish(self, result: Result<()>) {
        match (result, &self.done) {
            (Ok(()), Some(d)) => self.publish(
                "done",
                json!({
                    "pct": 100, "playlistId": format!("pl-{}", d.playlist_seq),
                    "matched": d.counts.matched + d.counts.fixed, "unmatched": d.counts.unmatched,
                    "skipped": d.counts.skipped,
                }),
            ),
            (Ok(()), None) => self.publish("failed", json!({ "error": "the import stopped before it finished" })),
            (Err(e), _) => {
                tracing::warn!(error = %e, job = %self.id, "link import failed");
                self.publish("failed", json!({ "error": e.to_string() }));
            }
        }
    }

    fn publish(&self, state: &str, extra: Value) {
        let mut row = json!({
            "jobId": self.id, "kind": "import", "link": self.link,
            "name": if self.name.is_empty() { "Playlist link" } else { self.name.as_str() },
            "state": state, "started": self.started,
        });
        if let (Some(row), Value::Object(extra)) = (row.as_object_mut(), extra) {
            row.extend(extra);
        }
        self.svc.ctx.bus.publish("job", row);
    }
}

// ---------------------------------------------------------------------------------------------
// Reviewing an import

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum Status {
    /// The catalog's match.
    Matched,
    /// The listener's choice: a song from search, or the song as the link names it.
    Fixed,
    Skipped,
    Unmatched,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Line {
    pos: usize,
    artist: String,
    title: String,
    album: Option<String>,
    duration: Option<u32>,
    status: Status,
    /// The song in the playlist: the match or the fix. `None` when skipped or unmatched.
    track_id: Option<TrackId>,
}

#[derive(Default, Serialize)]
struct Counts {
    matched: usize,
    fixed: usize,
    skipped: usize,
    unmatched: usize,
}

fn counts(lines: &[Line]) -> Counts {
    let mut c = Counts::default();
    for l in lines {
        *match l.status {
            Status::Matched => &mut c.matched,
            Status::Fixed => &mut c.fixed,
            Status::Skipped => &mut c.skipped,
            Status::Unmatched => &mut c.unmatched,
        } += 1;
    }
    c
}

/// The link's songs in order, each with the listener's choice applied.
fn lines(conn: &Connection, seq: i64) -> Result<Vec<Line>> {
    let mut stmt = conn.prepare(
        "SELECT s.pos, s.artist, s.title, s.album, s.duration, s.track_id, c.song IS NOT NULL, c.track_id
         FROM collection_link_songs s
         LEFT JOIN collection_link_choices c ON c.link_seq = s.link_seq AND c.song = s.song
         WHERE s.link_seq = ?1 ORDER BY s.pos",
    )?;
    let rows = stmt.query_map([seq], |r| {
        let (matched, chosen, choice): (Option<String>, bool, Option<String>) = (r.get(5)?, r.get(6)?, r.get(7)?);
        let (status, track_id) = match (chosen, choice, matched) {
            (true, Some(id), _) => (Status::Fixed, Some(id)),
            (true, None, _) => (Status::Skipped, None),
            (false, _, Some(id)) => (Status::Matched, Some(id)),
            (false, _, None) => (Status::Unmatched, None),
        };
        Ok(Line {
            pos: r.get::<_, i64>(0)? as usize,
            artist: r.get(1)?,
            title: r.get(2)?,
            album: r.get(3)?,
            duration: r.get(4)?,
            status,
            track_id: track_id.map(TrackId),
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// `GET /api/import/links`: every link imported, newest first.
async fn list(State(svc): State<Arc<Service>>) -> Result<Json<Value>> {
    let running: HashSet<String> = svc.linking.lock().map(|r| r.clone()).unwrap_or_default();
    svc.ctx
        .db
        .call(move |c| {
            let seqs: Vec<i64> = c
                .prepare("SELECT seq FROM collection_links ORDER BY imported_at DESC, seq DESC")?
                .query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            let links = seqs
                .into_iter()
                .map(|seq| summary(c, seq, &running))
                .collect::<Result<Vec<_>>>()?;
            Ok(Json(json!({ "links": links })))
        })
        .await
}

/// `GET /api/import/links/{id}`: the import and each song's status, with `entities`.
async fn one(State(svc): State<Arc<Service>>, Path(id): Path<String>) -> Result<Json<Value>> {
    let seq = seq_of(&id)?;
    let running: HashSet<String> = svc.linking.lock().map(|r| r.clone()).unwrap_or_default();
    svc.ctx.db.call(move |c| detail(c, seq, &running)).await.map(Json)
}

fn summary(conn: &Connection, seq: i64, running: &HashSet<String>) -> Result<Value> {
    /// key, name, playlist_seq, note, imported_at
    type Row = (String, String, Option<i64>, Option<String>, i64);
    let row: Option<Row> = conn
        .query_row(
            "SELECT key, name, playlist_seq, note, imported_at FROM collection_links WHERE seq = ?1",
            [seq],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()?;
    let (key, name, playlist, note, at) = row.ok_or_else(|| Error::NotFound(format!("No import lk-{seq}.")))?;
    let playlist = match playlist {
        Some(p) => playlists::one(conn, p)?.map(|_| format!("pl-{p}")),
        None => None,
    };
    let link = Link::from_key(&key)?;
    Ok(json!({
        "id": format!("lk-{seq}"), "url": link.url(), "site": link.site.name(), "name": name,
        "playlistId": playlist, "note": note, "importedAt": (at > 0).then_some(at),
        "running": running.contains(&key), "counts": counts(&lines(conn, seq)?),
    }))
}

fn detail(conn: &Connection, seq: i64, running: &HashSet<String>) -> Result<Value> {
    let mut out = summary(conn, seq, running)?;
    let lines = lines(conn, seq)?;
    let entities = entities(conn, lines.iter().filter_map(|l| l.track_id.as_ref()))?;
    if let Some(o) = out.as_object_mut() {
        o.insert("songs".into(), json!(lines));
        o.insert("entities".into(), json!(entities));
    }
    Ok(out)
}

#[derive(Deserialize)]
#[serde(tag = "choice", rename_all = "kebab-case")]
enum Choice {
    /// A song the listener picked (from search): it must be in the catalog.
    #[serde(rename_all = "camelCase")]
    Track {
        track_id: TrackId,
    },
    /// The song as the link names it, searched on Soulseek when played.
    AsIs,
    Skip,
    /// Back to the catalog's match, or unmatched.
    None,
}

/// `PUT /api/import/links/{id}/songs/{pos}` `{ choice: track|as-is|skip|none, trackId? }`: the
/// listener's choice for one song, kept across re-imports; the playlist follows at once.
async fn choose(
    State(svc): State<Arc<Service>>,
    Path((id, pos)): Path<(String, usize)>,
    body: Result<Json<Choice>, JsonRejection>,
) -> Result<Json<Value>> {
    let (seq, Json(choice)) = (seq_of(&id)?, body.map_err(bad_json)?);
    let running: HashSet<String> = svc.linking.lock().map(|r| r.clone()).unwrap_or_default();
    svc.write(move |tx, events| {
        /// song, artist, title, album, duration
        type Row = (String, String, String, Option<String>, Option<u32>);
        let song: Option<Row> = tx
            .query_row(
                "SELECT song, artist, title, album, duration FROM collection_link_songs WHERE link_seq = ?1 AND pos = ?2",
                params![seq, pos as i64],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?;
        let (song, artist, title, album, duration) =
            song.ok_or_else(|| Error::NotFound(format!("Import lk-{seq} has no song {pos}.")))?;
        let chosen: Option<Option<TrackId>> = match choice {
            Choice::Track { track_id } => Some(Some(known_track(tx, &track_id)?.id)),
            Choice::AsIs => {
                let track = import::unsaved_track(&artist, album.as_deref().unwrap_or(""), &title, duration);
                if catalog::track(tx, &track.id)?.is_none() {
                    catalog::remember(tx, std::slice::from_ref(&track))?;
                }
                Some(Some(track.id))
            }
            Choice::Skip => Some(None),
            Choice::None => None,
        };
        match chosen {
            Some(id) => tx.execute(
                "INSERT INTO collection_link_choices (link_seq, song, track_id) VALUES (?1, ?2, ?3)
                 ON CONFLICT(link_seq, song) DO UPDATE SET track_id = excluded.track_id",
                params![seq, song, id.as_ref().map(TrackId::as_str)],
            )?,
            None => tx.execute(
                "DELETE FROM collection_link_choices WHERE link_seq = ?1 AND song = ?2",
                params![seq, song],
            )?,
        };
        let (name, key): (String, String) = tx.query_row(
            "SELECT name, key FROM collection_links WHERE seq = ?1",
            [seq],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        fill_playlist(tx, events, seq, &name, &Link::from_key(&key)?.url(), lull_core::now())?;
        detail(tx, seq, &running)
    })
    .await
    .map(Json)
}

fn seq_of(id: &str) -> Result<i64> {
    id.strip_prefix("lk-")
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| Error::NotFound(format!("No import {id}.")))
}

#[cfg(test)]
mod tests;
