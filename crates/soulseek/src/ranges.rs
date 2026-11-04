//! Which bytes of a file have arrived: a set of half-open byte ranges, kept sorted and merged.
//! A song's copy arrives as legs that start where the listener seeks (ADR-0014), so what is on
//! disk is no longer one run from byte 0; the file's length says nothing about it.

use serde::{Deserialize, Serialize};

/// Sorted, disjoint, non-touching `[start, end)` ranges.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ranges(Vec<(u64, u64)>);

impl Ranges {
    /// Everything from byte 0 to `end`: what a file written front to back holds.
    pub fn front(end: u64) -> Self {
        let mut r = Ranges::default();
        r.add(0, end);
        r
    }

    /// Adds `[start, end)`, merging with what it overlaps or touches.
    pub fn add(&mut self, start: u64, end: u64) {
        if end <= start {
            return;
        }
        let (mut start, mut end) = (start, end);
        self.0.retain(|&(a, b)| {
            let apart = b < start || a > end;
            if !apart {
                start = start.min(a);
                end = end.max(b);
            }
            apart
        });
        let at = self.0.partition_point(|&(a, _)| a < start);
        self.0.insert(at, (start, end));
    }

    /// The end of the range holding `pos`: how far a reader at `pos` can read now.
    pub fn end_from(&self, pos: u64) -> Option<u64> {
        self.0.iter().find(|&&(a, b)| a <= pos && pos < b).map(|&(_, b)| b)
    }

    pub fn has(&self, pos: u64) -> bool {
        self.end_from(pos).is_some()
    }

    /// Bytes arrived in all.
    pub fn total(&self) -> u64 {
        self.0.iter().map(|(a, b)| b - a).sum()
    }

    /// The first byte at or after `pos` that hasn't arrived, below `size`.
    pub fn next_missing(&self, pos: u64, size: u64) -> Option<u64> {
        let at = self.end_from(pos).unwrap_or(pos);
        (at < size).then_some(at)
    }

    /// The ranges as fractions of `size`, for the player's seek bar.
    pub fn fractions(&self, size: u64) -> Vec<[f64; 2]> {
        let size = size.max(1) as f64;
        self.0
            .iter()
            .map(|&(a, b)| [a as f64 / size, b as f64 / size])
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn of(spans: &[(u64, u64)]) -> Ranges {
        let mut r = Ranges::default();
        for &(a, b) in spans {
            r.add(a, b);
        }
        r
    }

    #[test]
    fn ranges_merge_when_they_overlap_or_touch_and_stay_apart_otherwise() {
        assert_eq!(of(&[(0, 10), (20, 30)]), Ranges(vec![(0, 10), (20, 30)]));
        assert_eq!(of(&[(20, 30), (0, 10), (10, 20)]), Ranges(vec![(0, 30)]));
        assert_eq!(of(&[(0, 10), (5, 25), (40, 50), (24, 41)]), Ranges(vec![(0, 50)]));
        assert_eq!(of(&[(5, 5), (7, 3)]), Ranges::default());
        assert_eq!(of(&[(0, 10), (20, 30)]).total(), 20);
    }

    #[test]
    fn a_reader_reads_to_the_end_of_its_range_and_waits_in_a_gap() {
        let r = of(&[(0, 100), (500, 800)]);
        assert_eq!(r.end_from(0), Some(100));
        assert_eq!(r.end_from(99), Some(100));
        assert_eq!(r.end_from(100), None, "the gap starts at the range's end");
        assert_eq!(r.end_from(600), Some(800));
        assert_eq!(r.fractions(1000), vec![[0.0, 0.1], [0.5, 0.8]]);
    }

    #[test]
    fn the_next_leg_starts_at_the_first_byte_missing_and_none_is_left_once_all_is_here() {
        let r = of(&[(0, 100), (500, 800)]);
        assert_eq!(r.next_missing(0, 1000), Some(100));
        assert_eq!(r.next_missing(600, 1000), Some(800));
        assert_eq!(of(&[(200, 1000)]).next_missing(0, 1000), Some(0));
        assert_eq!(of(&[(0, 600), (600, 1000)]).next_missing(0, 1000), None);
    }
}
