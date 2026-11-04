//! The only calls that cross between domain crates. Each is implemented by one crate and
//! handed to the others by `server` at start-up, so crates never depend on each other.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use axum::http::HeaderValue;
use axum::response::Response;

use crate::error::{Error, Result};
use crate::ids::{AlbumId, ArtistId, TrackId};
use crate::model::{self, Album, Artist, PlayReport, Playlist, PlaylistEdit, SourceRef, Track};
use crate::quality::SaveCandidate;
use crate::youtube::{Found, FoundPlaylist, Listing, Want};

/// The saved library. Implemented by `library`, from a music folder or Jellyfin (ADR-0003).
#[async_trait]
pub trait Library: Send + Sync {
    /// Saved tracks matching free text: the instant half of search.
    async fn search(&self, query: &str, limit: usize) -> Result<Vec<Track>>;
    /// The saved tracks among `ids` (duplicate notes, "saved" badges on non-library results).
    async fn saved(&self, ids: &[TrackId]) -> Result<Vec<Track>>;

    // Browsing and streaming, for the Subsonic API (ADR-0003). Defaults until `library` has them.

    /// Every artist with saved music, by name.
    async fn artists(&self) -> Result<Vec<Artist>> {
        Err(Error::unavailable("library", "browsing is not built yet"))
    }
    /// Saved albums, all or one artist's, by artist then year.
    async fn albums(&self, _artist: Option<&ArtistId>) -> Result<Vec<Album>> {
        Err(Error::unavailable("library", "browsing is not built yet"))
    }
    /// One saved album and its tracks in order.
    async fn album(&self, _id: &AlbumId) -> Result<Option<(Album, Vec<Track>)>> {
        Err(Error::unavailable("library", "browsing is not built yet"))
    }
    /// A saved track's audio, honouring `range` (the request's Range header): 200 or 206.
    async fn stream(&self, _id: &TrackId, _range: Option<HeaderValue>) -> Result<Response> {
        Err(Error::unavailable(
            "library",
            "streaming through the seam is not built yet",
        ))
    }
    /// An album's cover, scaled to fit `size` pixels when given.
    async fn cover(&self, _id: &AlbumId, _size: Option<u32>) -> Result<Response> {
        Err(Error::unavailable(
            "library",
            "covers through the seam are not built yet",
        ))
    }
    /// Albums an answer just handed out whose cover the UI will ask library for (`/api/art`: no
    /// catalog picture): fetched in the background before it asks (docs/context/library.md).
    fn warm_covers(&self, _albums: &[AlbumId]) {}
}

/// The listener's collection: likes, playlists and plays. Implemented by `collection`, for
/// crates that act on it outside the web UI's own endpoints (Subsonic, ADR-0002).
#[async_trait]
pub trait Collection: Send + Sync {
    /// Liked tracks, newest first.
    async fn liked(&self) -> Result<Vec<TrackId>>;
    /// Likes or unlikes; the same `like` events go out as from the web UI.
    async fn set_liked(&self, track: &TrackId, liked: bool) -> Result<()>;
    async fn playlists(&self) -> Result<Vec<Playlist>>;
    async fn playlist(&self, id: &str) -> Result<Option<Playlist>>;
    async fn create_playlist(&self, name: &str, tracks: &[TrackId]) -> Result<Playlist>;
    async fn edit_playlist(&self, id: &str, edit: PlaylistEdit) -> Result<Playlist>;
    async fn delete_playlist(&self, id: &str) -> Result<()>;
    /// Records a play as if the web player had reported it (history, taste, scrobbling).
    async fn record_play(&self, play: PlayReport) -> Result<()>;
    /// The playlist's own cover (uploaded, or brought by a link import) as an image response,
    /// at most `size` pixels square. `None` when it has none: callers show its songs' covers.
    async fn playlist_cover(&self, _id: &str, _size: Option<u32>) -> Result<Option<Response>> {
        Ok(None)
    }
}

/// Last.fm's side of History's "Send to Last.fm". Implemented by `lastfm`, for `collection`, which
/// owns the plays and decides which may be offered.
#[async_trait]
pub trait Scrobbles: Send + Sync {
    /// Why Last.fm would turn this play down (its own rule, or too old), in a sentence for the
    /// listener; `None` when it would take it. Says nothing about the account.
    fn refusal(&self, play: &model::LatePlay, now: i64) -> Option<String>;
    /// Queues the plays, all or none: every one must pass `refusal`, none may have been queued
    /// before, and Last.fm must be connected with scrobbling on (409 otherwise). They then go out
    /// like any scrobble and publish the usual `scrobble` events.
    async fn submit(&self, plays: Vec<model::LatePlay>) -> Result<()>;
}

/// Lyrics. Implemented by `lyrics`, for Subsonic's `getLyricsBySongId`.
#[async_trait]
pub trait Lyrics: Send + Sync {
    /// The track's lyrics, `kind: none` when there are none: `NotFound` for a track lull doesn't
    /// know, `Unavailable` while the lyrics source is failing and nothing is cached.
    async fn lyrics(&self, track: &TrackId) -> Result<model::Lyrics>;
}

