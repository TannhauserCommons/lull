//! The commands the pages call (DESKTOP.md 4). Arguments are checked here, at the boundary; anything
//! unknown is refused with a message the caller logs.

use crate::desktop::classic::{self, Handoff};
use crate::desktop::discord::{Presence, StatusLine, WhenPaused};
use crate::desktop::local_hub::{self, PORT};
use crate::desktop::main_window;
use crate::desktop::mini;
use crate::desktop::platform;
use crate::desktop::player;
use crate::desktop::recovery::{self, Resume};
use crate::desktop::settings::{HubChoice, MiniStyle};
use crate::desktop::state::{Desktop, NowPlaying};
use crate::desktop::tailscale::{self, Status};
use crate::desktop::updater;
use crate::hub;
use crate::pages;
use crate::updates::Report;
use serde::{Deserialize, Serialize};
use tauri::async_runtime::spawn_blocking;
use tauri::{AppHandle, Manager};
use tauri_plugin_autostart::ManagerExt;

#[derive(Debug, Deserialize)]
#[serde(
    tag = "action",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum WindowAction {
    /// Mini player: open the full window.
    Full,
    /// Mini player: close it (the full window comes back).
    MiniClose,
    /// Web UI: Ctrl+Shift+M or the player bar's Device view button.
    Mini,
    /// Welcome page: this computer is the hub.
    ThisComputer,
    /// Connect page: pair with the hub at `url` using a pair code from one of its devices.
    Pair { url: String, code: String },
    /// Welcome page: pair with a hub elsewhere.
    Connect,
    /// Unreachable page or Settings > App: back to the welcome page to pick a hub.
    ChangeServer,
    /// Unreachable or hub-error page, or "back" on the welcome page: open the chosen hub again.
    Retry,
    /// The "keep playing in the tray?" dialog.
    CloseChoice { keep: bool, remember: bool },
    /// Web UI without Media Session (some WebKitGTK builds): grab the media keys instead.
    MediaKeys,
    /// The Classic window's page handed the queue back to `main`: close it, wake `main`.
    ClassicDone,
}

#[tauri::command]
pub async fn window_action(app: AppHandle, request: WindowAction) -> Result<(), String> {
    let appearance = app.state::<Desktop>().settings().appearance;
    let done = match request {
        WindowAction::Full | WindowAction::MiniClose => mini::close(&app),
        WindowAction::Mini => mini::open(&app),
        WindowAction::ThisComputer => {
            choose(&app, HubChoice::Local, None);
            main_window::open_chosen(&app)
        }
        WindowAction::Pair { url, code } => return pair(&app, &url, code).await,
        WindowAction::Connect => main_window::open_page(&app, pages::connect(None, false, &appearance)),
        WindowAction::ChangeServer => main_window::open_page(&app, pages::welcome(&appearance)),
        WindowAction::Retry => {
            recovery::forget(&app, main_window::LABEL);
            main_window::open_chosen(&app)
        }
        WindowAction::CloseChoice { keep, remember } => main_window::close_choice(&app, keep, remember),
        WindowAction::MediaKeys => return platform::media_keys(&app),
        WindowAction::ClassicDone => classic::done(&app),
    };
    done.map_err(|e| e.to_string())
}

async fn pair(app: &AppHandle, url: &str, code: String) -> Result<(), String> {
    let origin = hub::origin(url)?;
    let o = origin.clone();
    let token = spawn_blocking(move || hub::pair(&o, &code, &hub::device_name()))
        .await
        .map_err(|e| e.to_string())??;
    let was_hub = app.state::<Desktop>().local_hub().is_some();
    choose(
        app,
        HubChoice::Remote {
            origin: origin.to_string(),
        },
        Some(token.clone()),
    );
    if was_hub {
        // The hub on this computer can't be stopped under its open connections: start afresh.
        app.restart();
    }
    main_window::open_hub(app, &origin, &token).map_err(|e| e.to_string())
}

