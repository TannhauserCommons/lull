//! The connect page's commands, the same on desktop and Android: hubs on this network, and whether
//! an address is a lull hub.

use crate::discovery::{self, Found};
use crate::hub::{self, Health};
use serde::Serialize;
use std::time::Duration;
use tauri::async_runtime::spawn_blocking;

/// How long "Connect to a hub" listens for hubs on the network.
const DISCOVERY_WINDOW: Duration = Duration::from_secs(3);

/// The lull hubs on this network.
#[tauri::command]
pub async fn discover_hubs(app: tauri::AppHandle) -> Result<Vec<Found>, String> {
    // Android drops multicast unless the app holds a lock, so only while looking.
    #[cfg(target_os = "android")]
    let multicast = {
        use tauri_plugin_lull_player::PlayerExt;
        app.player().multicast(true)?;
        move || app.player().multicast(false)
    };
    #[cfg(not(target_os = "android"))]
    let multicast = move || -> Result<(), String> {
        let _ = app;
        Ok(())
    };
    let found = spawn_blocking(|| discovery::browse(DISCOVERY_WINDOW))
        .await
        .map_err(|e| e.to_string());
    multicast()?;
    found?
}

#[derive(Debug, Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum FindResult {
    Ok { url: String, health: Health },
    Unreachable { error: String },
}

/// Is there a lull hub at what the listener typed. An invalid address is an error.
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
