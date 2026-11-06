//! Songs that are on YouTube only inside a full-album upload (ADR-0016, "Full-album uploads"):
//! the album's track list (its chapters, else its description's timestamps) names each song's
//! bounds; a song is cut out of the album's audio as its own M4A, its AAC frames copied as they
//! are (no re-encoding), so starting, seeking, saving and scrobbling treat it like any copy. The
//! album's audio is fetched once for all its songs. Without a track list that names the song at
//! its length, there is no copy.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use lull_core::m4a;

use crate::matching::{self, Wanted};

/// One entry of an upload's track list: its title and where it starts and ends (seconds).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Chapter {
    pub title: String,
    pub start_time: f64,
    pub end_time: f64,
}

/// The upload's track list: its chapters, else the timestamps in its description ("01. The
/// Flowers Smell Like You 00:00", "4:54 Don't Look At Me Like That"), each ending where the next
/// starts and the last at the upload's end.
pub fn track_list(chapters: &[Chapter], description: &str, duration: f64) -> Vec<Chapter> {
    if chapters.len() > 1 {
        return chapters.to_vec();
    }
    let mut marks: Vec<(f64, String)> = description.lines().filter_map(timestamped).collect();
    marks.sort_by(|a, b| a.0.total_cmp(&b.0));
    marks.dedup_by(|a, b| a.0 == b.0);
    if marks.len() < 2 {
        return Vec::new();
    }
    let ends: Vec<f64> = marks.iter().skip(1).map(|m| m.0).chain([duration]).collect();
    marks
        .into_iter()
        .zip(ends)
        .filter(|((start, _), end)| end > start)
        .map(|((start_time, title), end_time)| Chapter {
            title,
            start_time,
            end_time,
        })
        .collect()
}

/// A description line with a timestamp ("m:ss" or "h:mm:ss") and a title beside it.
fn timestamped(line: &str) -> Option<(f64, String)> {
    let words: Vec<&str> = line.split_whitespace().collect();
    let (at, stamp) = words
        .iter()
        .enumerate()
        .find_map(|(i, w)| Some((i, seconds(w.trim_matches(['(', ')', '[', ']', '-']))?)))?;
    let title: String = words
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != at)
        .map(|(_, w)| *w)
        .collect::<Vec<_>>()
        .join(" ");
    let title = title.trim_matches(|c: char| c == '-' || c == '–' || c == '|' || c.is_whitespace());
    (!title.is_empty()).then(|| (stamp, title.to_owned()))
}

