//! What discovery keeps and reads: the taste signals (core tables plus its own), the albums and
//! artists it has shown (ids are hashes, so `/api/albums/{id}` needs to know what an id was),
//! and turning source records into the UI's entities, recorded in the catalog with saved state
//! from the `Library` seam.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use lull_core::catalog;
use lull_core::db::Db;
use lull_core::ids::{AlbumId, ArtistId, TrackId};
use lull_core::model::{Album, Artist, Entities, Saved, Track};
use lull_core::seams::Library;
use lull_core::{Error, Result};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::sources::{ArtistInfo, Release, Song, year};
use crate::taste::{Feedback, Mark, Play, Signals};

/// Plays older than this no longer count (at a 45-day half-life they weigh under 0.4%).
const SIGNAL_WINDOW_S: i64 = 365 * 86_400;

#[derive(Clone)]
pub struct Store {
    pub db: Db,
    library: Arc<dyn Library>,
}

impl Store {
    pub fn new(db: Db, library: Arc<dyn Library>) -> Self {
        Store { db, library }
    }

    /// Source songs as catalog tracks with saved state. A track the catalog already knows keeps
    /// its record (another crate may have added file facts); only `saved` is refreshed.
    pub async fn tracks(&self, songs: Vec<Song>) -> Result<Vec<Track>> {
        let fresh: Vec<Track> = songs.into_iter().map(track).collect();
        let mut tracks = self.db.call(move |c| record(c, fresh)).await?;
        let saved = self.saved(tracks.iter().map(|t| t.id.clone()).collect()).await;
        for t in &mut tracks {
            t.saved = saved.contains(&t.id);
        }
        Ok(tracks)
    }

    /// Releases as albums, remembered so their ids resolve later. A release already known keeps
    /// what is known (its tracks), and saved state comes from its catalog tracks.
    pub async fn albums(&self, releases: Vec<Release>) -> Result<Vec<Album>> {
        self.remember_albums(releases.into_iter().map(album).collect()).await
    }

    /// `albums`, for albums already in the UI's shape.
    pub async fn remember_albums(&self, albums: Vec<Album>) -> Result<Vec<Album>> {
        let (mut albums, members) = self
            .db
            .call(move |c| {
                let albums = merge_put(c, "discovery_albums", albums, |a| &a.id.0, merge_album)?;
                let ids: Vec<AlbumId> = albums.iter().map(|a| a.id.clone()).collect();
                Ok((albums, catalog_members(c, &ids)?))
            })
            .await?;
        let saved = self.saved(members.values().flatten().cloned().collect()).await;
        for a in &mut albums {
            a.saved = album_saved(a, members.get(&a.id).map(Vec::as_slice).unwrap_or_default(), &saved);
        }
        Ok(albums)
    }

    /// An album whose tracks are known: remembered with them and its saved state.
    pub async fn full_album(&self, mut album: Album, tracks: &[Track]) -> Result<Album> {
        album.track_ids = tracks.iter().map(|t| t.id.clone()).collect();
        let total: u32 = tracks.iter().filter_map(|t| t.duration).sum();
        album.duration = (total > 0).then_some(total);
        let saved: HashSet<TrackId> = tracks.iter().filter(|t| t.saved).map(|t| t.id.clone()).collect();
        album.saved = album_saved(&album, &album.track_ids, &saved);
        let stored = album.clone();
        self.db
            .call(move |c| merge_put(c, "discovery_albums", vec![stored], |a| &a.id.0, |_, new| new))
            .await?;
        Ok(album)
    }

    /// Artists, remembered so their ids resolve later; what is known already fills gaps.
    pub async fn artists(&self, artists: Vec<Artist>) -> Result<Vec<Artist>> {
        self.db
            .call(move |c| merge_put(c, "discovery_artists", artists, |a| &a.id.0, merge_artist))
            .await
    }

    /// What an album id stands for: a remembered album, else one of the catalog's tracks.
    pub async fn album_ref(&self, id: AlbumId) -> Result<Option<Album>> {
        self.db
            .call(move |c| {
                if let Some(album) = get::<Album>(c, "discovery_albums", &id.0)? {
                    return Ok(Some(album));
                }
                let track = catalog::track_on_album(c, &id)?;
                Ok(track.map(|t| {
                    album(Release {
                        mbid: None,
                        title: t.album,
                        artist: t.artist,
                        artist_mbid: None,
                        kind: None,
                        date: t.year.map(|y| y.to_string()),
                        tags: t.tags,
                    })
                }))
            })
            .await
    }

