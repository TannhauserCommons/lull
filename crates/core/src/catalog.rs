//! Every track lull has shown, by id. Ids are hashes, so they can't be turned back into a song;
//! any crate that hands a track to the UI records it here, and any crate can resolve a track id
//! it is given (queue, playlists, likes, history) without asking the crate that found it.
//! Albums and artists found by catalog search are kept the same way, with where their picture
//! comes from (ADR-0008).
//!
//! **Bare songs.** A crate that records a song from another source's text (an imported line, a
//! playlist's song lull has no match for, a Last.fm-only result, file tags) records it bare: no
//! album lull has a picture for. It may add what the source said (`hint`: an ISRC, its own
//! picture). Discovery resolves bare songs when the UI shows them (`POST /api/catalog/resolve`):
//! it finds the catalog's song and `enrich`es the bare record in place, keeping its id, and
//! remembers each attempt here, so no crate has to resolve songs itself.

use rusqlite::{Connection, OptionalExtension, params};

use crate::db::Migrations;
use crate::error::{Error, Result};
use crate::ids::{AlbumId, ArtistId, TrackId};
use crate::model::{Album, Artist, Track};

pub const MIGRATIONS: Migrations = Migrations {
    owner: "catalog",
    steps: &[
        r#"
        CREATE TABLE catalog_tracks (
            id          TEXT PRIMARY KEY,
            json        TEXT NOT NULL,
            updated_at  INTEGER NOT NULL
        );
    "#,
        r#"
        ALTER TABLE catalog_tracks
            ADD COLUMN album_id TEXT GENERATED ALWAYS AS (json_extract(json, '$.albumId')) VIRTUAL;
        CREATE INDEX catalog_tracks_album ON catalog_tracks(album_id);
    "#,
        r#"
        -- `art` / `image`: an outside picture URL; `{size}` in it is the wanted size in pixels.
        CREATE TABLE catalog_albums (
            id          TEXT PRIMARY KEY,
            json        TEXT NOT NULL,
            art         TEXT,
            updated_at  INTEGER NOT NULL
        );
        CREATE TABLE catalog_artists (
            id          TEXT PRIMARY KEY,
            json        TEXT NOT NULL,
            image       TEXT,
            updated_at  INTEGER NOT NULL
        );
    "#,
        r#"
        -- Soulseek searches used to overwrite a known track with the peer's tags, moving it to the
        -- tags' album ("Cyberpunk_ Edgerunners", the track artist's): a track that a catalog album
        -- lists but that sits on no album listing it goes back to that album and its title.
        UPDATE catalog_tracks
           SET json = json_set(json, '$.albumId', fix.album_id, '$.album', fix.title)
          FROM (SELECT j.value AS track_id, MIN(a.id) AS album_id, json_extract(a.json, '$.title') AS title
                  FROM catalog_albums a, json_each(a.json, '$.trackIds') j
                 GROUP BY j.value) AS fix
         WHERE catalog_tracks.id = fix.track_id
           AND NOT EXISTS (SELECT 1 FROM catalog_albums own, json_each(own.json, '$.trackIds') o
                            WHERE own.id = catalog_tracks.album_id AND o.value = catalog_tracks.id);
    "#,
        r#"
        -- Bare songs (see the module doc): what their source said, and the last attempt to find
        -- them in the catalog. `resolved` = enriched; a miss is tried again after a day.
        CREATE TABLE catalog_resolutions (
            track_id    TEXT PRIMARY KEY,
            isrc        TEXT,
            checked_at  INTEGER,
            resolved    INTEGER NOT NULL DEFAULT 0
        );
    "#,
    ],
};

/// A miss is looked up again after this long (seconds): the catalog gains songs.
pub const RESOLVE_AGAIN_S: i64 = 86_400;

/// Records songs a source found (search, Soulseek's files) and returns each as the catalog then
/// has it: a song already known keeps what it has (title, artist and album as first recorded, or
/// as `enrich` set them) and only gains the facts it lacks; file facts are the newest.
/// Sources that find songs use this, so none can overwrite a richer record with its own text.
pub fn remember_found(conn: &Connection, tracks: Vec<Track>) -> Result<Vec<Track>> {
    let mut out = Vec::with_capacity(tracks.len());
    let mut changed = Vec::new();
    for t in tracks {
        let known = track(conn, &t.id)?;
        let merged = match known.clone() {
            Some(k) => fill_gaps(k, t),
            None => t,
        };
        if known.as_ref() != Some(&merged) {
            changed.push(merged.clone());
        }
        out.push(merged);
    }
    remember(conn, &changed)?;
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
    known.file = new.file.or(known.file);
    known
}

