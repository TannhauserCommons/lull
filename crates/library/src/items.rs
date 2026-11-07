//! Jellyfin audio items as lull tracks: content-derived ids, file facts and quality.

use lull_core::ids::{AlbumId, ArtistId, TrackId, is_various};
use lull_core::model::{FileFacts, Track};
pub use lull_core::quality::is_lossless;
use serde::{Deserialize, Serialize};

use crate::jellyfin::Item;

/// A saved file: the track the UI sees, plus what lull needs to stream it, show its cover and
/// replace it. Persisted as JSON, so the library lists at once after a restart (and while
/// Jellyfin is down).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Saved {
    pub track: Track,
    /// The album's owner: the album artist, or "Various Artists" for a compilation.
    pub album_artist: String,
    pub location: Location,
    /// When it was added: Jellyfin's date, or the file's modified time (ISO 8601 UTC, so it
    /// sorts as text).
    pub added: String,
    pub art: Option<Art>,
    pub mb_release: Option<String>,
    pub mb_release_group: Option<String>,
    pub mb_artist: Option<String>,
}

/// Where a saved song's audio lives (ADR-0003).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Location {
    /// A Jellyfin item, and its file's path as Jellyfin sees it.
    Jellyfin { item: String, path: Option<String> },
    /// A file in the music folder: its path under `LULL_MUSIC_DIR`, `/`-separated.
    File { path: String },
}

impl Location {
    /// The index key: unique within one source.
    pub fn key(&self) -> &str {
        match self {
            Location::Jellyfin { item, .. } => item,
            Location::File { path } => path,
        }
    }
}

/// Where an album's cover comes from. `stamp` changes when the image does, so a cached copy
/// is replaced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Art {
    /// Jellyfin's primary image of an item; `tag` is Jellyfin's image tag.
    Jellyfin { item: String, tag: String },
    /// A music-folder file: an image next to the songs, or a song with an embedded picture.
    File {
        path: String,
        embedded: bool,
        stamp: String,
    },
}

pub fn saved_from(item: Item) -> Option<Saved> {
    let title = item.name.clone().filter(|n| !n.trim().is_empty())?;
    let album = item.album.clone().unwrap_or_default();
    let (credited, owner) = artists(item.album_artist.as_deref(), &item.artists);
    let file = file_facts(&item);
    let art = item
        .album_id
        .clone()
        .zip(item.album_primary_image_tag.clone())
        .or_else(|| item.image_tags.get("Primary").map(|tag| (item.id.clone(), tag.clone())))
        .map(|(item, tag)| Art::Jellyfin { item, tag });
    let year = item
        .production_year
        .or_else(|| item.premiere_date.as_deref().and_then(|d| d.get(..4)?.parse().ok()));
    let provider = |key: &str| item.provider_ids.get(key).filter(|v| !v.is_empty()).cloned();
    Some(Saved {
        track: track(
            &credited,
            &owner,
            album,
            title,
            Extra {
                year,
                no: item.index_number,
                disc: item.parent_index_number,
                duration: item.run_time_ticks.map(|t| (t / 10_000_000) as u32),
                tags: item.genres.clone(),
                file,
            },
        ),
        album_artist: owner,
        added: item.date_created.clone().unwrap_or_default(),
        art,
        mb_release: provider("MusicBrainzAlbum"),
        mb_release_group: provider("MusicBrainzReleaseGroup"),
        mb_artist: provider("MusicBrainzAlbumArtist").or_else(|| provider("MusicBrainzArtist")),
        location: Location::Jellyfin {
            path: item.path.clone(),
            item: item.id,
        },
    })
}

/// A track's fields beyond its names.
pub struct Extra {
    pub year: Option<u16>,
    pub no: Option<u32>,
    pub disc: Option<u32>,
    pub duration: Option<u32>,
    pub tags: Vec<String>,
    pub file: Option<FileFacts>,
}

