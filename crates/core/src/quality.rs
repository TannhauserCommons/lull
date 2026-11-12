//! One rule for audio quality, and the listener's source rules for streaming and saving
//! (ADR-0008). Soulseek ranks and gates sources with these; library decides upgrades with them;
//! collection stores the two profiles and publishes them in the `settings` event.

use serde::{Deserialize, Serialize};

use crate::model::{FileFacts, SourceRef};

/// How good a file is, as far as lull can tell. Ordered lossless first, then by bitrate; use
/// `beats` to decide an upgrade, since a few kbps more is no better.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
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

    /// Whether a file with this quality is within a streaming cap of `max_kbps` (0 = no cap).
    /// Lossless is above any cap; a lossy file of unknown bitrate counts as within it, since
    /// lossy files are small either way.
    pub fn within(&self, max_kbps: u32) -> bool {
        max_kbps == 0 || (!self.lossless && self.kbps <= max_kbps)
    }

    /// Whether a file with this quality and `format` (lowercase extension) passes `min`.
    /// A lossy file of unknown bitrate fails any bitrate floor: it can't be shown to pass.
    pub fn meets(&self, format: &str, min: &Minimum) -> bool {
        min.allows(format) && (self.lossless || (!min.lossless_only && self.kbps >= min.kbps))
    }
}

/// The lowest quality a profile accepts.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Minimum {
    pub lossless_only: bool,
    /// Lossy floor in kbps; lossless always passes it.
    pub kbps: u32,
    /// Allowed formats (lowercase extensions); empty means any.
    #[serde(default)]
    pub formats: Vec<String>,
}

impl Minimum {
    /// Whether `format` (`format_of` an extension) is one the listener left on: any, with none
    /// named. Streaming holds to it whatever `below` says (`policy::allowed` in soulseek).
    pub fn allows(&self, format: &str) -> bool {
        self.formats.is_empty() || self.formats.iter().any(|f| f.eq_ignore_ascii_case(format))
    }

    /// The formats left on, for messages: "MP3 or OGG".
    pub fn formats_named(&self) -> String {
        self.formats.join(" or ").to_ascii_uppercase()
    }
}

/// "320 kbps or lossless", "lossless only, FLAC": for messages that name the listener's rule.
impl std::fmt::Display for Minimum {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.lossless_only, self.kbps) {
            (true, _) => f.write_str("lossless only")?,
            (false, 0) => f.write_str("any bitrate")?,
            (false, k) => write!(f, "{k} kbps or lossless")?,
        }
        if !self.formats.is_empty() {
            write!(f, ", {}", self.formats_named())?;
        }
        Ok(())
    }
}

/// Which passing copy to take first. Either way a copy that stalls or never starts falls back
/// to the next one in the same order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Pick {
    /// The best-scoring copy among those that pass (speed, free slot, queue, then quality).
    Fastest,
    /// The highest quality among those that pass, speed breaking ties.
    Highest,
}

/// Streaming when no copy passes the minimum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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

/// A source preset's profile (ADR-0017): each device streams by the one its preset holds. Fetches
/// of one song for different profiles are kept apart, so it is a key (`Hash`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamProfile {
    pub minimum: Minimum,
    pub pick: Pick,
    pub below: StreamBelow,
    /// The highest lossy bitrate worth streaming, in kbps; 0 = no cap. Copies within it are
    /// taken before copies above it, and lossless counts as above any cap. Streaming only:
    /// saving keeps its own profile.
    #[serde(default)]
    pub max_kbps: u32,
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
            max_kbps: 0,
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

/// The kbps from which a lossy copy is in the high band: V0 counts with 320 (ADR-0018).
pub const HIGH_LOSSY_KBPS: u32 = 220;

/// A coarse quality band; a higher band is a better copy, a copy in the same band is not
/// (ADR-0018). A race tries the highest allowed band first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Band {
    LowerLossy,
    HighLossy,
    Lossless,
}

