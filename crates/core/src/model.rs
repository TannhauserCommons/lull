//! Shapes shared across crates and sent to the UI (camelCase JSON, as `web/` reads them).

use serde::{Deserialize, Serialize};

use crate::ids::{AlbumId, ArtistId, TrackId};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: TrackId,
    pub title: String,
    pub artist_id: ArtistId,
    pub artist: String,
    pub album_id: AlbumId,
    pub album: String,
    pub year: Option<u16>,
    pub no: Option<u32>,
    pub disc: Option<u32>,
    /// Seconds.
    pub duration: Option<u32>,
    pub tags: Vec<String>,
    /// In the Jellyfin library (plays from Jellyfin, not Soulseek).
    pub saved: bool,
    /// Flattened into the track (`format`, `codec`, `khz`...), as the UI's mock data has it.
    #[serde(flatten)]
    pub file: Option<FileFacts>,
}

/// What the file is: the UI's quality chip and format line ("FLAC :: 44.1 kHz, 1040 kbps, 30.2 MB").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileFacts {
    /// Short label: "FLAC 16/44.1", "MP3 320", "MP3 V0", "OPUS 160".
    pub format: String,
    pub codec: String,
    pub lossless: bool,
    pub khz: Option<f32>,
    pub bits: Option<u8>,
    pub kbps: Option<u32>,
    pub size_bytes: Option<u64>,
}

/// One file on one Soulseek peer: what a transfer is asked for.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRef {
    pub user: String,
    /// The peer's full path, as returned by search.
    pub path: String,
    pub size: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Saved {
    All,
    Partial,
    None,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Album {
    pub id: AlbumId,
    pub title: String,
    pub artist_id: ArtistId,
    pub artist: String,
    pub year: Option<u16>,
    /// "album", "EP", "single", "compilation"...
    #[serde(rename = "type")]
    pub kind: Option<String>,
    /// ISO date, as precise as known ("2026", "2026-10", "2026-10-29").
    pub released: Option<String>,
    pub label: Option<String>,
    pub tags: Vec<String>,
    pub saved: Saved,
    pub track_ids: Vec<TrackId>,
    /// Seconds.
    pub duration: Option<u32>,
    pub musicbrainz_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Artist {
    pub id: ArtistId,
    pub name: String,
    pub tags: Vec<String>,
    pub listeners: Option<u64>,
    pub bio: Option<String>,
    pub musicbrainz_id: Option<String>,
}

/// Every response that names tracks also carries them, so the UI's lookup cache
/// (`M.track(id)`) can resolve any id it is handed (docs/context/api.md#conventions).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Entities {
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty", default)]
    pub tracks: std::collections::BTreeMap<TrackId, Track>,
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty", default)]
    pub albums: std::collections::BTreeMap<AlbumId, Album>,
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty", default)]
    pub artists: std::collections::BTreeMap<ArtistId, Artist>,
}

/// A playlist as the `Collection` seam hands it out (ADR-0002).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Playlist {
    /// `pl-<n>`, never reused.
    pub id: String,
    pub name: String,
    pub description: String,
    pub track_ids: Vec<TrackId>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// One change to a playlist; `None` fields stay as they are. Removals are positions in the
/// playlist before the change, applied before additions, which go to the end.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistEdit {
    pub name: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub remove_positions: Vec<usize>,
    #[serde(default)]
    pub add: Vec<TrackId>,
}

/// A finished or abandoned play reported by something other than the web player (Subsonic
/// scrobbles, the offline outbox).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayReport {
    pub track_id: TrackId,
    /// Unix seconds.
    pub started_at: i64,
    pub played_s: u32,
    pub completed: bool,
}
