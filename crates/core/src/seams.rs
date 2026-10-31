//! The only calls that cross between domain crates. Each is implemented by one crate and
//! handed to the others by `server` at start-up, so crates never depend on each other.

use std::path::PathBuf;

use async_trait::async_trait;

use crate::error::Result;
use crate::ids::TrackId;
use crate::model::{SourceRef, Track};

/// The saved library. Implemented by `library` (Jellyfin).
#[async_trait]
pub trait Library: Send + Sync {
    /// Saved tracks matching free text: the instant half of search.
    async fn search(&self, query: &str, limit: usize) -> Result<Vec<Track>>;
    /// The saved tracks among `ids` (duplicate notes, "saved" badges on non-library results).
    async fn saved(&self, ids: &[TrackId]) -> Result<Vec<Track>>;
}

/// Soulseek transfers. Implemented by `soulseek`.
#[async_trait]
pub trait Transfers: Send + Sync {
    /// Transfers `source` to the end, reusing what the stream cache already holds, and returns
    /// the finished file. Progress goes out as `transfer` events; a stall falls back to the
    /// next-best peer for the same song.
    async fn complete(&self, track: &TrackId, source: &SourceRef) -> Result<PathBuf>;
}