/// Soulseek transfers. Implemented by `soulseek`.
#[async_trait]
pub trait Transfers: Send + Sync {
    /// Transfers `source` to the end, reusing what the stream cache already holds, and returns
    /// the finished file. Progress goes out as `transfer` events; a stall falls back to the
    /// next-best peer for the same song.
    async fn complete(&self, track: &TrackId, source: &SourceRef) -> Result<PathBuf>;

    /// The source to `complete` when the listener picked none: the one the song streamed
    /// from, else the best-ranked known one. `None` when no peer is known to share it.
    async fn source_for(&self, _track: &TrackId) -> Result<Option<SourceRef>> {
        Ok(None)
    }

    /// The copy to save by the listener's saving profile (ADR-0008): the best by its `pick`
    /// among copies that pass its minimum, else the best of the rest, marked `meets: false`.
    /// Searches Soulseek when no copy is known, and always when `fresh` (auto-upgrade).
    /// `None` when no peer shares the song.
    async fn save_candidate(&self, _track: &TrackId, _fresh: bool) -> Result<Option<SaveCandidate>> {
        Ok(None)
    }

    /// Whether Soulseek peers share an album, by its artist and title, for shelves that should
    /// only show what can play (new releases). A remembered answer comes back at once; otherwise
    /// this searches. `None` when Soulseek can't answer now (offline, or not checked yet).
    async fn album_available(&self, _artist: &str, _album: &str) -> Result<Option<bool>> {
        Ok(None)
    }

    /// `album_available`'s remembered answer only: never searches, so a caller can fill a
    /// shelf at once before spending searches on the rest. `None` when nothing is remembered.
    async fn album_known(&self, _artist: &str, _album: &str) -> Result<Option<bool>> {
        Ok(None)
    }
}

/// YouTube through yt-dlp. Implemented by `youtube`, handed to `soulseek`, whose fetches write
/// the audio into the stream cache like a peer's file (docs/context/youtube.md).
#[async_trait]
pub trait Youtube: Send + Sync {
    /// The YouTube copy that best matches the track by the listener's match rules, searching
    /// YouTube unless a recent answer is remembered. `None` when no upload matches;
    /// `Unavailable` while YouTube is backed off (a sign-in check, too many requests).
    async fn find(&self, track: &Track, want: Want) -> Result<Option<Found>>;

    /// Writes the copy's audio to `dest` front to back, the native stream with no transcoding,
    /// and returns once all of it is there. Dropping the future stops the download.
    async fn download(&self, source: &SourceRef, dest: &Path) -> Result<()>;

    /// Up to `count` songs of a YouTube, YouTube Music or SoundCloud playlist, from position
    /// `from` (0 = the first), as the uploads name them. For link imports, whatever YouTube's
    /// play mode; `Unavailable` while YouTube is backed off.
    async fn playlist(&self, _url: &str, _from: usize, _count: usize) -> Result<Listing> {
        Err(Error::unavailable("youtube", "this hub can't list playlists"))
    }

    /// Up to `limit` playlists a YouTube search for `query` finds, in YouTube's order, for
    /// search's Playlists section (YouTube Music's own search lists the same community
    /// playlists, without their names). Whatever YouTube's play mode; `Unavailable` while backed off.
    async fn search_playlists(&self, _query: &str, _limit: usize) -> Result<Vec<FoundPlaylist>> {
        Ok(Vec::new())
    }
}

/// lull's catalog (iTunes or Deezer search, ADR-0008) for songs another service names.
/// Implemented by `discovery`, handed to `collection` for playlist link imports.
#[async_trait]
pub trait Catalog: Send + Sync {
    /// The song as lull knows it: the library's copy, else the catalog's same song (the same
    /// ISRC, else the same loose artist and title with lengths within 3 s; a name in another
    /// script than the store's leaves the length to decide, for the store's top hit), recorded so
    /// its id resolves. `None` when nothing matches. Waits its turn within the catalog's rate
    /// limits, leaving room for the listener's own searches.
    async fn find_song(&self, song: &model::SongQuery) -> Result<Option<Track>>;

    /// Whether `track` is the song by today's rule, without a lookup: for matches an older
    /// `FIND_SONG_RULE` made, which collection re-checks.
    fn same_take(&self, song: &model::SongQuery, track: &Track) -> bool;
}

/// The version of `Catalog::find_song`'s rule: bump it with any change to the rule. Collection
/// keeps "nothing matched" under it, so a rule that matches more isn't answered with yesterday's
/// misses; and a match an older rule made that today's rule doesn't take (`Catalog::same_take`)
/// is looked up again, the listener's own picks aside.
/// 3: an edit or a live take is not the original when a length is missing (`merge::same_take`).
pub const FIND_SONG_RULE: i64 = 3;
