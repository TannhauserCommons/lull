//! One search budget for the whole crate. The Soulseek server throttles an account that
//! searches too often (seen live on 2026-11-02: at about one search every 5 s, no peer answered
//! and peers' addresses timed out), so every network search asks here first. Searches start at
//! least 1 s apart, at most 30 in any minute, fewer for the kinds nobody waits on, and in
//! priority order; once no peer answers, the background kinds stop until one does
//! (docs/context/soulseek.md#search-budget).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Who a search is for, highest priority first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Lane {
    /// The song playing (or being saved, or whose copies the picker lists).
    Play = 0,
    /// Songs queued next (`PUT /api/stream/prefetch`).
    Prefetch = 1,
    /// The listener's own search.
    Listener = 2,
    /// Pre-warming the catalog's top results.
    Prewarm = 3,
    /// Whether peers share an album (new releases).
    Availability = 4,
}

pub const LANES: usize = 5;

impl Lane {
    fn from_u8(n: u8) -> Lane {
        match n {
            0 => Lane::Play,
            1 => Lane::Prefetch,
            2 => Lane::Listener,
            3 => Lane::Prewarm,
            _ => Lane::Availability,
        }
    }

    /// Searches started in the last `WINDOW` above which this lane waits: the gap below
    /// `MAX_PER_MINUTE` is kept for the lanes above it.
    fn ceiling(self) -> usize {
        match self {
            Lane::Play => MAX_PER_MINUTE,
            Lane::Prefetch | Lane::Listener => 24,
            Lane::Prewarm => 12,
            Lane::Availability => 6,
        }
    }

    /// Whether this lane waits while the Soulseek server isn't answering the session: its
    /// searches would only go unheard. The player and the listener always get theirs.
    fn waits_for_the_server(self) -> bool {
        !matches!(self, Lane::Play | Lane::Listener)
    }
}

/// Searches start at least this far apart.
const SPACING: Duration = Duration::from_secs(1);
/// The most searches in any `WINDOW`, all lanes together.
const MAX_PER_MINUTE: usize = 30;
const WINDOW: Duration = Duration::from_secs(60);
/// How often a waiting search looks for its turn again.
const RETRY: Duration = Duration::from_millis(200);

/// A search's lane, which a higher-priority caller joining it may raise while it waits.
#[derive(Debug, Clone)]
pub struct Priority(Arc<AtomicU8>);

impl Priority {
    pub fn new(lane: Lane) -> Self {
        Priority(Arc::new(AtomicU8::new(lane as u8)))
    }

    pub fn get(&self) -> Lane {
        Lane::from_u8(self.0.load(Ordering::Relaxed))
    }

    pub fn raise(&self, lane: Lane) {
        self.0.fetch_min(lane as u8, Ordering::Relaxed);
    }
}

#[derive(Clone, Default)]
pub struct Budget(Arc<Mutex<State>>);

#[derive(Default)]
struct State {
    /// When each search of the last `WINDOW` started, oldest first.
    started: VecDeque<Instant>,
    /// Searches waiting for their turn, by lane.
    waiting: [u32; LANES],
}

/// Takes a waiting search off the count however its wait ends.
struct Waiting<'a> {
    budget: &'a Budget,
    lane: Option<usize>,
}

impl Waiting<'_> {
    fn as_lane(&mut self, state: &mut State, lane: Lane) {
        if self.lane != Some(lane as usize) {
            if let Some(old) = self.lane {
                state.waiting[old] = state.waiting[old].saturating_sub(1);
            }
            state.waiting[lane as usize] += 1;
            self.lane = Some(lane as usize);
        }
    }

    fn done(&mut self, state: &mut State) {
        if let Some(old) = self.lane.take() {
            state.waiting[old] = state.waiting[old].saturating_sub(1);
        }
    }
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        if self.lane.is_some()
            && let Ok(mut state) = self.budget.0.lock()
        {
            self.done(&mut state);
        }
    }
}

impl Budget {
    /// Waits for this search's turn and counts it as started. `false` when its lane waits for a
    /// server that isn't `answering` (the connection's status check): don't search.
    pub async fn admit(&self, priority: &Priority, answering: impl Fn() -> bool) -> bool {
        let mut waiting = Waiting {
            budget: self,
            lane: None,
        };
        loop {
            let lane = priority.get();
            if lane.waits_for_the_server() && !answering() {
                return false;
            }
            {
                let Ok(mut state) = self.0.lock() else { return true };
                waiting.as_lane(&mut state, lane);
                if state.turn(lane, Instant::now()) {
                    waiting.done(&mut state);
                    return true;
                }
            }
            tokio::time::sleep(RETRY).await;
        }
    }
}

impl State {
    /// Starts a search of `lane` now if nothing above it waits, its ceiling has room and the
    /// last one started long enough ago.
    fn turn(&mut self, lane: Lane, now: Instant) -> bool {
        while self.started.front().is_some_and(|t| now.duration_since(*t) >= WINDOW) {
            self.started.pop_front();
        }
        let first = self.waiting[..lane as usize].iter().all(|n| *n == 0);
        let room = self.started.len() < lane.ceiling();
        let spaced = self.started.back().is_none_or(|t| now.duration_since(*t) >= SPACING);
        if first && room && spaced {
            self.started.push_back(now);
        }
        first && room && spaced
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn searches_are_spaced_capped_per_minute_and_background_kinds_get_less() {
        let t0 = Instant::now();
        let mut s = State::default();
        let mut at = t0;
        let mut started = 0;
        // Background searches as fast as they're let: availability stops at its ceiling.
        for _ in 0..20 {
            if s.turn(Lane::Availability, at) {
                started += 1;
            }
            at += Duration::from_millis(500);
        }
        assert_eq!(started, 6, "at most 6 availability checks a minute, 1 s apart");
        assert!(!s.turn(Lane::Prewarm, t0 + Duration::from_millis(5500)), "1 s apart");
        // The player still has room in the same minute, up to the total.
        let mut play = 0;
        for i in 0..60 {
            if s.turn(Lane::Play, t0 + Duration::from_secs(11 + i)) {
                play += 1;
            }
        }
        assert!(play >= 24, "the playing song gets the rest of the budget");
        assert!(s.started.len() <= MAX_PER_MINUTE);
    }

    #[test]
    fn a_waiting_higher_lane_goes_first() {
        let mut s = State::default();
        s.waiting[Lane::Play as usize] = 1;
        assert!(!s.turn(Lane::Listener, Instant::now()), "the player waits too");
        s.waiting[Lane::Play as usize] = 0;
        assert!(s.turn(Lane::Listener, Instant::now()));
    }

    #[tokio::test]
    async fn background_kinds_wait_while_the_server_is_silent_but_the_player_never_does() {
        let budget = Budget::default();
        let silent = || false;
        assert!(!budget.admit(&Priority::new(Lane::Availability), silent).await);
        assert!(!budget.admit(&Priority::new(Lane::Prewarm), silent).await);
        assert!(!budget.admit(&Priority::new(Lane::Prefetch), silent).await);
        assert!(budget.admit(&Priority::new(Lane::Play), silent).await);
        assert!(budget.admit(&Priority::new(Lane::Listener), silent).await);
        assert!(
            budget.admit(&Priority::new(Lane::Prewarm), || true).await,
            "the server answers: on they go"
        );
        let raised = Priority::new(Lane::Availability);
        raised.raise(Lane::Play);
        assert_eq!(raised.get(), Lane::Play, "the player joined it");
    }
}
