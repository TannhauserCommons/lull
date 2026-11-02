//! What the app keeps while it runs: its settings, what the web UI says is playing, the tray, and
//! the hub when this computer is the hub.

use crate::desktop::classic::Handoff;
use crate::desktop::discord::Presence;
use crate::desktop::local_hub::LocalHub;
use crate::desktop::settings::{HubChoice, Settings};
use crate::desktop::tray::Tray;
use crate::store::Store;
use serde::Deserialize;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use tauri::Url;

/// `set_now_playing` from the web UI (DESKTOP.md 4): drives the tray, the taskbar's buttons and
/// Discord (player.rs). Idle: every text is empty.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
    pub playing: bool,
    pub liked: bool,
    pub saved: bool,
    /// False while the web UI shows its "Can't reach your server" banner.
    pub connected: bool,
    /// The song's id on the hub, its artist's and its album's (their share links), and its album.
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub artist_id: String,
    #[serde(default)]
    pub album_id: String,
    #[serde(default)]
    pub album: String,
    /// Seconds into the song when this was sent, and its length (0 while unknown).
    #[serde(default)]
    pub position: f64,
    #[serde(default)]
    pub duration: f64,
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
    store: Store<Settings>,
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
    /// What `main` hands the Classic window, until its page takes it (classic.rs).
    pub classic: Mutex<Option<Handoff>>,
    /// Discord Rich Presence (discord.rs).
    pub discord: Presence,
}

impl Desktop {
    pub fn new(store: Store<Settings>, platform: Platform) -> Self {
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
            classic: Mutex::new(None),
            discord: Presence::start(),
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

    /// Callers then call `player::refresh`, which shows the change.
    pub fn set_on_hub(&self, on: bool) {
        self.on_hub.store(on, Ordering::Relaxed);
    }

    /// Callers then call `player::refresh`, which shows the change.
    pub fn set_now_playing(&self, now: NowPlaying) {
        *lock(&self.now) = now;
    }

    pub fn now_playing(&self) -> NowPlaying {
        lock(&self.now).clone()
    }

    /// The hub in use and this app's token there: what Rust needs to call its API.
    pub fn hub_access(&self) -> Option<(Url, String)> {
        if let Some(local) = self.local_hub() {
            return Some((local.origin.clone(), local.token.clone()));
        }
        let settings = self.settings();
        match (settings.hub, settings.token) {
            (Some(HubChoice::Remote { origin }), Some(token)) => Url::parse(&origin).ok().map(|o| (o, token)),
            _ => None,
        }
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

    /// A hub origin the app opened this run (and granted IPC): its pages belong in lull's windows.
    pub fn is_hub(&self, origin: &str) -> bool {
        lock(&self.granted).contains(origin)
    }
}

/// A poisoned lock only means another thread panicked mid-update; the data is still usable.
pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}