    /// What an artist id stands for: a remembered artist, else a catalog track's artist.
    pub async fn artist_ref(&self, id: ArtistId) -> Result<Option<Artist>> {
        self.db
            .call(move |c| {
                if let Some(artist) = get::<Artist>(c, "discovery_artists", &id.0)? {
                    return Ok(Some(artist));
                }
                let track = catalog_by_artist(c, &id)?;
                Ok(track.map(|t| {
                    artist(ArtistInfo {
                        mbid: None,
                        name: t.artist,
                    })
                }))
            })
            .await
    }

    /// The catalog's tracks of an album, in album order, with saved state.
    pub async fn catalog_album_tracks(&self, id: AlbumId) -> Result<Vec<Track>> {
        let mut tracks = self
            .db
            .call(move |c| {
                let ids = catalog_members(c, std::slice::from_ref(&id))?
                    .remove(&id)
                    .unwrap_or_default();
                let mut tracks = Vec::with_capacity(ids.len());
                for t in &ids {
                    tracks.extend(catalog::track(c, t)?);
                }
                Ok(tracks)
            })
            .await?;
        tracks.sort_by_key(|t| (t.disc.unwrap_or(1), t.no.unwrap_or(u32::MAX)));
        let saved = self.saved(tracks.iter().map(|t| t.id.clone()).collect()).await;
        for t in &mut tracks {
            t.saved = saved.contains(&t.id);
        }
        Ok(tracks)
    }

    pub async fn track(&self, id: TrackId) -> Result<Option<Track>> {
        self.db.call(move |c| catalog::track(c, &id)).await
    }

    /// The saved tracks among `ids`. Jellyfin being down only hides the badges, so the
    /// failure is logged, not raised.
    async fn saved(&self, ids: Vec<TrackId>) -> HashSet<TrackId> {
        if ids.is_empty() {
            return HashSet::new();
        }
        match self.library.saved(&ids).await {
            Ok(tracks) => tracks.into_iter().map(|t| t.id).collect(),
            Err(e) => {
                tracing::info!(error = %e, "saved state unavailable");
                HashSet::new()
            }
        }
    }
}

/// Entities for a response, from what a builder collected.
pub fn entities(tracks: Vec<Track>, albums: Vec<Album>, artists: Vec<Artist>) -> Entities {
    Entities {
        tracks: tracks.into_iter().map(|t| (t.id.clone(), t)).collect(),
        albums: albums.into_iter().map(|a| (a.id.clone(), a)).collect(),
        artists: artists.into_iter().map(|a| (a.id.clone(), a)).collect(),
    }
}

pub fn track(s: Song) -> Track {
    let album_artist = s.album_artist.as_deref().unwrap_or(&s.artist);
    Track {
        id: TrackId::of(&s.artist, &s.album, &s.title),
        artist_id: ArtistId::of(&s.artist),
        album_id: AlbumId::of(album_artist, &s.album),
        title: s.title,
        artist: s.artist,
        album: s.album,
        year: s.year,
        no: s.no,
        disc: s.disc,
        duration: s.duration_s,
        tags: s.tags,
        saved: false,
        file: None,
    }
}

pub fn album(r: Release) -> Album {
    Album {
        id: AlbumId::of(&r.artist, &r.title),
        artist_id: ArtistId::of(&r.artist),
        year: r.date.as_deref().and_then(year),
        title: r.title,
        artist: r.artist,
        kind: r.kind,
        released: r.date,
        label: None,
        tags: r.tags,
        saved: Saved::None,
        track_ids: Vec::new(),
        duration: None,
        musicbrainz_id: r.mbid,
    }
}

pub fn artist(a: ArtistInfo) -> Artist {
    Artist {
        id: ArtistId::of(&a.name),
        name: a.name,
        tags: Vec::new(),
        listeners: None,
        bio: None,
        musicbrainz_id: a.mbid,
    }
}

/// All of the album's tracks saved, some, or none (as far as lull knows its tracks).
fn album_saved(album: &Album, members: &[TrackId], saved: &HashSet<TrackId>) -> Saved {
    let ids = if album.track_ids.is_empty() {
        members
    } else {
        &album.track_ids
    };
    let n = ids.iter().filter(|id| saved.contains(id)).count();
    match n {
        0 => Saved::None,
        n if n == ids.len() => Saved::All,
        _ => Saved::Partial,
    }
}

fn merge_album(old: Album, mut new: Album) -> Album {
    if new.track_ids.is_empty() {
        new.track_ids = old.track_ids;
        new.duration = new.duration.or(old.duration);
    }
    if new.tags.is_empty() {
        new.tags = old.tags;
    }
    new.musicbrainz_id = new.musicbrainz_id.or(old.musicbrainz_id);
    new.kind = new.kind.or(old.kind);
    new.released = new.released.or(old.released);
    new.year = new.year.or(old.year);
    new
}

