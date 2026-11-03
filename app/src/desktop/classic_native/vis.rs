//! The main window's spectrum analyser: Winamp's own constants and behaviour (its `classic_vis.cpp`),
//! adapted from Spotifast's `src/vis.rs` (https://github.com/crmne/spotifast, commit 672d665), which
//! cross-checked them against Webamp's `VisPainter.ts` and `FFTNullsoft.ts`. MIT License,
//! Copyright (c) 2026 Carmine Paolino; the notice is in app/licenses/THIRD-PARTY.md.

use std::time::{Duration, Instant};

/// Samples that go into one spectrum (mono, -1 to 1).
pub const FFT_SAMPLES: usize = 512;
const SPECTRUM_BINS: usize = FFT_SAMPLES / 2;
/// The analyser's columns and rows in skin pixels.
const COLUMNS: usize = 75;
pub const ROWS: u8 = 16;
/// The bars, each three columns wide with one between.
pub const BARS: usize = 19;
const MAX_HEIGHT: f32 = 15.0;
/// How far a bar falls each step, and how peaks pick up speed.
const FALLOFF: f32 = 12.0 / 16.0;
const PEAK_FALLOFF: f32 = 1.1;
/// Winamp moved its analyser sixty times a second on a timer of its own, so the frame rate never
/// changes how fast the bars fall.
pub const STEP: Duration = Duration::from_micros(16_667);
/// The most steps one frame catches up on.
const CATCH_UP: u128 = 2;
/// Converts a channel mean to Winamp's channel sum.
const CHANNEL_SUM: f32 = 2.0;
/// Winamp's own scale on every magnitude.
const SPEC_SCALE: f32 = 0.5;

/// One bar, in rows from the bottom.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Bar {
    /// How tall the bar is, 0 to 15.
    pub height: u8,
    /// Where the peak mark sits, as the height of a bar whose top row it would be, 1 to 16;
    /// `None` while it is out of sight.
    pub peak: Option<u8>,
}

/// Where each bar and its peak are.
pub struct Analyser {
    fft: Fft,
    spectrum: [f32; SPECTRUM_BINS],
    wave: [f32; FFT_SAMPLES],
    falloff: [f32; BARS],
    /// Each peak, in 256ths of a row, and how fast it is dropping.
    peaks: [i32; BARS],
    peak_speed: [f32; BARS],
    last_step: Option<Instant>,
    bars: [Bar; BARS],
}

impl Default for Analyser {
    fn default() -> Self {
        Self {
            fft: Fft::new(FFT_SAMPLES),
            spectrum: [0.0; SPECTRUM_BINS],
            wave: [0.0; FFT_SAMPLES],
            falloff: [0.0; BARS],
            peaks: [0; BARS],
            peak_speed: [0.0; BARS],
            last_step: None,
            bars: [Bar::default(); BARS],
        }
    }
}

impl Analyser {
    /// One frame: the spectrum of `samples` moves the bars, which come back. A frame sooner than
    /// `STEP` after the last shows them where they were; a late one catches up.
    pub fn step(&mut self, samples: &[f32], now: Instant) -> [Bar; BARS] {
        let due = self.last_step.map_or(now, |last| last + STEP);
        if now + Duration::from_millis(1) < due {
            return self.bars;
        }
        let behind = now.saturating_duration_since(due).as_nanos() / STEP.as_nanos();
        let owed = (1 + behind).min(CATCH_UP) as u32;
        self.last_step = Some((due + STEP * (owed - 1)).max(now.checked_sub(STEP).unwrap_or(now)));
        self.wave.fill(0.0);
        for (slot, sample) in self.wave.iter_mut().zip(samples) {
            *slot = sample * CHANNEL_SUM;
        }
        self.fft.spectrum(&self.wave, &mut self.spectrum);
        let columns = bands(&self.spectrum);
        for bar in 0..BARS {
            let chunk = &columns[4 * bar..4 * bar + 4];
            // Winamp kept the target as a whole number of rows.
            let target = (chunk.iter().sum::<f32>() / 4.0).min(MAX_HEIGHT).trunc();
            let mut peak_row = 0;
            for _ in 0..owed {
                let falloff = &mut self.falloff[bar];
                *falloff = (*falloff - FALLOFF).max(target);
                let peak = &mut self.peaks[bar];
                if *peak <= (*falloff * 256.0).round() as i32 {
                    *peak = (*falloff * 256.0) as i32;
                    self.peak_speed[bar] = 3.0;
                }
                peak_row = *peak / 256;
                *peak = (*peak - self.peak_speed[bar].round() as i32).max(0);
                self.peak_speed[bar] *= PEAK_FALLOFF;
            }
            self.bars[bar] = Bar {
                height: self.falloff[bar].round() as u8,
                peak: (peak_row >= 1).then_some((peak_row + 1) as u8),
            };
        }
        self.bars
    }

    /// Whether every bar and peak has come to rest: nothing moves until there is sound again.
    pub fn settled(&self) -> bool {
        self.falloff.iter().all(|f| *f <= 0.0) && self.peaks.iter().all(|p| *p == 0)
    }
}