/// Records or refreshes tracks as given (latest facts win): for the crate that owns a song's
/// facts (the library's saved songs). Others use `remember_found` or `remember_named`.
pub fn remember(conn: &Connection, tracks: &[Track]) -> Result<()> {
    let mut stmt = conn.prepare_cached(
        "INSERT INTO catalog_tracks (id, json, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(id) DO UPDATE SET json = excluded.json, updated_at = excluded.updated_at",
    )?;
    for t in tracks {
        let json = serde_json::to_string(t).map_err(|e| Error::internal(format!("track json: {e}")))?;
        stmt.execute(params![t.id.as_str(), json, crate::now()])?;
    }
    Ok(())
}

pub fn track(conn: &Connection, id: &TrackId) -> Result<Option<Track>> {
    let json: Option<String> = conn
        .query_row("SELECT json FROM catalog_tracks WHERE id = ?1", [id.as_str()], |r| {
            r.get(0)
        })
        .optional()?;
    json.map(|j| serde_json::from_str(&j).map_err(|e| Error::internal(format!("catalog row {id}: {e}"))))
        .transpose()
}

/// Any one recorded track on `album`: how an album id is turned back into artist and title
/// (cover art for albums that aren't saved).
pub fn track_on_album(conn: &Connection, album: &AlbumId) -> Result<Option<Track>> {
    let json: Option<String> = conn
        .query_row(
            "SELECT json FROM catalog_tracks WHERE album_id = ?1 LIMIT 1",
            [album.as_str()],
            |r| r.get(0),
        )
        .optional()?;
    json.map(|j| serde_json::from_str(&j).map_err(|e| Error::internal(format!("catalog row on {album}: {e}"))))
        .transpose()
}

/// Records or refreshes an album (latest facts win). `art` is an outside cover URL, where `{size}`
/// stands for the wanted size in pixels; a known one is kept when `art` is `None`.
pub fn remember_album(conn: &Connection, album: &Album, art: Option<&str>) -> Result<()> {
    let json = serde_json::to_string(album).map_err(|e| Error::internal(format!("album json: {e}")))?;
    conn.prepare_cached(
        "INSERT INTO catalog_albums (id, json, art, updated_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(id) DO UPDATE SET json = excluded.json, art = COALESCE(excluded.art, art),
                                       updated_at = excluded.updated_at",
    )?
    .execute(params![album.id.as_str(), json, art, crate::now()])?;
    Ok(())
}

pub fn album(conn: &Connection, id: &AlbumId) -> Result<Option<Album>> {
    row(conn, "SELECT json FROM catalog_albums WHERE id = ?1", id.as_str())
}

/// The album's outside cover URL (`{size}` = pixels), when one is known.
pub fn album_art(conn: &Connection, id: &AlbumId) -> Result<Option<String>> {
    picture(conn, "SELECT art FROM catalog_albums WHERE id = ?1", id.as_str())
}

/// Records the cover found for an album lull knew only by name (`art` with `{size}`), on its
/// catalog record, or on a record of its own named after a song on it. `false` when lull knows
/// neither.
pub fn found_album_art(conn: &Connection, id: &AlbumId, art: &str) -> Result<bool> {
    if conn.execute(
        "UPDATE catalog_albums SET art = ?2 WHERE id = ?1",
        params![id.as_str(), art],
    )? > 0
    {
        return Ok(true);
    }
    let Some(song) = track_on_album(conn, id)? else {
        return Ok(false);
    };
    let album = Album {
        id: id.clone(),
        title: song.album.clone(),
        artist_id: song.artist_id.clone(),
        artist: song.artist.clone(),
        year: None,
        kind: None,
        released: None,
        label: None,
        tags: Vec::new(),
        saved: crate::model::Saved::None,
        track_ids: vec![song.id],
        duration: None,
        musicbrainz_id: None,
    };
    remember_album(conn, &album, Some(art))?;
    Ok(true)
}

