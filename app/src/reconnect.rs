//! Reaching a hub that didn't answer (docs/context/desktop.md, Launch). The app keeps trying by
//! itself: after 2, 4, 8 and 16 s, then every 30 s, and at once when the network changes (a VPN
//! such as Tailscale coming up, Wi-Fi back: `platform::follow_network`) or the listener presses
//! Retry now. Each probe has a hard limit, whatever it waits on (a name lookup, a connection, TLS),
//! and every outcome goes to the unreachable page as `reconnect`, which only shows it. So being
//! reachable again never waits on a page's timer, a hidden window, or a probe that never returns.

use serde::Serialize;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

/// What the unreachable page hears, and `reconnect_status` answers.
pub const EVENT: &str = "reconnect";

/// Where an attempt stands: `trying` while a probe runs, else waiting `next_in` seconds for the
/// next. `attempt` counts from the first probe that failed; `error` is the last failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub attempt: u32,
    pub trying: bool,
    pub error: String,
    pub next_in: u64,
}

/// Why to try before the wait is over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wake {
    /// The listener pressed Retry now.
    Now,
    /// An address came or went: give name resolution a moment to follow, then try. (Desktop only:
    /// `platform::follow_network`.)
    #[cfg_attr(mobile, allow(dead_code))]
    Network,
}

enum Msg {
    Wake(Wake),
    Stop,
}

/// The waits, the hard limit on a probe, and the pause after a network change.
pub struct Timing {
    pub backoff: Vec<Duration>,
    pub limit: Duration,
    pub settle: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Timing {
            backoff: [2, 4, 8, 16, 30].map(Duration::from_secs).to_vec(),
            // a probe asks twice (/health, /api/ping), each with 10 s (hub.rs): past this it has failed
            limit: Duration::from_secs(25),
            // Tailscale's names resolve a moment after its address appears
            settle: Duration::from_millis(1500),
        }
    }
}

/// The one reconnect running, if any (managed state; the page asks `reconnect_status`).
#[derive(Default)]
pub struct Reconnect {
    run: Mutex<Option<Run>>,
    next: Mutex<u64>,
}

struct Run {
    id: u64,
    tx: Sender<Msg>,
    status: Status,
}

/// Is the hub there and does it know this device: `Ok(true)` paired, `Ok(false)` not, `Err` no answer.
pub type Probe = Arc<dyn Fn() -> Result<bool, String> + Send + Sync>;

/// Keep trying the hub that just failed with `error`, until it answers (then `reached(app,
/// paired)`) or `stop`. Replaces a reconnect already running.
pub fn start(app: &AppHandle, error: String, probe: Probe, reached: impl FnOnce(&AppHandle, bool) + Send + 'static) {
    stop(app);
    let Some(state) = app.try_state::<Reconnect>() else {
        return;
    };
    let id = {
        let mut next = state.next.lock().unwrap_or_else(PoisonError::into_inner);
        *next += 1;
        *next
    };
    let (tx, rx) = mpsc::channel();
    *state.run.lock().unwrap_or_else(PoisonError::into_inner) = Some(Run {
        id,
        tx,
        status: Status {
            attempt: 1,
            trying: false,
            error: error.clone(),
            next_in: 0,
        },
    });
    let app = app.clone();
    let spawned = std::thread::Builder::new().name("reconnect".into()).spawn(move || {
        let report = |status: Status| {
            if let Some(state) = app.try_state::<Reconnect>() {
                match state.run.lock().unwrap_or_else(PoisonError::into_inner).as_mut() {
                    Some(run) if run.id == id => run.status = status.clone(),
                    _ => return,
                }
            }
            let _ = app.emit_to("main", EVENT, status);
        };
        let outcome = keep_trying(&rx, error, &Timing::default(), &probe, &report);
        if let Some(state) = app.try_state::<Reconnect>() {
            let mut run = state.run.lock().unwrap_or_else(PoisonError::into_inner);
            if run.as_ref().is_some_and(|r| r.id == id) {
                *run = None;
            } else {
                return; // replaced meanwhile: the new one decides
            }
        }
        if let Some(paired) = outcome {
            reached(&app, paired);
        }
    });
    if let Err(e) = spawned {
        eprintln!("lull: no reconnect: {e}");
    }
}

