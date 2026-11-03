//! The device's equaliser in Classic's own player: the same settings as the web UI's (`web/eq.js`,
//! `lull:eq`: ten bands of ±12 dB, a hand-set preamp or an automatic level) and the same filters as
//! Web Audio's BiquadFilterNode, as Android's player has them (`Equaliser.kt`): a low shelf at 60 Hz,
//! peaks with Q 1 up to 14 kHz, a high shelf at 16 kHz. The page hands its settings over with the
//! queue and takes them back.

use crate::desktop::classic::Equaliser;

/// The bands' frequencies (web/eq.js BANDS).
pub const BANDS: [f64; 10] = [
    60.0, 170.0, 310.0, 600.0, 1000.0, 3000.0, 6000.0, 12000.0, 14000.0, 16000.0,
];
pub const MAX_DB: f64 = 12.0;
/// The rate eq.js draws its curve at: close enough for every rate, only the top band moves.
const CURVE_RATE: f64 = 48_000.0;
/// The automatic level stays this far under 0 dB.
const MARGIN_DB: f64 = 0.1;

impl Equaliser {
    pub fn flat() -> Self {
        Equaliser {
            on: true,
            bands: [0.0; 10],
            preamp: None,
        }
    }

    /// Anything to do: on, and not flat at 0 dB (eq.js `active`).
    fn active(&self) -> bool {
        self.on && (self.bands.iter().any(|&b| b != 0.0) || self.preamp.is_some_and(|p| p != 0.0))
    }

    /// The level before the filters: the hand-set preamp, or the curve's highest point taken back
    /// down to just under 0 dB, so a boost never clips (eq.js `level`).
    pub fn level(&self) -> f64 {
        self.preamp.unwrap_or_else(|| {
            let peak = grid().map(|hz| self.response(hz)).fold(f64::NEG_INFINITY, f64::max);
            if peak > 0.0 {
                -((peak + MARGIN_DB) * 100.0).round() / 100.0
            } else {
                0.0
            }
        })
    }

    /// The curve's gain at `hz`, in dB, as the filters draw it.
    pub fn response(&self, hz: f64) -> f64 {
        let magnitude: f64 = self
            .bands
            .iter()
            .enumerate()
            .map(|(i, &gain)| magnitude(&coefficients(i, gain, CURVE_RATE), hz, CURVE_RATE))
            .product();
        20.0 * magnitude.log10()
    }

    /// The same settings, every value inside ±12 dB at a tenth of a dB (eq.js `normalise`).
    pub fn normalised(mut self) -> Self {
        let db = |v: f64| ((v.clamp(-MAX_DB, MAX_DB) * 10.0).round() / 10.0) + 0.0;
        for band in &mut self.bands {
            *band = db(*band);
        }
        self.preamp = self.preamp.map(db);
        self
    }
}

