//! Which of an upload's streams to take: audio only, in the listener's format (Opus in WebM or
//! AAC in M4A) and as delivered, so nothing is transcoded. The size must be known up front, since
//! the stream cache serves byte ranges against it while the download still runs.

use lull_core::youtube::{Copy, Format, Want};
use serde::Deserialize;

/// What lull reads of yt-dlp's `--dump-json` for one upload.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Info {
    pub id: String,
    #[serde(default)]
    pub title: String,
    pub channel: Option<String>,
    pub uploader: Option<String>,
    pub duration: Option<f64>,
    /// Music metadata YouTube attaches to official audio.
    pub track: Option<String>,
    pub artists: Option<Vec<String>>,
    pub artist: Option<String>,
    pub live_status: Option<String>,
    #[serde(default)]
    pub formats: Vec<Stream>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Stream {
    pub format_id: String,
    #[serde(default)]
    pub ext: String,
    pub vcodec: Option<String>,
    pub acodec: Option<String>,
    /// Average kbps.
    pub abr: Option<f64>,
    /// Exact bytes; `filesize_approx` is not good enough for byte ranges.
    pub filesize: Option<u64>,
    pub protocol: Option<String>,
    pub has_drm: Option<bool>,
    pub format_note: Option<String>,
}

/// The stream picked: its copy and exact size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chosen {
    pub copy: Copy,
    pub size: u64,
}

/// A streaming cap is compared with YouTube's average bitrate, which runs a little over the
/// nominal one (AAC 128 reports 129.5): this much over still counts as within.
const CAP_SLACK: f64 = 1.1;

pub fn choose(info: &Info, want: Want, preferred: Format) -> Option<Chosen> {
    let order: &[Format] = match (want, preferred) {
        (Want::Save, _) => &[Format::Aac],
        (Want::Stream { .. }, Format::Opus) => &[Format::Opus, Format::Aac],
        (Want::Stream { .. }, Format::Aac) => &[Format::Aac, Format::Opus],
    };
    let cap = match want {
        Want::Stream { max_kbps } if max_kbps > 0 => Some(f64::from(max_kbps) * CAP_SLACK),
        _ => None,
    };
    let usable: Vec<&Stream> = info.formats.iter().filter(|s| usable(s)).collect();
    // Dubbed audio tracks carry a language suffix ("251-1"); the original one is marked.
    let plain: Vec<&Stream> = match usable.iter().any(|s| !s.format_id.contains('-')) {
        true => usable.into_iter().filter(|s| !s.format_id.contains('-')).collect(),
        false => usable
            .into_iter()
            .filter(|s| s.format_note.as_deref().is_some_and(|n| n.contains("original")))
            .collect(),
    };
    order.iter().find_map(|format| {
        let mut of: Vec<&Stream> = plain.iter().copied().filter(|s| codec(s) == Some(*format)).collect();
        of.sort_by(|a, b| abr(a).total_cmp(&abr(b)));
        let pick = match cap {
            Some(cap) => of.iter().rev().find(|s| abr(s) <= cap).or(of.first()),
            None => of.last(),
        }?;
        Some(Chosen {
            copy: Copy {
                video: info.id.clone(),
                format: pick.format_id.clone(),
                ext: pick.ext.clone(),
            },
            size: pick.filesize?,
        })
    })
}

/// Audio only, one plain HTTPS file of known size, no DRM, not YouTube's loudness-squashed copy.
fn usable(s: &Stream) -> bool {
    s.vcodec.as_deref() == Some("none")
        && s.acodec.as_deref().is_some_and(|a| a != "none")
        && s.protocol.as_deref() == Some("https")
        && s.filesize.is_some_and(|n| n > 0)
        && s.has_drm != Some(true)
        && !s.format_id.ends_with("-drc")
        && !s.format_id.is_empty()
        && s.format_id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

fn codec(s: &Stream) -> Option<Format> {
    let a = s.acodec.as_deref()?;
    match s.ext.as_str() {
        "webm" if a.starts_with("opus") => Some(Format::Opus),
        "m4a" if a.starts_with("mp4a") => Some(Format::Aac),
        _ => None,
    }
}

fn abr(s: &Stream) -> f64 {
    s.abr.unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(id: &str, ext: &str, acodec: &str, abr: f64, size: u64) -> Stream {
        Stream {
            format_id: id.into(),
            ext: ext.into(),
            vcodec: Some("none".into()),
            acodec: Some(acodec.into()),
            abr: Some(abr),
            filesize: Some(size),
            protocol: Some("https".into()),
            has_drm: Some(false),
            format_note: None,
        }
    }

    /// The audio streams of a real upload (LUjGtyYEi90, 2026-11-02), plus a video stream and HLS.
    fn info() -> Info {
        let video = Stream {
            vcodec: Some("avc1".into()),
            ..stream("137", "mp4", "none", 0.0, 90_000_000)
        };
        let hls = Stream {
            protocol: Some("m3u8_native".into()),
            filesize: None,
            ..stream("234", "mp4", "mp4a.40.2", 128.0, 0)
        };
        Info {
            id: "LUjGtyYEi90".into(),
            formats: vec![
                video,
                hls,
                stream("139", "m4a", "mp4a.40.5", 48.838, 1_942_474),
                stream("249", "webm", "opus", 56.433, 2_244_660),
                stream("250", "webm", "opus", 73.973, 2_942_321),
                stream("140", "m4a", "mp4a.40.2", 129.546, 5_152_490),
                stream("251", "webm", "opus", 144.067, 5_730_310),
                stream("251-drc", "webm", "opus", 144.0, 5_730_000),
            ],
            ..Info::default()
        }
    }

    fn pick(want: Want, format: Format) -> Option<(String, u64)> {
        choose(&info(), want, format).map(|c| (c.copy.format, c.size))
    }

    #[test]
    fn the_best_audio_stream_in_the_listeners_format_with_its_exact_size() {
        assert_eq!(
            pick(Want::Stream { max_kbps: 0 }, Format::Opus),
            Some(("251".into(), 5_730_310))
        );
        assert_eq!(
            pick(Want::Stream { max_kbps: 0 }, Format::Aac),
            Some(("140".into(), 5_152_490))
        );
        assert_eq!(
            pick(Want::Save, Format::Opus),
            Some(("140".into(), 5_152_490)),
            "saves take AAC, which the save pipeline can tag"
        );
    }

    #[test]
    fn a_streaming_cap_takes_the_best_stream_within_it_else_the_smallest() {
        assert_eq!(
            pick(Want::Stream { max_kbps: 128 }, Format::Aac).map(|p| p.0),
            Some("140".into())
        );
        assert_eq!(
            pick(Want::Stream { max_kbps: 128 }, Format::Opus).map(|p| p.0),
            Some("250".into()),
            "Opus 160 is over a 128 cap"
        );
        let only_251 = Info {
            formats: vec![stream("251", "webm", "opus", 144.0, 5_000_000)],
            ..info()
        };
        let c = choose(&only_251, Want::Stream { max_kbps: 128 }, Format::Opus);
        assert_eq!(c.map(|c| c.copy.format), Some("251".into()), "a cap is a preference");
    }

    #[test]
    fn no_stream_without_a_known_size_or_with_drm() {
        let mut i = info();
        for s in &mut i.formats {
            s.filesize = None;
        }
        assert_eq!(choose(&i, Want::Stream { max_kbps: 0 }, Format::Opus), None);
        let mut i = info();
        for s in &mut i.formats {
            s.has_drm = Some(true);
        }
        assert_eq!(choose(&i, Want::Save, Format::Aac), None);
    }
}
