//! Classic's queue, kept by the web UI's rules (web/app.js, the player's queue): the same entries
//! (a song, its group, a picked source, its place before shuffling), the same moves at a song's end
//! (`nextIndex`), the same Previous, shuffle as an order that keeps "next up" first, repeat off, all
//! or one, and the same radio top-up. The page hands its queue over in its own saved shape and takes
//! it back in that shape (`P.adopt`), so the queue never forks: these rules mirror app.js line for
//! line, and the tests pin the cases app.js decides.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A picked copy of a song on a Soulseek peer (`?peer=&path=`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Source {
    pub peer: String,
    pub path: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Repeat {
    #[default]
    Off,
    All,
    One,
}

impl Repeat {
    /// The repeat button: off, all, one, off (app.js `cycleRepeat`).
    pub fn cycled(self) -> Self {
        match self {
            Repeat::Off => Repeat::All,
            Repeat::All => Repeat::One,
            Repeat::One => Repeat::Off,
        }
    }
}

/// One place in the queue (app.js `entry`): `group` is `context`, `next` (the listener's "next
/// up") or `radio`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub track_id: String,
    pub group: String,
    #[serde(default)]
    pub source: Option<Source>,
    /// Its place before shuffling, for turning shuffle off.
    #[serde(default)]
    pub orig: Option<usize>,
}

/// The radio that keeps the queue going (app.js `ps.radio`): what seeds it and how far it strays.
/// Its other fields (its name, an error) pass through as the page keeps them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Radio {
    pub seed: Seed,
    #[serde(default)]
    pub keeps_adding: bool,
    #[serde(default = "default_steer")]
    pub steer: f64,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

fn default_steer() -> f64 {
    STEER
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Seed {
    pub kind: String,
    pub id: String,
}

/// How far a radio seeded by the queue's end strays from its seed (app.js).
const STEER: f64 = 0.4;
/// Radio keeps at least this many songs after the playing one.
const RADIO_AHEAD: usize = 3;

/// The queue and its modes, as the page saves them (`persist`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Queue {
    pub queue: Vec<Entry>,
    pub index: usize,
    pub shuffle: bool,
    pub repeat: Repeat,
    #[serde(default)]
    pub radio: Option<Radio>,
    /// Where the listening started (a page of the web UI): the page's own, passed through.
    #[serde(default)]
    pub context: Value,
}

/// What Previous does (app.js `P.prev`).
#[derive(Debug, PartialEq)]
pub enum Previous {
    /// Back to the song's start.
    Restart,
    Load(usize),
}

impl Queue {
    /// The place the queue moves to next: the same song for repeat one when it ended by itself,
    /// the next entry, the first again with repeat all, or none (app.js `nextIndex`).
    pub fn next_index(&self, by_itself: bool) -> Option<usize> {
        if by_itself && self.repeat == Repeat::One {
            return Some(self.index);
        }
        if self.index + 1 < self.queue.len() {
            return Some(self.index + 1);
        }
        (self.repeat == Repeat::All && !self.queue.is_empty()).then_some(0)
    }

    /// Previous: past 3 s, or on the first song, the song starts again; else the one before.
    pub fn previous(&self, position: f64) -> Previous {
        if position > 3.0 || self.index == 0 {
            Previous::Restart
        } else {
            Previous::Load(self.index - 1)
        }
    }

    /// Shuffle on: the songs after "next up" shuffle, each keeping its place to come back to; off:
    /// they go back in order, songs added since after them (app.js `setShuffle`).
    pub fn set_shuffle(&mut self, on: bool, random: &mut impl FnMut(usize) -> usize) {
        if on == self.shuffle {
            return;
        }
        self.shuffle = on;
        let from = self.shuffle_from();
        if on {
            for (i, entry) in self.queue.iter_mut().enumerate() {
                entry.orig = Some(i);
            }
            let rest = &mut self.queue[from..];
            for i in (1..rest.len()).rev() {
                rest.swap(i, random(i + 1));
            }
        } else {
            self.queue[from..].sort_by_key(|e| e.orig.unwrap_or(usize::MAX));
        }
    }

