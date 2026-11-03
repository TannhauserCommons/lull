//! A background queue for work worth doing before anyone asks (covers the UI is about to show):
//! prioritised, one job per key, bounded, and dropping work that went stale. A crate makes one
//! with the number of jobs it runs at once (`Warmer::start`), and pushes jobs with a key and a
//! priority; a request that needs the same thing at once doesn't wait for the queue, it runs the
//! work itself and the crate's own fetch sharing (one fetch per key) makes the two meet.

use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use tokio::sync::Notify;

use crate::Ctx;

/// Most urgent first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Priority {
    /// What a device plays now and next.
    Playing = 0,
    /// What an answer just handed out (a page, a search).
    Page = 1,
    /// Everything else, while nothing more urgent waits.
    Idle = 2,
}

const LANES: usize = 3;
/// Jobs kept waiting at most; past it the oldest of the least urgent are dropped.
const CAPACITY: usize = 600;
/// A page's job not started by then is dropped: the listener has moved on.
const PAGE_STALE: Duration = Duration::from_secs(120);

type Job = Pin<Box<dyn Future<Output = ()> + Send>>;

struct Waiting {
    key: String,
    at: Instant,
    job: Job,
}

#[derive(Default)]
struct Queue {
    lanes: [VecDeque<Waiting>; LANES],
    queued: HashMap<String, Priority>,
    running: HashSet<String>,
}

impl Queue {
    fn len(&self) -> usize {
        self.lanes.iter().map(VecDeque::len).sum()
    }

    fn remove(&mut self, key: &str, priority: Priority) {
        self.lanes[priority as usize].retain(|w| w.key != key);
        self.queued.remove(key);
    }

    /// The next job, most urgent first, skipping pages gone stale.
    fn next(&mut self) -> Option<(String, Job)> {
        for (lane, waiting) in self.lanes.iter_mut().enumerate() {
            while let Some(w) = waiting.pop_front() {
                self.queued.remove(&w.key);
                if lane == Priority::Page as usize && w.at.elapsed() > PAGE_STALE {
                    continue;
                }
                self.running.insert(w.key.clone());
                return Some((w.key, w.job));
            }
        }
        None
    }
}

pub struct Warmer {
    queue: Mutex<Queue>,
    wake: Notify,
}

impl Warmer {
    /// A queue with `workers` jobs running at once, until the hub stops this set of crates.
    pub fn start(ctx: &Ctx, workers: usize) -> Arc<Warmer> {
        let warmer = Arc::new(Warmer {
            queue: Mutex::default(),
            wake: Notify::new(),
        });
        for _ in 0..workers.max(1) {
            let w = warmer.clone();
            ctx.spawn(async move {
                loop {
                    let next = w.lock().next();
                    match next {
                        Some((key, job)) => {
                            job.await;
                            w.lock().running.remove(&key);
                        }
                        None => w.wake.notified().await,
                    }
                }
            });
        }
        warmer
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Queues `job` under `key`, unless that key is running or waiting at the same or a more
    /// urgent priority; a more urgent push moves it up.
    pub fn push(&self, key: impl Into<String>, priority: Priority, job: impl Future<Output = ()> + Send + 'static) {
        let key = key.into();
        {
            let mut q = self.lock();
            if q.running.contains(&key) {
                return;
            }
            match q.queued.get(&key).copied() {
                Some(was) if was <= priority => return,
                Some(was) => q.remove(&key, was),
                None => {}
            }
            if q.len() >= CAPACITY {
                // The oldest of the least urgent goes; the newest page is what the listener sees.
                if let Some(lane) = (0..LANES).rev().find(|&l| !q.lanes[l].is_empty())
                    && let Some(old) = q.lanes[lane].pop_front()
                {
                    q.queued.remove(&old.key);
                }
            }
            q.queued.insert(key.clone(), priority);
            q.lanes[priority as usize].push_back(Waiting {
                key,
                at: Instant::now(),
                job: Box::pin(job),
            });
        }
        self.wake.notify_one();
    }

    /// Drops every job waiting at `priority`: what played next has changed.
    pub fn clear(&self, priority: Priority) {
        let mut q = self.lock();
        let gone: Vec<String> = q.lanes[priority as usize].drain(..).map(|w| w.key).collect();
        for key in gone {
            q.queued.remove(&key);
        }
    }

    /// Jobs waiting, for tests and status.
    pub fn waiting(&self) -> usize {
        self.lock().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn order(q: &mut Queue) -> Vec<String> {
        std::iter::from_fn(|| q.next().map(|(k, _)| k)).collect()
    }

    fn warmer() -> Warmer {
        Warmer {
            queue: Mutex::default(),
            wake: Notify::new(),
        }
    }

    #[test]
    fn the_most_urgent_runs_first_and_a_key_waits_once() {
        let w = warmer();
        w.push("idle", Priority::Idle, async {});
        w.push("page", Priority::Page, async {});
        w.push("page", Priority::Page, async {});
        w.push("idle", Priority::Playing, async {});
        assert_eq!(w.waiting(), 2, "one job per key");
        assert_eq!(order(&mut w.lock()), ["idle", "page"], "moved up to playing");
    }

    #[test]
    fn a_running_key_is_not_queued_again_and_the_queue_stays_bounded() {
        let w = warmer();
        w.push("a", Priority::Page, async {});
        assert!(w.lock().next().is_some());
        w.push("a", Priority::Playing, async {});
        assert_eq!(w.waiting(), 0, "already being fetched");
        for i in 0..CAPACITY + 50 {
            w.push(format!("idle-{i}"), Priority::Idle, async {});
        }
        w.push("now", Priority::Playing, async {});
        assert_eq!(w.waiting(), CAPACITY);
        assert_eq!(w.lock().next().map(|(k, _)| k).as_deref(), Some("now"));
    }

    #[test]
    fn what_played_next_can_be_dropped() {
        let w = warmer();
        w.push("next", Priority::Playing, async {});
        w.push("page", Priority::Page, async {});
        w.clear(Priority::Playing);
        assert_eq!(order(&mut w.lock()), ["page"]);
        w.push("next", Priority::Playing, async {});
        assert_eq!(w.waiting(), 1, "a cleared key can come back");
    }
}
