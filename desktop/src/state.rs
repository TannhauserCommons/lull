//! What the wrapper keeps while it runs: its settings, what the web UI says is playing, the tray.

use crate::settings::{Settings, Store};
use crate::tray::Tray;
use serde::Deserialize;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

/// `set_now_playing` from the web UI (DESKTOP.md 4): drives the tray's labels and tooltip.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
    pub playing: bool,
    pub liked: bool,
    pub saved: bool,
    /// False while the web UI shows its "Can't reach your server" banner.
    pub connected: bool,
}

/// Facts about the desktop session, read once at startup on the main thread (GTK needs it).
pub struct Platform {
    pub tray: bool,
    /// The compositor gives windows an alpha channel (mini player transparency).
    pub transparent: bool,
    /// Wayland: no keep-above, no window placement by the app.
    pub wayland: bool,
    /// GNOME's window-button order, for the optional lull title bar on Linux.
    pub button_layout: Option<String>,
}

pub struct Desktop {
    store: Store,
    settings: Mutex<Settings>,
    now: Mutex<NowPlaying>,
    /// `main` shows the server's web UI (not a bundled page).
    on_server: AtomicBool,
    /// Server origins already given their IPC capability this run.
    granted: Mutex<HashSet<String>>,
    pub platform: Platform,
    pub tray: OnceLock<Tray>,
}

impl Desktop {
    pub fn new(store: Store, platform: Platform) -> Self {
        let settings = store.load();
        Desktop {
            store,
            settings: Mutex::new(settings),
            now: Mutex::new(NowPlaying::default()),
            on_server: AtomicBool::new(false),
            granted: Mutex::new(HashSet::new()),
            platform,
            tray: OnceLock::new(),
        }
    }

    pub fn settings(&self) -> Settings {
        lock(&self.settings).clone()
    }

    /// Change and persist the settings.
    pub fn update(&self, change: impl FnOnce(&mut Settings)) {
        let mut settings = lock(&self.settings);
        change(&mut settings);
        self.store.save(&settings);
    }

    pub fn has_tray(&self) -> bool {
        self.tray.get().is_some()
    }

    pub fn on_server(&self) -> bool {
        self.on_server.load(Ordering::Relaxed)
    }

    pub fn set_on_server(&self, on: bool) {
        self.on_server.store(on, Ordering::Relaxed);
        self.refresh_tray();
    }

    pub fn set_now_playing(&self, now: NowPlaying) {
        *lock(&self.now) = now;
        self.refresh_tray();
    }

    /// True the first time an origin is seen, so its capability is added once.
    pub fn first_grant(&self, origin: &str) -> bool {
        lock(&self.granted).insert(origin.to_owned())
    }

    fn refresh_tray(&self) {
        if let Some(tray) = self.tray.get() {
            tray.show(&lock(&self.now), self.on_server());
        }
    }
}

/// A poisoned lock only means another thread panicked mid-update; the data is still usable.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}
