//! Listening history by the web UI's rules (web/app.js `recordPlay`, `beginPlay`, `trackListening`):
//! a play starts at the first sound of a queue entry, not at its load; the seconds heard add up from
//! the position's steps while it plays, a step of 1.5 s or more (a seek, a stall caught up) or a step
//! back adding nothing; it ends completed when the song ended by itself, skipped when the listener
//! moved on, stopped when Classic closed. The hub turns the end into history and, by Last.fm's rule,
//! a scrobble (crates/lastfm/src/rules.rs), so the same steps give the same scrobbles as the page.

/// A step of the position this long or longer isn't listening.
const GAP: f64 = 1.5;

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

/// A play that ends: its load and the whole seconds heard.
#[derive(Debug, PartialEq)]
pub struct Ended {
    pub load: u64,
    pub played: f64,
    pub how: How,
}

impl Listening {
    /// The position moved (`load` names the queue entry's load the sound is from). Returns the
    /// load whose play starts now: the first sound of a load that has no play yet. Whoever loads
    /// another entry ends the open play first, with its reason (app.js `load`).
    pub fn time(&mut self, load: u64, position: f64, playing: bool, waiting: bool) -> Option<u64> {
        let started = if self.play.as_ref().is_none_or(|(l, _, _)| *l != load) && playing && !waiting {
            self.play = Some((load, 0.0, None));
            Some(load)
        } else {
            None
        };
        if let Some((_, heard, last)) = self.play.as_mut().filter(|(l, _, _)| *l == load) {
            if let Some(was) = *last {
                let step = position - was;
                if playing && step > 0.0 && step < GAP {
                    *heard += step;
                }
            }
            *last = Some(position);
        }
        started
    }

    /// The play ends (the listener moved on, the song ended, Classic closed).
    pub fn end(&mut self, how: How) -> Option<Ended> {
        self.play.take().map(|(load, heard, _)| Ended {
            load,
            played: heard.round(),
            how,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Time events every 250 ms, as the page's output sends them.
    fn steps(l: &mut Listening, load: u64, from: f64, to: f64, playing: bool) {
        let mut at = from;
        while at <= to + 1e-9 {
            l.time(load, at, playing, false);
            at += 0.25;
        }
    }

    #[test]
    fn a_play_starts_at_the_first_sound_not_while_waiting() {
        let mut l = Listening::default();
        assert_eq!(l.time(1, 0.0, true, true), None);
        assert_eq!(l.time(1, 0.0, false, false), None);
        assert_eq!(l.time(1, 0.0, true, false), Some(1));
        assert_eq!(l.time(1, 0.25, true, false), None);
    }

    #[test]
    fn seconds_heard_leave_out_seeks_pauses_and_steps_back() {
        let mut l = Listening::default();
        steps(&mut l, 1, 0.0, 30.0, true);
        l.time(1, 90.0, true, false); // a seek ahead
        steps(&mut l, 1, 90.0, 100.0, true);
        steps(&mut l, 1, 100.0, 100.0, false); // paused
        l.time(1, 20.0, true, false); // a seek back
        steps(&mut l, 1, 20.0, 25.0, true);
        assert_eq!(
            l.end(How::Skipped),
            Some(Ended {
                load: 1,
                played: 45.0,
                how: How::Skipped
            })
        );
    }

    #[test]
    fn a_replay_is_a_new_play_and_a_new_load_ends_the_last() {
        let mut l = Listening::default();
        steps(&mut l, 1, 0.0, 200.0, true);
        assert_eq!(l.end(How::Completed).map(|e| e.played), Some(200.0));
        assert_eq!(l.time(2, 0.0, true, false), Some(2));
        assert_eq!(
            l.end(How::Stopped).map(|e| (e.played, e.how)),
            Some((0.0, How::Stopped))
        );
        assert_eq!(l.end(How::Stopped), None);
    }
}
