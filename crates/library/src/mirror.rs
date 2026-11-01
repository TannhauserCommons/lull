//! The Jellyfin mirror, one way (ADR-0003). When a Jellyfin is first read, its favourites become
//! likes and its playlists become lull playlists, once per server. After that lull's likes and
//! playlists are mirrored into Jellyfin (favourites, and one Jellyfin playlist per lull
//! playlist), so Jellyfin's own apps see them. Nothing flows back: lull stays the source of truth.

use std::collections::HashSet;
use std::sync::Arc;

use lull_core::events::Event;
use lull_core::ids::TrackId;
use lull_core::status::State;
use lull_core::{Error, Result};
use serde_json::Value;
use tokio::sync::broadcast::error::RecvError;

use crate::Service;
use crate::items::Location;

/// Runs until the hub stops the crates (`Ctx::spawn`).
pub async fn run(svc: Arc<Service>) {
    wait_for_library(&svc).await;
    if let Err(e) = import_once(&svc).await {
        tracing::warn!(error = %e, "Jellyfin favourites and playlists weren't imported; trying again next start");
    }
    // Subscribed after the import, so the likes and playlists it made aren't sent straight back.
    let mut events = svc.ctx.bus.subscribe();
    loop {
        let event = match events.recv().await {
            Ok(e) => e,
            Err(RecvError::Lagged(n)) => {
                tracing::warn!(missed = n, "the Jellyfin mirror missed events");
                continue;
            }
            Err(RecvError::Closed) => return,
        };
        let mirrored = match event.topic.as_str() {
            "like" => mirror_like(&svc, &event).await,
            "playlist" => mirror_playlist(&svc, &event).await,
            _ => Ok(()),
        };
        if let Err(e) = mirrored {
            tracing::info!(topic = event.topic, error = %e, "a change wasn't mirrored into Jellyfin");
        }
    }
}

/// Waits for the first full read of Jellyfin: the mirror maps songs to Jellyfin items through
/// the index.
async fn wait_for_library(svc: &Service) {
    let mut events = svc.ctx.bus.subscribe();
    let ready = || {
        svc.ctx
            .status
            .snapshot()
            .get("jellyfin")
            .is_some_and(|j| j.state == State::Ok)
    };
    while !ready() {
        match events.recv().await {
            Ok(_) | Err(RecvError::Lagged(_)) => {}
            Err(RecvError::Closed) => return,
        }
    }
}

async fn import_once(svc: &Arc<Service>) -> Result<()> {
    let (Some(collection), Ok(jellyfin)) = (svc.collection.get(), svc.jellyfin()) else {
        return Ok(());
    };
    let key = format!("imported:{}", jellyfin.server_id().await?);
    if stored(svc, &key).await?.is_some() {
        return Ok(());
    }

    let liked: HashSet<TrackId> = collection.liked().await?.into_iter().collect();
    let favourites = songs_of(svc, &jellyfin.favourite_audio().await?);
    let new_likes: Vec<&TrackId> = favourites.iter().filter(|id| !liked.contains(*id)).collect();
    for id in &new_likes {
        collection.set_liked(id, true).await?;
    }

    let mut playlists = 0;
    for (jf_id, name) in jellyfin.playlists().await? {
        // Made by an import that stopped part-way.
        if imported_playlist(svc, &jf_id).await? {
            continue;
        }
        let tracks = songs_of(svc, &jellyfin.playlist_items(&jf_id).await?);
        if tracks.is_empty() {
            continue;
        }
        let made = collection.create_playlist(&name, &tracks).await?;
        store(svc, &format!("playlist:{}", made.id), &jf_id).await?;
        playlists += 1;
    }
    store(svc, &key, &lull_core::now().to_string()).await?;
    tracing::info!(
        likes = new_likes.len(),
        playlists,
        "imported Jellyfin favourites and playlists"
    );
    Ok(())
}

/// The songs of Jellyfin items, in order; items the index doesn't hold are skipped.
fn songs_of(svc: &Service, items: &[String]) -> Vec<TrackId> {
    let index = svc.index();
    items
        .iter()
        .filter_map(|item| index.item(item).map(|s| s.track.id.clone()))
        .collect()
}