/// "4:54" or "1:02:03" in seconds.
fn seconds(s: &str) -> Option<f64> {
    let parts: Vec<&str> = s.split(':').collect();
    if !(2..=3).contains(&parts.len())
        || parts
            .iter()
            .any(|p| p.is_empty() || p.len() > 2 || !p.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    let n: Vec<u32> = parts.iter().filter_map(|p| p.parse().ok()).collect();
    if n[1..].iter().any(|v| *v >= 60) {
        return None;
    }
    Some(f64::from(n.iter().fold(0, |acc, v| acc * 60 + v)))
}

/// The entry that is the song: its title the song's, once a track number is dropped ("01.
/// The Flowers Smell Like You"), and its length within `slack_s` of the song's. `None` when no
/// entry, or more than one, names it so.
pub fn song_in(list: &[Chapter], wanted: &Wanted, slack_s: u32) -> Option<(u32, u32)> {
    let mut hits = list.iter().filter(|c| {
        let length = (c.end_time - c.start_time).round() as u32;
        matching::title_matches(wanted, &numbered_off(&c.title)) && wanted.length_agrees(length, slack_s)
    });
    let hit = hits.next()?;
    if hits.next().is_some() {
        return None;
    }
    Some(((hit.start_time * 1000.0) as u32, (hit.end_time * 1000.0) as u32))
}

/// "01. Title", "1 - Title", "A1 Title": the title alone.
fn numbered_off(title: &str) -> String {
    let t = title.trim();
    let first = t.split_whitespace().next().unwrap_or_default();
    let numbered = first
        .trim_end_matches(['.', ')', ':', '-'])
        .chars()
        .all(|c| c.is_ascii_alphanumeric())
        && first.chars().any(|c| c.is_ascii_digit())
        && first.len() <= 4;
    if numbered {
        t[first.len()..]
            .trim_start_matches([' ', '-', '–', '.', ')'])
            .to_owned()
    } else {
        t.to_owned()
    }
}

/// Copies the AAC frames of `album` (an M4A) between `from_ms` and `to_ms` into `dest`, no
/// re-encoding: an M4A of its own (`m4a::write`), so it is saved, tagged, seeked and read like any
/// YouTube AAC copy. A `dest` ending `.aac` gets raw AAC (ADTS) instead, as an earlier lull's cut
/// named so. Its size.
pub fn cut(album: &Path, from_ms: u32, to_ms: u32, dest: &Path) -> Result<u64, String> {
    let file = std::fs::File::open(album).map_err(|e| format!("can't open {}: {e}", album.display()))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("m4a");
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| format!("not an album lull can cut: {e}"))?;
    let mut format = probed.format;
    let track = format.default_track().ok_or("no audio in the album")?.clone();
    let asc = track.codec_params.extra_data.clone().unwrap_or_default();
    let config = AudioConfig::of(&asc).ok_or("the album's audio is no AAC lull can cut")?;
    let rate = track.codec_params.sample_rate.unwrap_or(config.rate());
    let (from, to) = (
        u64::from(from_ms) * u64::from(rate) / 1000,
        u64::from(to_ms) * u64::from(rate) / 1000,
    );
    let mut frames = Vec::new();
    while let Ok(packet) = format.next_packet() {
        if packet.track_id() != track.id || packet.ts() < from {
            continue;
        }
        if packet.ts() >= to {
            break;
        }
        let duration = u32::try_from(packet.dur()).unwrap_or(1024);
        frames.push(m4a::Frame {
            data: packet.data.to_vec(),
            duration,
        });
    }
    if frames.is_empty() {
        return Err("nothing of the album's audio lies there".into());
    }
    let out = if dest.extension().is_some_and(|e| e == "aac") {
        frames
            .iter()
            .flat_map(|f| {
                config
                    .adts_header(f.data.len())
                    .into_iter()
                    .chain(f.data.iter().copied())
            })
            .collect()
    } else {
        m4a::write(&asc, rate, config.channels(), &frames)
    };
    std::fs::write(dest, &out).map_err(|e| format!("can't write {}: {e}", dest.display()))?;
    Ok(out.len() as u64)
}

/// What an ADTS header repeats from the AAC stream's AudioSpecificConfig.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioConfig {
    object_type: u8,
    rate_index: u8,
    channels: u8,
}

impl AudioConfig {
    /// From the first two bytes of an AudioSpecificConfig.
    pub fn of(asc: &[u8]) -> Option<AudioConfig> {
        let (a, b) = (*asc.first()?, *asc.get(1)?);
        let c = AudioConfig {
            object_type: a >> 3,
            rate_index: ((a & 0b111) << 1) | (b >> 7),
            channels: (b >> 3) & 0b1111,
        };
        (c.object_type >= 1 && c.object_type <= 4 && c.rate_index < 13).then_some(c)
    }

    fn channels(self) -> u16 {
        u16::from(self.channels)
    }

    fn rate(self) -> u32 {
        const RATES: [u32; 13] = [
            96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350,
        ];
        RATES[usize::from(self.rate_index)]
    }

    /// The 7-byte header (no CRC) of a frame of `payload` bytes.
    pub fn adts_header(self, payload: usize) -> [u8; 7] {
        let len = payload + 7;
        [
            0xFF,
            0xF1,
            ((self.object_type - 1) << 6) | (self.rate_index << 2) | (self.channels >> 2),
            ((self.channels & 3) << 6) | ((len >> 11) as u8 & 0b11),
            (len >> 3) as u8,
            (((len & 0b111) as u8) << 5) | 0x1F,
            0xFC,
        ]
    }
}

/// Where an album's audio and its songs' cuts are kept.
pub fn album_path(dir: &Path, video: &str) -> PathBuf {
    dir.join("albums").join(format!("{video}.m4a"))
}