/// How much of a Bass or Treble move each band takes (web/eq.js BASS_W, TREBLE_W).
const BASS_W: [f64; 10] = [1.0, 0.9, 0.2, 0.05, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
const TREBLE_W: [f64; 10] = [0.0, 0.0, 0.0, 0.0, 0.0, 0.3, 0.8, 0.5, 0.15, 0.75];

/// lull's presets, by name (web/eq.js PRESETS): the same list Settings › Playback offers.
pub fn presets() -> [(&'static str, [f64; 10]); 6] {
    let shape = |bass: f64, treble: f64| -> [f64; 10] {
        std::array::from_fn(|i| ((BASS_W[i] * bass + TREBLE_W[i] * treble) * 10.0).round() / 10.0 + 0.0)
    };
    [
        ("Flat", [0.0; 10]),
        ("Bass boost", shape(6.0, 0.0)),
        ("Treble boost", shape(0.0, 6.0)),
        ("Bass and treble", shape(5.0, 4.0)),
        ("Voices", [-2.0, -1.5, 0.0, 1.5, 3.0, 3.5, 2.0, 0.0, 0.0, 0.0]),
        ("Less bass", shape(-6.0, 0.0)),
    ]
}

impl Equaliser {
    /// A preset's bands, on, at the automatic level (web/eq.js `withPreset`).
    pub fn with_preset(bands: [f64; 10]) -> Self {
        Equaliser {
            on: true,
            bands,
            preamp: None,
        }
    }
}

/// 20 Hz to 20 kHz, a few points per third of an octave, and every band's own frequency.
fn grid() -> impl Iterator<Item = f64> {
    (0..=120)
        .map(|i| 20.0 * 1000f64.powf(f64::from(i) / 120.0))
        .chain(BANDS)
}

/// b0, b1, b2, a1, a2, normalised by a0: Web Audio's BiquadFilterNode with Q 1 and shelf slope 1.
fn coefficients(band: usize, gain_db: f64, rate: f64) -> [f64; 5] {
    let a = 10f64.powf(gain_db / 40.0);
    let w0 = 2.0 * std::f64::consts::PI * BANDS[band].min(rate / 2.0 - 1.0) / rate;
    let (s, c) = w0.sin_cos();
    let r = 2.0 * a.sqrt() * (s / 2.0 * std::f64::consts::SQRT_2);
    let k = match band {
        0 => [
            a * ((a + 1.0) - (a - 1.0) * c + r),
            2.0 * a * ((a - 1.0) - (a + 1.0) * c),
            a * ((a + 1.0) - (a - 1.0) * c - r),
            (a + 1.0) + (a - 1.0) * c + r,
            -2.0 * ((a - 1.0) + (a + 1.0) * c),
            (a + 1.0) + (a - 1.0) * c - r,
        ],
        9 => [
            a * ((a + 1.0) + (a - 1.0) * c + r),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * c),
            a * ((a + 1.0) + (a - 1.0) * c - r),
            (a + 1.0) - (a - 1.0) * c + r,
            2.0 * ((a - 1.0) - (a + 1.0) * c),
            (a + 1.0) - (a - 1.0) * c - r,
        ],
        _ => {
            let al = s / 2.0;
            [
                1.0 + al * a,
                -2.0 * c,
                1.0 - al * a,
                1.0 + al / a,
                -2.0 * c,
                1.0 - al / a,
            ]
        }
    };
    [k[0] / k[3], k[1] / k[3], k[2] / k[3], k[4] / k[3], k[5] / k[3]]
}

fn magnitude(k: &[f64; 5], hz: f64, rate: f64) -> f64 {
    let w = 2.0 * std::f64::consts::PI * hz / rate;
    let (s1, c1) = w.sin_cos();
    let (s2, c2) = (2.0 * w).sin_cos();
    let (br, bi) = (k[0] + k[1] * c1 + k[2] * c2, -(k[1] * s1 + k[2] * s2));
    let (ar, ai) = (1.0 + k[3] * c1 + k[4] * c2, -(k[3] * s1 + k[4] * s2));
    ((br * br + bi * bi) / (ar * ar + ai * ai)).sqrt()
}

/// The filters running on stereo sound at one rate: the level, then the ten bands in order.
pub struct Chain {
    gain: f64,
    coefficients: [[f64; 5]; 10],
    /// Per channel and band: x1, x2, y1, y2.
    memory: [[[f64; 4]; 10]; 2],
}

impl Chain {
    /// `None` when the settings change nothing: the sound passes untouched.
    pub fn new(settings: &Equaliser, rate: u32) -> Option<Self> {
        if !settings.active() {
            return None;
        }
        let rate = f64::from(rate);
        Some(Self {
            gain: 10f64.powf(settings.level() / 20.0),
            coefficients: std::array::from_fn(|i| coefficients(i, settings.bands[i], rate)),
            memory: [[[0.0; 4]; 10]; 2],
        })
    }

    pub fn process(&mut self, channel: usize, input: f32) -> f32 {
        let mut x = f64::from(input) * self.gain;
        for (k, m) in self.coefficients.iter().zip(self.memory[channel].iter_mut()) {
            let y = k[0] * x + k[1] * m[0] + k[2] * m[1] - k[3] * m[2] - k[4] * m[3];
            *m = [x, m[0], y, m[2]];
            x = y;
        }
        x.clamp(-1.0, 1.0) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flat_equaliser_does_nothing_and_a_boost_is_levelled_under_zero_db() {
        assert!(Chain::new(&Equaliser::flat(), 44_100).is_none());
        let mut bass = Equaliser::flat();
        bass.bands = [6.0, 5.4, 1.2, 0.3, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let level = bass.level();
        assert!((-7.0..=-6.0).contains(&level), "{level}");
        bass.preamp = Some(-3.0);
        assert_eq!(bass.level(), -3.0);
    }

    #[test]
    fn the_curve_peaks_at_a_boosted_bands_frequency() {
        let mut eq = Equaliser::flat();
        eq.bands[4] = 9.0;
        assert!((eq.response(1000.0) - 9.0).abs() < 0.2);
        assert!(eq.response(100.0).abs() < 1.0);
    }

    #[test]
    fn stored_values_are_kept_inside_twelve_db() {
        let mut eq = Equaliser::flat();
        eq.bands[0] = 30.0;
        eq.preamp = Some(-14.04);
        let eq = eq.normalised();
        assert_eq!((eq.bands[0], eq.preamp), (12.0, Some(-12.0)));
    }
}