    /// Where shuffling starts: after the playing song and the listener's "next up".
    fn shuffle_from(&self) -> usize {
        let mut at = self.index + 1;
        while self.queue.get(at).is_some_and(|e| e.group == "next") {
            at += 1;
        }
        at.min(self.queue.len())
    }

    /// Whether radio should add songs now, and from what: with "radio when the queue ends" on,
    /// repeat off and nothing after the playing song, the playing song seeds a radio; a radio that
    /// keeps adding keeps three songs ahead (app.js `topUpRadio`). The ids already queued are left
    /// out of what it adds.
    pub fn radio_wanted(
        &mut self,
        radio_when_queue_ends: bool,
        name: impl FnOnce(&str) -> Option<String>,
    ) -> Option<RadioAsk> {
        let left = self.queue.len().saturating_sub(self.index + 1);
        if self.radio.is_none() && radio_when_queue_ends && self.repeat == Repeat::Off && left == 0 {
            let last = self.queue.get(self.index)?.track_id.clone();
            self.radio = Some(Radio {
                name: name(&last),
                seed: Seed {
                    kind: "track".into(),
                    id: last,
                },
                keeps_adding: true,
                steer: STEER,
                error: None,
            });
        }
        let radio = self.radio.as_ref().filter(|r| r.keeps_adding && r.error.is_none())?;
        (left < RADIO_AHEAD).then(|| RadioAsk {
            seed: radio.seed.clone(),
            steer: radio.steer,
            exclude: self.queue.iter().map(|e| e.track_id.clone()).collect(),
        })
    }

    /// Radio's answer: the songs not queued yet go on the end, in the radio group. How many.
    pub fn add_radio(&mut self, ids: &[String]) -> usize {
        let fresh: Vec<String> = ids
            .iter()
            .filter(|id| !self.queue.iter().any(|e| &e.track_id == *id))
            .cloned()
            .collect();
        if let Some(radio) = self.radio.as_mut() {
            radio.error = None;
        }
        self.queue.extend(fresh.iter().map(|id| Entry {
            track_id: id.clone(),
            group: "radio".into(),
            source: None,
            orig: None,
        }));
        fresh.len()
    }

    /// Radio couldn't answer: it says so and waits (the page tries again once the hub answers).
    pub fn radio_failed(&mut self, why: String) {
        if let Some(radio) = self.radio.as_mut() {
            radio.error = Some(why);
        }
    }

    /// Takes songs out (the playlist's REM, Delete): `keep` says, for each, whether it stays. The
    /// playing song always does.
    pub fn keep(&mut self, keep: &[bool]) {
        let stays = |i: usize| i == self.index || keep.get(i).copied().unwrap_or(true);
        let index = (0..self.index).filter(|&i| stays(i)).count();
        let mut i = 0;
        let queue = std::mem::take(&mut self.queue);
        self.queue = queue
            .into_iter()
            .filter(|_| {
                i += 1;
                stays(i - 1)
            })
            .collect();
        self.index = index;
    }

    /// Moves the songs at `rows` (in order) to stand before `to` (the playlist's drag).
    pub fn reorder(&mut self, rows: &[usize], to: usize) {
        let mut moving = Vec::new();
        let mut rest = Vec::new();
        for (i, entry) in std::mem::take(&mut self.queue).into_iter().enumerate() {
            if rows.contains(&i) {
                moving.push((i == self.index, entry));
            } else {
                rest.push((i == self.index, entry));
            }
        }
        let before = rows.iter().filter(|&&r| r < to).count();
        let at = to.saturating_sub(before).min(rest.len());
        rest.splice(at..at, moving);
        self.index = rest.iter().position(|(p, _)| *p).unwrap_or(0);
        self.queue = rest.into_iter().map(|(_, e)| e).collect();
    }
}

/// What to ask radio for (`GET /api/radio`).
#[derive(Debug, PartialEq)]
pub struct RadioAsk {
    pub seed: Seed,
    pub steer: f64,
    pub exclude: Vec<String>,
}

impl RadioAsk {
    /// The request's path and query, as app.js `fetchRadio` writes it.
    pub fn path(&self) -> String {
        let exclude = if self.exclude.is_empty() {
            String::new()
        } else {
            let ids: Vec<String> = self.exclude.iter().map(|id| encode(id)).collect();
            format!("&exclude={}", ids.join(","))
        };
        format!(
            "/api/radio?kind={}&id={}&steer={:.2}{exclude}",
            self.seed.kind,
            encode(&self.seed.id),
            self.steer
        )
    }
}