/// `ext`: the copy's, `m4a` (an earlier lull's cut is `aac`).
pub fn cut_path(dir: &Path, video: &str, from_ms: u32, to_ms: u32, ext: &str) -> PathBuf {
    dir.join("albums").join(format!("{video}~{from_ms}-{to_ms}.{ext}"))
}

#[cfg(test)]
mod tests {
    use lull_core::ids::{AlbumId, ArtistId, TrackId};
    use lull_core::model::Track;

    use super::*;

    fn song(title: &str, duration: u32) -> Wanted {
        Wanted::of(&Track {
            id: TrackId("t".into()),
            title: title.into(),
            artist_id: ArtistId("a".into()),
            artist: "Mabisyo".into(),
            album_id: AlbumId("al".into()),
            album: "What Are We?".into(),
            year: None,
            no: Some(1),
            disc: None,
            duration: Some(duration),
            tags: vec![],
            saved: false,
            file: None,
        })
    }

    /// Mabisyo's "What Are We?" as one upload (2026-11-05): its chapters, and the same track list
    /// in its description.
    fn chapters() -> Vec<Chapter> {
        [
            ("01. The Flowers Smell Like You", 0.0, 294.0),
            ("02. Don't Look At Me Like That", 294.0, 610.0),
            ("03. I Can't Lie To You", 610.0, 942.0),
            ("08. Your Messages Are Memories", 2066.0, 2431.0),
        ]
        .into_iter()
        .map(|(t, s, e)| Chapter {
            title: t.into(),
            start_time: s,
            end_time: e,
        })
        .collect()
    }

    const DESCRIPTION: &str = "Mabisyo - What Are We?\nAlbum Link: https://example.invalid/what-are-we\n\nTracklist:\n\
        01. The Flowers Smell Like You 00:00\n02. Don't Look At Me Like That 04:54\n03. I Can't Lie To You 10:10\n\
        Instagram: https://example.invalid/me\n";

    #[test]
    fn a_song_is_found_in_the_albums_chapters_at_its_length() {
        let list = track_list(&chapters(), "", 2431.0);
        assert_eq!(
            song_in(&list, &song("The Flowers Smell Like You", 293), 5),
            Some((0, 294_000))
        );
        assert_eq!(
            song_in(&list, &song("I Can't Lie To You", 330), 5),
            Some((610_000, 942_000))
        );
        // The length must agree, as for any upload.
        assert_eq!(song_in(&list, &song("The Flowers Smell Like You", 240), 5), None);
        // A song the album doesn't list: no copy.
        assert_eq!(song_in(&list, &song("Another Song", 294), 5), None);
    }

    #[test]
    fn without_chapters_the_descriptions_timestamps_are_the_track_list() {
        let list = track_list(&[], DESCRIPTION, 2431.0);
        assert_eq!(list.len(), 3, "{list:?}");
        assert_eq!(list[1].title, "02. Don't Look At Me Like That");
        assert_eq!((list[1].start_time, list[1].end_time), (294.0, 610.0));
        assert_eq!(
            song_in(&list, &song("Don't Look At Me Like That", 316), 5),
            Some((294_000, 610_000))
        );
        // No track list at all: no copy.
        assert!(track_list(&[], "Just one long mix, enjoy!", 2431.0).is_empty());
    }

    #[test]
    fn an_adts_header_names_the_stream_and_the_frames_length() {
        // AAC LC, 44.1 kHz, stereo: AudioSpecificConfig 0x12 0x10.
        let c = AudioConfig::of(&[0x12, 0x10]).unwrap_or(AudioConfig {
            object_type: 0,
            rate_index: 0,
            channels: 0,
        });
        assert_eq!(c.rate(), 44_100);
        let h = c.adts_header(371);
        assert_eq!(h[..2], [0xFF, 0xF1]);
        let len = ((usize::from(h[3]) & 3) << 11) | (usize::from(h[4]) << 3) | (usize::from(h[5]) >> 5);
        assert_eq!(len, 378);
        assert_eq!(h[2] >> 6, 1, "profile LC");
        assert_eq!((h[2] >> 2) & 0xF, 4, "44.1 kHz");
    }

