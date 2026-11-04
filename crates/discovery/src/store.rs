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

use crate::artist::identity::{Evidence, IdRow, Ids, Source};
use crate::sources::{AlbumSource, ArtistInfo, Release, Song, year};
use crate::taste::{Feedback, Mark, Named, Play, Signals};

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

    /// Asks library to fetch these saved albums' covers ahead: they have no catalog picture, so
    /// the UI asks library's `/api/art` for them.
    pub fn warm_covers(&self, albums: &[AlbumId]) {
        self.library.warm_covers(albums);
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
    /// what is known (its tracks), and saved state comes from its catalog tracks. Their artists
    /// are remembered by name: the MBID a release names is its artist's, not necessarily the
    /// listener's artist of that name (`artist::identity`).
    pub async fn albums(&self, releases: Vec<Release>) -> Result<Vec<Album>> {
        self.remember_albums(releases.into_iter().map(album).collect()).await
    }

    /// `albums`, for albums already in the UI's shape.
    pub async fn remember_albums(&self, albums: Vec<Album>) -> Result<Vec<Album>> {
        let (mut albums, members) = self
            .db
            .call(move |c| {
                let artists: Vec<Artist> = albums
                    .iter()
                    .map(|a| {
                        artist(ArtistInfo {
                            mbid: None,
                            name: a.artist.clone(),
                        })
                    })
                    .collect();
                put_artists(c, artists)?;
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
    pub async fn full_album(&self, album: Album, tracks: &[Track]) -> Result<Album> {
        let album = with_tracks(album, tracks);
        let stored = album.clone();
        self.db
            .call(move |c| merge_put(c, "discovery_albums", vec![stored], |a| &a.id.0, |_, new| new))
            .await?;
        Ok(album)
    }

    /// What MusicBrainz knows of a remembered album (its MBID, tags, type, date) fills its gaps.
    pub async fn enrich_album(&self, id: AlbumId, group: Release) -> Result<()> {
        self.db
            .call(move |c| {
                let Some(mut known) = get::<Album>(c, "discovery_albums", &id.0)? else {
                    return Ok(());
                };
                let found = album(group);
                known.musicbrainz_id = known.musicbrainz_id.or(found.musicbrainz_id);
                if known.tags.is_empty() {
                    known.tags = found.tags;
                }
                known.kind = known.kind.or(found.kind);
                known.released = known.released.or(found.released);
                known.year = known.year.or(found.year);
                merge_put(c, "discovery_albums", vec![known], |a| &a.id.0, |_, new| new)?;
                Ok(())
            })
            .await
    }

    /// Remembers which catalog album each album id came from (the latest finder wins).
    pub async fn remember_sources(&self, sources: Vec<(AlbumId, AlbumSource)>) -> Result<()> {
        if sources.is_empty() {
            return Ok(());
        }
        self.db
            .call(move |c| {
                let mut put = c.prepare_cached(
                    "INSERT INTO discovery_album_sources (album_id, json, updated_at) VALUES (?1, ?2, ?3)
                     ON CONFLICT(album_id) DO UPDATE SET json = excluded.json, updated_at = excluded.updated_at",
                )?;
                for (id, source) in &sources {
                    let json = serde_json::to_string(source)
                        .map_err(|e| Error::internal(format!("album source json: {e}")))?;
                    put.execute(params![id.as_str(), json, lull_core::now()])?;
                }
                Ok(())
            })
            .await
    }

    /// The catalog album behind an album id, when a catalog found it.
    pub async fn album_source(&self, id: AlbumId) -> Result<Option<AlbumSource>> {
        let json: Option<String> = self
            .db
            .call(move |c| {
                Ok(c.query_row(
                    "SELECT json FROM discovery_album_sources WHERE album_id = ?1",
                    [id.as_str()],
                    |r| r.get(0),
                )
                .optional()?)
            })
            .await?;
        Ok(json.and_then(|j| serde_json::from_str(&j).ok()))
    }

    /// Artists, remembered so their ids resolve later; what is known already fills gaps.
    pub async fn artists(&self, artists: Vec<Artist>) -> Result<Vec<Artist>> {
        self.db.call(move |c| put_artists(c, artists)).await
    }

    /// The exact ids lull holds for the artist (`artist::identity`).
    pub async fn artist_ids(&self, id: ArtistId) -> Result<Ids> {
        self.db.call(move |c| artist_ids(c, &id)).await
    }

    /// When the listener's evidence for the artist was last weighed (`artist::identity`).
    pub async fn ids_checked(&self, id: ArtistId) -> Result<Option<i64>> {
        self.db
            .call(move |c| {
                Ok(c.query_row(
                    "SELECT checked_at FROM discovery_artist_checked WHERE artist_id = ?1",
                    [&id.0],
                    |r| r.get(0),
                )
                .optional()?)
            })
            .await
    }

    /// Replaces the artist's ids with what the listener's evidence decided (`checked` when it
    /// was weighed). An artist whose MBID changes or goes loses the tags stored for it and its
    /// photo's verdict: they were that MBID's (or a name's), so its photo is vetted again.
    pub async fn set_artist_ids(&self, id: ArtistId, ids: Ids, checked: i64) -> Result<()> {
        self.db.call(move |c| replace_ids(c, &id, &ids, checked)).await
    }

    /// Adds ids the artist has no id of that source for yet; returns the ids it then has. A
    /// new MBID drops what was kept for the old one (none), as `set_artist_ids` does.
    pub async fn add_artist_ids(&self, id: ArtistId, rows: Vec<IdRow>) -> Result<Ids> {
        self.db
            .call(move |c| {
                let tx = c.transaction()?;
                let before = artist_ids(&tx, &id)?;
                for row in &rows {
                    if before.get(row.source).is_none() {
                        put_artist_id(&tx, &id, row)?;
                    }
                }
                let after = artist_ids(&tx, &id)?;
                if before.mbid() != after.mbid() {
                    forget_named(&tx, &id)?;
                }
                tx.commit()?;
                Ok(after)
            })
            .await
    }

    /// The artist's own tags (MusicBrainz's, by its MBID), replacing what was kept.
    pub async fn set_artist_tags(&self, id: ArtistId, tags: Vec<String>) -> Result<()> {
        let json = serde_json::to_string(&tags).map_err(|e| Error::internal(format!("tags json: {e}")))?;
        self.db
            .call(move |c| {
                c.execute(
                    "UPDATE discovery_artists SET json = json_set(json, '$.tags', json(?2)) WHERE id = ?1",
                    params![id.0, json],
                )?;
                Ok(())
            })
            .await
    }

    /// A song's own tags, where the catalog has none (`catalog::remember_found` fills gaps).
    pub async fn set_track_tags(&self, id: TrackId, tags: Vec<String>) -> Result<()> {
        self.db
            .call(move |c| {
                if let Some(mut track) = catalog::track(c, &id)? {
                    track.tags = tags;
                    record(c, vec![track])?;
                }
                Ok(())
            })
            .await
    }

    /// The artists among `ids` that hold any id already.
    pub async fn identified(&self, ids: Vec<ArtistId>) -> Result<HashSet<ArtistId>> {
        self.db
            .call(move |c| {
                let mut found = HashSet::new();
                let mut stmt = c.prepare_cached("SELECT 1 FROM discovery_artist_ids WHERE artist_id = ?1 LIMIT 1")?;
                for id in ids {
                    if stmt.exists([&id.0])? {
                        found.insert(id);
                    }
                }
                Ok(found)
            })
            .await
    }

    /// The MBID the saved files' tags give the artist (the library's own record of it).
    pub async fn library_mbid(&self, id: &ArtistId) -> Result<Option<String>> {
        Ok(self
            .library_artists()
            .await?
            .into_iter()
            .find(|a| a.id == *id)
            .and_then(|a| a.musicbrainz_id))
    }

    /// The library's artists, each with the MBID its saved files' tags give, if any.
    pub async fn library_artists(&self) -> Result<Vec<Artist>> {
        self.library.artists().await
    }

    /// A value discovery keeps in `discovery_state`.
    pub async fn state(&self, key: &'static str) -> Result<Option<String>> {
        self.db
            .call(move |c| {
                Ok(
                    c.query_row("SELECT value FROM discovery_state WHERE key = ?1", [key], |r| r.get(0))
                        .optional()?,
                )
            })
            .await
    }

    pub async fn set_state(&self, key: &'static str, value: &'static str) -> Result<()> {
        self.db
            .call(move |c| {
                c.execute(
                    "INSERT INTO discovery_state (key, value) VALUES (?1, ?2)
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    [key, value],
                )?;
                Ok(())
            })
            .await
    }

    /// Every remembered artist: (id, name).
    pub async fn remembered_artists(&self) -> Result<Vec<(ArtistId, String)>> {
        self.db
            .call(|c| {
                rows(c, "SELECT json FROM discovery_artists", [], |r| r.get::<_, String>(0)).map(|all| {
                    all.iter()
                        .filter_map(|j| serde_json::from_str::<Artist>(j).ok())
                        .map(|a| (a.id, a.name))
                        .collect()
                })
            })
            .await
    }

    /// The songs the listener played or liked, with their artist credit.
    pub async fn listener_songs(&self) -> Result<Vec<(TrackId, String)>> {
        self.db
            .call(|c| {
                rows(
                    c,
                    "SELECT track_id, artist FROM plays UNION SELECT track_id, artist FROM likes",
                    [],
                    |r| Ok((TrackId(r.get(0)?), r.get(1)?)),
                )
            })
            .await
    }

    /// The ISRC a source named for the track (`catalog::hint_isrc`), if any.
    pub async fn isrc(&self, id: TrackId) -> Result<Option<String>> {
        self.db
            .call(move |c| {
                Ok(c.query_row(
                    "SELECT isrc FROM catalog_resolutions WHERE track_id = ?1 AND isrc IS NOT NULL",
                    [id.0],
                    |r| r.get(0),
                )
                .optional()?)
            })
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
                let found = match get::<Artist>(c, "discovery_artists", &id.0)? {
                    Some(artist) => Some(artist),
                    None => catalog_by_artist(c, &id)?.map(|t| {
                        artist(ArtistInfo {
                            mbid: None,
                            name: t.artist,
                        })
                    }),
                };
                found.map(|a| identified(c, a)).transpose()
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

    /// Saved songs matching free text (the `Library` seam).
    pub async fn search_saved(&self, q: &str, limit: usize) -> Result<Vec<Track>> {
        self.library.search(q, limit).await
    }

    /// What is the listener's own, for ranking search results: liked tracks, followed artists.
    pub async fn own(&self) -> Result<(HashSet<TrackId>, HashSet<ArtistId>)> {
        self.db
            .call(|c| {
                let liked = rows(c, "SELECT track_id FROM likes", [], |r| Ok(TrackId(r.get(0)?)))?;
                let followed = followed_artists(c)?.into_iter().map(|(id, _)| id).collect();
                Ok((liked.into_iter().collect(), followed))
            })
            .await
    }

    /// Records where search found pictures for albums and artists (`catalog`), with the albums
    /// and artists themselves, so the art route and other crates can resolve their ids.
    pub async fn remember_pictures(
        &self,
        albums: Vec<(Album, Option<String>)>,
        artists: Vec<(Artist, Option<String>)>,
    ) -> Result<()> {
        self.db
            .call(move |c| {
                for (album, art) in &albums {
                    catalog::remember_album(c, album, art.as_deref())?;
                }
                for (artist, image) in &artists {
                    catalog::remember_artist(c, artist, image.as_deref())?;
                }
                Ok(())
            })
            .await
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
        // Who the artist is comes from its identity, never with a name (`artist::identity`).
        musicbrainz_id: None,
    }
}

/// The album with its tracks: their ids, total length and saved state.
pub fn with_tracks(mut album: Album, tracks: &[Track]) -> Album {
    album.track_ids = tracks.iter().map(|t| t.id.clone()).collect();
    let total: u32 = tracks.iter().filter_map(|t| t.duration).sum();
    album.duration = (total > 0).then_some(total);
    let saved: HashSet<TrackId> = tracks.iter().filter(|t| t.saved).map(|t| t.id.clone()).collect();
    album.saved = album_saved(&album, &album.track_ids, &saved);
    album
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
    new
}

/// Stores artists, merged with what is stored, and returns them as the store has them. Who an
/// artist is comes only from its ids (`artist::identity`): an MBID a source gives with a name
/// isn't kept, and an artist without an MBID keeps no artist tags (a name's tags may be any
/// namesake's; MusicBrainz's by MBID are the artist's).
fn put_artists(c: &Connection, mut artists: Vec<Artist>) -> Result<Vec<Artist>> {
    let mut confirmed = HashSet::new();
    for a in &mut artists {
        a.musicbrainz_id = None;
        if artist_ids(c, &a.id)?.mbid().is_some() {
            confirmed.insert(a.id.clone());
        } else {
            a.tags.clear();
        }
    }
    let merge = |old, new| {
        let mut merged = merge_artist(old, new);
        if !confirmed.contains(&merged.id) {
            merged.tags.clear();
        }
        merged
    };
    let stored = merge_put(c, "discovery_artists", artists, |a| &a.id.0, merge)?;
    stored.into_iter().map(|a| identified(c, a)).collect()
}

/// The artist with the MBID its identity holds (none when it holds none).
fn identified(c: &Connection, mut a: Artist) -> Result<Artist> {
    a.musicbrainz_id = artist_ids(c, &a.id)?.mbid().map(str::to_owned);
    Ok(a)
}

fn artist_ids(c: &Connection, id: &ArtistId) -> Result<Ids> {
    let rows = rows(
        c,
        "SELECT source, id, evidence FROM discovery_artist_ids WHERE artist_id = ?1",
        [&id.0],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)),
    )?;
    let mut ids = Ids::default();
    for (source, value, evidence) in rows {
        // A row an older build wrote that no longer parses is left out.
        if let (Ok(source), Ok(evidence)) = (
            serde_json::from_value::<Source>(serde_json::Value::String(source)),
            serde_json::from_str::<Evidence>(&evidence),
        ) {
            ids.set(IdRow {
                source,
                id: value,
                evidence,
            });
        }
    }
    Ok(ids)
}

fn replace_ids(c: &mut Connection, id: &ArtistId, ids: &Ids, checked: i64) -> Result<()> {
    let tx = c.transaction()?;
    let before = artist_ids(&tx, id)?;
    tx.execute("DELETE FROM discovery_artist_ids WHERE artist_id = ?1", [&id.0])?;
    for row in ids.rows() {
        put_artist_id(&tx, id, row)?;
    }
    tx.execute(
        "INSERT INTO discovery_artist_checked (artist_id, checked_at) VALUES (?1, ?2)
         ON CONFLICT(artist_id) DO UPDATE SET checked_at = excluded.checked_at",
        params![id.0, checked],
    )?;
    if before.mbid() != ids.mbid() {
        forget_named(&tx, id)?;
    }
    tx.commit()?;
    Ok(())
}

fn put_artist_id(c: &Connection, id: &ArtistId, row: &IdRow) -> Result<()> {
    let evidence = serde_json::to_string(&row.evidence).map_err(|e| Error::internal(format!("evidence json: {e}")))?;
    c.execute(
        "INSERT INTO discovery_artist_ids (artist_id, source, id, evidence) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(artist_id, source) DO UPDATE SET id = excluded.id, evidence = excluded.evidence",
        params![id.0, row.source.as_str(), row.id, evidence],
    )?;
    Ok(())
}

/// Drops what was kept for an artist's old MBID: its tags and its photo's verdict.
fn forget_named(c: &Connection, id: &ArtistId) -> Result<()> {
    c.execute(
        "UPDATE discovery_artists SET json = json_set(json, '$.tags', json('[]')) WHERE id = ?1",
        [&id.0],
    )?;
    c.execute("DELETE FROM discovery_artist_photos WHERE artist_id = ?1", [&id.0])?;
    Ok(())
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

/// Records tracks in the catalog and returns each as the catalog then has it: a known track
/// keeps every fact it has; this source only fills its gaps (`catalog::remember_found`).
fn record(c: &Connection, tracks: Vec<Track>) -> Result<Vec<Track>> {
    catalog::remember_found(c, tracks)
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
        songs: rows(
            c,
            "SELECT artist, title, album FROM plays WHERE started_at >= ?1
             UNION SELECT artist, title, album FROM likes",
            [since],
            |r| {
                Ok(Named {
                    artist: r.get(0)?,
                    title: r.get(1)?,
                    album: r.get(2)?,
                })
            },
        )?,
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

/// The plays of the last `days` that were heard (to the end or past half, as the taste model
/// counts them), as (track, artist, plays, last played), most played first. A play that ended
/// at once (a stream that never started, a queue skipped through) isn't listening.
pub fn recent_plays(c: &Connection, now: i64, days: i64) -> Result<Vec<(TrackId, String, u32, i64)>> {
    rows(
        c,
        "SELECT track_id, artist, COUNT(*), MAX(started_at) FROM plays
         WHERE started_at >= ?1 AND (completed = 1 OR (duration_s > 0 AND played_s * 2 >= duration_s))
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

#[cfg(test)]
mod tests {
    use lull_core::Result;
    use lull_core::ids::ArtistId;
    use lull_core::model::Artist;
    use rusqlite::Connection;

    use super::{artist, artist_ids, put_artists, recent_plays, replace_ids};
    use crate::artist::identity::{Evidence, IdRow, Ids, Source};
    use crate::sources::ArtistInfo;

    fn discovery_db() -> Result<Connection> {
        let c = Connection::open_in_memory()?;
        for step in crate::MIGRATIONS.steps {
            c.execute_batch(step)?;
        }
        Ok(c)
    }

    fn tagged(name: &str, mbid: &str, tags: &[&str]) -> Artist {
        let mut a = artist(ArtistInfo {
            mbid: None,
            name: name.into(),
        });
        a.musicbrainz_id = Some(mbid.into());
        a.tags = tags.iter().map(|t| (*t).to_owned()).collect();
        a
    }

    #[test]
    fn an_artist_is_who_its_ids_say_never_what_comes_with_its_name() -> Result<()> {
        // Live, Last.fm's MBID for "DOUDOU" (Doudou Masta's) came with the name, and its tags
        // (hip hop, reggae) led the listener's taste.
        let mut c = discovery_db()?;
        let stored = put_artists(&c, vec![tagged("DOUDOU", "562b88d2-masta", &["hip hop", "reggae"])])?;
        assert_eq!(stored[0].musicbrainz_id, None, "an MBID given with a name isn't kept");
        assert!(stored[0].tags.is_empty(), "without an MBID of its own, no artist tags");

        let id = ArtistId::of("DOUDOU");
        let mut ids = Ids::default();
        ids.set(IdRow {
            source: Source::Musicbrainz,
            id: "f0fa7600-group".into(),
            evidence: Evidence::Link { source: Source::Itunes },
        });
        replace_ids(&mut c, &id, &ids, 100)?;
        let stored = put_artists(&c, vec![tagged("DOUDOU", "562b88d2-masta", &["mandopop"])])?;
        assert_eq!(stored[0].musicbrainz_id.as_deref(), Some("f0fa7600-group"));
        assert_eq!(stored[0].tags, ["mandopop"], "its MBID is its own: its tags count");

        replace_ids(&mut c, &id, &Ids::default(), 200)?;
        assert!(artist_ids(&c, &id)?.is_empty());
        let stored = put_artists(&c, vec![tagged("DOUDOU", "x", &[])])?;
        assert!(stored[0].tags.is_empty(), "the MBID gone, its tags go with it");
        Ok(())
    }

    #[test]
    fn played_lately_counts_only_plays_that_were_heard() -> Result<()> {
        // Live, 83 plays of one song that ended at 0 s (test traffic) made a "played 80 times
        // lately" radio starter.
        let c = Connection::open_in_memory()?;
        c.execute_batch(
            "CREATE TABLE plays (track_id TEXT, artist TEXT, started_at INTEGER, played_s INTEGER,
                                 duration_s INTEGER, completed INTEGER, skipped INTEGER);
             INSERT INTO plays VALUES
               ('t-loop', 'Velvet Orchard', 100, 0, 285, 0, 0), ('t-loop', 'Velvet Orchard', 120, 0, 285, 0, 0),
               ('t-loop', 'Velvet Orchard', 140, 0, 285, 0, 0), ('t-loop', 'Velvet Orchard', 160, 285, 285, 1, 0),
               ('t-half', 'Glow Tape', 200, 150, 280, 0, 0), ('t-half', 'Glow Tape', 300, 160, 280, 0, 0),
               ('t-skip', 'Pale Harbor', 400, 20, 200, 0, 1);",
        )?;
        let plays: Vec<(String, u32)> = recent_plays(&c, 1_000, 1)?
            .into_iter()
            .map(|(id, _, n, _)| (id.0, n))
            .collect();
        assert_eq!(plays, [("t-half".to_owned(), 2), ("t-loop".to_owned(), 1)]);
        Ok(())
    }
}
