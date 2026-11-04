//! Playlist imports by link (docs/context/collection.md#playlist-links): a Spotify, Deezer,
//! YouTube, YouTube Music or SoundCloud link becomes a lull playlist whose songs play from
//! Soulseek (or YouTube) when played. A `job` row shows the progress in Activity.
//!
//! - **Matching** each song to the catalog is discovery's (`Catalog::find_song`): the library,
//!   then the catalog by ISRC or by artist, title and length.
//! - **Every song the link lists goes in the playlist.** A match is enrichment, not a gate: a song
//!   the catalog didn't match (or couldn't look up) goes in as the link names it, played by
//!   searching for it, as an M3U line is, and waits in the review until the listener picks a song
//!   from search, keeps it as named or skips it. Choices are kept by the source's id for the song,
//!   so they outlive re-imports.
//! - **Re-importing** the same link updates its playlist to the link's songs, in the link's
//!   order. A song matched before is not looked up again.

mod deezer;
pub(crate) mod parse;
mod spotify;
mod uploads;

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use futures_util::{StreamExt, stream};
use lull_core::ids::TrackId;
use lull_core::{Error, Result, catalog};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use self::parse::{Link, Parsed, Site};
use crate::{Events, Service, bad_json, covers, entities, known_track, playlists};

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
    /// The source's pictures for the playlist, best first (the first that loads is kept as its
    /// cover), and its description.
    pub(crate) covers: Vec<String>,
    pub(crate) description: Option<String>,
    /// Who made it (a Deezer user, a YouTube channel), when the site says.
    pub(crate) owner: Option<String>,
}

/// A song as the link lists it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// When it was added to the source (unix seconds), where the source says (Deezer playlists).
    pub(crate) added: Option<i64>,
    /// The source's own picture for the song (a Deezer album cover, an upload's thumbnail).
    #[serde(default)]
    pub(crate) picture: Option<String>,
}

impl Song {
    /// The song as the source names it, for `catalog::remember_named`.
    pub(crate) fn named(&self) -> catalog::Named<'_> {
        catalog::Named {
            artist: &self.artist,
            album: self.album.as_deref(),
            title: &self.title,
            duration: self.duration,
            isrc: self.isrc.as_deref(),
            picture: self.picture.as_deref(),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Starting an import

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartBody {
    url: String,
    /// Save the playlist's songs too (`saves.request`); kept for the link, so a re-import
    /// without it does as the last one did.
    save_songs: Option<bool>,
}

/// `{ url, saveSongs? }`: 202 `{ jobId, id }`. A link lull can't read, an unknown or private playlist, or an
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
    let (key, title, save) = (link.key(), name.clone(), body.save_songs);
    let (seq, now) = (
        svc.write(move |tx, _| begin(tx, &key, &title, save)).await?,
        lull_core::now(),
    );
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
            // The link is near the top; a huge page is cut at a character, never mid-character.
            let head: String = page.chars().take(1 << 20).collect();
            parse::resolved(&landed, &head)
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
fn begin(conn: &Connection, key: &str, name: &str, save_songs: Option<bool>) -> Result<i64> {
    conn.execute(
        "INSERT INTO collection_links (key, name, imported_at) VALUES (?1, ?2, 0) ON CONFLICT(key) DO NOTHING",
        params![key, name],
    )?;
    if let Some(save) = save_songs {
        conn.execute(
            "UPDATE collection_links SET save_songs = ?2 WHERE key = ?1",
            params![key, save],
        )?;
    }
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
    let (matches, unlooked) = match_songs(svc, &listing.songs, job).await;
    let (key, url, pictures) = (link.key(), link.url(), listing.covers.clone());
    let mut done = svc
        .write(move |tx, events| store(tx, events, &key, &url, &listing, &matches))
        .await?;
    done.unlooked = unlooked;
    // The playlist is there either way; a picture that can't be had leaves its songs' covers.
    if let Err(e) = covers::from_link(svc, done.playlist_seq, &pictures).await {
        tracing::info!(error = %e, "a link import's cover wasn't kept");
    }
    job.done = Some(done);
    Ok(())
}

/// The whole listing, without a job to tell (a preview of an outside playlist).
pub(crate) async fn fetch(svc: &Service, link: &Link) -> Result<Listing> {
    match link.site {
        Site::Spotify => spotify::read(svc, link).await,
        Site::Deezer => deezer::read(svc, link).await,
        Site::Youtube | Site::Soundcloud => uploads::read(svc, link, |_, _| {}).await,
    }
}

async fn read(svc: &Service, link: &Link, job: &mut Job) -> Result<Listing> {
    match link.site {
        Site::Spotify => spotify::read(svc, link).await,
        Site::Deezer => deezer::read(svc, link).await,
        Site::Youtube | Site::Soundcloud => uploads::read(svc, link, |n, total| job.reading(n, total)).await,
    }
}

/// Each song's track id, `None` when the catalog has no match, and how many lookups failed. A
/// song matched before, by this link or any other or a preview, keeps its match without a lookup
/// (`matches`). A song whose lookup fails is unmatched (it goes in as named; a re-import tries it
/// again).
async fn match_songs(svc: &Arc<Service>, songs: &[Song], job: &mut Job) -> (Vec<Option<TrackId>>, usize) {
    let total = songs.len();
    let mut lookups = stream::iter(songs.iter().cloned())
        .map(|song| {
            let svc = svc.clone();
            async move { crate::matches::find(&svc, &song).await }
        })
        .buffered(PARALLEL);
    let (mut matches, mut failures) = (Vec::with_capacity(total), 0);
    job.matching(0, total, 0);
    while let Some(found) = lookups.next().await {
        match found {
            Ok(id) => matches.push(id),
            Err(e) => {
                tracing::warn!(error = %e, "a link import's song wasn't looked up");
                failures += 1;
                matches.push(None);
            }
        }
        job.matching(matches.len(), total, matches.iter().filter(|m| m.is_some()).count());
    }
    (matches, failures)
}

/// What a finished import tells Activity.
struct Done {
    playlist_seq: i64,
    counts: Counts,
    /// Songs asked to be saved (the link's "Save the songs too").
    saving: usize,
    /// Songs whose lookup failed (among `counts.unmatched`).
    unlooked: usize,
}

/// Writes the listing and its matches, then makes or updates the link's playlist. An unmatched
/// song is recorded in the catalog as the link names it, its place in the playlist.
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
        "INSERT INTO collection_link_songs (link_seq, pos, song, artist, title, album, duration, isrc, track_id, added_at, picture, named_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
    )?;
    for (pos, (song, id)) in listing.songs.iter().zip(matches).enumerate() {
        let named = match id {
            Some(_) => None,
            None => Some(catalog::remember_named(conn, song.named())?.id),
        };
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
            song.added,
            song.picture,
            named.as_ref().map(TrackId::as_str),
        ])?;
    }
    let desc = match &listing.description {
        Some(d) => d.chars().take(1000).collect(),
        None => format!("Imported from {url}."),
    };
    let playlist_seq = fill_playlist(conn, events, seq, &listing.name, &desc)?;
    let lines = lines(conn, seq)?;
    let saving = ask_saves(
        conn,
        events,
        seq,
        lines.iter().filter_map(|l| l.track_id.clone()).collect(),
    )?;
    Ok(Done {
        playlist_seq,
        counts: counts(&lines),
        saving,
        unlooked: 0,
    })
}

