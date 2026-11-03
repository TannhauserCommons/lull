//! What the catalog matched a song another service lists to, by that service's id for the song
//! (`deezer:track:<id>`, `youtube:<video>`). Link imports and outside playlist previews share it,
//! so a song looked up once (at iTunes' pace, about 12 a minute) isn't looked up again: a
//! preview's matches make its copy instant, and the other way round.

use std::collections::HashMap;

use lull_core::ids::TrackId;
use lull_core::model::SongQuery;
use lull_core::{Result, catalog};
use rusqlite::params;

use crate::Service;
use crate::links::Song;

/// "Nothing matches" is believed this long (seconds): the catalog gains songs.
const NONE_FOR: i64 = 86_400;

/// The song's track id: remembered, else looked up through the `Catalog` seam and remembered.
/// `None` when nothing matches. A failed lookup is an error and is not remembered.
pub(crate) async fn find(svc: &Service, song: &Song) -> Result<Option<TrackId>> {
    if let Some(known) = known(svc, vec![song.key.clone()]).await?.remove(&song.key) {
        return Ok(known);
    }
    let query = SongQuery {
        artist: song.artist.clone(),
        title: song.title.clone(),
        album: song.album.clone(),
        duration: song.duration,
        isrc: song.isrc.clone(),
    };
    let found = svc.catalog.find_song(&query).await?.map(|t| t.id);
    remember(svc, &song.key, found.clone()).await?;
    Ok(found)
}

/// The remembered answers among `keys`: `Some(id)` matched (and still in the catalog), `None`
/// matched nothing lately. Keys without a usable answer are left out.
pub(crate) async fn known(svc: &Service, keys: Vec<String>) -> Result<HashMap<String, Option<TrackId>>> {
    svc.ctx
        .db
        .call(move |c| {
            let now = lull_core::now();
            let mut stmt = c.prepare("SELECT track_id, checked_at FROM collection_song_matches WHERE song = ?1")?;
            let mut out = HashMap::new();
            for key in keys {
                let row: Option<(Option<String>, i64)> =
                    rusqlite::OptionalExtension::optional(stmt.query_row([&key], |r| Ok((r.get(0)?, r.get(1)?))))?;
                match row {
                    Some((Some(id), _)) if catalog::track(c, &TrackId(id.clone()))?.is_some() => {
                        out.insert(key, Some(TrackId(id)));
                    }
                    Some((None, at)) if now - at < NONE_FOR => {
                        out.insert(key, None);
                    }
                    _ => {}
                }
            }
            Ok(out)
        })
        .await
}

async fn remember(svc: &Service, key: &str, found: Option<TrackId>) -> Result<()> {
    let key = key.to_owned();
    svc.ctx
        .db
        .call(move |c| {
            c.execute(
                "INSERT INTO collection_song_matches (song, track_id, checked_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(song) DO UPDATE SET track_id = excluded.track_id, checked_at = excluded.checked_at",
                params![key, found.as_ref().map(TrackId::as_str), lull_core::now()],
            )?;
            Ok(())
        })
        .await
}
