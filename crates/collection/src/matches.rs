//! What the catalog matched a song another service lists to, by that service's id for the song
//! (`deezer:track:<id>`, `youtube:<video>`). Link imports and outside playlist previews share it,
//! so a song looked up once (at iTunes' pace, about 12 a minute) isn't looked up again: a
//! preview's matches make its copy instant, and the other way round. "Nothing matches" is kept
//! with the rule that said so (`seams::FIND_SONG_RULE`): a newer rule asks again. A match stays a
//! match, unless today's rule doesn't take it (`recheck`: rule 3 told a drum edit from the plain
//! song it had been matched to); the listener's own picks are never touched.

use std::collections::HashMap;
use std::sync::Arc;

use lull_core::ids::TrackId;
use lull_core::model::SongQuery;
use lull_core::seams::FIND_SONG_RULE;
use lull_core::{Result, catalog};
use rusqlite::params;

use crate::Service;
use crate::links::{self, Song};

/// "Nothing matches" is believed this long (seconds): the catalog gains songs.
const NONE_FOR: i64 = 86_400;

/// The song's track id: remembered, else looked up through the `Catalog` seam and remembered.
/// `None` when nothing matches. A failed lookup is an error and is not remembered.
pub(crate) async fn find(svc: &Service, song: &Song) -> Result<Option<TrackId>> {
    if let Some(known) = known(svc, vec![song.key.clone()]).await?.remove(&song.key) {
        return Ok(known);
    }
    let found = svc.catalog.find_song(&query_of(song)).await?.map(|t| t.id);
    remember(svc, &song.key, found.clone()).await?;
    Ok(found)
}

/// The remembered answers among `keys`: `Some(id)` matched (and still in the catalog), `None`
/// matched nothing lately under today's rule. Keys without a usable answer are left out.
pub(crate) async fn known(svc: &Service, keys: Vec<String>) -> Result<HashMap<String, Option<TrackId>>> {
    svc.ctx
        .db
        .call(move |c| {
            let now = lull_core::now();
            let mut stmt =
                c.prepare("SELECT track_id, checked_at, rule FROM collection_song_matches WHERE song = ?1")?;
            let mut out = HashMap::new();
            for key in keys {
                let row: Option<(Option<String>, i64, i64)> = rusqlite::OptionalExtension::optional(
                    stmt.query_row([&key], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))),
                )?;
                match row {
                    Some((Some(id), _, _)) if catalog::track(c, &TrackId(id.clone()))?.is_some() => {
                        out.insert(key, Some(TrackId(id)));
                    }
                    Some((None, at, rule)) if rule == FIND_SONG_RULE && now - at < NONE_FOR => {
                        out.insert(key, None);
                    }
                    _ => {}
                }
            }
            Ok(out)
        })
        .await
}

/// Matches an older rule made, for songs a link lists: each is checked by today's rule without a
/// lookup (`Catalog::same_take`); one it still takes is stamped with the rule, one it doesn't is
/// looked up again and the playlists of the links that list it refilled. The listener's own picks
/// stay as they are (`collection_link_choices` comes first in a link's lines). Returns how many
/// matches changed. Run once at start: a rule changes with a release.
pub(crate) async fn recheck(svc: &Arc<Service>) -> Result<usize> {
    type Old = (Song, TrackId, Option<lull_core::model::Track>);
    let old: Vec<Old> = svc
        .ctx
        .db
        .call(|c| {
            let mut stmt = c.prepare(
                "SELECT m.song, m.track_id, s.artist, s.title, s.album, s.duration, s.isrc, s.picture
                 FROM collection_song_matches m JOIN collection_link_songs s ON s.song = m.song
                 WHERE m.track_id IS NOT NULL AND m.rule < ?1 GROUP BY m.song",
            )?;
            let rows = stmt.query_map([FIND_SONG_RULE], |r| {
                Ok((
                    Song {
                        key: r.get(0)?,
                        artist: r.get(2)?,
                        title: r.get(3)?,
                        album: r.get(4)?,
                        duration: r.get(5)?,
                        isrc: r.get(6)?,
                        added: None,
                        picture: r.get(7)?,
                    },
                    TrackId(r.get(1)?),
                ))
            })?;
            let mut out = Vec::new();
            for row in rows {
                let (song, id) = row?;
                let track = catalog::track(c, &id)?;
                out.push((song, id, track));
            }
            Ok(out)
        })
        .await?;
    let mut changed = 0;
    for (song, id, track) in old {
        let query = query_of(&song);
        let still = track.is_some_and(|t| svc.catalog.same_take(&query, &t));
        if still {
            stamp(svc, &song.key).await?;
            continue;
        }
        let found = svc.catalog.find_song(&query).await?.map(|t| t.id);
        remember(svc, &song.key, found.clone()).await?;
        if found.as_ref() != Some(&id) {
            changed += 1;
            svc.write(move |tx, events| links::rematched(tx, events, &song, found))
                .await?;
        }
    }
    Ok(changed)
}

fn query_of(song: &Song) -> SongQuery {
    SongQuery {
        artist: song.artist.clone(),
        title: song.title.clone(),
        album: song.album.clone(),
        duration: song.duration,
        isrc: song.isrc.clone(),
    }
}

/// Today's rule still takes the song's match.
async fn stamp(svc: &Service, key: &str) -> Result<()> {
    let key = key.to_owned();
    svc.ctx
        .db
        .call(move |c| {
            c.execute(
                "UPDATE collection_song_matches SET rule = ?2 WHERE song = ?1",
                params![key, FIND_SONG_RULE],
            )?;
            Ok(())
        })
        .await
}

async fn remember(svc: &Service, key: &str, found: Option<TrackId>) -> Result<()> {
    let key = key.to_owned();
    svc.ctx
        .db
        .call(move |c| {
            c.execute(
                "INSERT INTO collection_song_matches (song, track_id, checked_at, rule) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(song) DO UPDATE SET track_id = excluded.track_id, checked_at = excluded.checked_at,
                 rule = excluded.rule",
                params![
                    key,
                    found.as_ref().map(TrackId::as_str),
                    lull_core::now(),
                    FIND_SONG_RULE
                ],
            )?;
            Ok(())
        })
        .await
}