/// A song's match changed (`matches::recheck`): every link listing it takes the new match (or,
/// with none, the song as the link names it), and its playlist is refilled; a listener's pick for
/// the song stays first (`lines`).
pub(crate) fn rematched(conn: &Connection, events: &mut Events, song: &Song, found: Option<TrackId>) -> Result<()> {
    let named = match &found {
        Some(_) => None,
        None => Some(catalog::remember_named(conn, song.named())?.id),
    };
    let links: Vec<(i64, String, String)> = conn
        .prepare(
            "SELECT DISTINCT l.seq, l.name, l.key FROM collection_link_songs s
             JOIN collection_links l ON l.seq = s.link_seq WHERE s.song = ?1",
        )?
        .query_map([&song.key], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    conn.execute(
        "UPDATE collection_link_songs SET track_id = ?2, named_id = ?3 WHERE song = ?1",
        params![
            song.key,
            found.as_ref().map(TrackId::as_str),
            named.as_ref().map(TrackId::as_str)
        ],
    )?;
    for (seq, name, key) in links {
        let desc = format!("Imported from {}.", Link::from_key(&key)?.url());
        fill_playlist(conn, events, seq, &name, &desc)?;
    }
    Ok(())
}

/// Asks library to save `ids` (`saves.request`, after the change commits) when the link's "Save
/// the songs too" is on; how many were asked for.
fn ask_saves(conn: &Connection, events: &mut Events, seq: i64, ids: Vec<TrackId>) -> Result<usize> {
    let save: bool = conn.query_row("SELECT save_songs FROM collection_links WHERE seq = ?1", [seq], |r| {
        r.get(0)
    })?;
    if !save || ids.is_empty() {
        return Ok(0);
    }
    let n = ids.len();
    events.push("saves.request", json!({ "trackIds": ids }));
    Ok(n)
}

/// Sets the link's playlist to its songs, making the playlist when it has none (or the
/// listener deleted it). The name and description are the link's only when it is made: the
/// listener's edits stay. Each song is dated when the source added it, else by its place in the
/// link: distinct, increasing seconds ending at the import, the same whichever songs are skipped
/// or fixed, so "Date added" follows the source.
fn fill_playlist(conn: &Connection, events: &mut Events, seq: i64, name: &str, desc: &str) -> Result<i64> {
    let imported: i64 = conn.query_row("SELECT imported_at FROM collection_links WHERE seq = ?1", [seq], |r| {
        r.get(0)
    })?;
    let lines = lines(conn, seq)?;
    let last = lines.len().saturating_sub(1) as i64;
    let items: Vec<playlists::Dated> = lines
        .into_iter()
        .filter_map(|l| {
            let added = l.added.unwrap_or(imported - (last - l.pos as i64));
            l.track_id.map(|id| (id, Some(added)))
        })
        .collect();
    let existing: Option<i64> = conn
        .query_row(
            "SELECT p.seq FROM collection_links l JOIN collection_playlists p ON p.seq = l.playlist_seq WHERE l.seq = ?1",
            [seq],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(p) = existing {
        playlists::refill(conn, events, p, &items, imported)?;
        return Ok(p);
    }
    let name: String = name.trim().chars().take(100).collect();
    let name = if name.is_empty() {
        "Imported playlist".to_owned()
    } else {
        name
    };
    let p = playlists::make_dated(conn, events, &name, desc, &items, imported)?;
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
                    "songs": d.counts.matched + d.counts.fixed + d.counts.unmatched,
                    "matched": d.counts.matched + d.counts.fixed, "unmatched": d.counts.unmatched,
                    "unlooked": d.unlooked, "skipped": d.counts.skipped, "saving": d.saving,
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
    /// No catalog match (or no lookup): in the playlist as the link names it, to review.
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
    /// The song in the playlist: the match, the fix, or (unmatched) the song as the link names
    /// it. `None` when skipped, or unmatched in an import from before unmatched songs went in.
    track_id: Option<TrackId>,
    /// When the source added it (`Song::added`).
    #[serde(skip)]
    added: Option<i64>,
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
        "SELECT s.pos, s.artist, s.title, s.album, s.duration, s.track_id, c.song IS NOT NULL, c.track_id, s.added_at,
                s.named_id
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
            (false, _, None) => (Status::Unmatched, r.get(9)?),
        };
        Ok(Line {
            pos: r.get::<_, i64>(0)? as usize,
            artist: r.get(1)?,
            title: r.get(2)?,
            album: r.get(3)?,
            duration: r.get(4)?,
            status,
            track_id: track_id.map(TrackId),
            added: r.get(8)?,
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
    /// key, name, playlist_seq, note, imported_at, save_songs
    type Row = (String, String, Option<i64>, Option<String>, i64, bool);
    let row: Option<Row> = conn
        .query_row(
            "SELECT key, name, playlist_seq, note, imported_at, save_songs FROM collection_links WHERE seq = ?1",
            [seq],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        )
        .optional()?;
    let (key, name, playlist, note, at, save_songs) =
        row.ok_or_else(|| Error::NotFound(format!("No import lk-{seq}.")))?;
    let playlist = match playlist {
        Some(p) => playlists::one(conn, p)?.map(|_| format!("pl-{p}")),
        None => None,
    };
    let link = Link::from_key(&key)?;
    Ok(json!({
        "id": format!("lk-{seq}"), "url": link.url(), "site": link.site.name(), "name": name,
        "playlistId": playlist, "note": note, "importedAt": (at > 0).then_some(at),
        "running": running.contains(&key), "counts": counts(&lines(conn, seq)?), "saveSongs": save_songs,
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
        let song: Option<Song> = tx
            .query_row(
                "SELECT song, artist, title, album, duration, isrc, picture, added_at FROM collection_link_songs
                 WHERE link_seq = ?1 AND pos = ?2",
                params![seq, pos as i64],
                |r| {
                    Ok(Song {
                        key: r.get(0)?,
                        artist: r.get(1)?,
                        title: r.get(2)?,
                        album: r.get(3)?,
                        duration: r.get(4)?,
                        isrc: r.get(5)?,
                        picture: r.get(6)?,
                        added: r.get(7)?,
                    })
                },
            )
            .optional()?;
        let song = song.ok_or_else(|| Error::NotFound(format!("Import lk-{seq} has no song {pos}.")))?;
        let chosen: Option<Option<TrackId>> = match choice {
            Choice::Track { track_id } => Some(Some(known_track(tx, &track_id)?.id)),
            // As the link names it, with its own picture; discovery resolves it when shown.
            Choice::AsIs => Some(Some(catalog::remember_named(tx, song.named())?.id)),
            Choice::Skip => Some(None),
            Choice::None => None,
        };
        let picked = chosen.clone().flatten();
        match chosen {
            Some(id) => tx.execute(
                "INSERT INTO collection_link_choices (link_seq, song, track_id) VALUES (?1, ?2, ?3)
                 ON CONFLICT(link_seq, song) DO UPDATE SET track_id = excluded.track_id",
                params![seq, song.key, id.as_ref().map(TrackId::as_str)],
            )?,
            None => tx.execute(
                "DELETE FROM collection_link_choices WHERE link_seq = ?1 AND song = ?2",
                params![seq, song.key],
            )?,
        };
        let (name, key): (String, String) =
            tx.query_row("SELECT name, key FROM collection_links WHERE seq = ?1", [seq], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?;
        let desc = format!("Imported from {}.", Link::from_key(&key)?.url());
        fill_playlist(tx, events, seq, &name, &desc)?;
        // A song the listener picked or kept joins the saves when the link saves its songs.
        if let Some(id) = picked {
            ask_saves(tx, events, seq, vec![id])?;
        }
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