/// Stores the hub choice. A different hub means a different device: its token goes with it.
fn choose(app: &AppHandle, choice: HubChoice, token: Option<String>) {
    app.state::<Desktop>().update(|s| {
        if s.hub.as_ref() != Some(&choice) || token.is_some() {
            s.token = token;
        }
        s.hub = Some(choice);
    });
}

/// The web UI opens Classic mode: Webamp in its own window takes over the queue (classic.rs).
/// Async: a window built from a sync command, which runs on the UI thread, deadlocks on Windows.
#[tauri::command]
pub async fn classic_open(app: AppHandle, handoff: Handoff) -> Result<(), String> {
    classic::open(&app, handoff)
}

/// The Classic window's page, once it runs: the queue to play. Null if it was taken already.
#[tauri::command]
pub fn classic_handoff(app: AppHandle) -> Option<Handoff> {
    classic::take_handoff(&app)
}

#[tauri::command]
pub fn set_now_playing(app: AppHandle, now: NowPlaying) {
    app.state::<Desktop>().set_now_playing(now);
    player::refresh(&app);
}

/// Settings > App inside the app (DESKTOP.md 7).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopSettings {
    /// The hub's address: `http://127.0.0.1:8484` when this computer is the hub.
    server: Option<String>,
    /// `local` (this computer is the hub) or `remote`; null before first run is answered.
    hub: Option<&'static str>,
    /// Without a tray, Close always quits and "Close to tray" / "Start in the tray" don't apply.
    tray: bool,
    close_to_tray: bool,
    launch_at_login: bool,
    start_in_tray: bool,
    mini_style: MiniStyle,
    mini_on_top: bool,
    /// None where the setting doesn't exist (only Linux has it).
    own_titlebar: Option<bool>,
    /// None while this build has no Discord application (discord.rs), and so is its look:
    /// the member list's words, the album cover, the buttons, and what a pause does.
    discord_presence: Option<bool>,
    discord_status: Option<StatusLine>,
    discord_cover: Option<bool>,
    discord_buttons: Option<bool>,
    discord_paused: Option<WhenPaused>,
}

