//! The commands the pages call (DESKTOP.md 4). Arguments are checked here, at the boundary; anything
//! unknown is refused with a message the caller logs.

use crate::main_window;
use crate::mini;
use crate::platform;
use crate::server::{self, TestResult};
use crate::settings::MiniStyle;
use crate::state::{Desktop, NowPlaying};
use serde::{Deserialize, Serialize};
use tauri::async_runtime::spawn_blocking;
use tauri::{AppHandle, Manager, Url};
use tauri_plugin_autostart::ManagerExt;

/// Connect page: is `url` a lull server, and does it take `token`.
#[tauri::command]
pub async fn test_server(url: String, token: String) -> Result<TestResult, String> {
    let origin = server::origin(&url)?;
    spawn_blocking(move || server::test(&origin, token.trim()))
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
    /// Connect page: save this server and load the web UI from it.
    Connected { url: String, token: String, remember: bool },
    /// Unreachable page or Settings > App: open the connect page.
    ChangeServer,
    /// Unreachable page: try the saved server again; loads the web UI if it answers.
    Retry,
    /// The "keep playing in the tray?" dialog.
    CloseChoice { keep: bool, remember: bool },
    /// Web UI without Media Session (some WebKitGTK builds): grab the media keys instead.
    MediaKeys,
}

#[tauri::command]
pub async fn window_action(app: AppHandle, request: WindowAction) -> Result<(), String> {
    let done = match request {
        WindowAction::Full | WindowAction::MiniClose => mini::close(&app),
        WindowAction::Mini => mini::open(&app),
        WindowAction::Connected { url, token, remember } => return connect(&app, &url, token.trim(), remember).await,
        WindowAction::ChangeServer => main_window::open_connect(&app),
        WindowAction::Retry => return retry(&app).await,
        WindowAction::CloseChoice { keep, remember } => main_window::close_choice(&app, keep, remember),
        WindowAction::MediaKeys => return platform::media_keys(&app),
    };
    done.map_err(|e| e.to_string())
}

async fn connect(app: &AppHandle, url: &str, token: &str, remember: bool) -> Result<(), String> {
    let origin = server::origin(url)?;
    if !token.is_empty() {
        let (o, t) = (origin.clone(), token.to_owned());
        let cookie = spawn_blocking(move || server::session_cookie(&o, &t, remember))
            .await
            .map_err(|e| e.to_string())??;
        let main = app
            .get_webview_window(main_window::LABEL)
            .ok_or("The main window is gone")?;
        main.set_cookie(cookie).map_err(|e| e.to_string())?;
    }
    app.state::<Desktop>().update(|s| s.server = Some(origin.to_string()));
    main_window::open_server(app, &origin).map_err(|e| e.to_string())
}

async fn retry(app: &AppHandle) -> Result<(), String> {
    let server = app.state::<Desktop>().settings().server;
    let origin = server
        .as_deref()
        .and_then(|s| Url::parse(s).ok())
        .ok_or("No server is set")?;
    let probe = origin.clone();
    spawn_blocking(move || server::health(&probe))
        .await
        .map_err(|e| e.to_string())??;
    main_window::open_server(app, &origin).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_now_playing(app: AppHandle, now: NowPlaying) {
    app.state::<Desktop>().set_now_playing(now);
}

/// Settings > App inside the wrapper (DESKTOP.md 7).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopSettings {
    server: Option<String>,
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
    Ok(DesktopSettings {
        server: s.server,
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
