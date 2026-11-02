//! Last.fm's scrobbling rules (https://www.last.fm/api/scrobbling#when-is-a-scrobble-a-scrobble):
//! a track counts when it is longer than 30 seconds and was played for at least half its length
//! or for 4 minutes, whichever comes first.

pub const MIN_LENGTH_S: u32 = 30;
const ENOUGH_S: u32 = 240;

/// Whether a play of `played_s` seconds of a `duration_s`-second track is a scrobble.
/// With the length unknown (0), only the 4-minute arm can be judged.
pub fn scrobbles(duration_s: u32, played_s: u32) -> bool {
    if duration_s == 0 {
        return played_s >= ENOUGH_S;
    }
    duration_s > MIN_LENGTH_S && (played_s.saturating_mul(2) >= duration_s || played_s >= ENOUGH_S)
}

#[cfg(test)]
mod tests {
    use super::scrobbles;

    #[test]
    fn half_or_four_minutes_of_a_track_over_thirty_seconds() {
        for (duration, played, expected) in [
            (30, 30, false), // not longer than 30 s
            (31, 16, true),  // half of 31 s, rounded up
            (31, 15, false),
            (200, 99, false),
            (200, 100, true), // half
            (600, 240, true), // 4 minutes before half
            (600, 239, false),
            (0, 239, false), // unknown length: only 4 minutes counts
            (0, 240, true),
        ] {
            assert_eq!(scrobbles(duration, played), expected, "{played} s of {duration} s");
        }
    }
}