/// One probe within the hard limit: the first, when the app opens the hub, before any page shows.
pub fn once(probe: &Probe) -> Result<bool, String> {
    within(Timing::default().limit, probe)
}

/// Try now, if a reconnect is running. False when none is.
pub fn wake(app: &AppHandle, why: Wake) -> bool {
    app.try_state::<Reconnect>().is_some_and(|state| {
        state
            .run
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .is_some_and(|run| run.tx.send(Msg::Wake(why)).is_ok())
    })
}

/// Stop trying: the window shows something else now.
pub fn stop(app: &AppHandle) {
    if let Some(state) = app.try_state::<Reconnect>()
        && let Some(run) = state.run.lock().unwrap_or_else(PoisonError::into_inner).take()
    {
        let _ = run.tx.send(Msg::Stop);
    }
}

/// The reconnect's last status, for a page that opens after it began.
#[tauri::command]
pub fn reconnect_status(app: AppHandle) -> Option<Status> {
    let state = app.try_state::<Reconnect>()?;
    let run = state.run.lock().unwrap_or_else(PoisonError::into_inner);
    run.as_ref().map(|r| r.status.clone())
}

/// The loop: wait (the backoff, or until woken), probe within the limit, say how it went; again
/// until the hub answers (`Some(paired)`) or a stop (`None`).
fn keep_trying(
    rx: &Receiver<Msg>,
    error: String,
    timing: &Timing,
    probe: &Probe,
    report: &dyn Fn(Status),
) -> Option<bool> {
    let (mut attempt, mut error) = (1u32, error);
    loop {
        let wait = timing
            .backoff
            .get((attempt as usize).saturating_sub(1))
            .or(timing.backoff.last())
            .copied()
            .unwrap_or(Duration::from_secs(30));
        report(Status {
            attempt,
            trying: false,
            error: error.clone(),
            next_in: wait.as_secs(),
        });
        match rx.recv_timeout(wait) {
            Ok(Msg::Stop) | Err(RecvTimeoutError::Disconnected) => return None,
            Ok(Msg::Wake(Wake::Network)) => {
                if matches!(
                    rx.recv_timeout(timing.settle),
                    Ok(Msg::Stop) | Err(RecvTimeoutError::Disconnected)
                ) {
                    return None;
                }
            }
            Ok(Msg::Wake(Wake::Now)) | Err(RecvTimeoutError::Timeout) => {}
        }
        // wakes that came meanwhile are this attempt's
        while let Ok(msg) = rx.try_recv() {
            if matches!(msg, Msg::Stop) {
                return None;
            }
        }
        report(Status {
            attempt,
            trying: true,
            error: error.clone(),
            next_in: 0,
        });
        let outcome = within(timing.limit, probe);
        if matches!(rx.try_recv(), Ok(Msg::Stop)) {
            return None;
        }
        match outcome {
            Ok(paired) => return Some(paired),
            Err(e) => {
                attempt += 1;
                error = e;
            }
        }
    }
}

