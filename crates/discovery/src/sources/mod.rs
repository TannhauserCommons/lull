//! The outside sources discovery reads (docs/context/discovery.md, ADR-0030): ListenBrainz for
//! recommendations, radio, similar artists and fresh releases; MusicBrainz for releases,
//! tracklists and tags; Last.fm (when configured) for bios, listener counts, tag descriptions
//! and tag charts; Deezer and iTunes for catalog search and pictures. All of it is cached (`cache`) and
//! paced (`Http::pace`, Deezer's own window).

pub mod cache;
pub mod deezer;
pub mod itunes;
pub mod lastfm;
pub mod listenbrainz;
pub mod musicbrainz;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use lull_core::{Error, Result};
use serde::{Deserialize, Serialize};

/// A recording as the sources describe it, before it becomes a `Track`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Song {
    pub mbid: Option<String>,
    pub title: String,
    pub artist: String,
    pub artist_mbid: Option<String>,
    /// Empty when no source knows the release.
    pub album: String,
    /// The release's artist when it differs from the track's (compilations).
    pub album_artist: Option<String>,
    /// The release group's MBID.
    pub album_mbid: Option<String>,
    pub year: Option<u16>,
    pub no: Option<u32>,
    pub disc: Option<u32>,
    pub duration_s: Option<u32>,
    pub tags: Vec<String>,
}

/// A release group (album, EP, single) as the sources describe it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Release {
    pub mbid: Option<String>,
    pub title: String,
    pub artist: String,
    pub artist_mbid: Option<String>,
    /// "Album", "EP", "Single"...
    pub kind: Option<String>,
    /// ISO date, as precise as known.
    pub date: Option<String>,
    pub tags: Vec<String>,
}

/// An artist as the sources describe it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtistInfo {
    pub mbid: Option<String>,
    pub name: String,
}

/// The year of an ISO date ("2026-10-24" → 2026).
pub fn year(date: &str) -> Option<u16> {
    date.get(..4)?.parse().ok()
}

/// Lowercase tag names, most used first, without repeats.
pub fn top_tags(mut counted: Vec<(String, i64)>, limit: usize) -> Vec<String> {
    counted.sort_by_key(|t| std::cmp::Reverse(t.1));
    let mut seen = std::collections::HashSet::new();
    counted
        .into_iter()
        .map(|(t, _)| t.trim().to_lowercase())
        .filter(|t| !t.is_empty() && seen.insert(t.clone()))
        .take(limit)
        .collect()
}

/// A source the listener can switch off in Settings > Discovery; on until told otherwise.
pub struct Switch {
    service: &'static str,
    on: AtomicBool,
}

impl Switch {
    pub fn new(service: &'static str) -> Arc<Self> {
        Arc::new(Switch {
            service,
            on: AtomicBool::new(true),
        })
    }

    pub fn is_on(&self) -> bool {
        self.on.load(Ordering::Relaxed)
    }

    /// Returns whether this changed anything.
    pub fn set(&self, on: bool) -> bool {
        self.on.swap(on, Ordering::Relaxed) != on
    }

    /// Every use of a source starts here, so a switched-off source isn't read, cached or not.
    pub fn check(&self) -> Result<()> {
        if self.is_on() {
            Ok(())
        } else {
            Err(Error::unavailable(
                self.service,
                "it is switched off in Settings > Discovery",
            ))
        }
    }
}
