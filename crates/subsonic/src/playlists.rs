//! Playlists through the `Collection` seam. A Subsonic client sees only the saved songs of a
//! playlist (it can't stream the others), so `songIndexToRemove` counts saved songs and is
//! mapped back to positions in the whole playlist.

use std::collections::HashMap;

use lull_core::ids::{AlbumId, TrackId};
use lull_core::model::{Playlist, PlaylistEdit, Track};
use serde_json::{Value, json};

use crate::params::Params;
use crate::response::Fail;
use crate::{Reply, Service, shape};

pub async fn list(svc: &Service) -> Result<Reply, Fail> {
    let playlists = svc.collection.playlists().await?;
    let ids: Vec<TrackId> = playlists.iter().flat_map(|p| p.track_ids.iter().cloned()).collect();
    let saved = svc.saved(&ids).await?;
    let playlist: Vec<Value> = playlists.iter().map(|p| summary(p, &visible(p, &saved))).collect();
    Ok(Reply::of("playlists", json!({ "playlist": playlist })))
}

pub async fn get(svc: &Service, p: &Params) -> Result<Reply, Fail> {
    let playlist = find(svc, p.required("id")?).await?;
    full(svc, &playlist).await
}

/// With `playlistId`, replaces that playlist's songs; otherwise creates one named `name`.
pub async fn create(svc: &Service, p: &Params) -> Result<Reply, Fail> {
    let songs: Vec<TrackId> = p.all("songId").map(|id| TrackId(id.to_owned())).collect();
    let playlist = match p.get("playlistId") {
        Some(id) => {
            let current = find(svc, id).await?;
            let edit = PlaylistEdit {
                name: p.get("name").map(str::to_owned),
                remove_positions: (0..current.track_ids.len()).collect(),
                add: songs,
                ..PlaylistEdit::default()
            };
            svc.collection.edit_playlist(id, edit).await?
        }
        None => svc.collection.create_playlist(p.required("name")?, &songs).await?,
    };
    full(svc, &playlist).await
}

pub async fn update(svc: &Service, p: &Params) -> Result<Reply, Fail> {
    let id = p.required("playlistId")?;
    let current = find(svc, id).await?;
    let saved = svc.saved(&current.track_ids).await?;
    let shown = visible(&current, &saved);
    let remove_positions = p
        .all("songIndexToRemove")
        .map(|i| {
            i.parse::<usize>()
                .ok()
                .and_then(|i| shown.get(i))
                .map(|(pos, _)| *pos)
                .ok_or_else(|| Fail::generic(format!("songIndexToRemove {i} is not in the playlist")))
        })
        .collect::<Result<_, _>>()?;
    let edit = PlaylistEdit {
        name: p.get("name").map(str::to_owned),
        description: p.get("comment").map(str::to_owned),
        remove_positions,
        add: p.all("songIdToAdd").map(|id| TrackId(id.to_owned())).collect(),
    };
    svc.collection.edit_playlist(id, edit).await?;
    Ok(Reply::empty())
}

pub async fn delete(svc: &Service, p: &Params) -> Result<Reply, Fail> {
    svc.collection.delete_playlist(p.required("id")?).await?;
    Ok(Reply::empty())
}

async fn find(svc: &Service, id: &str) -> Result<Playlist, Fail> {
    svc.collection
        .playlist(id)
        .await?
        .ok_or_else(|| Fail::not_found(format!("no playlist {id}")))
}

async fn full(svc: &Service, playlist: &Playlist) -> Result<Reply, Fail> {
    let saved = svc.saved(&playlist.track_ids).await?;
    let shown = visible(playlist, &saved);
    let starred = svc.starred().await?;
    let mut body = summary(playlist, &shown);
    body["entry"] = shown.iter().map(|(_, t)| shape::song(t, &starred)).collect();
    Ok(Reply::of("playlist", body))
}

/// The album whose cover stands for a playlist without its own: its first saved song's.
pub async fn first_album(svc: &Service, id: &str) -> Result<Option<AlbumId>, Fail> {
    let playlist = find(svc, id).await?;
    let saved = svc.saved(&playlist.track_ids).await?;
    Ok(visible(&playlist, &saved).first().map(|(_, t)| t.album_id.clone()))
}

/// The playlist's saved songs with their positions in the whole playlist.
fn visible<'a>(playlist: &Playlist, saved: &'a HashMap<TrackId, Track>) -> Vec<(usize, &'a Track)> {
    playlist
        .track_ids
        .iter()
        .enumerate()
        .filter_map(|(pos, id)| saved.get(id).map(|t| (pos, t)))
        .collect()
}

fn summary(p: &Playlist, shown: &[(usize, &Track)]) -> Value {
    json!({
        "id": p.id,
        "name": p.name,
        "comment": p.description,
        "owner": crate::auth::USER,
        "public": false,
        "songCount": shown.len(),
        "duration": shown.iter().map(|(_, t)| t.duration.unwrap_or(0)).sum::<u32>(),
        "created": shape::iso(p.created_at),
        "changed": shape::iso(p.updated_at),
        // getCoverArt serves the playlist's own cover, else its first song's album's.
        "coverArt": p.id,
    })
}