fn merge_artist(old: Artist, mut new: Artist) -> Artist {
    if new.tags.is_empty() {
        new.tags = old.tags;
    }
    new.listeners = new.listeners.or(old.listeners);
    new.bio = new.bio.or(old.bio);
    new.musicbrainz_id = new.musicbrainz_id.or(old.musicbrainz_id);
    new
}

/// Stores rows by id, merging each with what is stored; returns the merged rows.
fn merge_put<T: Serialize + DeserializeOwned>(
    c: &Connection,
    table: &str,
    rows: Vec<T>,
    id: impl Fn(&T) -> &str,
    merge: impl Fn(T, T) -> T,
) -> Result<Vec<T>> {
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let row = match get::<T>(c, table, id(&row))? {
            Some(old) => merge(old, row),
            None => row,
        };
        let json = serde_json::to_string(&row).map_err(|e| Error::internal(format!("{table} json: {e}")))?;
        c.prepare_cached(&format!(
            "INSERT INTO {table} (id, json, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(id) DO UPDATE SET json = excluded.json, updated_at = excluded.updated_at"
        ))?
        .execute(params![id(&row), json, lull_core::now()])?;
        out.push(row);
    }
    Ok(out)
}

fn get<T: DeserializeOwned>(c: &Connection, table: &str, id: &str) -> Result<Option<T>> {
    let json: Option<String> = c
        .prepare_cached(&format!("SELECT json FROM {table} WHERE id = ?1"))?
        .query_row([id], |r| r.get(0))
        .optional()?;
    // A row an older build wrote that no longer parses is treated as unknown.
    Ok(json.and_then(|j| serde_json::from_str(&j).ok()))
}

/// Records tracks in the catalog and returns each as the catalog then has it. A known track
/// keeps every fact it has; this source only fills its gaps.
fn record(c: &Connection, tracks: Vec<Track>) -> Result<Vec<Track>> {
    let mut out = Vec::with_capacity(tracks.len());
    let mut changed = Vec::new();
    for t in tracks {
        let merged = match catalog::track(c, &t.id)? {
            Some(known) => fill_gaps(known, t),
            None => t,
        };
        if catalog::track(c, &merged.id)?.as_ref() != Some(&merged) {
            changed.push(merged.clone());
        }
        out.push(merged);
    }
    catalog::remember(c, &changed)?;
    Ok(out)
}

fn fill_gaps(mut known: Track, new: Track) -> Track {
    known.year = known.year.or(new.year);
    known.no = known.no.or(new.no);
    known.disc = known.disc.or(new.disc);
    known.duration = known.duration.or(new.duration);
    if known.tags.is_empty() {
        known.tags = new.tags;
    }
    known
}

/// Any one catalog track by the artist (a scan, but only for ids discovery never showed).
fn catalog_by_artist(c: &Connection, id: &ArtistId) -> Result<Option<Track>> {
    let json: Option<String> = c
        .query_row(
            "SELECT json FROM catalog_tracks WHERE json_extract(json, '$.artistId') = ?1 LIMIT 1",
            [id.as_str()],
            |r| r.get(0),
        )
        .optional()?;
    Ok(json.and_then(|j| serde_json::from_str(&j).ok()))
}

