//! The app's own settings, one JSON file in the app config dir. They matter before the web UI
//! loads (which hub, start in the tray, title bar), so they live here and not in the hub.
//! Launch at login is not stored: the OS autostart entry is its only source of truth.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

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
    pub close_to_tray: bool,
    /// The one-time "keep playing in the tray?" question was answered with "Don't ask again".
    pub close_asked: bool,
    pub start_in_tray: bool,
    pub mini_style: MiniStyle,
    pub mini_on_top: bool,
    /// Linux only: draw the lull title bar instead of the native one. Needs a restart.
    pub own_titlebar: bool,
    /// Wayland ignores keep-above; the mini player says so once.
    pub wayland_hint_shown: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            hub: None,
            token: None,
            last_connected: None,
            appearance: Appearance::default(),
            close_to_tray: true,
            close_asked: false,
            start_in_tray: false,
            mini_style: MiniStyle::Device,
            mini_on_top: true,
            own_titlebar: false,
            wayland_hint_shown: false,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Appearance {
    pub design: Design,
    pub theme: Theme,
}

impl Default for Appearance {
    fn default() -> Self {
        Appearance {
            design: Design("lull".into()),
            theme: Theme::System,
        }
    }
}

/// A design name as the web UI reports it. Only its shape is checked here (it goes into a URL
/// hash); `Designs.wanted()` on the page decides whether it names a real design.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct Design(String);

impl TryFrom<String> for Design {
    type Error = String;
    fn try_from(name: String) -> Result<Self, String> {
        let shaped = !name.is_empty()
            && name.len() <= 32
            && name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if shaped {
            Ok(Design(name))
        } else {
            Err(format!("not a design name: {name:?}"))
        }
    }
}

impl Design {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Day,
    Night,
    System,
}

impl Theme {
    pub fn as_str(self) -> &'static str {
        match self {
            Theme::Day => "day",
            Theme::Night => "night",
            Theme::System => "system",
        }
    }
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

pub struct Store {
    path: PathBuf,
}

impl Store {
    pub fn new(config_dir: &Path) -> Self {
        Store {
            path: config_dir.join("settings.json"),
        }
    }

    /// A missing file is a first run. An unreadable one is reported and replaced by defaults on the
    /// next save, so a damaged file can't keep the app from starting.
    pub fn load(&self) -> Settings {
        match fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
                eprintln!("lull: ignoring {}: {e}", self.path.display());
                Settings::default()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Settings::default(),
            Err(e) => {
                eprintln!("lull: can't read {}: {e}", self.path.display());
                Settings::default()
            }
        }
    }

    /// Write to a temporary file, then rename: a crash mid-write never leaves half a file.
    pub fn save(&self, settings: &Settings) {
        let write = || -> std::io::Result<()> {
            if let Some(dir) = self.path.parent() {
                fs::create_dir_all(dir)?;
            }
            let tmp = self.path.with_extension("json.tmp");
            fs::write(&tmp, serde_json::to_vec_pretty(settings)?)?;
            fs::rename(&tmp, &self.path)
        };
        if let Err(e) = write() {
            eprintln!("lull: can't save {}: {e}", self.path.display());
        }
    }
}
