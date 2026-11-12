//! Acceptance tests for ADR-0018: the builder must not edit this file.
//!
//! Quality bands, defined once in `lull_core::quality` (R1-R4): which band a copy is in, which
//! bands a preset allows, and what counts as a better copy. A table each, one line per case.

use lull_core::presets::phone_from;
use lull_core::quality::{Band, HIGH_LOSSY_KBPS, Minimum, Quality, StreamProfile};

fn lossy(kbps: u32) -> Quality {
    Quality { lossless: false, kbps }
}

fn lossless(kbps: u32) -> Quality {
    Quality { lossless: true, kbps }
}

#[test]
fn bands_by_quality() {
    use Band::*;
    assert_eq!(HIGH_LOSSY_KBPS, 220);
    let table = [
        ("FLAC", lossless(900), Lossless),
        ("MP3 320", lossy(320), HighLossy),
        ("MP3 V0 (~245)", lossy(245), HighLossy),
        ("AAC 256", lossy(256), HighLossy),
        ("at 220", lossy(220), HighLossy),
        ("at 219", lossy(219), LowerLossy),
        ("MP3 192", lossy(192), LowerLossy),
        ("Opus 160", lossy(160), LowerLossy),
        ("MP3 128", lossy(128), LowerLossy),
        ("unknown lossy", lossy(0), LowerLossy),
    ];
    for (name, q, band) in table {
        assert_eq!(Band::of(q, false), band, "R1 {name}");
    }
}

#[test]
fn an_estimated_bitrate_counts_one_band_down() {
    use Band::*;
    let table = [
        ("320 from size and length", lossy(320), LowerLossy),
        ("128 from size and length", lossy(128), LowerLossy),
        ("FLAC: known by its format", lossless(900), Lossless),
    ];
    for (name, q, band) in table {
        assert_eq!(Band::of(q, true), band, "R2 {name}");
    }
}

#[test]
fn a_preset_allows_its_bands_highest_first() {
    use Band::*;
    let desktop = StreamProfile::default();
    let with = |change: &dyn Fn(&mut StreamProfile)| {
        let mut p = StreamProfile::default();
        change(&mut p);
        p
    };
    let formats = |f: &[&str]| f.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    let table: Vec<(&str, StreamProfile, Vec<Band>)> = vec![
        (
            "Desktop (192 minimum, no cap)",
            desktop.clone(),
            vec![Lossless, HighLossy, LowerLossy],
        ),
        (
            "Phone (320 cap, lossless off)",
            phone_from(&desktop),
            vec![HighLossy, LowerLossy],
        ),
        (
            "a 256 cap: never lossless",
            with(&|p| p.max_kbps = 256),
            vec![HighLossy, LowerLossy],
        ),
        ("a 192 cap", with(&|p| p.max_kbps = 192), vec![LowerLossy]),
        (
            "a 320 minimum",
            with(&|p| p.minimum.kbps = 320),
            vec![Lossless, HighLossy],
        ),
        (
            "a 220 minimum",
            with(&|p| p.minimum.kbps = 220),
            vec![Lossless, HighLossy],
        ),
        (
            "lossless only",
            with(&|p| p.minimum.lossless_only = true),
            vec![Lossless],
        ),
        (
            "FLAC only",
            with(&|p| p.minimum.formats = formats(&["flac"])),
            vec![Lossless],
        ),
        (
            "MP3 only",
            with(&|p| p.minimum.formats = formats(&["mp3"])),
            vec![HighLossy, LowerLossy],
        ),
        (
            "lossless only with a cap: nothing",
            with(&|p| {
                p.minimum = Minimum {
                    lossless_only: true,
                    kbps: 0,
                    formats: Vec::new(),
                };
                p.max_kbps = 320;
            }),
            vec![],
        ),
    ];
    for (name, p, bands) in table {
        assert_eq!(p.bands(), bands, "R3 {name}");
    }
}

#[test]
fn a_better_copy_is_a_higher_band() {
    let band = |q: Quality| Band::of(q, false);
    // (playing, candidate, better?)
    let table = [
        ("Opus 160 -> MP3 192", lossy(160), lossy(192), false),
        ("Opus 160 -> MP3 320", lossy(160), lossy(320), true),
        ("MP3 256 -> MP3 320 (a small step)", lossy(256), lossy(320), false),
        ("MP3 128 -> MP3 V0", lossy(128), lossy(245), true),
        ("MP3 320 -> FLAC", lossy(320), lossless(900), true),
        ("FLAC -> FLAC", lossless(900), lossless(1400), false),
        ("FLAC -> MP3 320", lossless(900), lossy(320), false),
    ];
    for (name, playing, candidate, better) in table {
        assert_eq!(band(candidate) > band(playing), better, "R4 {name}");
    }
    assert!(
        Band::of(lossy(320), true) <= band(lossy(192)),
        "R4 an estimated 320 is no better than a 192"
    );
}