/// The catalog's tracks of each album.
fn catalog_members(c: &Connection, albums: &[AlbumId]) -> Result<HashMap<AlbumId, Vec<TrackId>>> {
    let mut stmt = c.prepare_cached("SELECT id FROM catalog_tracks WHERE album_id = ?1")?;
    let mut members = HashMap::new();
    for album in albums {
        let ids = stmt
            .query_map([album.as_str()], |r| Ok(TrackId(r.get(0)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if !ids.is_empty() {
            members.insert(album.clone(), ids);
        }
    }
    Ok(members)
}

// ---------- taste signals ----------

pub fn reset_at(c: &Connection) -> Result<i64> {
    Ok(c.query_row(
        "SELECT value FROM discovery_state WHERE key = 'taste_reset_at'",
        [],
        |r| r.get::<_, String>(0),
    )
    .optional()?
    .and_then(|v| v.parse().ok())
    .unwrap_or(0))
}

/// Everything the taste model learns from, as of `now`.
pub fn signals(c: &Connection, now: i64) -> Result<Signals> {
    let reset = reset_at(c)?;
    let since = reset.max(now - SIGNAL_WINDOW_S);
    let mut s = Signals {
        plays: rows(
            c,
            "SELECT track_id, artist, started_at, played_s, duration_s, completed, skipped FROM plays
             WHERE started_at >= ?1",
            [since],
            |r| {
                Ok(Play {
                    track: TrackId(r.get(0)?),
                    artist: r.get(1)?,
                    at: r.get(2)?,
                    played_s: r.get(3)?,
                    duration_s: r.get(4)?,
                    completed: r.get(5)?,
                    skipped: r.get(6)?,
                })
            },
        )?,
        likes: rows(
            c,
            "SELECT track_id, artist, liked_at FROM likes WHERE liked_at >= ?1",
            [reset],
            mark,
        )?,
        saves: rows(
            c,
            "SELECT track_id, artist, at FROM discovery_saves WHERE at >= ?1",
            [reset],
            mark,
        )?,
        feedback: rows(
            c,
            "SELECT track_id, artist, at, positive FROM discovery_feedback WHERE at >= ?1",
            [reset],
            |r| {
                Ok(Feedback {
                    track: r.get::<_, Option<String>>(0)?.map(TrackId),
                    artist: r.get(1)?,
                    at: r.get(2)?,
                    positive: r.get(3)?,
                })
            },
        )?,
        followed_artists: rows(c, "SELECT name FROM follows WHERE kind = 'artist'", [], |r| r.get(0))?,
        followed_tags: followed_tags(c)?,
        history: rows(c, "SELECT artist, plays FROM discovery_history", [], |r| {
            Ok((r.get(0)?, r.get::<_, i64>(1)?.max(0) as u64))
        })?,
        hidden: hidden(c)?,
        ..Default::default()
    };
    let tracks: HashSet<TrackId> = s
        .plays
        .iter()
        .map(|p| p.track.clone())
        .chain(s.likes.iter().chain(&s.saves).map(|m| m.track.clone()))
        .chain(s.feedback.iter().filter_map(|f| f.track.clone()))
        .collect();
    for id in tracks {
        if let Some(t) = catalog::track(c, &id)?
            && !t.tags.is_empty()
        {
            s.track_tags.insert(id, t.tags);
        }
    }
    let mut stmt = c.prepare_cached("SELECT json FROM discovery_artists")?;
    for json in stmt.query_map([], |r| r.get::<_, String>(0))? {
        if let Ok(a) = serde_json::from_str::<Artist>(&json?)
            && !a.tags.is_empty()
        {
            s.artist_tags.insert(a.id, a.tags);
        }
    }
    Ok(s)
}

pub fn followed_tags(c: &Connection) -> Result<Vec<String>> {
    rows(
        c,
        "SELECT id FROM follows WHERE kind = 'tag' ORDER BY followed_at",
        [],
        |r| r.get(0),
    )
}

pub fn followed_artists(c: &Connection) -> Result<Vec<(ArtistId, String)>> {
    rows(
        c,
        "SELECT id, name FROM follows WHERE kind = 'artist' ORDER BY followed_at",
        [],
        |r| Ok((ArtistId(r.get(0)?), r.get(1)?)),
    )
}

pub fn hidden(c: &Connection) -> Result<HashSet<ArtistId>> {
    Ok(rows(c, "SELECT artist_id FROM discovery_hidden", [], |r| {
        Ok(ArtistId(r.get(0)?))
    })?
    .into_iter()
    .collect())
}

/// The plays of the last `days` (skips left out), as (track, artist, plays, last played),
/// most played first.
pub fn recent_plays(c: &Connection, now: i64, days: i64) -> Result<Vec<(TrackId, String, u32, i64)>> {
    rows(
        c,
        "SELECT track_id, artist, COUNT(*), MAX(started_at) FROM plays WHERE started_at >= ?1 AND skipped = 0
         GROUP BY track_id ORDER BY COUNT(*) DESC, MAX(started_at) DESC",
        [now - days * 86_400],
        |r| Ok((TrackId(r.get(0)?), r.get(1)?, r.get(2)?, r.get(3)?)),
    )
}

/// Saves an artist list from Last.fm history, replacing the previous one.
pub fn put_history(c: &mut Connection, artists: &BTreeMap<String, u64>) -> Result<()> {
    let tx = c.transaction()?;
    tx.execute("DELETE FROM discovery_history", [])?;
    for (artist, plays) in artists {
        tx.execute(
            "INSERT INTO discovery_history (artist, plays) VALUES (?1, ?2)",
            params![artist, *plays as i64],
        )?;
    }
    tx.commit()?;
    Ok(())
}

fn mark(r: &rusqlite::Row) -> rusqlite::Result<Mark> {
    Ok(Mark {
        track: TrackId(r.get(0)?),
        artist: r.get(1)?,
        at: r.get(2)?,
    })
}

fn rows<T>(
    c: &Connection,
    sql: &str,
    args: impl rusqlite::Params,
    f: impl FnMut(&rusqlite::Row) -> rusqlite::Result<T>,
) -> Result<Vec<T>> {
    let mut stmt = c.prepare_cached(sql)?;
    let out = stmt.query_map(args, f)?.collect::<rusqlite::Result<Vec<T>>>()?;
    Ok(out)
}