/// Records or refreshes an artist, like `remember_album`; `image` is an outside photo URL.
pub fn remember_artist(conn: &Connection, artist: &Artist, image: Option<&str>) -> Result<()> {
    let json = serde_json::to_string(artist).map_err(|e| Error::internal(format!("artist json: {e}")))?;
    conn.prepare_cached(
        "INSERT INTO catalog_artists (id, json, image, updated_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(id) DO UPDATE SET json = excluded.json, image = COALESCE(excluded.image, image),
                                       updated_at = excluded.updated_at",
    )?
    .execute(params![artist.id.as_str(), json, image, crate::now()])?;
    Ok(())
}

pub fn artist(conn: &Connection, id: &ArtistId) -> Result<Option<Artist>> {
    row(conn, "SELECT json FROM catalog_artists WHERE id = ?1", id.as_str())
}

/// The artist's outside photo URL (`{size}` = pixels), when one is known.
pub fn artist_image(conn: &Connection, id: &ArtistId) -> Result<Option<String>> {
    picture(conn, "SELECT image FROM catalog_artists WHERE id = ?1", id.as_str())
}

/// A song as another source names it (an imported line, a playlist's song lull has no match
/// for), for `remember_named`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Named<'a> {
    pub artist: &'a str,
    /// The album the source names, if any.
    pub album: Option<&'a str>,
    pub title: &'a str,
    pub duration: Option<u32>,
    pub isrc: Option<&'a str>,
    /// The source's own picture for the song (an outside URL), its cover until the catalog's
    /// song is found.
    pub picture: Option<&'a str>,
}

/// Records a song as its source names it, the one way every crate does so: the id is
/// `TrackId::of(artist, album, title)`, a richer record already there is kept, the source's
/// ISRC is kept as a hint, and its own picture becomes the cover of its album (of the song alone
/// when the source names no album). Discovery resolves it to the catalog's song when shown.
pub fn remember_named(conn: &Connection, n: Named) -> Result<Track> {
    let (artist, title, album) = (n.artist.trim(), n.title.trim(), n.album.map(str::trim).unwrap_or(""));
    let id = TrackId::of(artist, album, title);
    if let Some(isrc) = n.isrc.filter(|i| !i.trim().is_empty()) {
        hint_isrc(conn, &id, isrc.trim())?;
    }
    if let Some(known) = track(conn, &id)? {
        return Ok(known);
    }
    // Without an album, a picture belongs to the song alone: an album of its own, named after it.
    let album_id = match (album.is_empty(), n.picture) {
        (true, Some(_)) => AlbumId::of(artist, title),
        _ => AlbumId::of(artist, album),
    };
    let song = Track {
        id,
        title: title.to_owned(),
        artist_id: ArtistId::of(artist),
        artist: artist.to_owned(),
        album_id: album_id.clone(),
        album: album.to_owned(),
        year: None,
        no: None,
        disc: None,
        duration: n.duration,
        tags: Vec::new(),
        saved: false,
        file: None,
    };
    remember(conn, std::slice::from_ref(&song))?;
    if let Some(picture) = n.picture.filter(|p| p.starts_with("https://"))
        && album_art(conn, &album_id)?.is_none()
    {
        let own = Album {
            id: album_id,
            title: if album.is_empty() {
                title.to_owned()
            } else {
                album.to_owned()
            },
            artist_id: song.artist_id.clone(),
            artist: artist.to_owned(),
            year: None,
            kind: Some(if album.is_empty() { "single" } else { "album" }.to_owned()),
            released: None,
            label: None,
            tags: Vec::new(),
            saved: crate::model::Saved::None,
            track_ids: vec![song.id.clone()],
            duration: None,
            musicbrainz_id: None,
        };
        remember_album(conn, &own, Some(picture))?;
    }
    Ok(song)
}

/// Whether the UI would show the song without a cover: not saved (the library has its own), and
/// its album has no picture source here.
pub fn is_bare(conn: &Connection, track: &Track) -> Result<bool> {
    Ok(!track.saved && album_art(conn, &track.album_id)?.is_none())
}

/// What the source said about a bare song: its ISRC, which resolves it exactly.
pub fn hint_isrc(conn: &Connection, id: &TrackId, isrc: &str) -> Result<()> {
    conn.prepare_cached(
        "INSERT INTO catalog_resolutions (track_id, isrc) VALUES (?1, ?2)
         ON CONFLICT(track_id) DO UPDATE SET isrc = excluded.isrc",
    )?
    .execute(params![id.as_str(), isrc])?;
    Ok(())
}

