//! Updates on Android (ADR-0006): Tauri's updater doesn't run here, so the app reads the same
//! `latest.json` as the desktop (`plugins.updater.endpoints` in `tauri.conf.json`) and, when a newer
//! release has an APK, opens it in the browser on the listener's click. Android installs it over this
//! version because both are signed with the release key.

use super::lock;
use crate::updates::{self, NO_RELEASE, OFFLINE, Report, Status};
use std::sync::Mutex;
use std::time::Duration;
use tauri::async_runtime::spawn_blocking;
use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;
use ureq::http::StatusCode;
use ureq::tls::{RootCerts, TlsConfig};

/// GitHub gets this long to answer a check.
const TIMEOUT: Duration = Duration::from_secs(20);

/// Where the check stands, and the APK it found.
pub struct Updates(Mutex<(Status, Option<String>)>);

impl Default for Updates {
    fn default() -> Self {
        Self(Mutex::new((Status::Idle, None)))
    }
}

fn report(app: &AppHandle) -> Report {
    Report {
        current: app.package_info().version.to_string(),
        status: lock(&app.state::<Updates>().0).0.clone(),
    }
}

/// Settings > App: this build's version and where the check stands.
#[tauri::command]
pub fn app_update(app: AppHandle) -> Report {
    report(&app)
}

/// Settings > App's "Check for updates".
#[tauri::command]
pub async fn app_update_check(app: AppHandle) -> Report {
    let updates = app.state::<Updates>();
    {
        let mut state = lock(&updates.0);
        if state.0.busy() {
            drop(state);
            return report(&app);
        }
        state.0 = Status::Checking;
    }
    let current = app.package_info().version.to_string();
    let endpoint = endpoint(&app);
    let found = spawn_blocking(move || newest(&endpoint?, &current))
        .await
        .map_err(|e| e.to_string())
        .and_then(|r| r);
    *lock(&updates.0) = match found {
        Ok(Some(apk)) => (Status::Available { version: apk.version }, Some(apk.url)),
        Ok(None) => (Status::Current, None),
        Err(error) => (Status::Failed { error }, None),
    };
    report(&app)
}

/// Settings > App's "Download": the browser fetches the APK, and Android offers to install it.
#[tauri::command]
pub fn app_update_install(app: AppHandle) -> Result<(), String> {
    let url = lock(&app.state::<Updates>().0).1.clone();
    let url = url.ok_or("No newer version has been found")?;
    app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
}

/// The desktop updater's manifest address, the one source of where releases live.
fn endpoint(app: &AppHandle) -> Result<String, String> {
    app.config()
        .plugins
        .0
        .get("updater")
        .and_then(|u| u["endpoints"][0].as_str())
        .map(str::to_owned)
        .ok_or_else(|| "This build has no release address".to_owned())
}

fn newest(endpoint: &str, current: &str) -> Result<Option<updates::Android>, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .http_status_as_error(false)
        // As for the hub (hub.rs): trust what Android trusts.
        .tls_config(TlsConfig::builder().root_certs(RootCerts::PlatformVerifier).build())
        .user_agent(concat!("lull-app/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    let mut response = agent.get(endpoint).call().map_err(|_| OFFLINE.to_owned())?;
    match response.status() {
        StatusCode::OK => {}
        StatusCode::NOT_FOUND => return Err(NO_RELEASE.into()),
        other => return Err(format!("GitHub answered {other}")),
    }
    let body = response.body_mut().read_to_string().map_err(|e| e.to_string())?;
    updates::newer_apk(current, &body)
}