#[tauri::command]
pub fn desktop_settings(app: AppHandle) -> Result<DesktopSettings, String> {
    let desktop = app.state::<Desktop>();
    let s = desktop.settings();
    let (server, hub) = match &s.hub {
        Some(HubChoice::Local) => (Some(format!("http://127.0.0.1:{PORT}/")), Some("local")),
        Some(HubChoice::Remote { origin }) => (Some(origin.clone()), Some("remote")),
        None => (None, None),
    };
    Ok(DesktopSettings {
        server,
        hub,
        tray: desktop.has_tray(),
        close_to_tray: s.close_to_tray,
        launch_at_login: app.autolaunch().is_enabled().map_err(|e| e.to_string())?,
        start_in_tray: s.start_in_tray,
        mini_style: s.mini_style,
        mini_on_top: s.mini_on_top,
        own_titlebar: cfg!(target_os = "linux").then_some(s.own_titlebar),
        discord_presence: Presence::available().then_some(s.discord_presence),
        discord_status: Presence::available().then_some(s.discord.status),
        discord_cover: Presence::available().then_some(s.discord.cover),
        discord_buttons: Presence::available().then_some(s.discord.buttons),
        discord_paused: Presence::available().then_some(s.discord.paused),
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsPatch {
    close_to_tray: Option<bool>,
    launch_at_login: Option<bool>,
    start_in_tray: Option<bool>,
    mini_style: Option<MiniStyle>,
    mini_on_top: Option<bool>,
    own_titlebar: Option<bool>,
    discord_presence: Option<bool>,
    discord_status: Option<StatusLine>,
    discord_cover: Option<bool>,
    discord_buttons: Option<bool>,
    discord_paused: Option<WhenPaused>,
}

impl SettingsPatch {
    fn touches_discord(&self) -> bool {
        self.discord_presence.is_some()
            || self.discord_status.is_some()
            || self.discord_cover.is_some()
            || self.discord_buttons.is_some()
            || self.discord_paused.is_some()
    }
}

#[tauri::command]
pub fn set_desktop_settings(app: AppHandle, patch: SettingsPatch) -> Result<(), String> {
    if let Some(on) = patch.launch_at_login {
        let autolaunch = app.autolaunch();
        if on { autolaunch.enable() } else { autolaunch.disable() }.map_err(|e| e.to_string())?;
    }
    if let Some(on) = patch.mini_on_top {
        mini::set_on_top(&app, on).map_err(|e| e.to_string())?;
    }
    if patch.touches_discord() && !Presence::available() {
        return Err("This build of lull has no Discord application".into());
    }
    app.state::<Desktop>().update(|s| {
        if let Some(on) = patch.close_to_tray {
            s.close_to_tray = on;
            s.close_asked = true; // chosen in Settings: the first close doesn't ask any more
        }
        if let Some(on) = patch.start_in_tray {
            s.start_in_tray = on;
        }
        if let Some(style) = patch.mini_style {
            s.mini_style = style;
        }
        if let Some(on) = patch.mini_on_top {
            s.mini_on_top = on;
        }
        if let Some(on) = patch.own_titlebar {
            s.own_titlebar = on;
        }
        if let Some(on) = patch.discord_presence {
            s.discord_presence = on;
        }
        if let Some(status) = patch.discord_status {
            s.discord.status = status;
        }
        if let Some(on) = patch.discord_cover {
            s.discord.cover = on;
        }
        if let Some(on) = patch.discord_buttons {
            s.discord.buttons = on;
        }
        if let Some(paused) = patch.discord_paused {
            s.discord.paused = paused;
        }
    });
    if patch.touches_discord() {
        player::refresh(&app);
    }
    Ok(())
}

/// Settings > App, when this computer is the hub: can Tailscale give it an https address.
#[tauri::command]
pub async fn tailscale_status(app: AppHandle) -> Result<Status, String> {
    local_only(&app)?;
    spawn_blocking(|| tailscale::status(PORT))
        .await
        .map_err(|e| e.to_string())
}

/// The listener pressed "Use Tailscale": serve the hub on the tailnet, and make that address the
/// one the hub puts in pair codes.
#[tauri::command]
pub async fn tailscale_serve(app: AppHandle) -> Result<Status, String> {
    let local = local_only(&app)?;
    let status = spawn_blocking(|| tailscale::serve(PORT))
        .await
        .map_err(|e| e.to_string())??;
    if let Some(url) = &status.url {
        let url = url.clone();
        spawn_blocking(move || local_hub::set_public_url(&local, &url))
            .await
            .map_err(|e| e.to_string())??;
    }
    Ok(status)
}

/// The hub on this computer, as (origin, token); Tailscale serve is only for it.
fn local_only(app: &AppHandle) -> Result<(tauri::Url, String), String> {
    app.state::<Desktop>()
        .local_hub()
        .map(|h| (h.origin.clone(), h.token.clone()))
        .ok_or_else(|| "This computer isn't your hub".to_owned())
}

/// Settings > App: this build's version and where the update check stands (updater.rs). Changes
/// arrive as `app-update` events.
#[tauri::command]
pub fn app_update(app: AppHandle) -> Report {
    updater::report(&app)
}

/// Settings > App's "Check for updates": answers once GitHub has; a download goes on meanwhile.
#[tauri::command]
pub async fn app_update_check(app: AppHandle) -> Report {
    updater::check(&app).await
}

/// Settings > App's "Restart to update". Returns only when the install failed.
#[tauri::command]
pub async fn app_update_install(app: AppHandle) -> Result<(), String> {
    spawn_blocking(move || updater::install(&app))
        .await
        .map_err(|e| e.to_string())?
}

/// `main`'s page once it has reloaded after its process crashed: the song, second and play state to
/// resume (recovery.rs). Null otherwise, and after the first call.
#[tauri::command]
pub fn resume_after_crash(app: AppHandle) -> Option<Resume> {
    recovery::take_resume(&app)
}
