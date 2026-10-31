//! Every track lull has shown, by id. Ids are hashes, so they can't be turned back into a song;
//! any crate that hands a track to the UI records it here, and any crate can resolve a track id
//! it is given (queue, playlists, likes, history) without asking the crate that found it.

use rusqlite::{Connection, OptionalExtension, params};

use crate::db::Migrations;
use crate::error::{Error, Result};
use crate::ids::TrackId;
use crate::model::Track;

pub const MIGRATIONS: Migrations = Migrations {
    owner: "catalog",
    steps: &[r#"
        CREATE TABLE catalog_tracks (
            id          TEXT PRIMARY KEY,
            json        TEXT NOT NULL,
            updated_at  INTEGER NOT NULL
        );
    "#],
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