/// A saved track with its content-derived ids (docs/context/library.md#ids).
pub fn track(credited: &str, owner: &str, album: String, title: String, extra: Extra) -> Track {
    Track {
        id: TrackId::of(credited, &album, &title),
        title,
        artist_id: ArtistId::of(credited),
        artist: credited.to_owned(),
        album_id: AlbumId::of(owner, &album),
        album,
        year: extra.year,
        no: extra.no,
        disc: extra.disc,
        duration: extra.duration,
        tags: extra.tags,
        saved: true,
        file: extra.file,
    }
}

/// (the artist a song is credited to, the album's owner). The album artist names the song when
/// there is one, because that is what a Soulseek folder gives (`Artist/Album/01 - Title`), so a
/// saved song keeps the id it had as a search result. Compilations use the track artist.
pub fn artists(album_artist: Option<&str>, track_artists: &[String]) -> (String, String) {
    let album_artist = album_artist.map(str::trim).filter(|a| !a.is_empty());
    let track_artist = track_artists.iter().map(|a| a.trim()).find(|a| !a.is_empty());
    let credited = match album_artist {
        Some(a) if !is_various(a) => a,
        _ => track_artist.or(album_artist).unwrap_or("Unknown Artist"),
    };
    (credited.to_owned(), album_artist.unwrap_or(credited).to_owned())
}

fn file_facts(item: &Item) -> Option<FileFacts> {
    let source = item.media_sources.first()?;
    let audio = source.media_streams.iter().find(|s| s.kind.as_deref() == Some("Audio"));
    let codec = audio
        .and_then(|a| a.codec.clone())
        .or_else(|| source.container.clone())
        .or_else(|| item.container.clone())?
        .to_ascii_lowercase();
    let codec = if codec.starts_with("pcm") {
        "wav".to_owned()
    } else {
        codec
    };
    let lossless = is_lossless(&codec);
    let khz = audio
        .and_then(|a| a.sample_rate)
        .map(|hz| (hz as f32 / 100.0).round() / 10.0);
    let bits = audio.and_then(|a| a.bit_depth).filter(|_| lossless);
    let kbps = audio
        .and_then(|a| a.bit_rate)
        .or(source.bitrate)
        .map(|b| (b / 1000) as u32);
    Some(FileFacts {
        format: format_label(&codec, lossless, bits, khz, kbps),
        codec: codec.to_ascii_uppercase(),
        lossless,
        khz,
        bits,
        kbps,
        size_bytes: source.size,
    })
}

/// The UI's short label: "FLAC 16/44.1", "FLAC 24/96", "MP3 320", "OPUS 160".
pub fn format_label(codec: &str, lossless: bool, bits: Option<u8>, khz: Option<f32>, kbps: Option<u32>) -> String {
    let name = codec.to_ascii_uppercase();
    let khz = khz.map(|k| format!("{k}"));
    match (lossless, bits, khz, kbps) {
        (true, Some(b), Some(k), _) => format!("{name} {b}/{k}"),
        (true, None, Some(k), _) => format!("{name} {k}"),
        (false, _, _, Some(r)) => format!("{name} {}", round_bitrate(r)),
        _ => name,
    }
}

/// VBR averages wobble (318, 322); show the nominal rate when one is close.
fn round_bitrate(kbps: u32) -> u32 {
    const NOMINAL: [u32; 9] = [96, 128, 160, 192, 224, 256, 288, 320, 384];
    NOMINAL.into_iter().find(|n| kbps.abs_diff(*n) <= 4).unwrap_or(kbps)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compilations_credit_the_track_artist_but_stay_one_album() {
        let va = ["Moonpail".to_owned()];
        assert_eq!(
            artists(Some("Various Artists"), &va),
            ("Moonpail".into(), "Various Artists".into())
        );
        let feat = ["Glass Harbour feat. Neon Wren".to_owned()];
        assert_eq!(
            artists(Some("Glass Harbour"), &feat),
            ("Glass Harbour".into(), "Glass Harbour".into())
        );
        assert_eq!(artists(None, &[]), ("Unknown Artist".into(), "Unknown Artist".into()));
    }
}
