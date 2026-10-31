//! Shuffling for picks and radio: a different order each time, no dependency needed.

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
