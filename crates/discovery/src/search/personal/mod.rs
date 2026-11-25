//! Search led by the listener's own music (ADR-0036).
//!
//! A snapshot of their music and what it is worth to them (`snapshot`) is built when lull starts
//! and again when something it holds changes, never per search: a search reads the snapshot and
//! calls no database. Several changes in a row rebuild it once more, not once each.

pub mod index;
pub mod score;
pub mod snapshot;

use std::collections::HashSet;
use std::sync::{Arc, PoisonError, RwLock};

use lull_core::Result;
use lull_core::ids::TrackId;
use lull_core::model::Playlist;
use tokio::sync::Notify;

use crate::Service;
use index::Song;
use snapshot::Snapshot;

/// The bus topics after which the snapshot is out of date: a play ended, a like, a follow, a
/// playlist, a save finished, the library switched or its index changed (files added or removed
/// outside lull), the Last.fm history came.
const CHANGES: &[&str] = &[
    "play.ended",
    "like",
    "follow",
    "playlist",
    "save",
    "library.source",
    "library.index",
    "lastfm.history",
];

/// Whether an event on `topic` makes the snapshot stale.
pub fn is_change(topic: &str) -> bool {
    CHANGES.contains(&topic)
}

/// The shared snapshot and the nudge that rebuilds it.
pub struct Personal {
    snapshot: RwLock<Option<Arc<Snapshot>>>,
    stale: Notify,
}

impl Personal {
    /// Stale from the start: the first build is the first thing `keep_fresh` does.
    pub fn new() -> Personal {
        let personal = Personal {
            snapshot: RwLock::new(None),
            stale: Notify::new(),
        };
        personal.changed();
        personal
    }

    /// The snapshot, once the first build has finished. Until then (or while every build fails)
    /// searches rank by the catalogs and the listener's likes and follows alone (`merge::rank`),
    /// rather than answer as a listener with no music.
    pub fn snapshot(&self) -> Option<Arc<Snapshot>> {
        self.snapshot.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Something the snapshot holds changed. Changes that come while it is rebuilding make one
    /// more rebuild between them, not one each (`Notify` keeps a single wake-up).
    pub fn changed(&self) {
        self.stale.notify_one();
    }

    fn set(&self, snapshot: Snapshot) {
        *self.snapshot.write().unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(snapshot));
    }
}

/// Builds the snapshot, then again each time it is stale. Runs for the life of the service.
pub async fn keep_fresh(svc: Arc<Service>) {
    let personal = &svc.personal;
    loop {
        personal.stale.notified().await;
        let started = std::time::Instant::now();
        match build(&svc).await {
            Ok(snapshot) => {
                personal.set(snapshot);
                tracing::info!(ms = started.elapsed().as_millis() as u64, "personal search index built");
            }
            Err(e) => tracing::warn!(error = %e, "personal search index not built; searches go without it"),
        }
    }
}

/// Reads what the listener did and has, and builds the snapshot from it.
async fn build(svc: &Service) -> Result<Snapshot> {
    let now = lull_core::now();
    let signals = svc.taste_signals().await?;
    // A library that can't be read now (Jellyfin away) only leaves its songs out this once.
    let library = svc.store.library_songs().await.unwrap_or_else(|e| {
        tracing::info!(error = %e, "personal search without the library this time");
        Vec::new()
    });
    let playlist_songs = playlisted(svc).await?;
    let saved: HashSet<&TrackId> = library.iter().map(|(t, _)| &t.id).collect();
    let met: Vec<TrackId> = first_met(
        signals
            .plays
            .iter()
            .map(|p| &p.track)
            .chain(signals.likes.iter().map(|m| &m.track))
            .chain(&playlist_songs),
    )
    .into_iter()
    .filter(|id| !saved.contains(id))
    .cloned()
    .collect();
    let playlisted: HashSet<TrackId> = playlist_songs.into_iter().collect();
    let others = svc.store.catalog_songs(met).await?;
    let ids = svc.store.listener_artist_ids().await?;
    let songs: Vec<Song> = library
        .into_iter()
        .map(|(track, album_artist)| Song {
            track,
            saved: true,
            album_artist: Some(album_artist),
        })
        .chain(others.into_iter().map(|track| Song {
            track,
            saved: false,
            album_artist: None,
        }))
        .collect();
    tokio::task::spawn_blocking(move || Snapshot::from_signals(signals, songs, &playlisted, &ids, now))
        .await
        .map_err(|e| lull_core::Error::internal(format!("personal search index build: {e}")))
}

/// The songs in any of the listener's playlists, once each in the playlists' order, asked of
/// collection, which owns them. Before collection is attached (`attach_collection` nudges a
/// rebuild) there are none to read.
async fn playlisted(svc: &Service) -> Result<Vec<TrackId>> {
    match svc.collection.get() {
        Some(collection) => Ok(songs_of(&collection.playlists().await?)),
        None => Ok(Vec::new()),
    }
}

fn songs_of(playlists: &[Playlist]) -> Vec<TrackId> {
    first_met(playlists.iter().flat_map(|p| &p.track_ids))
        .into_iter()
        .cloned()
        .collect()
}

/// The ids once each, in the order they are first met. The index keeps the first record of a take
/// it is given, and answers equal rows in its order, so the order songs go in is the order a
/// search breaks ties by: a hash set's would differ from one start of the hub to the next.
pub fn first_met<'a>(ids: impl IntoIterator<Item = &'a TrackId>) -> Vec<&'a TrackId> {
    let mut seen = HashSet::new();
    ids.into_iter().filter(|id| seen.insert(*id)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn until_the_first_build_searches_go_as_they_do_without_it() {
        let personal = Personal::new();
        assert!(personal.snapshot().is_none());
        personal.set(Snapshot::default());
        assert!(personal.snapshot().is_some());
    }

    #[test]
    fn a_song_in_two_playlists_is_listed_once_and_an_empty_playlist_lists_none() {
        let playlist = |id: &str, tracks: &[&str]| Playlist {
            id: id.into(),
            name: id.into(),
            description: String::new(),
            track_ids: tracks.iter().map(|t| TrackId((*t).into())).collect(),
            created_at: 0,
            updated_at: 0,
        };
        let listed = songs_of(&[
            playlist("pl-1", &["a", "b"]),
            playlist("pl-2", &["b"]),
            playlist("pl-3", &[]),
        ]);
        assert_eq!(listed, ["a", "b"].map(|t| TrackId(t.into())), "in the playlists' order");
    }

    #[test]
    fn only_what_the_snapshot_holds_makes_it_stale() {
        for topic in [
            "play.ended",
            "like",
            "follow",
            "playlist",
            "save",
            "library.source",
            "library.index",
        ] {
            assert!(is_change(topic), "{topic}");
        }
        for topic in ["stream", "transfer", "device.state", "search.more", "settings", "job"] {
            assert!(!is_change(topic), "{topic}");
        }
    }
}
