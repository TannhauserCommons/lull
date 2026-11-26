//! lull's models as Subsonic's `Child` (a song), `AlbumID3` and `ArtistID3`. lull's own ids are
//! the Subsonic ids; an album id doubles as the cover art id of the album and its songs.

use std::collections::HashMap;

use lull_core::ids::TrackId;
use lull_core::model::{Album, Artist, Track};
use serde_json::{Value, json};

/// When each liked song was liked (unix seconds): Subsonic's `starred`.
pub type Starred = HashMap<TrackId, i64>;

pub fn song(t: &Track, starred: &Starred) -> Value {
    let file = t.file.as_ref();
    let codec = file.map(|f| f.codec.to_ascii_lowercase());
    let (suffix, content_type) = codec.as_deref().map(audio_type).unzip();
    json!({
        "id": t.id,
        "parent": t.album_id,
        "isDir": false,
        "title": t.title,
        "album": t.album,
        "artist": t.artist,
        "track": t.no,
        "discNumber": t.disc,
        "year": t.year,
        "genre": t.tags.first(),
        "genres": genres(&t.tags),
        "coverArt": t.album_id,
        "size": file.and_then(|f| f.size_bytes),
        "contentType": content_type,
        "suffix": suffix,
        "duration": t.duration,
        "bitRate": file.and_then(|f| f.kbps),
        "bitDepth": file.and_then(|f| f.bits),
        "samplingRate": file.and_then(|f| f.khz).map(|k| (k * 1000.0).round() as u32),
        "albumId": t.album_id,
        "artistId": t.artist_id,
        "type": "music",
        "mediaType": "song",
        "starred": starred.get(&t.id).map(|at| lull_core::iso_utc(*at)),
    })
}

pub fn album(a: &Album) -> Value {
    json!({
        "id": a.id,
        "name": a.title,
        "artist": a.artist,
        "artistId": a.artist_id,
        "coverArt": a.id,
        "songCount": a.track_ids.len(),
        "duration": a.duration.unwrap_or(0),
        "year": a.year,
        "genre": a.tags.first(),
        "genres": genres(&a.tags),
        "musicBrainzId": a.musicbrainz_id,
        "releaseTypes": a.kind.as_ref().map(|k| [k]),
    })
}

/// `cover` is one of the artist's albums: lull has no artist images of its own.
pub fn artist(a: &Artist, album_count: usize, cover: Option<&Album>) -> Value {
    json!({
        "id": a.id,
        "name": a.name,
        "albumCount": album_count,
        "coverArt": cover.map(|c| &c.id),
        "musicBrainzId": a.musicbrainz_id,
    })
}

fn genres(tags: &[String]) -> Vec<Value> {
    tags.iter().map(|t| json!({ "name": t })).collect()
}

/// File suffix and MIME type for a codec as `library` reports it.
fn audio_type(codec: &str) -> (&str, &'static str) {
    match codec {
        "mp3" => ("mp3", "audio/mpeg"),
        "flac" => ("flac", "audio/flac"),
        "opus" => ("opus", "audio/ogg"),
        "vorbis" | "ogg" => ("ogg", "audio/ogg"),
        "aac" | "alac" | "m4a" | "mp4" => ("m4a", "audio/mp4"),
        "wav" => ("wav", "audio/wav"),
        "aiff" | "aif" => ("aiff", "audio/aiff"),
        "wma" => ("wma", "audio/x-ms-wma"),
        other => (other, "application/octet-stream"),
    }
}
