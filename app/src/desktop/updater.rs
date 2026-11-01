//! Auto-update (ADR-0006). At launch the app asks GitHub Releases (`latest.json` of the newest `v*`
//! release) for a newer build; the updater checks its signature against the public key in
//! `tauri.conf.json`. A found update downloads in the background, then the tray offers "Restart to
//! update". Nothing installs without that click.

use crate::desktop::state::{Desktop, lock};
use std::sync::Mutex;
use tauri::{AppHandle, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

/// The downloaded update, waiting for the listener.
#[derive(Default)]
pub struct Pending(Mutex<Option<(Update, Vec<u8>)>>);

pub fn check(app: &AppHandle) {
    app.manage(Pending::default());
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = fetch(&app).await {
            // Offline, or the release has no updater files yet: try again next launch.
            eprintln!("lull: no update check: {e}");
        }
    });
}

async fn fetch(app: &AppHandle) -> Result<(), String> {
    // Windows ends the process itself to run the installer: stop the hub and slskd first.
    let stopping = app.clone();
    let updater = app
        .updater_builder()
        .on_before_exit(move || stop_hub(&stopping))
        .build()
        .map_err(|e| e.to_string())?;
    let Some(update) = updater.check().await.map_err(|e| e.to_string())? else {
        return Ok(());
    };
    let bytes = update.download(|_, _| {}, || {}).await.map_err(|e| e.to_string())?;
    let version = update.version.clone();
    *lock(&app.state::<Pending>().0) = Some((update, bytes));
    if let Some(tray) = app.state::<Desktop>().tray.get() {
        tray.offer_update(app, &version);
    }
    Ok(())
}

/// The tray's "Restart to update": install, then start the new version. On Windows the installer
/// restarts lull itself and `install` doesn't return.
pub fn install(app: &AppHandle) -> Result<(), String> {
    let Some((update, bytes)) = lock(&app.state::<Pending>().0).take() else {
        return Ok(());
    };
    stop_hub(app);
    update.install(bytes).map_err(|e| e.to_string())?;
    app.restart();
}

fn stop_hub(app: &AppHandle) {
    if let Some(local) = app.state::<Desktop>().local_hub() {
        local.stop();
    }
}
