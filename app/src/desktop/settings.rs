//! The app's own settings, one JSON file in the app config dir. They matter before the web UI
//! loads (which hub, start in the tray, title bar), so they live here and not in the hub.
//! Launch at login is not stored: the OS autostart entry is its only source of truth.

use crate::desktop::discord::Look;
use crate::desktop::glass::Request;
use crate::desktop::graphics::Mode;
use crate::pages::Appearance;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Which hub this app uses. None until first run is answered.
    pub hub: Option<HubChoice>,
    /// This app's device token on that hub (ADR-0002). Changing the hub forgets it.
    pub token: Option<String>,
    /// Unix seconds of the last successful load of the web UI, for the unreachable page's details.
    pub last_connected: Option<u64>,
    /// Design and theme the web UI last reported, passed to the bundled pages (DESKTOP.md 6.1).
    pub appearance: Appearance,
    /// What the theme in use last asked to show behind `main` (glass.rs), so the next launch opens
    /// the window as the theme wants it. None: a solid theme.
    pub window: Option<Request>,
    /// "See-through window" (Settings › Appearance): off keeps `main` solid whatever the theme asks.
    pub glass: bool,
    pub close_to_tray: bool,
    /// The one-time "keep playing in the tray?" question was answered with "Don't ask again".
    pub close_asked: bool,
    pub start_in_tray: bool,
    /// "Keep lull running when closed" (ADR-0090; this computer is the hub): it starts at login with no window,
    /// closing hides it with or without a tray, and the computer stays awake on mains power (awake.rs).
    pub keep_running: bool,
    pub mini_style: MiniStyle,
    pub mini_on_top: bool,
    /// Linux only: draw the lull title bar instead of the native one. Needs a restart.
    pub own_titlebar: bool,
    /// Wayland ignores keep-above; the mini player says so once.
    pub wayland_hint_shown: bool,
    /// "Show what I'm playing on Discord" (discord.rs), and how it looks.
    pub discord_presence: bool,
    pub discord: Look,
    /// Settings › App › Graphics (graphics.rs): takes effect at the next start.
    pub graphics: Mode,
    /// A start in this mode showed `main` and the listener hasn't used the page yet: if the run ends
    /// so, the next start falls back to Automatic (graphics.rs).
    pub graphics_trial: Option<Mode>,
    /// The choice lull left for Automatic after a start that didn't come up; Settings says so until
    /// the listener chooses again.
    pub graphics_fell_back: Option<Mode>,
    /// The full look even where the processor draws the page (graphics.rs).
    pub full_look: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            hub: None,
            token: None,
            last_connected: None,
            appearance: Appearance::default(),
            window: None,
            glass: true,
            close_to_tray: true,
            close_asked: false,
            start_in_tray: false,
            keep_running: false,
            mini_style: MiniStyle::Device,
            mini_on_top: true,
            own_titlebar: false,
            wayland_hint_shown: false,
            discord_presence: false,
            discord: Look::default(),
            graphics: Mode::Auto,
            graphics_trial: None,
            graphics_fell_back: None,
            full_look: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "lowercase")]
pub enum HubChoice {
    /// "This computer is my hub": the app runs it (local_hub.rs).
    Local,
    /// A hub elsewhere, e.g. `http://192.168.1.20:8484` or `https://lull.example.ts.net`.
    Remote { origin: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MiniStyle {
    Device,
    Strip,
}

impl MiniStyle {
    /// Window label, which is also the bundled page's view name.
    pub fn label(self) -> &'static str {
        match self {
            MiniStyle::Device => "mini",
            MiniStyle::Strip => "strip",
        }
    }
}
