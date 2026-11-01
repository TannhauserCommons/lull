//! Recovering a stale Soulseek session. The server can keep a session that reports "logged in"
//! while none of its searches reach peers; a fresh login cures it. When several searches in a
//! row get no answer at all, lull asks slskd to log in again, at most every `EVERY`, and never
//! while a song is transferring (a new login drops peer connections) (docs/context/soulseek.md).

use std::time::Duration;

/// Searches in a row that no peer answered before lull acts. One silent search can be a rare
/// query; three popular-enough ones in a row are the session.
pub const UNANSWERED: usize = 3;
/// At most one fresh login this often, so a quiet network never turns into a reconnect loop.
pub const EVERY: Duration = Duration::from_secs(15 * 60);

/// Whether to log in again now: the session looks stale, the last attempt is long enough ago,
/// and nothing would be cut off.
pub fn reconnect_due(unanswered: usize, since_last: Option<Duration>, transferring: bool) -> bool {
    unanswered >= UNANSWERED && since_last.is_none_or(|d| d >= EVERY) && !transferring
}

/// "04:05 UTC" for a unix time: when the last fresh login happened, in the status line.
pub fn clock(unix: i64) -> String {
    let day = unix.rem_euclid(86_400);
    format!("{:02}:{:02} UTC", day / 3600, day % 3600 / 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: Duration = Duration::from_secs(60);

    #[test]
    fn logs_in_again_only_after_repeated_silence_and_never_in_a_loop_or_mid_song() {
        // A rare query or two that nobody answered: leave the session alone.
        assert!(!reconnect_due(UNANSWERED - 1, None, false));
        // Silence that keeps happening: act, the first time and again after the pause.
        assert!(reconnect_due(UNANSWERED, None, false));
        assert!(reconnect_due(UNANSWERED + 4, Some(EVERY), false));
        // Too soon after the last fresh login: wait, however many searches went unanswered.
        assert!(!reconnect_due(UNANSWERED + 10, Some(EVERY - MIN), false));
        // A song is transferring: a new login would cut it off.
        assert!(!reconnect_due(UNANSWERED, None, true));
        assert_eq!(clock(1_793_505_900), "04:05 UTC");
    }
}
