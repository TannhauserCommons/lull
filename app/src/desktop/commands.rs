//! The commands the pages call (DESKTOP.md 4). Arguments are checked here, at the boundary; anything
//! unknown is refused with a message the caller logs.

use crate::desktop::local_hub::{self, PORT};
use crate::desktop::main_window;
use crate::desktop::mini;
use crate::desktop::pages;
use crate::desktop::platform;
use crate::desktop::settings::{HubChoice, MiniStyle};
use crate::desktop::state::{Desktop, NowPlaying};
use crate::desktop::tailscale::{self, Status};
use crate::discovery::{self, Found};
use crate::hub::{self, Health};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tauri::async_runtime::spawn_blocking;
use tauri::{AppHandle, Manager};
use tauri_plugin_autostart::ManagerExt;

/// How long "Connect to a hub" listens for hubs on the network.
const DISCOVERY_WINDOW: Duration = Duration::from_secs(3);

/// Connect page: the lull hubs on this network.
#[tauri::command]
pub async fn discover_hubs() -> Result<Vec<Found>, String> {
    spawn_blocking(|| discovery::browse(DISCOVERY_WINDOW))
        .await
        .map_err(|e| e.to_string())?
}

#[derive(Debug, Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum FindResult {
    Ok { url: String, health: Health },
    Unreachable { error: String },
}

/// Connect page: is there a lull hub at what the listener typed. An invalid address is an error.
#[tauri::command]
pub async fn find_hub(url: String) -> Result<FindResult, String> {
    let origin = hub::origin(&url)?;
    spawn_blocking(move || match hub::health(&origin) {
        Ok(health) => FindResult::Ok {
            url: origin.to_string(),
            health,
        },
        Err(error) => FindResult::Unreachable { error },
    })
    .await
    .map_err(|e| e.to_string())
}

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
        WindowAction::Retry => main_window::open_chosen(&app),
        WindowAction::CloseChoice { keep, remember } => main_window::close_choice(&app, keep, remember),
        WindowAction::MediaKeys => return platform::media_keys(&app),
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

#[tauri::command]
pub fn set_now_playing(app: AppHandle, now: NowPlaying) {
    app.state::<Desktop>().set_now_playing(now);
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
    });
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