async fn mirror_like(svc: &Service, event: &Event) -> Result<()> {
    let data = &event.data;
    let (Some(id), Some(liked)) = (
        data.get("trackId").and_then(Value::as_str),
        data.get("liked").and_then(Value::as_bool),
    ) else {
        return Err(Error::internal(format!("unexpected like event {data}")));
    };
    // Only saved songs exist in Jellyfin.
    match jellyfin_items(svc, &[TrackId(id.to_owned())]).first() {
        Some(item) => svc.jellyfin()?.set_favourite(item, liked).await,
        None => Ok(()),
    }
}

/// `playlist { playlistId, deleted }`: the Jellyfin playlist that mirrors it is made, replaced
/// (name and songs) or deleted.
async fn mirror_playlist(svc: &Service, event: &Event) -> Result<()> {
    let data = &event.data;
    let Some(id) = data.get("playlistId").and_then(Value::as_str) else {
        return Err(Error::internal(format!("unexpected playlist event {data}")));
    };
    let (Some(collection), Ok(jellyfin)) = (svc.collection.get(), svc.jellyfin()) else {
        return Ok(());
    };
    let key = format!("playlist:{id}");
    let mirror = stored(svc, &key).await?;
    let deleted = data.get("deleted").and_then(Value::as_bool).unwrap_or(false);
    let playlist = if deleted { None } else { collection.playlist(id).await? };
    match (playlist, mirror) {
        (None, Some(jf_id)) => {
            jellyfin.delete_item(&jf_id).await?;
            forget(svc, &key).await
        }
        (None, None) => Ok(()),
        (Some(p), Some(jf_id)) => {
            let items = jellyfin_items(svc, &p.track_ids);
            jellyfin.update_playlist(&jf_id, &p.name, &items).await
        }
        (Some(p), None) => {
            let items = jellyfin_items(svc, &p.track_ids);
            let jf_id = jellyfin.create_playlist(&p.name, &items).await?;
            store(svc, &key, &jf_id).await
        }
    }
}

/// The Jellyfin items of saved songs, in order; songs only on Soulseek are left out.
fn jellyfin_items(svc: &Service, tracks: &[TrackId]) -> Vec<String> {
    let index = svc.index();
    tracks
        .iter()
        .filter_map(|t| match &index.saved(t)?.location {
            Location::Jellyfin { item, .. } => Some(item.clone()),
            Location::File { .. } => None,
        })
        .collect()
}

async fn stored(svc: &Service, key: &str) -> Result<Option<String>> {
    let key = key.to_owned();
    svc.ctx
        .db
        .call(move |c| {
            let mut stmt = c.prepare("SELECT value FROM library_mirror WHERE key = ?1")?;
            let mut rows = stmt.query_map([&key], |r| r.get::<_, String>(0))?;
            Ok(rows.next().transpose()?)
        })
        .await
}

async fn forget(svc: &Service, key: &str) -> Result<()> {
    let key = key.to_owned();
    svc.ctx
        .db
        .call(move |c| {
            c.execute("DELETE FROM library_mirror WHERE key = ?1", [&key])?;
            Ok(())
        })
        .await
}

/// Whether a lull playlist already mirrors this Jellyfin playlist.
async fn imported_playlist(svc: &Service, jf_id: &str) -> Result<bool> {
    let jf_id = jf_id.to_owned();
    svc.ctx
        .db
        .call(move |c| {
            let found: i64 = c.query_row(
                "SELECT COUNT(*) FROM library_mirror WHERE key LIKE 'playlist:%' AND value = ?1",
                [&jf_id],
                |r| r.get(0),
            )?;
            Ok(found > 0)
        })
        .await
}

async fn store(svc: &Service, key: &str, value: &str) -> Result<()> {
    let (key, value) = (key.to_owned(), value.to_owned());
    svc.ctx
        .db
        .call(move |c| {
            c.execute(
                "INSERT INTO library_mirror (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [&key, &value],
            )?;
            Ok(())
        })
        .await
}
