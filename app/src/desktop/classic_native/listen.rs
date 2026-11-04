//! Listening history by the web UI's rules (web/app.js `recordPlay`, `beginPlay`, `trackListening`;
//! the shared rules in rules.rs and tests/player-rules.json `listening`): a play starts at the first
//! sound of a queue entry, not at its load; the report that starts it adds nothing, nor does the
//! next (the page applies a report to the open play before starting one, and a new play has no last
//! position); from then on the position's steps add up while it plays, a step of 1.5 s or more (a
//! seek, a stall caught up) or a step back adding nothing. It ends completed when the song ended by
//! itself, skipped when the listener moved on, stopped when Classic closed. The hub turns the end
//! into history and, by Last.fm's rule, a scrobble (crates/lastfm/src/rules.rs), so the same steps
//! give the same scrobbles as the page.

use super::rules;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum How {
    Completed,
    Skipped,
    Stopped,
}

/// The play open now: which load of a queue entry it is (a replay is a new one), the seconds heard,
/// and the position last seen.
#[derive(Debug, Default)]
pub struct Listening {
    play: Option<(u64, f64, Option<f64>)>,
}

/// A play that ends: its load and the seconds heard.
#[derive(Debug, PartialEq)]
pub struct Ended {
    pub load: u64,
    pub heard: f64,
    pub how: How,
}

impl Listening {
    /// The position moved (`load` names the queue entry's load the sound is from). Returns the
    /// load whose play starts now: the first sound of a load that has no play yet. Whoever loads
    /// another entry ends the open play first, with its reason (app.js `load`).
    pub fn time(&mut self, load: u64, position: f64, playing: bool, waiting: bool) -> Option<u64> {
        if let Some((_, heard, last)) = self.play.as_mut().filter(|(l, _, _)| *l == load) {
            if let Some(was) = *last {
                *heard += rules::heard(position - was, playing);
            }
            *last = Some(position);
            return None;
        }
        (playing && !waiting).then(|| {
            self.play = Some((load, 0.0, None));
            load
        })
    }

    /// The play ends (the listener moved on, the song ended, Classic closed).
    pub fn end(&mut self, how: How) -> Option<Ended> {
        self.play.take().map(|(load, heard, _)| Ended { load, heard, how })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn plays_start_and_add_up_as_the_pages_do() -> Result<(), String> {
        let rules: Value =
            serde_json::from_str(include_str!("../../../../tests/player-rules.json")).map_err(|e| e.to_string())?;
        let cases = rules["listening"]["cases"].as_array().ok_or("no listening cases")?;
        for (i, case) in cases.iter().enumerate() {
            let reports: Vec<(f64, bool, bool)> =
                serde_json::from_value(case["reports"].clone()).map_err(|e| e.to_string())?;
            let mut l = Listening::default();
            let mut starts = -1;
            for (k, (position, playing, waiting)) in reports.into_iter().enumerate() {
                if l.time(7, position, playing, waiting).is_some() {
                    starts = k as i64;
                }
            }
            assert_eq!(Some(starts), case["starts"].as_i64(), "listening #{i} starts");
            let heard = l.end(How::Stopped).map_or(0.0, |e| e.heard);
            let expected = case["heard"].as_f64().ok_or("no heard")?;
            assert!(
                (heard - expected).abs() < 1e-9,
                "listening #{i}: heard {heard}, not {expected}"
            );
        }
        Ok(())
    }

    #[test]
    fn a_replay_is_a_new_play_and_a_new_load_ends_the_last() {
        let mut l = Listening::default();
        assert_eq!(l.time(1, 0.0, true, false), Some(1));
        l.time(1, 0.25, true, false);
        l.time(1, 0.5, true, false);
        assert_eq!(l.end(How::Completed).map(|e| e.heard), Some(0.25));
        assert_eq!(l.time(2, 0.0, true, false), Some(2));
        assert_eq!(l.end(How::Stopped).map(|e| (e.heard, e.how)), Some((0.0, How::Stopped)));
        assert_eq!(l.end(How::Stopped), None);
    }
}
