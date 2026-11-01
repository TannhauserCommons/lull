//! Music-folder files as lull tracks: the folder source's counterpart of `items.rs`, reading
//! tags and audio properties with lofty. Blocking: call from `spawn_blocking`.

use std::path::Path;

use lofty::file::{AudioFile, FileType, TaggedFileExt};
use lofty::tag::{Accessor, ItemKey};
use lull_core::model::FileFacts;

use crate::items::{self, Extra, Location, Saved, format_label, is_lossless};

/// Extensions the folder index reads; everything else in the folder is ignored.
const AUDIO: &[&str] = &[
    "flac", "mp3", "ogg", "oga", "opus", "m4a", "aac", "wav", "aif", "aiff", "ape", "wv",
];

pub fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| AUDIO.contains(&e.to_ascii_lowercase().as_str()))
}

/// A file read for the index, before its album's cover is decided (folder.rs).
pub struct Read {
    pub saved: Saved,
    /// It carries a picture of its own (the cover when the folder has no image file).
    pub has_picture: bool,
}

/// Reads `rel` (a `/`-separated path under `root`). `modified` is unix seconds.
pub fn read(root: &Path, rel: &str, modified: i64, size: u64) -> Result<Read, String> {
    let path = rel.split('/').fold(root.to_owned(), |p, part| p.join(part));
    let file = lofty::read_from_path(&path).map_err(|e| format!("{rel}: {e}"))?;
    let tag = file.primary_tag().or_else(|| file.first_tag());
    let text = |key| {
        tag.and_then(|t| t.get_string(key))
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    let title = text(ItemKey::TrackTitle).unwrap_or_else(|| stem(rel));
    let album = text(ItemKey::AlbumTitle).unwrap_or_default();
    let track_artists: Vec<String> = text(ItemKey::TrackArtist).into_iter().collect();
    let (credited, owner) = items::artists(text(ItemKey::AlbumArtist).as_deref(), &track_artists);
    let duration = file.properties().duration().as_secs();
    let year = tag
        .and_then(|t| t.date())
        .map(|d| d.year)
        .or_else(|| text(ItemKey::Year).and_then(|y| y.get(..4)?.parse().ok()));
    let saved = Saved {
        track: items::track(
            &credited,
            &owner,
            album,
            title,
            Extra {
                year,
                no: tag.and_then(|t| t.track()),
                disc: tag.and_then(|t| t.disk()),
                duration: (duration > 0).then_some(duration as u32),
                tags: genres(tag.map(|t| t.get_strings(ItemKey::Genre).collect()).unwrap_or_default()),
                file: Some(facts(&file, rel, size)),
            },
        ),
        album_artist: owner,
        location: Location::File { path: rel.to_owned() },
        added: iso_utc(modified),
        art: None,
        mb_release: text(ItemKey::MusicBrainzReleaseId),
        mb_release_group: text(ItemKey::MusicBrainzReleaseGroupId),
        mb_artist: text(ItemKey::MusicBrainzReleaseArtistId).or_else(|| text(ItemKey::MusicBrainzArtistId)),
    };
    Ok(Read {
        saved,
        has_picture: tag.is_some_and(|t| !t.pictures().is_empty()),
    })
}

/// "03 - Rope & Tide.flac" → "03 - Rope & Tide": an untagged file is still listed by name.
fn stem(rel: &str) -> String {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    name.rsplit_once('.').map_or(name, |(s, _)| s).to_owned()
}

/// Genre values as Jellyfin splits them ("a; b", "a/b" stay apart).
fn genres(values: Vec<&str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for g in values.iter().flat_map(|v| v.split([';', '/'])) {
        let g = g.trim();
        if !g.is_empty() && !out.iter().any(|o| o.eq_ignore_ascii_case(g)) {
            out.push(g.to_owned());
        }
    }
    out
}

fn facts(file: &lofty::file::TaggedFile, rel: &str, size: u64) -> FileFacts {
    let props = file.properties();
    let ext = rel
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    let codec = match file.file_type() {
        FileType::Flac => "flac",
        FileType::Mpeg => "mp3",
        FileType::Opus => "opus",
        FileType::Vorbis => "vorbis",
        FileType::Wav => "wav",
        FileType::Aiff => "aiff",
        FileType::Ape => "ape",
        FileType::WavPack => "wavpack",
        FileType::Aac => "aac",
        // MP4 holds AAC or ALAC; only ALAC reports a bit depth.
        FileType::Mp4 if props.bit_depth().is_some() => "alac",
        FileType::Mp4 => "aac",
        _ => ext.as_str(),
    }
    .to_owned();
    let lossless = is_lossless(&codec);
    let khz = props.sample_rate().map(|hz| (hz as f32 / 100.0).round() / 10.0);
    let bits = props.bit_depth().filter(|_| lossless);
    let kbps = props.audio_bitrate().or(props.overall_bitrate());
    FileFacts {
        format: format_label(&codec, lossless, bits, khz, kbps),
        codec: codec.to_ascii_uppercase(),
        lossless,
        khz,
        bits,
        kbps,
        size_bytes: Some(size),
    }
}

/// Unix seconds as "2024-05-01T12:00:00Z" (the civil-from-days algorithm, proleptic Gregorian).
pub fn iso_utc(unix: i64) -> String {
    let (days, secs) = (unix.div_euclid(86_400), unix.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        secs / 3600,
        secs % 3600 / 60,
        secs % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modified_times_sort_as_text() {
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_utc(1_714_564_800), "2024-05-01T12:00:00Z");
        assert_eq!(iso_utc(951_782_400), "2000-02-29T00:00:00Z");
    }
}
