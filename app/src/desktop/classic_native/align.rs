//! Lining a better copy up with the song playing, as the web player does before it switches: this
//! is web/align.js in Rust, step for step, and tests/align.json holds the cases both are tested
//! against (here and in tests/web/align-cases.test.mjs). Two recordings of the same moment, the
//! song and the copy, are compared over every lag within a limit, coarse at about 3 kHz (the box
//! average is its low-pass: encoders and masters differ most in the highs, never in when a note
//! is), then to the sample around the best one, with the parabola through the top three for a
//! fraction of a sample. Copies that don't clearly line up at one lag are never switched.

/// The coarse search's rate.
const COARSE_RATE: f64 = 3000.0;
/// Seconds around the best lag that belong to its peak (a bass note's period).
const GUARD: f64 = 0.03;
/// RMS below which a recording says nothing.
const SILENT: f64 = 1e-4;
/// What `locked` asks of a measurement: alike (`similar`), and clearly at one lag.
const CLEAR: f64 = 0.1;
const ALIKE: f64 = 0.45;

/// The song and the copy are recorded this long; the copy is searched this far either way.
pub const RECORD_S: f64 = 2.5;
pub const WIDE_S: f64 = 1.0;

/// How the copy lines up with the song.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Measure {
    /// Seconds the copy sounds later than the song (negative: earlier), to a fraction of a sample.
    pub lag: f64,
    /// Normalised correlation at that lag below ~1.5 kHz, 0-1.
    pub confidence: f64,
    /// How far that beats the best other lag more than 30 ms away.
    pub margin: f64,
    /// Too little sound in either to tell.
    pub quiet: bool,
}

pub fn rms(x: &[f32]) -> f64 {
    if x.is_empty() {
        return 0.0;
    }
    (x.iter().map(|v| f64::from(*v) * f64::from(*v)).sum::<f64>() / x.len() as f64).sqrt()
}

/// Box-averaged down by `f`, mean removed.
fn decimate(x: &[f32], f: usize) -> Vec<f64> {
    let n = x.len() / f;
    let mut out: Vec<f64> = (0..n)
        .map(|i| x[i * f..(i + 1) * f].iter().map(|v| f64::from(*v)).sum::<f64>() / f as f64)
        .collect();
    let mean = out.iter().sum::<f64>() / n.max(1) as f64;
    for v in &mut out {
        *v -= mean;
    }
    out
}

/// Prefix sums of squares.
fn squares(x: &[f64]) -> Vec<f64> {
    let mut p = Vec::with_capacity(x.len() + 1);
    p.push(0.0);
    for v in x {
        let last = p[p.len() - 1];
        p.push(last + v * v);
    }
    p
}

/// Normalised correlation of b[n] with a[n - k] over their overlap.
fn ncc(a: &[f64], b: &[f64], pa: &[f64], pb: &[f64], k: i64) -> f64 {
    let n = a.len().min(b.len()) as i64;
    let (from, to) = (k.max(0), n.min(n + k));
    if to - from < 2 {
        return 0.0;
    }
    let s: f64 = (from..to).map(|i| a[(i - k) as usize] * b[i as usize]).sum();
    let ea = pa[(to - k) as usize] - pa[(from - k) as usize];
    let eb = pb[to as usize] - pb[from as usize];
    if ea > 0.0 && eb > 0.0 {
        s / (ea * eb).sqrt()
    } else {
        0.0
    }
}

/// How much later `b` (the copy) sounds than `a` (the song), both recorded at `rate` over the same
/// moment, looking within `max_lag` seconds either way.
pub fn measure(a: &[f32], b: &[f32], rate: u32, max_lag: f64) -> Measure {
    let none = Measure {
        lag: 0.0,
        confidence: 0.0,
        margin: 0.0,
        quiet: true,
    };
    let n = a.len().min(b.len());
    if n == 0 || rms(a) < SILENT || rms(b) < SILENT {
        return none;
    }
    let rate = f64::from(rate);
    // Wide: every lag within max_lag (at most half the recording, so every overlap is long enough).
    let f = ((rate / COARSE_RATE).round() as usize).max(1);
    let (ca, cb) = (decimate(a, f), decimate(b, f));
    let k_max = ((max_lag * rate / f as f64).round() as i64).min((ca.len() / 2) as i64);
    let (cpa, cpb) = (squares(&ca), squares(&cb));
    let c: Vec<f64> = (-k_max..=k_max).map(|k| ncc(&ca, &cb, &cpa, &cpb, k)).collect();
    let mut best = -k_max;
    for k in -k_max..=k_max {
        if c[(k + k_max) as usize] > c[(best + k_max) as usize] {
            best = k;
        }
    }
    let guard = (GUARD * rate / f as f64).ceil() as i64;
    let other = (-k_max..=k_max)
        .filter(|k| (k - best).abs() > guard)
        .fold(0.0, |m: f64, k| m.max(c[(k + k_max) as usize]));
    // Fine: every sample around it at the full rate, then the parabola through the top three.
    let fa: Vec<f64> = a[..n].iter().map(|v| f64::from(*v)).collect();
    let fb: Vec<f64> = b[..n].iter().map(|v| f64::from(*v)).collect();
    let (pa, pb) = (squares(&fa), squares(&fb));
    let at = best * f as i64;
    let (mut top, mut peak) = (f64::NEG_INFINITY, at);
    for k in at - 2 * f as i64..=at + 2 * f as i64 {
        let v = ncc(&fa, &fb, &pa, &pb, k);
        if v > top {
            top = v;
            peak = k;
        }
    }
    let l = ncc(&fa, &fb, &pa, &pb, peak - 1);
    let r = ncc(&fa, &fb, &pa, &pb, peak + 1);
    let bend = l - 2.0 * top + r;
    let shift = if bend < 0.0 {
        (0.5 * (l - r) / bend).clamp(-0.5, 0.5)
    } else {
        0.0
    };
    let confidence = c[(best + k_max) as usize];
    Measure {
        lag: (peak as f64 + shift) / rate,
        confidence: confidence.max(0.0),
        margin: (confidence - other).max(0.0),
        quiet: false,
    }
}