/// `encodeURIComponent`.
pub fn encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queue(groups: &[&str], index: usize, repeat: Repeat) -> Queue {
        Queue {
            queue: groups
                .iter()
                .enumerate()
                .map(|(i, g)| Entry {
                    track_id: format!("t{i}"),
                    group: (*g).into(),
                    source: None,
                    orig: None,
                })
                .collect(),
            index,
            shuffle: false,
            repeat,
            radio: None,
            context: Value::Null,
        }
    }

    #[test]
    fn the_queue_moves_on_as_the_web_player_does() {
        let q = queue(&["context"; 3], 1, Repeat::Off);
        assert_eq!((q.next_index(true), q.next_index(false)), (Some(2), Some(2)));
        let last = queue(&["context"; 3], 2, Repeat::Off);
        assert_eq!(last.next_index(true), None);
        let all = queue(&["context"; 3], 2, Repeat::All);
        assert_eq!(all.next_index(false), Some(0));
        let one = queue(&["context"; 3], 1, Repeat::One);
        assert_eq!((one.next_index(true), one.next_index(false)), (Some(1), Some(2)));
    }

    #[test]
    fn previous_restarts_past_three_seconds_and_on_the_first_song() {
        let q = queue(&["context"; 3], 1, Repeat::Off);
        assert_eq!(
            (q.previous(3.5), q.previous(2.0)),
            (Previous::Restart, Previous::Load(0))
        );
        assert_eq!(queue(&["context"; 3], 0, Repeat::Off).previous(1.0), Previous::Restart);
    }

    #[test]
    fn shuffle_keeps_next_up_first_and_comes_back_in_order() {
        let mut q = queue(&["context", "next", "context", "context", "context"], 0, Repeat::Off);
        let mut draws = [0usize, 0, 0].into_iter();
        q.set_shuffle(true, &mut |_| draws.next().unwrap_or(0));
        assert_eq!(q.queue[1].track_id, "t1");
        assert_ne!(
            q.queue.iter().map(|e| e.track_id.as_str()).collect::<Vec<_>>(),
            ["t0", "t1", "t2", "t3", "t4"]
        );
        q.set_shuffle(false, &mut |_| 0);
        let order: Vec<&str> = q.queue.iter().map(|e| e.track_id.as_str()).collect();
        assert_eq!(order, ["t0", "t1", "t2", "t3", "t4"]);
    }

    #[test]
    fn the_last_song_seeds_a_radio_that_keeps_three_ahead() {
        let mut q = queue(&["context"; 2], 1, Repeat::Off);
        let ask = q.radio_wanted(true, |_| Some("Song".into()));
        assert_eq!(
            ask.as_ref().map(|a| a.path()),
            Some("/api/radio?kind=track&id=t1&steer=0.40&exclude=t0,t1".into())
        );
        assert_eq!(q.add_radio(&["t1".into(), "r1".into(), "r2".into()]), 2);
        assert_eq!(q.queue.last().map(|e| e.group.as_str()), Some("radio"));
        assert!(
            q.radio_wanted(true, |_| None).is_some(),
            "two ahead is fewer than three"
        );
        q.add_radio(&["r3".into()]);
        assert!(q.radio_wanted(true, |_| None).is_none());
        let mut repeating = queue(&["context"; 2], 1, Repeat::All);
        assert!(repeating.radio_wanted(true, |_| None).is_none());
    }

    #[test]
    fn songs_taken_out_and_moved_keep_the_playing_one_playing() {
        let mut q = queue(&["context"; 5], 2, Repeat::Off);
        q.keep(&[false, true, false, false, true]);
        let order: Vec<&str> = q.queue.iter().map(|e| e.track_id.as_str()).collect();
        assert_eq!((order, q.index), (vec!["t1", "t2", "t4"], 1));
        q.reorder(&[2], 0);
        let order: Vec<&str> = q.queue.iter().map(|e| e.track_id.as_str()).collect();
        assert_eq!((order, q.index), (vec!["t4", "t1", "t2"], 2));
    }
}
