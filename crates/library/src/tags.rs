//! Reading and writing a file's tags with lofty. Blocking: call from `spawn_blocking`.

use std::path::Path;

use lofty::config::WriteOptions;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::tag::{Accessor, ItemKey, ItemValue, Tag, TagExt, TagItem};

/// What a file already says about itself: the input to matching and to "keep existing tags".
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Existing {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album_artist: Option<String>,
    pub album: Option<String>,
    pub track: Option<u32>,
    pub disc: Option<u32>,
    pub disc_total: Option<u32>,
    pub year: Option<u16>,
    pub duration_s: Option<f64>,
    pub has_cover: bool,
}

/// The full, consistent tag set written after a MusicBrainz match.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct TagSet {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub track: Option<u32>,
    pub track_total: Option<u32>,
    pub disc: Option<u32>,
    pub disc_total: Option<u32>,
    /// ISO date of this release ("2024", "2024-05", "2024-05-01").
    pub date: Option<String>,
    /// ISO date the release group first came out.
    pub original_date: Option<String>,
    pub genres: Vec<String>,
    pub label: Option<String>,
    pub mb: MbIds,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct MbIds {
    pub recording: Option<String>,
    pub release_track: Option<String>,
    pub release: Option<String>,
    pub release_group: Option<String>,
    pub artist: Option<String>,
    pub album_artist: Option<String>,
}

