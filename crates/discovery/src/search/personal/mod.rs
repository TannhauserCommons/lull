//! Search led by the listener's own music (ADR-0036), behind the `personal-search` switch.
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
use lull_core::flags::{Flags, PERSONAL_SEARCH};
use lull_core::ids::TrackId;
use tokio::sync::Notify;

use crate::Service;
use index::Song;
use snapshot::Snapshot;

/// The bus topics after which the snapshot is out of date: a play ended, a like, a follow, a
/// playlist, a save finished, the library switched, the Last.fm history came.
const CHANGES: &[&str] = &[
    "play.ended",
    "like",
    "follow",
    "playlist",
    "save",
    "library.source",
    "lastfm.history",
];

/// Whether an event on `topic` makes the snapshot stale.
pub fn is_change(topic: &str) -> bool {
    CHANGES.contains(&topic)
}

/// The shared snapshot and the nudge that rebuilds it. Present in the service only with the
/// switch on: with it off nothing here exists.
pub struct Personal {
    snapshot: RwLock<Option<Arc<Snapshot>>>,
    stale: Notify,
}

impl Personal {
    /// The snapshot's keeper, when the `personal-search` switch is on; with it off, nothing: no
    /// index is built, no event is read for it, no search asks it.
    pub fn when(flags: &Flags) -> Option<Personal> {
        flags.on(&PERSONAL_SEARCH).then(Personal::new)
    }

    /// Stale from the start: the first build is the first thing `keep_fresh` does.
    fn new() -> Personal {
        let personal = Personal {
            snapshot: RwLock::new(None),
            stale: Notify::new(),
        };
        personal.changed();
        personal
    }

    /// The snapshot, once the first build has finished. Until then searches go as they do with
    /// the switch off, rather than answer as a listener with no music.
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
    let Some(personal) = svc.personal.as_ref() else {
        return;
    };
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
    let playlisted = svc.store.playlisted().await?;
    let saved: HashSet<&TrackId> = library.iter().map(|(t, _)| &t.id).collect();
    let met: Vec<TrackId> = signals
        .plays
        .iter()
        .map(|p| &p.track)
        .chain(signals.likes.iter().map(|m| &m.track))
        .chain(&playlisted)
        .filter(|id| !saved.contains(id))
        .cloned()
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_the_switch_off_there_is_no_keeper_and_so_no_index_no_rebuild_and_no_read() {
        let off = Flags::parse("").unwrap_or_default();
        assert!(Personal::when(&off).is_none());
        let on = Flags::parse("personal-search").unwrap_or_default();
        assert!(Personal::when(&on).is_some());
    }

    #[test]
    fn until_the_first_build_searches_go_as_they_do_without_it() {
        let personal = Personal::new();
        assert!(personal.snapshot().is_none());
        personal.set(Snapshot::default());
        assert!(personal.snapshot().is_some());
    }

    #[test]
    fn only_what_the_snapshot_holds_makes_it_stale() {
        for topic in ["play.ended", "like", "follow", "playlist", "save", "library.source"] {
            assert!(is_change(topic), "{topic}");
        }
        for topic in ["stream", "transfer", "device.state", "search.more", "settings", "job"] {
            assert!(!is_change(topic), "{topic}");
        }
    }
}
