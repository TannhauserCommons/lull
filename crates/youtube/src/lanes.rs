//! Who goes first for a yt-dlp lookup (docs/context/youtube.md#politeness). The listener's own
//! lookups (the playing song's, a search) go first. A lookup for later (`Youtube::find_later`:
//! warming a queued song's match, a preload's own) waits until none of the listener's is waiting
//! or running, one at a time; a `find` of the same song meanwhile raises it to the listener's.
//! On 2026-11-07 a song whose Soulseek copy was known took 16 s to start, and the next queued
//! songs' warm-ups held both lookup turns as it started.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use lull_core::ids::TrackId;
use tokio::sync::{Semaphore, SemaphorePermit, watch};

tokio::task_local! {
    /// Set while a lookup for later runs: whether it has been raised since.
    static LATER: Arc<AtomicBool>;
}

#[derive(Default)]
pub struct Lanes {
    /// The listener's own lookups waiting for a turn or running. Also changed (to the same
    /// value) to wake lookups for later that were raised.
    now: watch::Sender<usize>,
    /// Lookups for later run one at a time...
    later_turn: LaterTurn,
    /// ...and each song's is raised when that song is wanted now.
    later: Mutex<HashMap<TrackId, Arc<AtomicBool>>>,
}

struct LaterTurn(Semaphore);

impl Default for LaterTurn {
    fn default() -> Self {
        LaterTurn(Semaphore::new(1))
    }
}

/// A lookup's place, held while it runs.
pub enum Lane<'a> {
    /// The listener's own: counted while it waits and runs.
    Now(&'a watch::Sender<usize>),
    /// For later, in its one turn.
    Later { _turn: SemaphorePermit<'a> },
}

impl Drop for Lane<'_> {
    fn drop(&mut self) {
        if let Lane::Now(now) = self {
            now.send_modify(|n| *n = n.saturating_sub(1));
        }
    }
}

impl Lanes {
    /// Runs `lookup` (a `find`) as a lookup for later for `track`.
    pub async fn later<T>(&self, track: &TrackId, lookup: impl Future<Output = T>) -> T {
        let raised = match self.later.lock() {
            Ok(mut later) => later.entry(track.clone()).or_default().clone(),
            Err(_) => Arc::default(),
        };
        let out = LATER.scope(raised.clone(), lookup).await;
        if let Ok(mut later) = self.later.lock()
            && later.get(track).is_some_and(|r| Arc::ptr_eq(r, &raised))
        {
            later.remove(track);
        }
        out
    }

    /// `track` is wanted now (a `find` outside a lookup for later): its lookup for later, if one
    /// runs, goes as the listener's from here.
    pub fn wanted_now(&self, track: &TrackId) {
        if LATER.try_with(|_| ()).is_ok() {
            return;
        }
        let raised = self.later.lock().ok().and_then(|l| l.get(track).cloned());
        if let Some(r) = raised {
            r.store(true, Ordering::SeqCst);
            self.now.send_modify(|_| {});
        }
    }

    /// The place of one yt-dlp run of the task calling it: the listener's at once, or, in a
    /// lookup for later not raised, the later turn once none of the listener's waits or runs.
    pub async fn lane(&self) -> Lane<'_> {
        let now = || {
            self.now.send_modify(|n| *n += 1);
            Lane::Now(&self.now)
        };
        let Ok(raised) = LATER.try_with(Arc::clone) else {
            return now();
        };
        let is_raised = || raised.load(Ordering::SeqCst);
        let mut changes = self.now.subscribe();
        loop {
            if is_raised() {
                return now();
            }
            tokio::select! {
                turn = self.later_turn.0.acquire() => match turn {
                    Ok(turn) => {
                        let _ = changes.wait_for(|n| *n == 0 || is_raised()).await;
                        if is_raised() {
                            drop(turn);
                            return now();
                        }
                        return Lane::Later { _turn: turn };
                    }
                    // Never closed; were it, the lookup would go as the listener's.
                    Err(_) => return now(),
                },
                _ = changes.changed() => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn song(id: &str) -> TrackId {
        TrackId(id.into())
    }

    /// Whether `fut` is still waiting after a moment.
    async fn waits<F: Future>(fut: &mut std::pin::Pin<&mut F>) -> bool {
        tokio::time::timeout(Duration::from_millis(100), fut.as_mut())
            .await
            .is_err()
    }

    #[tokio::test]
    async fn a_warm_up_waits_while_the_playing_songs_lookup_waits_or_runs() {
        let lanes = Lanes::default();
        let playing = lanes.lane().await;
        let next = song("next");
        let warm = lanes.later(&next, async { lanes.lane().await.is_later() });
        tokio::pin!(warm);
        assert!(
            waits(&mut warm).await,
            "the warm-up waited for the playing song's lookup"
        );
        drop(playing);
        assert!(warm.await, "then took the later turn");
    }

    #[tokio::test]
    async fn warm_ups_go_one_at_a_time_and_never_hold_a_turn_the_listener_needs() {
        let lanes = Lanes::default();
        let (a, b) = (song("a"), song("b"));
        let first = lanes.later(&a, lanes.lane()).await;
        let second = lanes.later(&b, async { lanes.lane().await.is_later() });
        tokio::pin!(second);
        assert!(waits(&mut second).await, "one at a time");
        // The listener's lookup never waits on the lanes, whatever runs for later.
        let mine = tokio::time::timeout(Duration::from_millis(100), lanes.lane()).await;
        assert!(mine.is_ok_and(|l| !l.is_later()));
        drop(first);
        assert!(second.await);
    }

    #[tokio::test]
    async fn the_song_wanted_now_raises_its_own_warm_up() {
        let lanes = Lanes::default();
        let _busy = lanes.lane().await;
        let next = song("next");
        let warm = lanes.later(&next, async { lanes.lane().await.is_later() });
        tokio::pin!(warm);
        assert!(waits(&mut warm).await);
        // The listener plays that song: its lookup goes now, not after the others.
        lanes.wanted_now(&next);
        assert!(!warm.await, "raised to the listener's");
        lanes.wanted_now(&song("other"));
    }

    impl Lane<'_> {
        fn is_later(&self) -> bool {
            matches!(self, Lane::Later { .. })
        }
    }
}