/// A bare song's ISRC hint, and whether it is due a lookup: never tried, or missed over
/// `RESOLVE_AGAIN_S` ago, and not resolved.
pub fn resolution(conn: &Connection, id: &TrackId, now: i64) -> Result<(Option<String>, bool)> {
    let row: Option<(Option<String>, Option<i64>, bool)> = conn
        .prepare_cached("SELECT isrc, checked_at, resolved FROM catalog_resolutions WHERE track_id = ?1")?
        .query_row([id.as_str()], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .optional()?;
    Ok(match row {
        None => (None, true),
        Some((isrc, checked, resolved)) => (isrc, !resolved && checked.is_none_or(|at| now - at >= RESOLVE_AGAIN_S)),
    })
}

/// Records a lookup of a bare song that found nothing.
pub fn missed(conn: &Connection, id: &TrackId, now: i64) -> Result<()> {
    conn.prepare_cached(
        "INSERT INTO catalog_resolutions (track_id, checked_at) VALUES (?1, ?2)
         ON CONFLICT(track_id) DO UPDATE SET checked_at = excluded.checked_at",
    )?
    .execute(params![id.as_str(), now])?;
    Ok(())
}

/// Fills the bare song `id` with the catalog's facts for the same song (title and artist as the
/// catalog spells them, album, year, length), keeping its own id, so every playlist, like and
/// play that names it shows the catalog's song. A saved song is left alone. The enriched record.
pub fn enrich(conn: &Connection, id: &TrackId, found: &Track, now: i64) -> Result<Option<Track>> {
    let Some(bare) = track(conn, id)? else { return Ok(None) };
    if bare.saved {
        return Ok(None);
    }
    let enriched = Track {
        id: bare.id,
        saved: bare.saved,
        file: bare.file,
        ..found.clone()
    };
    remember(conn, std::slice::from_ref(&enriched))?;
    conn.prepare_cached(
        "INSERT INTO catalog_resolutions (track_id, checked_at, resolved) VALUES (?1, ?2, 1)
         ON CONFLICT(track_id) DO UPDATE SET checked_at = excluded.checked_at, resolved = 1",
    )?
    .execute(params![id.as_str(), now])?;
    Ok(Some(enriched))
}

fn row<T: serde::de::DeserializeOwned>(conn: &Connection, sql: &str, id: &str) -> Result<Option<T>> {
    let json: Option<String> = conn.prepare_cached(sql)?.query_row([id], |r| r.get(0)).optional()?;
    json.map(|j| serde_json::from_str(&j).map_err(|e| Error::internal(format!("catalog row {id}: {e}"))))
        .transpose()
}

fn picture(conn: &Connection, sql: &str, id: &str) -> Result<Option<String>> {
    Ok(conn
        .prepare_cached(sql)?
        .query_row([id], |r| r.get::<_, Option<String>>(0))
        .optional()?
        .flatten())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use crate::ids::ArtistId;

    #[tokio::test]
    async fn finds_a_track_by_its_album() -> Result<()> {
        let db = Db::memory()?;
        db.migrate(&MIGRATIONS)?;
        let track = Track {
            id: TrackId::of("Velvet Orchard", "Satellite Porch", "Paper Moon Static"),
            title: "Paper Moon Static".into(),
            artist_id: ArtistId::of("Velvet Orchard"),
            artist: "Velvet Orchard".into(),
            album_id: AlbumId::of("Velvet Orchard", "Satellite Porch"),
            album: "Satellite Porch".into(),
            year: None,
            no: None,
            disc: None,
            duration: None,
            tags: vec![],
            saved: false,
            file: None,
        };
        let album = track.album_id.clone();
        let found = db
            .call(move |c| {
                remember(c, &[track])?;
                track_on_album(c, &album)
            })
            .await?;
        assert_eq!(found.map(|t| t.title), Some("Paper Moon Static".into()));
        Ok(())
    }

    #[tokio::test]
    async fn an_album_keeps_its_cover_when_a_later_source_has_none() -> Result<()> {
        let db = Db::memory()?;
        db.migrate(&MIGRATIONS)?;
        let album = Album {
            id: AlbumId::of("Velvet Orchard", "Satellite Porch"),
            title: "Satellite Porch".into(),
            artist_id: ArtistId::of("Velvet Orchard"),
            artist: "Velvet Orchard".into(),
            year: Some(2026),
            kind: Some("single".into()),
            released: None,
            label: None,
            tags: vec![],
            saved: crate::model::Saved::None,
            track_ids: vec![],
            duration: None,
            musicbrainz_id: None,
        };
        let id = album.id.clone();
        let (art, kept) = db
            .call(move |c| {
                remember_album(c, &album, Some("https://img.example/{size}.jpg"))?;
                remember_album(c, &album, None)?;
                Ok((album_art(c, &album.id)?, self::album(c, &album.id)?))
            })
            .await?;
        assert_eq!(art.as_deref(), Some("https://img.example/{size}.jpg"));
        assert_eq!(kept.map(|a| a.id), Some(id));
        Ok(())
    }

    #[tokio::test]
    async fn a_track_a_peer_moved_to_its_tags_album_goes_back_to_the_album_listing_it() -> Result<()> {
        let db = Db::memory()?;
        db.migrate(&Migrations {
            owner: MIGRATIONS.owner,
            steps: &MIGRATIONS.steps[..3],
        })?;
        let album = Album {
            id: AlbumId::of("Various Artists", "Cyberpunk: Edgerunners 2"),
            title: "Cyberpunk: Edgerunners 2".into(),
            artist_id: ArtistId::of("Various Artists"),
            artist: "Various Artists".into(),
            year: Some(2026),
            kind: Some("compilation".into()),
            released: None,
            label: None,
            tags: vec![],
            saved: crate::model::Saved::None,
            track_ids: vec![TrackId::of(
                "Rico Nasty",
                "Cyberpunk: Edgerunners 2",
                "You Can't Run From Me",
            )],
            duration: None,
            musicbrainz_id: None,
        };
        let moved = Track {
            id: album.track_ids[0].clone(),
            title: "You Can't Run From Me".into(),
            artist_id: ArtistId::of("Rico Nasty"),
            artist: "Rico Nasty".into(),
            album_id: AlbumId::of("Rico Nasty", "Cyberpunk_ Edgerunners 2"),
            album: "Cyberpunk_ Edgerunners 2".into(),
            year: None,
            no: None,
            disc: None,
            duration: None,
            tags: vec![],
            saved: false,
            file: None,
        };
        let (id, want) = (moved.id.clone(), album.id.clone());
        db.call(move |c| {
            remember_album(c, &album, None)?;
            remember(c, &[moved])
        })
        .await?;
        db.migrate(&MIGRATIONS)?;
        let fixed = db
            .call(move |c| track(c, &id))
            .await?
            .ok_or(Error::internal("track gone"))?;
        assert_eq!(fixed.album_id, want);
        assert_eq!(fixed.album, "Cyberpunk: Edgerunners 2");
        Ok(())
    }
}

#[cfg(test)]
mod bare_tests {
    use super::*;
    use crate::db::Db;

    fn song(artist: &str, album: &str, title: &str) -> Track {
        Track {
            id: TrackId::of(artist, album, title),
            title: title.into(),
            artist_id: crate::ids::ArtistId::of(artist),
            artist: artist.into(),
            album_id: AlbumId::of(artist, album),
            album: album.into(),
            year: None,
            no: None,
            disc: None,
            duration: Some(200),
            tags: Vec::new(),
            saved: false,
            file: None,
        }
    }

    fn album(t: &Track) -> Album {
        Album {
            id: t.album_id.clone(),
            title: t.album.clone(),
            artist_id: t.artist_id.clone(),
            artist: t.artist.clone(),
            year: None,
            kind: Some("single".into()),
            released: None,
            label: None,
            tags: Vec::new(),
            saved: crate::model::Saved::None,
            track_ids: Vec::new(),
            duration: None,
            musicbrainz_id: None,
        }
    }

    #[tokio::test]
    async fn a_bare_song_is_enriched_in_place_and_not_looked_up_again() -> Result<()> {
        let db = Db::memory()?;
        db.migrate(&MIGRATIONS)?;
        db.call(|c| {
            let bare = song("LUUDE", "", "DOWN UNDER (FEAT. COLIN HAY)");
            let found = song("Luude", "Down Under (feat. Colin Hay)", "Down Under (feat. Colin Hay)");
            remember(c, std::slice::from_ref(&bare))?;
            remember_album(c, &album(&found), Some("https://cdn.example/{size}.jpg"))?;
            assert!(is_bare(c, &bare)?);
            assert_eq!(resolution(c, &bare.id, 100)?, (None, true));

            let enriched = enrich(c, &bare.id, &found, 100)?.ok_or_else(|| Error::internal("not enriched"))?;
            assert_eq!(enriched.id, bare.id, "the id every playlist holds stays");
            assert_eq!(
                (enriched.title.as_str(), enriched.album_id.clone()),
                ("Down Under (feat. Colin Hay)", found.album_id.clone())
            );
            assert!(!is_bare(c, &enriched)?);
            assert_eq!(resolution(c, &bare.id, 100 + RESOLVE_AGAIN_S * 9)?, (None, false));
            Ok(())
        })
        .await
    }

    #[tokio::test]
    async fn a_miss_is_tried_again_after_a_day_and_keeps_its_isrc_hint() -> Result<()> {
        let db = Db::memory()?;
        db.migrate(&MIGRATIONS)?;
        db.call(|c| {
            let bare = song("Someone", "", "Lost");
            remember(c, std::slice::from_ref(&bare))?;
            hint_isrc(c, &bare.id, "GBUM71505902")?;
            missed(c, &bare.id, 1000)?;
            assert_eq!(
                resolution(c, &bare.id, 1000 + 60)?,
                (Some("GBUM71505902".into()), false)
            );
            assert!(resolution(c, &bare.id, 1000 + RESOLVE_AGAIN_S)?.1);
            Ok(())
        })
        .await
    }

    #[tokio::test]
    async fn a_named_song_keeps_its_sources_picture_and_isrc_and_a_richer_record() -> Result<()> {
        let db = Db::memory()?;
        db.migrate(&MIGRATIONS)?;
        db.call(|c| {
            let named = remember_named(
                c,
                Named {
                    artist: " Luude ",
                    title: "Down Under (feat. Colin Hay)",
                    isrc: Some("AUUM72100123"),
                    picture: Some("https://i.ytimg.com/vi/x/hqdefault.jpg"),
                    ..Named::default()
                },
            )?;
            assert_eq!(named.id, TrackId::of("Luude", "", "Down Under (feat. Colin Hay)"));
            assert!(!is_bare(c, &named)?, "the source's picture is its cover");
            assert_eq!(resolution(c, &named.id, 1)?.0.as_deref(), Some("AUUM72100123"));

            let plain = remember_named(
                c,
                Named {
                    artist: "Someone",
                    title: "Lost",
                    ..Named::default()
                },
            )?;
            assert!(is_bare(c, &plain)?);
            let rich = Track {
                year: Some(2021),
                ..plain.clone()
            };
            remember(c, std::slice::from_ref(&rich))?;
            let again = remember_named(
                c,
                Named {
                    artist: "Someone",
                    title: "Lost",
                    ..Named::default()
                },
            )?;
            assert_eq!(again.year, Some(2021), "a richer record already there is kept");
            Ok(())
        })
        .await
    }

    #[tokio::test]
    async fn a_found_song_fills_gaps_and_never_overwrites_what_is_known() -> Result<()> {
        let db = Db::memory()?;
        db.migrate(&MIGRATIONS)?;
        db.call(|c| {
            let bare = song("LUUDE", "", "DOWN UNDER");
            let resolved = Track {
                title: "Down Under".into(),
                artist: "Luude".into(),
                duration: None,
                ..bare.clone()
            };
            remember(c, std::slice::from_ref(&resolved))?;
            // Soulseek's file tags for the same id: the raw text again, a length, file facts.
            let tagged = Track {
                year: Some(2021),
                ..bare.clone()
            };
            let out = remember_found(c, vec![tagged])?;
            assert_eq!((out[0].title.as_str(), out[0].artist.as_str()), ("Down Under", "Luude"));
            assert_eq!((out[0].year, out[0].duration), (Some(2021), Some(200)), "gaps filled");
            assert_eq!(track(c, &bare.id)?.map(|t| t.title), Some("Down Under".into()));
            Ok(())
        })
        .await
    }

    #[tokio::test]
    async fn a_saved_song_is_never_overwritten() -> Result<()> {
        let db = Db::memory()?;
        db.migrate(&MIGRATIONS)?;
        db.call(|c| {
            let mut saved = song("A", "Album", "Song");
            saved.saved = true;
            remember(c, std::slice::from_ref(&saved))?;
            assert!(!is_bare(c, &saved)?);
            assert_eq!(enrich(c, &saved.id, &song("B", "Other", "Song"), 1)?, None);
            Ok(())
        })
        .await
    }
}