/// The probe, given at most `limit`: one that hasn't returned by then has failed. Its thread is
/// left to end when the request does (ureq's own limits end it).
fn within(limit: Duration, probe: &Probe) -> Result<bool, String> {
    let (tx, rx) = mpsc::channel();
    let probe = probe.clone();
    std::thread::Builder::new()
        .name("probe".into())
        .spawn(move || {
            let _ = tx.send(probe());
        })
        .map_err(|e| e.to_string())?;
    match rx.recv_timeout(limit) {
        Ok(outcome) => outcome,
        Err(_) => Err(format!("The hub didn't answer within {} s", limit.as_secs())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Instant;

    fn timing() -> Timing {
        Timing {
            backoff: vec![Duration::from_millis(20), Duration::from_millis(40)],
            limit: Duration::from_millis(200),
            settle: Duration::from_millis(10),
        }
    }
    /// A probe that answers with `outcomes` in turn (the last one again after), counting its calls.
    fn probe(outcomes: Vec<Result<bool, String>>, calls: Arc<AtomicU32>) -> Probe {
        Arc::new(move || {
            let n = calls.fetch_add(1, Ordering::SeqCst) as usize;
            outcomes.get(n).or(outcomes.last()).cloned().unwrap_or(Ok(true))
        })
    }
    fn statuses() -> (Arc<Mutex<Vec<Status>>>, impl Fn(Status)) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s = seen.clone();
        (seen, move |status| {
            s.lock().unwrap_or_else(PoisonError::into_inner).push(status)
        })
    }

    #[test]
    fn a_name_that_doesnt_resolve_yet_is_tried_again_until_it_does() {
        const DNS: &str = "No such host is known. (os error 11001)";
        let dns = || Err(DNS.to_owned());
        let calls = Arc::new(AtomicU32::new(0));
        let (_tx, rx) = mpsc::channel();
        let (seen, report) = statuses();
        let reached = keep_trying(
            &rx,
            DNS.into(),
            &timing(),
            &probe(vec![dns(), dns(), Ok(true)], calls.clone()),
            &report,
        );
        assert_eq!(reached, Some(true));
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        let seen = seen.lock().unwrap_or_else(PoisonError::into_inner);
        let shown: Vec<(u32, bool)> = seen.iter().map(|s| (s.attempt, s.trying)).collect();
        assert_eq!(
            shown,
            [(1, false), (1, true), (2, false), (2, true), (3, false), (3, true)]
        );
        assert!(seen.iter().all(|s| s.error.contains("11001")), "{seen:?}");
    }

    #[test]
    fn a_probe_that_never_returns_fails_at_the_limit_and_the_next_one_runs() {
        let calls = Arc::new(AtomicU32::new(0));
        let c = calls.clone();
        let hangs: Probe = Arc::new(move || {
            if c.fetch_add(1, Ordering::SeqCst) == 0 {
                std::thread::sleep(Duration::from_secs(3600));
            }
            Ok(true)
        });
        let (_tx, rx) = mpsc::channel();
        let (seen, report) = statuses();
        let started = Instant::now();
        assert_eq!(keep_trying(&rx, "first".into(), &timing(), &hangs, &report), Some(true));
        assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
        let seen = seen.lock().unwrap_or_else(PoisonError::into_inner);
        assert!(
            seen.iter()
                .any(|s| s.attempt == 2 && !s.trying && s.error.contains("didn't answer within")),
            "{seen:?}"
        );
    }

    #[test]
    fn a_network_change_or_retry_now_tries_before_the_wait_is_over() {
        let long = Timing {
            backoff: vec![Duration::from_secs(3600)],
            ..timing()
        };
        for why in [Wake::Network, Wake::Now] {
            let calls = Arc::new(AtomicU32::new(0));
            let (tx, rx) = mpsc::channel();
            let p = probe(vec![Ok(true)], calls.clone());
            let started = Instant::now();
            let waker = std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(30));
                let _ = tx.send(Msg::Wake(why));
                tx
            });
            assert_eq!(
                keep_trying(&rx, "down".into(), &long, &p, &|_| {}),
                Some(true),
                "{why:?}"
            );
            assert!(
                started.elapsed() < Duration::from_secs(2),
                "{why:?}: {:?}",
                started.elapsed()
            );
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            drop(waker.join());
        }
    }

    #[test]
    fn a_stop_ends_it_waiting_or_probing() {
        let (tx, rx) = mpsc::channel();
        let _ = tx.send(Msg::Stop);
        let calls = Arc::new(AtomicU32::new(0));
        assert_eq!(
            keep_trying(
                &rx,
                "down".into(),
                &timing(),
                &probe(vec![Ok(true)], calls.clone()),
                &|_| {}
            ),
            None
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        // the hub answers, but the listener chose another meanwhile: not opened
        let (tx, rx) = mpsc::channel();
        let slow: Probe = Arc::new(move || {
            let _ = tx.send(Msg::Stop);
            Ok(true)
        });
        assert_eq!(keep_trying(&rx, "down".into(), &timing(), &slow, &|_| {}), None);
    }

    #[test]
    fn after_the_backoff_it_tries_every_30_seconds() {
        let t = Timing::default();
        assert_eq!(
            t.backoff.iter().map(Duration::as_secs).collect::<Vec<_>>(),
            [2, 4, 8, 16, 30]
        );
        assert!(
            t.limit > Duration::from_secs(20),
            "longer than the probe's two 10 s requests"
        );
    }
}