    /// AAC-LC's silent frame for stereo at 44.1 kHz (the one hls.js pads gaps with): an album of it.
    const SILENCE: [u8; 9] = [0x21, 0x00, 0x49, 0x90, 0x02, 0x19, 0x00, 0x23, 0x80];
    const ASC: [u8; 2] = [0x12, 0x10];

    fn silent_album(dir: &Path, seconds: u32) -> Result<PathBuf, std::io::Error> {
        let frames = (0..seconds * 44_100 / 1024)
            .map(|_| m4a::Frame {
                data: SILENCE.to_vec(),
                duration: 1024,
            })
            .collect::<Vec<_>>();
        let path = dir.join("album.m4a");
        std::fs::write(&path, m4a::write(&ASC, 44_100, 2, &frames))?;
        Ok(path)
    }

    /// Decodes the file as the player's own decoder would: how long its audio is.
    fn decoded_seconds(path: &Path) -> Result<f64, Box<dyn std::error::Error>> {
        let mss = MediaSourceStream::new(Box::new(std::fs::File::open(path)?), Default::default());
        let mut hint = Hint::new();
        hint.with_extension("m4a");
        let mut format = symphonia::default::get_probe()
            .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())?
            .format;
        let track = format.default_track().ok_or("no audio")?.clone();
        let mut decoder = symphonia::default::get_codecs().make(&track.codec_params, &Default::default())?;
        let mut samples = 0;
        while let Ok(packet) = format.next_packet() {
            samples += decoder.decode(&packet)?.frames();
        }
        Ok(samples as f64 / f64::from(track.codec_params.sample_rate.ok_or("no rate")?))
    }

    #[test]
    fn a_cut_is_an_m4a_of_the_songs_own_frames() -> Result<(), Box<dyn std::error::Error>> {
        let dir = std::env::temp_dir().join(format!("lull-cut-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let album = silent_album(&dir, 6)?;
        let dest = dir.join("song.m4a");
        let size = cut(&album, 2_000, 4_000, &dest)?;
        let bytes = std::fs::read(&dest)?;
        assert_eq!(bytes.len() as u64, size);
        assert_eq!(&bytes[4..8], b"ftyp", "an M4A");
        let seconds = decoded_seconds(&dest)?;
        assert!((seconds - 2.0).abs() < 0.05, "two seconds of the album: {seconds}");
        // an earlier lull's cut, named .aac, is raw AAC
        let earlier = dir.join("song.aac");
        cut(&album, 2_000, 4_000, &earlier)?;
        assert_eq!(&std::fs::read(&earlier)?[..2], [0xFF, 0xF1], "ADTS");
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    /// The whole cut on a real album upload's AAC copy: point `LULL_ALBUM_M4A` at one.
    #[test]
    #[ignore = "needs a local full-album M4A"]
    fn a_cut_is_one_songs_audio_and_plays_as_its_own_file() -> Result<(), Box<dyn std::error::Error>> {
        let album = std::path::PathBuf::from(std::env::var("LULL_ALBUM_M4A")?);
        // LULL_CUT_OUT keeps the cut there, to play it in a browser; LULL_CUT_MS its bounds ("from-to")
        let keep = std::env::var("LULL_CUT_OUT").ok();
        let dest = keep
            .clone()
            .map_or_else(|| std::env::temp_dir().join("lull-cut-test.m4a"), PathBuf::from);
        let bounds = std::env::var("LULL_CUT_MS").unwrap_or_else(|_| "0-294000".into());
        let (from, to) = bounds.split_once('-').ok_or("LULL_CUT_MS is from-to")?;
        let (from, to): (u32, u32) = (from.parse()?, to.parse()?);
        cut(&album, from, to, &dest)?;
        let print = crate::sound::print_file(&dest, None)?;
        let want = f64::from(to - from) / 1000.0;
        assert!((print.seconds - want).abs() < 1.0, "{} of {want}", print.seconds);
        if keep.is_none() {
            let _ = std::fs::remove_file(&dest);
        }
        Ok(())
    }
}