impl Band {
    /// The band of a copy. A bitrate lull only estimated (size / length) counts one band down;
    /// lossless is known by its format and never moves.
    pub fn of(q: Quality, estimated: bool) -> Band {
        if q.lossless {
            Band::Lossless
        } else if q.kbps >= HIGH_LOSSY_KBPS && !estimated {
            Band::HighLossy
        } else {
            Band::LowerLossy
        }
    }
}

impl StreamProfile {
    /// The bands this profile streams, highest first: with a cap lossless is never one, a band
    /// needs a format the profile leaves on, and the lower lossy band only when the minimum is
    /// under `HIGH_LOSSY_KBPS`.
    pub fn bands(&self) -> Vec<Band> {
        const LOSSLESS: [&str; 3] = ["flac", "wav", "aiff"];
        const LOSSY: [&str; 5] = ["m4a", "aac", "mp3", "ogg", "opus"];
        let min = &self.minimum;
        let any = |formats: &[&str]| formats.iter().any(|f| min.allows(f));
        let uncapped = self.max_kbps == 0;
        let mut bands = Vec::new();
        if uncapped && any(&LOSSLESS) {
            bands.push(Band::Lossless);
        }
        if !min.lossless_only && any(&LOSSY) {
            if uncapped || self.max_kbps >= HIGH_LOSSY_KBPS {
                bands.push(Band::HighLossy);
            }
            if min.kbps < HIGH_LOSSY_KBPS {
                bands.push(Band::LowerLossy);
            }
        }
        bands
    }
}

/// The copy the saving profile picks for a song (`Transfers::save_candidate`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveCandidate {
    pub source: SourceRef,
    pub quality: Quality,
    /// Whether it passes the saving minimum; when not, the profile's `below` decides.
    pub meets: bool,
}

/// What a minimum's `formats` may name: the formats lull can play (`.aif` counts as aiff).
pub const FORMATS: [&str; 8] = ["flac", "wav", "aiff", "m4a", "aac", "mp3", "ogg", "opus"];

pub fn is_lossless(codec_or_ext: &str) -> bool {
    matches!(
        codec_or_ext,
        "flac" | "alac" | "wav" | "aiff" | "aif" | "ape" | "wavpack" | "wv" | "tta"
    )
}

/// The format a minimum's `formats` names for a lowercase extension: `.aif` is AIFF too, and a
/// `.webm` is YouTube's Opus (`youtube::Format::Opus`), so Opus switched on takes it.
pub fn format_of(ext: &str) -> &str {
    match ext {
        "aif" => "aiff",
        "webm" => "opus",
        other => other,
    }
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
        assert_eq!(any320.to_string(), "320 kbps or lossless");
        let aiff = Minimum {
            lossless_only: true,
            kbps: 0,
            formats: vec!["aiff".into()],
        };
        assert!(q(true, 1400).meets(format_of("aif"), &aiff), ".aif is AIFF");
        let opus = Minimum {
            lossless_only: false,
            kbps: 128,
            formats: vec!["mp3".into(), "opus".into()],
        };
        assert!(q(false, 160).meets(format_of("webm"), &opus), "YouTube's .webm is Opus");
        assert_eq!(flac_only.to_string(), "lossless only, FLAC");
        assert!(!q(false, 320).meets(
            "flac",
            &Minimum {
                formats: vec![],
                ..flac_only
            }
        ));
    }

    #[test]
    fn a_streaming_cap_puts_lossless_and_higher_bitrates_above_it() -> serde_json::Result<()> {
        assert!(q(true, 900).within(0), "no cap");
        assert!(!q(true, 900).within(320), "lossless is above any cap");
        assert!(q(false, 256).within(256));
        assert!(!q(false, 320).within(256));
        assert!(q(false, 0).within(128), "unknown lossy bitrate counts as within");
        let stored: StreamProfile = serde_json::from_str(
            r#"{"minimum":{"losslessOnly":false,"kbps":192},"pick":"fastest","below":"playBest"}"#,
        )?;
        assert_eq!(stored.max_kbps, 0, "profiles stored before the cap have none");
        Ok(())
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
