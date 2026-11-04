//! Lining a better copy up with the song playing, as the web player does before it switches
//! (web/app.js, the better copy; streaming.md#better-copies): another rip of the same song may start
//! a little earlier or later, so the copy is placed by correlation: 2.5 s of what plays against the
//! copy around the same moment, within a second either way. Coarse at a low rate first, then to the
//! sample. Copies that don't clearly line up are never switched.

/// What plays is compared over this long, and the copy searched this far either way.
pub const MEASURE_S: f64 = 2.5;
pub const SEARCH_S: f64 = 1.0;
/// The coarse pass works at the rate divided by this.
const DECIMATE: usize = 12;
/// Below this normalised correlation the copies aren't the same recording here.
const MATCH: f64 = 0.7;

/// How many frames later the copy sounds than what plays: `copy[i + lag] ~ playing[i]`, or `None`
/// when they don't line up. `playing` is mono at `rate`; `copy` is mono at `rate` starting
/// `SEARCH_S` before `playing` does, and running `SEARCH_S` past its end.
pub fn lag(playing: &[f32], copy: &[f32], rate: u32) -> Option<i64> {
    let search = (SEARCH_S * f64::from(rate)) as usize;
    if playing.is_empty() || copy.len() < playing.len() + 2 * search {
        return None;
    }
    let small_playing = decimate(playing);
    let small_copy = decimate(copy);
    let small_search = search / DECIMATE;
    let (coarse, _) = best(&small_playing, &small_copy, 0..=2 * small_search)?;
    let around = coarse * DECIMATE;
    let from = around.saturating_sub(DECIMATE);
    let to = (around + DECIMATE).min(2 * search);
    let (fine, score) = best(playing, copy, from..=to)?;
    (score >= MATCH).then_some(fine as i64 - search as i64)
}

/// The offset into `copy` where `playing` fits best, and how well (normalised correlation).
fn best(playing: &[f32], copy: &[f32], offsets: std::ops::RangeInclusive<usize>) -> Option<(usize, f64)> {
    let energy = |s: &[f32]| s.iter().map(|x| f64::from(*x) * f64::from(*x)).sum::<f64>();
    let own = energy(playing);
    if own <= f64::EPSILON {
        return None;
    }
    offsets
        .filter(|&o| o + playing.len() <= copy.len())
        .map(|o| {
            let window = &copy[o..o + playing.len()];
            let dot: f64 = playing
                .iter()
                .zip(window)
                .map(|(a, b)| f64::from(*a) * f64::from(*b))
                .sum();
            let theirs = energy(window);
            let score = if theirs <= f64::EPSILON {
                0.0
            } else {
                dot / (own * theirs).sqrt()
            };
            (o, score)
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
}

/// Averaged down by `DECIMATE`: enough to find the neighbourhood, a twelfth of the work squared.
fn decimate(samples: &[f32]) -> Vec<f32> {
    samples
        .chunks(DECIMATE)
        .map(|c| c.iter().sum::<f32>() / c.len() as f32)
        .collect()
}

/// A quiet moment to switch in, as the page waits for one: the level now at most 0.6 of the median
/// of the last ones, once there are at least 8 of them.
pub fn quiet(levels: &[f32]) -> bool {
    if levels.len() < 8 {
        return false;
    }
    let mut sorted = levels.to_vec();
    sorted.sort_by(f32::total_cmp);
    let median = sorted[sorted.len() / 2];
    levels.last().is_some_and(|now| *now <= 0.6 * median)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Noise that sounds like music to a correlation: no two stretches alike.
    fn noise(n: usize, seed: u32) -> Vec<f32> {
        let mut x = seed;
        (0..n)
            .map(|i| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                ((x as f32 / u32::MAX as f32) - 0.5) * (1.0 + (i as f32 / 300.0).sin())
            })
            .collect()
    }

    #[test]
    fn a_copy_a_little_later_is_found_to_the_sample() {
        let rate = 8_000;
        let song = noise(rate as usize * 6, 7);
        let start = rate as usize * 2;
        let playing = &song[start..start + (MEASURE_S * f64::from(rate)) as usize];
        // The copy starts 0.3 s and 7 samples later than the song.
        let shift = (0.3 * f64::from(rate)) as usize + 7;
        let mut copy = vec![0.0; shift];
        copy.extend_from_slice(&song);
        let search = (SEARCH_S * f64::from(rate)) as usize;
        let around = &copy[start - search..start + playing.len() + search];
        assert_eq!(lag(playing, around, rate), Some(shift as i64));
    }

    #[test]
    fn another_song_is_never_lined_up() {
        let rate = 8_000;
        let playing = noise(20_000, 3);
        let other = noise(40_000, 99);
        assert_eq!(lag(&playing, &other, rate), None);
    }

    #[test]
    fn a_quiet_moment_is_well_under_the_usual_level() {
        assert!(!quiet(&[1.0; 7]));
        let mut levels = vec![1.0; 9];
        assert!(!quiet(&levels));
        levels.push(0.5);
        assert!(quiet(&levels));
    }
}
