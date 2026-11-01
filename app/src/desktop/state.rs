//! What the app keeps while it runs: its settings, what the web UI says is playing, the tray, and
//! the hub when this computer is the hub.

use crate::desktop::local_hub::LocalHub;
use crate::desktop::settings::{Settings, Store};
use crate::desktop::tray::Tray;
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
    /// `main` shows the hub's web UI (not a bundled page).
    on_hub: AtomicBool,
    /// Hub origins already given their IPC capability this run.
    granted: Mutex<HashSet<String>>,
    pub platform: Platform,
    pub tray: OnceLock<Tray>,
    /// Set once the hub on this computer runs; it stops only when the app quits.
    local: OnceLock<LocalHub>,
    /// Held while the hub starts: a second "This computer" click waits, then opens the same hub.
    pub starting: Mutex<()>,
}

impl Desktop {
    pub fn new(store: Store, platform: Platform) -> Self {
        let settings = store.load();
        Desktop {
            store,
            settings: Mutex::new(settings),
            now: Mutex::new(NowPlaying::default()),
            on_hub: AtomicBool::new(false),
            granted: Mutex::new(HashSet::new()),
            platform,
            tray: OnceLock::new(),
            local: OnceLock::new(),
            starting: Mutex::new(()),
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

    pub fn on_hub(&self) -> bool {
        self.on_hub.load(Ordering::Relaxed)
    }

    pub fn set_on_hub(&self, on: bool) {
        self.on_hub.store(on, Ordering::Relaxed);
        self.refresh_tray();
    }

    pub fn set_now_playing(&self, now: NowPlaying) {
        *lock(&self.now) = now;
        self.refresh_tray();
    }

    pub fn local_hub(&self) -> Option<&LocalHub> {
        self.local.get()
    }

    /// Callers hold `starting`, so only one hub is ever started.
    pub fn set_local_hub(&self, hub: LocalHub) {
        let _ = self.local.set(hub);
    }

    /// True the first time an origin is seen, so its capability is added once.
    pub fn first_grant(&self, origin: &str) -> bool {
        lock(&self.granted).insert(origin.to_owned())
    }

    fn refresh_tray(&self) {
        if let Some(tray) = self.tray.get() {
            tray.show(&lock(&self.now), self.on_hub());
        }
    }
}

/// A poisoned lock only means another thread panicked mid-update; the data is still usable.
pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}
