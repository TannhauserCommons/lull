//! The outside sources discovery reads (docs/context/discovery.md, ADR-0030): ListenBrainz for
//! recommendations, radio, similar artists and fresh releases; MusicBrainz for releases,
//! tracklists and tags; Last.fm (when configured) for bios, listener counts, tag descriptions
//! and tag charts; Deezer and iTunes for catalog search, discographies and pictures; Wikidata for
//! artist photos on Wikimedia Commons. All of it is cached (`cache`) and
//! paced (`Http::pace`, Deezer's own window).

pub mod cache;
pub mod deezer;
pub mod itunes;
pub mod lastfm;
pub mod listenbrainz;
pub mod musicbrainz;
pub mod wikidata;

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

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
    /// The catalog's own id for the song's artist, where the catalog gives one: exact, unlike
    /// the name.
    #[serde(default)]
    pub artist_source: Option<ArtistSource>,
    #[serde(default)]
    pub isrc: Option<String>,
}

/// A catalog's artist id (`Song::artist_source`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "source", content = "id")]
pub enum ArtistSource {
    /// Apple's `artistId`.
    Itunes(u64),
    /// Deezer's artist id.
    Deezer(u64),
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

/// Where a catalog album came from, so its page asks that catalog for the tracklist (one fast
/// call) instead of MusicBrainz (two paced ones).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "source", content = "id")]
pub enum AlbumSource {
    /// Apple's `collectionId`.
    Itunes(u64),
    /// Deezer's album id.
    Deezer(u64),
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

/// How long one call to MusicBrainz, ListenBrainz or Last.fm may take (`Http` retries a timed-out
/// call, so about 17 s at worst; the radio and search cut that shorter themselves).
pub const CALL_TIMEOUT: Duration = Duration::from_secs(5);
/// Failed calls in a row before a service is left alone, and for how long.
const FAILURES_BEFORE_REST: u32 = 2;
const REST: Duration = Duration::from_secs(60);

/// Leaves a failing service alone for a minute: after two failed calls in a row, its calls fail
/// at once instead of waiting on a server that isn't answering. A call abandoned before it ended
/// (a radio or search out of time) counts as failed.
pub struct Breaker {
    service: &'static str,
    failures: AtomicU32,
    resting_until: Mutex<Option<Instant>>,
}

/// One call through a `Breaker`; `end` records how it went.
pub struct Call<'a> {
    breaker: &'a Breaker,
    ended: bool,
}

impl Breaker {
    pub fn new(service: &'static str) -> Self {
        Breaker {
            service,
            failures: AtomicU32::new(0),
            resting_until: Mutex::new(None),
        }
    }

    /// Starts a call, or fails at once while the service is left alone.
    pub fn call(&self) -> Result<Call<'_>> {
        let resting = self
            .resting_until
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some_and(|until| Instant::now() < until);
        if resting {
            return Err(Error::unavailable(
                self.service,
                "isn't answering; lull asks again in a minute",
            ));
        }
        Ok(Call {
            breaker: self,
            ended: false,
        })
    }

    fn failed(&self) {
        if self.failures.fetch_add(1, Ordering::Relaxed) + 1 >= FAILURES_BEFORE_REST {
            self.failures.store(0, Ordering::Relaxed);
            *self.resting_until.lock().unwrap_or_else(PoisonError::into_inner) = Some(Instant::now() + REST);
            tracing::info!(service = self.service, "keeps failing; leaving it alone for a minute");
        }
    }
}

impl Call<'_> {
    /// Records the answer: unavailable or bad is a failure; anything else (not found included)
    /// means the service answers.
    pub fn end<T>(mut self, answer: Result<T>) -> Result<T> {
        self.ended = true;
        match &answer {
            Err(Error::Unavailable { .. } | Error::Upstream { .. }) => self.breaker.failed(),
            _ => self.breaker.failures.store(0, Ordering::Relaxed),
        }
        answer
    }
}

impl Drop for Call<'_> {
    fn drop(&mut self) {
        if !self.ended {
            self.breaker.failed();
        }
    }
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

#[cfg(test)]
mod tests {
    use lull_core::Error;

    use super::Breaker;

    #[test]
    fn a_service_that_keeps_failing_is_left_alone() {
        let b = Breaker::new("ListenBrainz");
        let failed = || Err::<(), _>(Error::unavailable("ListenBrainz", "error sending request"));
        assert!(b.call().map(|c| c.end(failed())).is_ok());
        assert!(b.call().is_ok(), "one failure isn't enough");
        // A call abandoned before it answered (a radio out of time) counts as the second.
        drop(b.call());
        assert!(b.call().is_err(), "two in a row: calls fail at once");
    }

    #[test]
    fn an_answer_resets_the_count() {
        let b = Breaker::new("Last.fm");
        let failed = || Err::<(), _>(Error::unavailable("Last.fm", "timed out"));
        let _ = b.call().map(|c| c.end(failed()));
        let _ = b
            .call()
            .map(|c| c.end(Err::<(), _>(Error::NotFound("not found".into()))));
        let _ = b.call().map(|c| c.end(failed()));
        assert!(b.call().is_ok(), "not found is an answer");
    }
}
