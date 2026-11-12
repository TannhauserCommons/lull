//! The hub's lane budget and the politeness count (ADR-0018). A **lane** is one request to a
//! peer that hasn't sent a byte yet: a race's lane, a song's start, a preload's ask. Together
//! they never exceed `LANES`, on the listener's line; playing songs ask before preloads. A peer
//! whose upload lull cancelled while it was sending `CANCELS_A_DAY` times today isn't raced again
//! (a peer can still start a song or be picked). Nothing here knows settings or fetches.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::oneshot;

/// Lanes at once, across every song on the hub.
pub(super) const LANES: usize = 4;
/// Cancels of an upload that was already sending, per peer per day.
pub(super) const CANCELS_A_DAY: u32 = 3;

/// Who asks for a lane; the lower goes first when lanes free up at one moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Priority {
    /// The song playing starts, or is picked.
    Start,
    /// A better copy raced for the song playing.
    Race,
    /// A preload, or a copy fetched for later.
    Later,
}

#[derive(Clone, Default)]
pub(super) struct Lanes {
    inner: Arc<Inner>,
}

#[derive(Default)]
struct Inner {
    state: Mutex<State>,
    /// Cancels of a sending upload by peer: the day (days since 1970) and how many.
    cancels: Mutex<HashMap<String, (i64, u32)>>,
}

#[derive(Default)]
struct State {
    used: usize,
    next: u64,
    waiting: Vec<(Priority, u64, oneshot::Sender<()>)>,
}

/// A lane held; given back when dropped.
pub(super) struct Permit {
    lanes: Lanes,
}

impl Drop for Permit {
    fn drop(&mut self) {
        self.lanes.release();
    }
}

/// A wait for a lane, which gives back one granted at the moment the wait is dropped.
struct Waiting {
    lanes: Lanes,
    ticket: u64,
    rx: oneshot::Receiver<()>,
}

impl Drop for Waiting {
    fn drop(&mut self) {
        self.rx.close();
        if let Ok(mut state) = self.lanes.inner.state.lock() {
            state.waiting.retain(|(_, ticket, _)| *ticket != self.ticket);
        }
        if self.rx.try_recv().is_ok() {
            self.lanes.release();
        }
    }
}

impl Lanes {
    /// A lane, waiting while all are in use. Dropping the future gives up the wait.
    pub(super) async fn acquire(&self, priority: Priority) -> Permit {
        let waiting = {
            let Ok(mut state) = self.inner.state.lock() else {
                return Permit { lanes: self.clone() };
            };
            let ticket = state.next;
            state.next += 1;
            let ahead = state.waiting.iter().any(|(p, t, _)| (*p, *t) < (priority, ticket));
            if state.used < LANES && !ahead {
                state.used += 1;
                return Permit { lanes: self.clone() };
            }
            let (tx, rx) = oneshot::channel();
            state.waiting.push((priority, ticket, tx));
            Waiting {
                lanes: self.clone(),
                ticket,
                rx,
            }
        };
        let mut waiting = waiting;
        // Granted when the sender fires; the grant already counted the lane as used.
        let _ = (&mut waiting.rx).await;
        Permit { lanes: self.clone() }
    }

    fn release(&self) {
        let Ok(mut state) = self.inner.state.lock() else {
            return;
        };
        state.used = state.used.saturating_sub(1);
        while state.used < LANES && !state.waiting.is_empty() {
            let best = state
                .waiting
                .iter()
                .enumerate()
                .min_by_key(|(_, (p, t, _))| (*p, *t))
                .map(|(i, _)| i);
            let Some(i) = best else { break };
            let (_, _, tx) = state.waiting.remove(i);
            if tx.send(()).is_ok() {
                state.used += 1;
            }
        }
    }

    /// Whether `peer` may be raced: fewer than `CANCELS_A_DAY` of its sending uploads were cancelled today.
    pub(super) fn polite(&self, peer: &str) -> bool {
        let today = today();
        self.inner
            .cancels
            .lock()
            .map(|c| c.get(peer).is_none_or(|(day, n)| *day != today || *n < CANCELS_A_DAY))
            .unwrap_or(true)
    }

    /// lull cancelled `peer`'s upload while it was sending.
    pub(super) fn cancelled(&self, peer: &str) {
        let today = today();
        if let Ok(mut cancels) = self.inner.cancels.lock() {
            let entry = cancels.entry(peer.to_owned()).or_insert((today, 0));
            if entry.0 != today {
                *entry = (today, 0);
            }
            entry.1 += 1;
        }
    }
}

fn today() -> i64 {
    lull_core::now() / 86_400
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    async fn within<T>(f: impl std::future::Future<Output = T>) -> Option<T> {
        tokio::time::timeout(Duration::from_millis(500), f).await.ok()
    }

    #[tokio::test]
    async fn four_lanes_at_once_and_the_playing_song_asks_before_a_preload() {
        let lanes = Lanes::default();
        let mut held = Vec::new();
        for _ in 0..LANES {
            held.push(lanes.acquire(Priority::Race).await);
        }
        assert!(
            within(lanes.acquire(Priority::Start)).await.is_none(),
            "a fifth lane waits"
        );
        let later = tokio::spawn({
            let lanes = lanes.clone();
            async move { lanes.acquire(Priority::Later).await }
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        let start = tokio::spawn({
            let lanes = lanes.clone();
            async move { lanes.acquire(Priority::Start).await }
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        // One lane frees: the song's start, which asked last, still goes before the preload.
        held.pop();
        let got = within(start).await;
        assert!(got.is_some(), "the playing song takes the lane first");
        assert!(!later.is_finished(), "the preload waits");
        later.abort();
    }

    #[tokio::test]
    async fn a_wait_given_up_leaves_no_lane_behind() {
        let lanes = Lanes::default();
        let held: Vec<Permit> = futures_util::future::join_all((0..LANES).map(|_| lanes.acquire(Priority::Race))).await;
        assert!(within(lanes.acquire(Priority::Race)).await.is_none());
        drop(held);
        assert!(
            within(lanes.acquire(Priority::Race)).await.is_some(),
            "the lanes are free again"
        );
    }

    #[test]
    fn a_peer_cancelled_three_times_while_sending_is_not_raced_today() {
        let lanes = Lanes::default();
        for _ in 0..CANCELS_A_DAY {
            assert!(lanes.polite("peer"));
            lanes.cancelled("peer");
        }
        assert!(!lanes.polite("peer"));
        assert!(lanes.polite("other"));
    }
}