pub fn read(path: &Path) -> Result<Existing, String> {
    let file = lofty::read_from_path(path).map_err(|e| format!("can't read the file's tags: {e}"))?;
    let duration = file.properties().duration().as_secs_f64();
    let mut out = Existing {
        duration_s: (duration > 0.0).then_some(duration),
        ..Existing::default()
    };
    let Some(tag) = file.primary_tag().or_else(|| file.first_tag()) else {
        return Ok(out);
    };
    let text = |key| {
        tag.get_string(key)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    out.title = text(ItemKey::TrackTitle);
    out.artist = text(ItemKey::TrackArtist);
    out.album_artist = text(ItemKey::AlbumArtist);
    out.album = text(ItemKey::AlbumTitle);
    out.track = tag.track();
    out.disc = tag.disk();
    out.disc_total = tag.disk_total();
    out.year = tag
        .date()
        .map(|d| d.year)
        .or_else(|| text(ItemKey::Year).and_then(|y| y.get(..4)?.parse().ok()));
    out.has_cover = tag.get_picture_type(PictureType::CoverFront).is_some() || !tag.pictures().is_empty();
    Ok(out)
}

/// Writes a matched release's tags (and the cover, when there is one) into the file's primary
/// tag, and drops the file's other tags so no stale ID3v1 or APE values contradict them.
pub fn write(path: &Path, set: &TagSet, cover: Option<&[u8]>) -> Result<(), String> {
    let mut file = lofty::read_from_path(path).map_err(|e| format!("can't read the file to tag it: {e}"))?;
    let kind = file.primary_tag_type();
    let mut tag = file.remove(kind).unwrap_or_else(|| Tag::new(kind));
    // Every key below is replaced; the rest of the old tag (ReplayGain, lyrics...) is kept.
    tag.remove_key(ItemKey::Year);
    let number = |n: Option<u32>| n.map(|n| n.to_string());
    let items = [
        (ItemKey::TrackTitle, Some(set.title.clone())),
        (ItemKey::TrackArtist, Some(set.artist.clone())),
        (ItemKey::AlbumTitle, Some(set.album.clone())),
        (ItemKey::AlbumArtist, Some(set.album_artist.clone())),
        (ItemKey::TrackNumber, number(set.track)),
        (ItemKey::TrackTotal, number(set.track_total)),
        (ItemKey::DiscNumber, number(set.disc)),
        (ItemKey::DiscTotal, number(set.disc_total)),
        (ItemKey::RecordingDate, set.date.clone()),
        (ItemKey::OriginalReleaseDate, set.original_date.clone()),
        // One value: every format and Jellyfin split "a; b" the same way.
        (ItemKey::Genre, (!set.genres.is_empty()).then(|| set.genres.join("; "))),
        (ItemKey::Label, set.label.clone()),
        (ItemKey::MusicBrainzRecordingId, set.mb.recording.clone()),
        (ItemKey::MusicBrainzTrackId, set.mb.release_track.clone()),
        (ItemKey::MusicBrainzReleaseId, set.mb.release.clone()),
        (ItemKey::MusicBrainzReleaseGroupId, set.mb.release_group.clone()),
        (ItemKey::MusicBrainzArtistId, set.mb.artist.clone()),
        (ItemKey::MusicBrainzReleaseArtistId, set.mb.album_artist.clone()),
    ];
    for (key, value) in items {
        tag.remove_key(key);
        if let Some(v) = value.filter(|v| !v.is_empty()) {
            tag.insert_text(key, v);
        }
    }
    if let Some(bytes) = cover {
        tag.remove_picture_type(PictureType::CoverFront);
        tag.push_picture(
            Picture::unchecked(bytes.to_vec())
                .pic_type(PictureType::CoverFront)
                .mime_type(mime(bytes))
                .build(),
        );
    }
    save(path, &tag)
}

/// "Keep existing tags": the file's tags stay as they are, except that a missing title,
/// artist or album is filled in, so Jellyfin shows the song under the name it was saved as.
pub fn fill_blanks(path: &Path, title: &str, artist: &str, album: &str) -> Result<(), String> {
    let mut file = lofty::read_from_path(path).map_err(|e| format!("can't read the file to tag it: {e}"))?;
    let kind = file.primary_tag_type();
    let mut tag = file.remove(kind).unwrap_or_else(|| Tag::new(kind));
    let mut changed = false;
    for (key, value) in [
        (ItemKey::TrackTitle, title),
        (ItemKey::TrackArtist, artist),
        (ItemKey::AlbumTitle, album),
    ] {
        let blank = tag.get_string(key).is_none_or(|v| v.trim().is_empty());
        if blank && !value.is_empty() {
            tag.insert(TagItem::new(key, ItemValue::Text(value.to_owned())));
            changed = true;
        }
    }
    if changed { save(path, &tag) } else { Ok(()) }
}

fn save(path: &Path, tag: &Tag) -> Result<(), String> {
    tag.save_to_path(path, WriteOptions::default().remove_others(true))
        .map_err(|e| format!("can't write tags: {e}"))
}

/// The image's type from its first bytes (Cover Art Archive serves JPEG, sometimes PNG).
pub fn mime(bytes: &[u8]) -> MimeType {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => MimeType::Png,
        [b'G', b'I', b'F', ..] => MimeType::Gif,
        _ => MimeType::Jpeg,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A FLAC file with only its STREAMINFO block: one second of 16-bit stereo at 44.1 kHz.
    fn tiny_flac() -> Vec<u8> {
        let mut f = b"fLaC".to_vec();
        f.extend([0x80, 0x00, 0x00, 0x22]); // last block, STREAMINFO, 34 bytes
        f.extend(4096u16.to_be_bytes()); // min block size
        f.extend(4096u16.to_be_bytes()); // max block size
        f.extend([0; 6]); // min / max frame size: unknown
        let packed: u64 = (44_100 << 44) | (1 << 41) | (15 << 36) | 44_100;
        f.extend(packed.to_be_bytes()); // rate, channels - 1, bits - 1, total samples
        f.extend([0; 16]); // MD5: unknown
        f
    }

    /// Ten silent MPEG-1 Layer III frames, 128 kbps at 44.1 kHz.
    fn tiny_mp3() -> Vec<u8> {
        let mut frame = vec![0xFF, 0xFB, 0x90, 0x00];
        frame.resize(417, 0);
        frame.repeat(10)
    }

    fn matched() -> TagSet {
        TagSet {
            title: "Rope & Tide".into(),
            artist: "Glass Harbour".into(),
            album: "Lantern Season".into(),
            album_artist: "Glass Harbour".into(),
            track: Some(3),
            track_total: Some(11),
            disc: Some(1),
            disc_total: Some(2),
            date: Some("2024-05-01".into()),
            original_date: Some("2023".into()),
            genres: vec!["slowcore".into(), "dream-gaze".into()],
            label: Some("Porchlight Press".into()),
            mb: MbIds {
                recording: Some("8f3e1c1a-0000-4000-8000-000000000001".into()),
                release_track: Some("8f3e1c1a-0000-4000-8000-000000000002".into()),
                release: Some("8f3e1c1a-0000-4000-8000-000000000003".into()),
                release_group: Some("8f3e1c1a-0000-4000-8000-000000000004".into()),
                artist: Some("8f3e1c1a-0000-4000-8000-000000000005".into()),
                album_artist: Some("8f3e1c1a-0000-4000-8000-000000000005".into()),
            },
        }
    }

    fn round_trip(name: &str, bytes: Vec<u8>) -> Result<(), String> {
        let dir = std::env::temp_dir().join(format!("lull-tags-{}", std::process::id()));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let path = dir.join(name);
        std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
        let cover = [0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3];

        write(&path, &matched(), Some(&cover))?;
        // Writing twice must not duplicate values or pictures.
        write(&path, &matched(), Some(&cover))?;

        let got = read(&path)?;
        assert_eq!(got.title.as_deref(), Some("Rope & Tide"), "{name}");
        assert_eq!(got.artist.as_deref(), Some("Glass Harbour"));
        assert_eq!(got.album_artist.as_deref(), Some("Glass Harbour"));
        assert_eq!(got.album.as_deref(), Some("Lantern Season"));
        assert_eq!((got.track, got.disc, got.disc_total), (Some(3), Some(1), Some(2)));
        assert_eq!(got.year, Some(2024));
        assert!(got.has_cover);

        let file = lofty::read_from_path(&path).map_err(|e| e.to_string())?;
        let tag = file.primary_tag().ok_or("no primary tag")?;
        assert_eq!(file.tags().len(), 1, "{name}: only the primary tag remains");
        assert_eq!(tag.pictures().len(), 1, "{name}: one cover");
        assert_eq!(tag.pictures()[0].data(), cover);
        for (key, want) in [
            (ItemKey::MusicBrainzRecordingId, "…0001"),
            (ItemKey::MusicBrainzReleaseId, "…0003"),
            (ItemKey::MusicBrainzReleaseGroupId, "…0004"),
            (ItemKey::Genre, "slowcore; dream-gaze"),
            (ItemKey::Label, "Porchlight Press"),
        ] {
            // ID3v2 keeps the label in TPUB, which reads back as Publisher.
            let value = tag
                .get_string(key)
                .or_else(|| {
                    (key == ItemKey::Label)
                        .then(|| tag.get_string(ItemKey::Publisher))
                        .flatten()
                })
                .ok_or(format!("{name}: no {key:?}"))?;
            let want = want.trim_start_matches('…');
            assert!(value.ends_with(want), "{name}: {key:?} = {value}");
        }
        assert_eq!(tag.get_items(ItemKey::TrackTitle).count(), 1);

        // Keep-tags mode fills only what is missing.
        fill_blanks(&path, "Other title", "Other artist", "Other album")?;
        assert_eq!(read(&path)?.title.as_deref(), Some("Rope & Tide"));
        std::fs::remove_file(&path).map_err(|e| e.to_string())
    }

    #[test]
    fn matched_tags_round_trip_in_flac() -> Result<(), String> {
        round_trip("song.flac", tiny_flac())
    }

    #[test]
    fn matched_tags_round_trip_in_mp3() -> Result<(), String> {
        round_trip("song.mp3", tiny_mp3())
    }

    #[test]
    fn keep_tags_fills_an_untagged_file() -> Result<(), String> {
        let dir = std::env::temp_dir().join(format!("lull-tags-blank-{}", std::process::id()));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let path = dir.join("bare.flac");
        std::fs::write(&path, tiny_flac()).map_err(|e| e.to_string())?;
        fill_blanks(&path, "Paper Moon Static", "Velvet Orchard", "Satellite Porch")?;
        let got = read(&path)?;
        assert_eq!(
            (got.title.as_deref(), got.artist.as_deref(), got.album.as_deref()),
            (
                Some("Paper Moon Static"),
                Some("Velvet Orchard"),
                Some("Satellite Porch")
            )
        );
        std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())
    }
}
