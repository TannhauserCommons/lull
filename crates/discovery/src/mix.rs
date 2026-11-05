//! Shuffling and weighted draws for picks and radio: a different order each time, no dependency
//! needed.

use std::time::{SystemTime, UNIX_EPOCH};

/// A seed that changes on every call.
pub fn seed() -> u64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9e37_79b9_7f4a_7c15);
    nanos | 1
}

/// Fisher-Yates with xorshift64: plenty for ordering songs.
pub fn shuffle<T>(items: &mut [T], mut seed: u64) {
    for i in (1..items.len()).rev() {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        items.swap(i, (seed % (i as u64 + 1)) as usize);
    }
}

/// Up to `n` of `items`, drawn without replacement with odds by weight (Efraimidis-Spirakis), in
/// the order drawn: the heaviest come most often, the lighter ones now and then. Items of no
/// positive weight are never drawn.
pub fn draw<T>(items: Vec<(T, f64)>, n: usize, mut seed: u64) -> Vec<T> {
    let mut keyed: Vec<(f64, T)> = items
        .into_iter()
        .filter(|(_, w)| *w > 0.0)
        .map(|(item, w)| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            // A uniform number in (0, 1], raised to 1/weight: larger for heavier items.
            let u = ((seed >> 11) as f64 + 1.0) / (1u64 << 53) as f64;
            (u.powf(1.0 / w), item)
        })
        .collect();
    keyed.sort_by(|a, b| b.0.total_cmp(&a.0));
    keyed.into_iter().take(n).map(|(_, item)| item).collect()
}

#[cfg(test)]
mod tests {
    use super::draw;

    #[test]
    fn a_draw_favours_the_heavy_and_still_reaches_the_light() {
        let items = || vec![("heavy", 10.0), ("middle", 3.0), ("light", 1.0), ("none", 0.0)];
        let mut first = std::collections::HashMap::new();
        let mut light = 0;
        for seed in 1..2001u64 {
            let got = draw(items(), 2, (seed * 0x9e37_79b9) | 1);
            assert_eq!(got.len(), 2);
            assert!(!got.contains(&"none"));
            *first.entry(got[0]).or_insert(0) += 1;
            light += got.contains(&"light") as i32;
        }
        assert!(first["heavy"] > first["middle"] && first["middle"] > first.get("light").copied().unwrap_or(0));
        assert!(light > 50, "the light one comes now and then ({light} of 2000)");
    }
}