/// The copies sound alike at the lag measured.
pub fn similar(m: &Measure) -> bool {
    !m.quiet && m.confidence >= ALIKE
}

/// Good enough to switch on: the copies clearly line up at one lag, and no other.
pub fn locked(m: &Measure) -> bool {
    similar(m) && m.margin >= CLEAR
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    /// tests/web/align-cases.mjs's generator: mulberry32, a smoothed random walk.
    fn walk(seed: u32, n: usize) -> Vec<f64> {
        let mut state = seed;
        let mut random = || {
            state = state.wrapping_add(0x6d2b_79f5);
            let mut t = (state ^ (state >> 15)).wrapping_mul(1 | state);
            t = t.wrapping_add((t ^ (t >> 7)).wrapping_mul(61 | t)) ^ t;
            f64::from(t ^ (t >> 14)) / 4_294_967_296.0
        };
        let mut y = 0.0;
        (0..n)
            .map(|_| {
                y = 0.95 * y + 0.05 * (random() * 2.0 - 1.0);
                y * 4.0
            })
            .collect()
    }

    /// The song and the copy of a case (`recordings` there).
    fn recordings(c: &Value) -> Option<(Vec<f32>, Vec<f32>, u32)> {
        let rate = c["rate"].as_f64()?;
        let n = (c["seconds"].as_f64()? * rate).round() as usize;
        let d = (c["delay"].as_f64()? * rate).round() as i64;
        let pad = (2.0 * rate).round() as usize;
        let seed = c["seed"].as_u64()? as u32;
        let (song, other) = (walk(seed, n + 2 * pad), walk(seed + 1, n + 2 * pad));
        let (gain, noise, another) = (c["gain"].as_f64()?, c["noise"].as_f64()?, c["other"].as_bool()?);
        let a = (0..n).map(|i| song[pad + i] as f32).collect();
        let b = (0..n)
            .map(|i| {
                let source = if another {
                    other[pad + i]
                } else {
                    song[(pad as i64 + i as i64 - d) as usize]
                };
                (gain * source + noise * other[pad + i]) as f32
            })
            .collect();
        Some((a, b, rate as u32))
    }

    #[test]
    fn copies_line_up_as_the_pages_do() -> Result<(), String> {
        let json: Value =
            serde_json::from_str(include_str!("../../../../tests/align.json")).map_err(|e| e.to_string())?;
        for c in json["cases"].as_array().ok_or("no cases")? {
            let what = c["what"].as_str().unwrap_or_default();
            let (a, b, rate) = recordings(c).ok_or_else(|| format!("{what}: an unreadable case"))?;
            let m = measure(&a, &b, rate, 1.0);
            let e = &c["expect"];
            let near = |x: f64, k: &str| e[k].as_f64().is_some_and(|y| (x - y).abs() < 1e-9);
            assert!(near(m.lag, "lag"), "{what}: lag {}", m.lag);
            assert!(near(m.confidence, "confidence"), "{what}: confidence {}", m.confidence);
            assert!(near(m.margin, "margin"), "{what}: margin {}", m.margin);
            assert_eq!(Some(m.quiet), e["quiet"].as_bool(), "{what}: quiet");
            assert_eq!(Some(similar(&m)), e["similar"].as_bool(), "{what}: similar");
            assert_eq!(Some(locked(&m)), e["locked"].as_bool(), "{what}: locked");
            let levels = [rms(&a), rms(&b)];
            for (i, level) in levels.into_iter().enumerate() {
                assert!(
                    e["levels"][i].as_f64().is_some_and(|y| (level - y).abs() < 1e-9),
                    "{what}: level {i} {level}"
                );
            }
        }
        Ok(())
    }
}