/// The analyser's FFT, as Winamp's source has it: 512 samples under a Hann window, 256 magnitudes
/// out, each halved.
struct Fft {
    bit_reversed: Vec<usize>,
    envelope: Vec<f32>,
    twiddles: Vec<(f32, f32)>,
    real: Vec<f32>,
    imaginary: Vec<f32>,
}

impl Fft {
    fn new(n: usize) -> Self {
        let mut bit_reversed: Vec<usize> = (0..n).collect();
        let mut j = 0;
        for i in 0..n {
            if j > i {
                bit_reversed.swap(i, j);
            }
            let mut m = n >> 1;
            while m >= 1 && j >= m {
                j -= m;
                m >>= 1;
            }
            j += m;
        }
        let envelope = (0..n)
            .map(|i| {
                let phase = i as f32 / n as f32 * std::f32::consts::TAU;
                0.5 + 0.5 * (phase - std::f32::consts::FRAC_PI_2).sin()
            })
            .collect();
        let mut twiddles = Vec::new();
        let mut size = 2;
        while size <= n {
            let theta = -std::f32::consts::TAU / size as f32;
            twiddles.push((theta.cos(), theta.sin()));
            size <<= 1;
        }
        Self {
            bit_reversed,
            envelope,
            twiddles,
            real: vec![0.0; n],
            imaginary: vec![0.0; n],
        }
    }

    fn spectrum(&mut self, wave: &[f32], out: &mut [f32]) {
        let n = self.real.len();
        for i in 0..n {
            let from = self.bit_reversed[i];
            self.real[i] = wave.get(from).copied().unwrap_or(0.0) * self.envelope[from];
            self.imaginary[i] = 0.0;
        }
        let (mut size, mut stage) = (2, 0);
        while size <= n {
            let (wpr, wpi) = self.twiddles[stage];
            let (mut wr, mut wi) = (1.0f32, 0.0f32);
            let half = size >> 1;
            for m in 0..half {
                let mut i = m;
                while i < n {
                    let j = i + half;
                    let tr = wr * self.real[j] - wi * self.imaginary[j];
                    let ti = wr * self.imaginary[j] + wi * self.real[j];
                    self.real[j] = self.real[i] - tr;
                    self.imaginary[j] = self.imaginary[i] - ti;
                    self.real[i] += tr;
                    self.imaginary[i] += ti;
                    i += size;
                }
                let previous = wr;
                wr = wr * wpr - wi * wpi;
                wi = wi * wpr + previous * wpi;
            }
            size <<= 1;
            stage += 1;
        }
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = (self.real[i] * self.real[i] + self.imaginary[i] * self.imaginary[i]).sqrt() * SPEC_SCALE;
        }
    }
}

/// Winamp's bands: seventy-five spans a semitone apart (`2^(x/12)`), each summing its share of the
/// 256 bins, the fractional edges read through a Hermite curve, the sum clipped at 255. One extra
/// silent column pads the last bar's group of four.
fn bands(spectrum: &[f32; SPECTRUM_BINS]) -> [f32; COLUMNS + 1] {
    let bla = 255.0 / 2f32.powf(75.0 / 12.0);
    let warp = |x: f32| (2f32.powf(x / 12.0) - 1.0) * bla;
    let sample = |index: usize| spectrum.get(index).copied().unwrap_or(0.0);
    let hermite = |x: f32, y0: f32, y1: f32, y2: f32, y3: f32| {
        let c1 = 0.5 * (y2 - y0);
        let c3 = 1.5 * (y1 - y2) + 0.5 * (y3 - y0);
        let c2 = y0 - y1 + c1 - c3;
        ((c3 * x + c2) * x + c1) * x + y1
    };
    let mut columns = [0.0; COLUMNS + 1];
    let mut next = warp(0.0) + 1.0;
    for (x, column) in columns.iter_mut().take(COLUMNS).enumerate() {
        let low = next;
        next = warp(x as f32 + 1.0) + 1.0;
        let mut value = 0.0f32;
        let mut bin = low.floor() as usize;
        let end = (next.floor() as usize).min(SPECTRUM_BINS - 1);
        let mut fraction = low;
        let mut mult = (bin as f32 + 1.0) - low;
        let mut herm = true;
        loop {
            if bin == end {
                mult = next - fraction;
                herm = true;
            }
            if herm {
                value += hermite(
                    fraction - bin as f32,
                    sample(bin.saturating_sub(1)),
                    sample(bin),
                    sample(bin + 1),
                    sample(bin + 2),
                ) * mult;
            } else {
                value += sample(bin);
            }
            herm = false;
            bin += 1;
            if bin > end {
                break;
            }
            fraction = bin as f32;
        }
        *column = value.min(255.0);
    }
    columns
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_loud_tone_raises_bars_and_silence_lets_them_settle() {
        let mut analyser = Analyser::default();
        let tone: Vec<f32> = (0..FFT_SAMPLES).map(|i| (i as f32 * 0.3).sin() * 0.8).collect();
        let start = Instant::now();
        let bars = analyser.step(&tone, start);
        assert!(bars.iter().any(|b| b.height > 0));
        let silence = [0.0; FFT_SAMPLES];
        let mut now = start;
        for _ in 0..600 {
            now += STEP;
            analyser.step(&silence, now);
        }
        assert!(analyser.settled());
    }
}
