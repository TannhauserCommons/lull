//! Every track lull has shown, by id. Ids are hashes, so they can't be turned back into a song;
//! any crate that hands a track to the UI records it here, and any crate can resolve a track id
//! it is given (queue, playlists, likes, history) without asking the crate that found it.
//! Albums and artists found by catalog search are kept the same way, with where their picture
//! comes from (ADR-0008).

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
    ],
};

/// Records or refreshes tracks (latest facts win).
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
