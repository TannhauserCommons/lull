//! What the app keeps while it runs: its settings, what the web UI says is playing, the tray, and
//! the hub when this computer is the hub.

use crate::desktop::classic::Held;
use crate::desktop::discord::Presence;
use crate::desktop::glass::{Grant, Material};
use crate::desktop::keyring::SystemKeyring;
use crate::desktop::local_hub::LocalHub;
use crate::desktop::open_links::Shared;
use crate::desktop::settings::{HubChoice, Settings};
use crate::desktop::theme::Package;
use crate::desktop::token::Tokens;
use crate::desktop::tray::Tray;
use crate::store::Store;
use serde::Deserialize;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Instant;
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
    /// What a see-through `main` can show here without lag (glass.rs), in no particular order.
    pub glass: Vec<Material>,
}

pub struct Desktop {
    store: Store<Settings>,
    settings: Mutex<Settings>,
    /// The system keyring: the hub's own credentials when this computer is the hub (ADR-0091).
    pub keyring: Arc<SystemKeyring>,
    /// This app's token on its hub, once that hub keeps credentials in a keyring.
    tokens: Tokens,
    /// What the web UI last reported, and when (recovery.rs works out where a playing song got to).
    now: Mutex<(NowPlaying, Instant)>,
    /// `main` is hidden and quiet while Classic mode plays (main_window::sleep).
    main_asleep: AtomicBool,
    /// `main` shows the hub's web UI (not a bundled page).
    on_hub: AtomicBool,
    /// `main` can show the desktop behind its page (glass.rs): always on Windows, on Linux only when
    /// it was made with an alpha channel.
    main_alpha: AtomicBool,
    /// What `main` shows behind its page now.
    glass: Mutex<Grant>,
    /// The listener's theme `main` shows, for the bundled pages (theme.rs). Kept for this run only.
    theme: Mutex<Option<Arc<Package>>>,
    /// Hub origins already given their IPC capability this run.
    granted: Mutex<HashSet<String>>,
    /// A shared link handed to the app, until the hub's web UI opens it (open_links.rs).
    link: Mutex<Option<Shared>>,
    pub platform: Platform,
    pub tray: OnceLock<Tray>,
    /// Set once the hub on this computer runs; it stops only when the app quits.
    local: OnceLock<LocalHub>,
    /// Held while the hub starts: a second "This computer" click waits, then opens the same hub.
    pub starting: Mutex<()>,
    /// What `main` handed the Classic window, kept current while it plays (classic.rs).
    pub classic: Mutex<Option<Held>>,
    /// Discord Rich Presence (discord.rs).
    pub discord: Presence,
}

impl Desktop {
    pub fn new(store: Store<Settings>, platform: Platform, keyring: Arc<SystemKeyring>) -> Self {
        let settings = store.load();
        Desktop {
            store,
            settings: Mutex::new(settings),
            tokens: Tokens::new(keyring.clone()),
            keyring,
            now: Mutex::new((NowPlaying::default(), Instant::now())),
            main_asleep: AtomicBool::new(false),
            on_hub: AtomicBool::new(false),
            main_alpha: AtomicBool::new(cfg!(windows)),
            glass: Mutex::new(Grant {
                material: None,
                held: None,
            }),
            theme: Mutex::new(None),
            granted: Mutex::new(HashSet::new()),
            link: Mutex::new(None),
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

    /// This app's device token on its hub (ADR-0002), wherever it is kept (token.rs).
    pub fn token(&self) -> Option<String> {
        self.tokens.get(&self.settings())
    }

    /// Keeps (or forgets) this app's token where tokens are kept now. A keyring that refuses puts it back in
    /// `settings.json`, so the app stays paired.
    pub fn set_token(&self, token: Option<String>) {
        if self.tokens.keep(&self.settings(), token.as_deref()) {
            self.update(|s| s.token = None);
        } else {
            self.update(|s| {
                s.token = token;
                s.token_in_keyring = false;
            });
        }
    }

    /// The hub turned on its `secrets` switch (its `/health` lists it): the token moves into the keyring, and
    /// out of `settings.json` once the keyring has read it back. A keyring that refuses leaves it where it was.
    pub fn keep_token_in_keyring(&self) {
        if self.tokens.take_in(&self.settings()) {
            self.update(|s| {
                s.token = None;
                s.token_in_keyring = true;
            });
        }
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
        *lock(&self.now) = (now, Instant::now());
    }

    pub fn now_playing(&self) -> NowPlaying {
        lock(&self.now).0.clone()
    }

    pub fn now_playing_at(&self) -> (NowPlaying, Instant) {
        lock(&self.now).clone()
    }

    pub fn main_asleep(&self) -> bool {
        self.main_asleep.load(Ordering::Relaxed)
    }

    pub fn set_main_asleep(&self, asleep: bool) {
        self.main_asleep.store(asleep, Ordering::Relaxed);
    }

    pub fn main_alpha(&self) -> bool {
        self.main_alpha.load(Ordering::Relaxed)
    }

    /// Linux: `main` was just made, with an alpha channel or without.
    pub fn set_main_alpha(&self, alpha: bool) {
        self.main_alpha.store(alpha, Ordering::Relaxed);
    }

    pub fn glass(&self) -> Grant {
        *lock(&self.glass)
    }

    pub fn set_glass(&self, grant: Grant) {
        *lock(&self.glass) = grant;
    }

    pub fn theme(&self) -> Option<Arc<Package>> {
        lock(&self.theme).clone()
    }

    pub fn set_theme(&self, package: Option<Arc<Package>>) {
        *lock(&self.theme) = package;
    }

    /// The hub in use and this app's token there: what Rust needs to call its API.
    pub fn hub_access(&self) -> Option<(Url, String)> {
        if let Some(local) = self.local_hub() {
            return Some((local.origin.clone(), local.token.clone()));
        }
        match (self.settings().hub, self.token()) {
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

    /// The newest link handed over wins.
    pub fn hold_link(&self, link: Shared) {
        *lock(&self.link) = Some(link);
    }

    pub fn take_link(&self) -> Option<Shared> {
        lock(&self.link).take()
    }
}

/// A poisoned lock only means another thread panicked mid-update; the data is still usable.
pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}
