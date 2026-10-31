//! Jellyfin audio items as lull tracks: content-derived ids, file facts and quality.

use lull_core::ids::{AlbumId, ArtistId, TrackId, normalize_artist};
use lull_core::model::{FileFacts, Track};
use serde::{Deserialize, Serialize};

use crate::jellyfin::Item;

/// A saved file: the track the UI sees, plus what lull needs to stream it, show its cover and
/// replace it. Persisted as JSON, so the library still lists while Jellyfin is down.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Saved {
    pub item_id: String,
    pub track: Track,
    /// The album's owner: the album artist, or "Various Artists" for a compilation.
    pub album_artist: String,
    /// The file's path as Jellyfin sees it.
    pub path: Option<String>,
    /// When Jellyfin added it (ISO 8601, so it sorts as text).
    pub added: String,
    /// Where the cover comes from: (item id, image tag); the tag changes when the image does.
    pub art: Option<(String, String)>,
    pub mb_release: Option<String>,
    pub mb_release_group: Option<String>,
    pub mb_artist: Option<String>,
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
        .or_else(|| item.image_tags.get("Primary").map(|tag| (item.id.clone(), tag.clone())));
    let year = item
        .production_year
        .or_else(|| item.premiere_date.as_deref().and_then(|d| d.get(..4)?.parse().ok()));
    let provider = |key: &str| item.provider_ids.get(key).filter(|v| !v.is_empty()).cloned();
    Some(Saved {
        track: Track {
            id: TrackId::of(&credited, &album, &title),
            title,
            artist_id: ArtistId::of(&credited),
            artist: credited,
            album_id: AlbumId::of(&owner, &album),
            album,
            year,
            no: item.index_number,
            disc: item.parent_index_number,
            duration: item.run_time_ticks.map(|t| (t / 10_000_000) as u32),
            tags: item.genres.clone(),
            saved: true,
            file,
        },
        album_artist: owner,
        path: item.path.clone(),
        added: item.date_created.clone().unwrap_or_default(),
        art,
        mb_release: provider("MusicBrainzAlbum"),
        mb_release_group: provider("MusicBrainzReleaseGroup"),
        mb_artist: provider("MusicBrainzAlbumArtist").or_else(|| provider("MusicBrainzArtist")),
        item_id: item.id,
    })
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

fn is_various(artist: &str) -> bool {
    matches!(normalize_artist(artist).as_str(), "various artists" | "various" | "va")
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

pub fn is_lossless(codec_or_ext: &str) -> bool {
    matches!(
        codec_or_ext,
        "flac" | "alac" | "wav" | "aiff" | "aif" | "ape" | "wavpack" | "wv" | "tta"
    )
}

/// The UI's short label: "FLAC 16/44.1", "FLAC 24/96", "MP3 320", "OPUS 160".
fn format_label(codec: &str, lossless: bool, bits: Option<u8>, khz: Option<f32>, kbps: Option<u32>) -> String {
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

/// How good a file is, for picking between copies of one song.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Quality {
    pub lossless: bool,
    pub kbps: u32,
}

impl Quality {
    pub fn of(facts: Option<&FileFacts>) -> Self {
        Quality {
            lossless: facts.is_some_and(|f| f.lossless),
            kbps: facts.and_then(|f| f.kbps).unwrap_or(0),
        }
    }

    /// A candidate file known only by its name, size and the song's length (a Soulseek source).
    pub fn of_file(path: &str, size: u64, duration_s: Option<u32>) -> Self {
        let ext = path
            .rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase())
            .unwrap_or_default();
        Quality {
            lossless: is_lossless(&ext),
            kbps: duration_s
                .filter(|d| *d > 0)
                .map(|d| (size * 8 / 1000 / u64::from(d)) as u32)
                .unwrap_or(0),
        }
    }

    /// Clearly better: lossless over lossy, or at least a fifth more bitrate of the same kind.
    /// The margin keeps a re-encode or a slightly different rip from counting as an upgrade.
    pub fn beats(&self, other: &Quality) -> bool {
        match (self.lossless, other.lossless) {
            (true, false) => true,
            (false, true) => false,
            _ => other.kbps > 0 && u64::from(self.kbps) * 5 > u64::from(other.kbps) * 6,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(lossless: bool, kbps: u32) -> Quality {
        Quality { lossless, kbps }
    }

    #[test]
    fn upgrade_needs_a_clear_quality_step() {
        assert!(q(true, 900).beats(&q(false, 320)), "lossless beats any lossy");
        assert!(!q(false, 320).beats(&q(true, 700)), "lossy never beats lossless");
        assert!(q(false, 320).beats(&q(false, 192)));
        assert!(
            !q(false, 330).beats(&q(false, 320)),
            "a few kbps is noise, not an upgrade"
        );
        assert!(q(true, 2800).beats(&q(true, 1000)), "24/96 over 16/44.1");
        assert!(!q(true, 1050).beats(&q(true, 1000)), "two rips of the same CD");
        assert!(
            !q(false, 320).beats(&q(false, 0)),
            "unknown bitrate is never beaten blindly"
        );
    }

    #[test]
    fn a_soulseek_file_is_judged_by_extension_size_and_length() {
        let flac = Quality::of_file("Music\\Artist\\Album\\01 - Song.FLAC", 30_000_000, Some(240));
        assert_eq!(flac, q(true, 1000));
        let mp3 = Quality::of_file("01 - Song.mp3", 9_600_000, Some(240));
        assert_eq!(mp3, q(false, 320));
        assert!(flac.beats(&mp3));
    }

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
