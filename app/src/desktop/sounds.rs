//! What the web UI's sounds need from the app (ADR-0081): whether one of lull's windows is in front,
//! and whether the system asks for quiet. `main`'s page plays every sound, the mini player's presses
//! and Classic mode's included, while `main` itself may be hidden, so it can't tell either from its
//! own focus; only `main` hears the changes.

use crate::desktop::main_window::{self, report};
use crate::desktop::platform;
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

/// How long the focus must stay put before it counts: moving from `main` to the mini player is one
/// window losing it and another taking it, which must not read as lull leaving the front.
const SETTLE: Duration = Duration::from_millis(150);

struct Sounds {
    /// A lull window gained or lost the focus: the front thread looks again once it settles.
    focus: Sender<()>,
    /// The system asks for quiet, as last heard.
    quiet: AtomicBool,
}

/// Follows both for the life of the app. On the main thread (GTK's settings live there).
pub fn start(app: &AppHandle) {
    let (focus, moves) = mpsc::channel();
    app.manage(Sounds {
        focus,
        quiet: AtomicBool::new(false),
    });
    follow_front(app.clone(), moves);
    let handle = app.clone();
    platform::follow_quiet(move |quiet| {
        let was = handle.state::<Sounds>().quiet.swap(quiet, Ordering::Relaxed);
        if was != quiet {
            report(handle.emit_to(main_window::LABEL, "sound-quiet", json!({ "quiet": quiet })));
        }
    });
}

/// One of lull's windows has the focus now (`window_front`). Never on the main thread.
pub fn front(app: &AppHandle) -> bool {
    platform::lull_in_front(app)
}

/// The system asks for quiet (`sound_quiet`).
pub fn quiet(app: &AppHandle) -> bool {
    app.state::<Sounds>().quiet.load(Ordering::Relaxed)
}

/// Any of lull's windows gained or lost the focus (a Tauri window's `Focused`).
pub fn focus_moved(app: &AppHandle) {
    if let Some(sounds) = app.try_state::<Sounds>() {
        let _ = sounds.focus.send(());
    }
}

/// Once the focus has settled after a move, says `window-front { front }` to `main` if lull came to
/// the front or left it.
fn follow_front(app: AppHandle, moves: Receiver<()>) {
    let spawned = std::thread::Builder::new().name("front".into()).spawn(move || {
        let mut front = false;
        while moves.recv().is_ok() {
            while moves.recv_timeout(SETTLE).is_ok() {}
            let now = platform::lull_in_front(&app);
            if now != front {
                front = now;
                report(app.emit_to(main_window::LABEL, "window-front", json!({ "front": now })));
            }
        }
    });
    if let Err(e) = spawned {
        eprintln!("lull: not following which window is in front: {e}");
    }
}
