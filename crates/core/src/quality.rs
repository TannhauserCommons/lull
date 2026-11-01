//! One rule for audio quality, and the listener's source rules for streaming and saving
//! (ADR-0008). Soulseek ranks and gates sources with these; library decides upgrades with them;
//! collection stores the two profiles and publishes them in the `settings` event.

use serde::{Deserialize, Serialize};

use crate::model::FileFacts;

/// How good a file is, as far as lull can tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Quality {
    pub lossless: bool,
    /// 0 when unknown.
    pub kbps: u32,
}

impl Quality {
    pub fn of(facts: Option<&FileFacts>) -> Self {
        Quality {
            lossless: facts.is_some_and(|f| f.lossless),
            kbps: facts.and_then(|f| f.kbps).unwrap_or(0),
        }
    }

    /// A file known only by its name, size and the song's length (a Soulseek source).
    pub fn of_file(path: &str, size: u64, duration_s: Option<u32>) -> Self {
        Quality {
            lossless: is_lossless(&extension(path)),
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

    /// Whether a file with this quality and `format` (lowercase extension) passes `min`.
    /// A lossy file of unknown bitrate fails any bitrate floor: it can't be shown to pass.
    pub fn meets(&self, format: &str, min: &Minimum) -> bool {
        if !min.formats.is_empty() && !min.formats.iter().any(|f| f.eq_ignore_ascii_case(format)) {
            return false;
        }
        self.lossless || (!min.lossless_only && self.kbps >= min.kbps)
    }
}

/// The lowest quality a profile accepts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Minimum {
    pub lossless_only: bool,
    /// Lossy floor in kbps; lossless always passes it.
    pub kbps: u32,
    /// Allowed formats (lowercase extensions); empty means any.
    #[serde(default)]
    pub formats: Vec<String>,
}

/// Which passing copy to take first. Either way a copy that stalls or never starts falls back
/// to the next one in the same order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Pick {
    /// The best-scoring copy among those that pass (speed, free slot, queue, then quality).
    Fastest,
    /// The highest quality among those that pass, speed breaking ties.
    Highest,
}

/// Streaming when no copy passes the minimum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StreamBelow {
    /// Play the best copy found anyway, marked as below the minimum.
    PlayBest,
    /// Keep searching; play only a copy that passes.
    KeepLooking,
    /// Show what was found and let the listener choose.
    Ask,
}

/// Saving when no copy passes the minimum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SaveBelow {
    /// Save the best copy now, marked below the minimum, and upgrade it when a passing copy appears.
    SaveBestUpgrade,
    /// Don't save until a passing copy appears; keep retrying in the background.
    Wait,
    /// Show what was found and let the listener choose.
    Ask,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamProfile {
    pub minimum: Minimum,
    pub pick: Pick,
    pub below: StreamBelow,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveProfile {
    pub minimum: Minimum,
    pub pick: Pick,
    pub below: SaveBelow,
    /// Re-search saved songs below the minimum and replace them when a passing copy appears.
    pub auto_upgrade: bool,
    /// Hours between re-searches of one song.
    pub upgrade_every_h: u32,
}

impl Default for StreamProfile {
    fn default() -> Self {
        StreamProfile {
            minimum: Minimum {
                lossless_only: false,
                kbps: 192,
                formats: Vec::new(),
            },
            pick: Pick::Fastest,
            below: StreamBelow::PlayBest,
        }
    }
}

impl Default for SaveProfile {
    fn default() -> Self {
        SaveProfile {
            minimum: Minimum {
                lossless_only: false,
                kbps: 320,
                formats: Vec::new(),
            },
            pick: Pick::Highest,
            below: SaveBelow::SaveBestUpgrade,
            auto_upgrade: true,
            upgrade_every_h: 24,
        }
    }
}

pub fn is_lossless(codec_or_ext: &str) -> bool {
    matches!(
        codec_or_ext,
        "flac" | "alac" | "wav" | "aiff" | "aif" | "ape" | "wavpack" | "wv" | "tta"
    )
}

/// The lowercase extension of a file path, or "".
pub fn extension(path: &str) -> String {
    path.rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(lossless: bool, kbps: u32) -> Quality {
        Quality { lossless, kbps }
    }

    #[test]
    fn upgrade_needs_a_clear_quality_step() {
        assert!(q(true, 900).beats(&q(false, 320)));
        assert!(!q(false, 320).beats(&q(true, 700)));
        assert!(q(false, 320).beats(&q(false, 192)));
        assert!(!q(false, 330).beats(&q(false, 320)), "a few kbps is noise");
        assert!(!q(true, 1050).beats(&q(true, 1000)), "two rips of the same CD");
        assert!(
            !q(false, 320).beats(&q(false, 0)),
            "unknown bitrate is never beaten blindly"
        );
    }

    #[test]
    fn a_minimum_gates_by_kind_bitrate_and_format() {
        let any320 = Minimum {
            lossless_only: false,
            kbps: 320,
            formats: vec![],
        };
        assert!(q(false, 320).meets("mp3", &any320));
        assert!(!q(false, 256).meets("mp3", &any320));
        assert!(q(true, 900).meets("flac", &any320), "lossless passes any lossy floor");
        assert!(
            !q(false, 0).meets("mp3", &any320),
            "unknown bitrate can't be shown to pass"
        );
        let flac_only = Minimum {
            lossless_only: true,
            kbps: 0,
            formats: vec!["flac".into()],
        };
        assert!(q(true, 900).meets("FLAC", &flac_only));
        assert!(
            !q(true, 900).meets("wav", &flac_only),
            "format list applies to lossless too"
        );
        assert!(!q(false, 320).meets(
            "flac",
            &Minimum {
                formats: vec![],
                ..flac_only
            }
        ));
    }

    #[test]
    fn a_soulseek_file_is_judged_by_extension_size_and_length() {
        assert_eq!(
            Quality::of_file("A\\B\\01 - Song.FLAC", 30_000_000, Some(240)),
            q(true, 1000)
        );
        assert_eq!(Quality::of_file("01 - Song.mp3", 9_600_000, Some(240)), q(false, 320));
    }
}
