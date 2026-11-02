//! Auto-update (ADR-0006). The app asks GitHub Releases (`latest.json` of the newest `v*` release)
//! for a newer build at launch, every 6 hours while it runs, and whenever the listener asks
//! (Settings > App or the tray). The updater checks the build's signature against the public key in
//! `tauri.conf.json`. A found update downloads in the background; then Settings and the tray offer
//! "Restart to update". Nothing installs without that click.

use crate::desktop::state::{Desktop, lock};
use crate::updates::{NO_RELEASE, OFFLINE, Report, Status};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Error, Update, UpdaterExt};

/// Background checks after the one at launch.
const EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// GitHub gets this long to connect, and to send each part of an answer.
const TIMEOUT: Duration = Duration::from_secs(30);
/// What the web UI listens for: the new `Report` on every change.
const EVENT: &str = "app-update";

pub struct Updates {
    status: Mutex<Status>,
    /// The downloaded update, waiting for the listener.
    pending: Mutex<Option<(Update, Vec<u8>)>>,
}

/// Checks now and then every few hours, silently: a found update only changes Settings and the tray.
pub fn start(app: &AppHandle) {
    app.manage(Updates {
        status: Mutex::new(Status::Idle),
        pending: Mutex::new(None),
    });
    let app = app.clone();
    std::thread::spawn(move || {
        loop {
            tauri::async_runtime::block_on(check(&app));
            std::thread::sleep(EVERY);
        }
    });
}

pub fn report(app: &AppHandle) -> Report {
    Report {
        current: app.package_info().version.to_string(),
        status: lock(&app.state::<Updates>().status).clone(),
    }
}

/// One check, unless one runs already or an update waits. Returns once GitHub has answered; a found
/// update goes on downloading meanwhile.
pub async fn check(app: &AppHandle) -> Report {
    if begin(app) {
        match find(app).await {
            Ok(Some(update)) => {
                set(app, downloading(&update.version, 0, None));
                let app = app.clone();
                tauri::async_runtime::spawn(async move { download(&app, update).await });
            }
            Ok(None) => set(app, Status::Current),
            Err(e) => set(app, Status::Failed { error: plain(&e) }),
        }
    }
    report(app)
}

fn begin(app: &AppHandle) -> bool {
    let updates = app.state::<Updates>();
    {
        let mut status = lock(&updates.status);
        if status.busy() {
            return false;
        }
        *status = Status::Checking;
    }
    tell(app);
    true
}

async fn find(app: &AppHandle) -> Result<Option<Update>, Error> {
    // Windows ends the process itself to run the installer: stop the hub and slskd first.
    let stopping = app.clone();
    app.updater_builder()
        .configure_client(|client| client.connect_timeout(TIMEOUT).read_timeout(TIMEOUT))
        .on_before_exit(move || stop_hub(&stopping))
        .build()?
        .check()
        .await
}

async fn download(app: &AppHandle, update: Update) {
    let version = update.version.clone();
    let mut received = 0;
    let result = update
        .download(
            |chunk, total| {
                let before = received;
                received += chunk as u64;
                if worth_telling(before, received, total) {
                    set(app, downloading(&version, received, total));
                }
            },
            || {},
        )
        .await;
    match result {
        Ok(bytes) => {
            *lock(&app.state::<Updates>().pending) = Some((update, bytes));
            set(app, Status::Ready { version });
        }
        Err(e) => set(app, Status::Failed { error: plain(&e) }),
    }
}

fn downloading(version: &str, received: u64, total: Option<u64>) -> Status {
    Status::Downloading {
        version: version.to_owned(),
        received,
        total,
    }
}

/// Progress goes out per whole percent (per MB while the size is unknown), not per chunk.
fn worth_telling(before: u64, after: u64, total: Option<u64>) -> bool {
    let step = |n: u64| match total {
        Some(t) if t > 0 => n * 100 / t,
        _ => n / 1_000_000,
    };
    step(before) != step(after)
}

fn set(app: &AppHandle, status: Status) {
    *lock(&app.state::<Updates>().status) = status;
    tell(app);
}

/// Settings > App (the `app-update` event) and the tray follow every change.
fn tell(app: &AppHandle) {
    let report = report(app);
    if let Some(tray) = app.state::<Desktop>().tray.get() {
        tray.show_update(&label(&report));
    }
    if let Err(e) = app.emit(EVENT, &report) {
        eprintln!("lull: {e}");
    }
}

/// The tray's update item: its text, and whether a click does anything.
pub fn label(report: &Report) -> (String, bool) {
    match &report.status {
        Status::Idle | Status::Available { .. } => ("Check for updates".into(), true),
        Status::Checking => ("Checking for updates…".into(), false),
        Status::Current => (format!("lull {} is up to date", report.current), true),
        Status::Downloading {
            version,
            received,
            total,
        } => {
            let done = match total {
                Some(t) if *t > 0 => format!(" {}%", received * 100 / t),
                _ => String::new(),
            };
            (format!("Downloading lull {version}…{done}"), false)
        }
        Status::Ready { version } => (format!("Restart to update to lull {version}"), true),
        Status::Failed { .. } => ("Couldn't check for updates: try again".into(), true),
    }
}

/// The updater's errors in the listener's words where they have a plain cause.
fn plain(e: &Error) -> String {
    match e {
        Error::ReleaseNotFound => NO_RELEASE.into(),
        Error::Reqwest(r) if r.is_connect() || r.is_timeout() => OFFLINE.into(),
        Error::TargetNotFound(_) | Error::TargetsNotFound(_) => {
            "The newest release has no update for this kind of install".into()
        }
        other => other.to_string(),
    }
}

/// The tray's update item: "Restart to update" once one is downloaded, else check now.
pub fn clicked(app: &AppHandle) {
    let ready = matches!(*lock(&app.state::<Updates>().status), Status::Ready { .. });
    let app = app.clone();
    if ready {
        // Off the main thread: a Linux package install waits for the password prompt.
        tauri::async_runtime::spawn_blocking(move || {
            if let Err(e) = install(&app) {
                eprintln!("lull: the update didn't install: {e}");
            }
        });
    } else {
        tauri::async_runtime::spawn(async move { check(&app).await });
    }
}

/// "Restart to update" (Settings or the tray): install, then start the new version. On Windows the
/// installer restarts lull itself and `install` doesn't return.
pub fn install(app: &AppHandle) -> Result<(), String> {
    let Some((update, bytes)) = lock(&app.state::<Updates>().pending).take() else {
        return Err("No update has been downloaded".into());
    };
    stop_hub(app);
    if let Err(e) = update.install(bytes) {
        set(app, Status::Failed { error: plain(&e) });
        return Err(plain(&e));
    }
    app.restart();
}

fn stop_hub(app: &AppHandle) {
    if let Some(local) = app.state::<Desktop>().local_hub() {
        local.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(status: Status) -> Report {
        Report {
            current: "0.1.0".into(),
            status,
        }
    }

    #[test]
    fn progress_is_told_per_percent_not_per_chunk() {
        let total = Some(1000);
        assert!(!worth_telling(0, 9, total));
        assert!(worth_telling(9, 10, total));
        assert!(!worth_telling(10, 19, total));
        // Size unknown: per MB.
        assert!(!worth_telling(0, 999_999, None));
        assert!(worth_telling(999_999, 1_000_000, None));
    }

    #[test]
    fn the_tray_offers_the_restart_only_once_the_update_is_downloaded() {
        let (text, on) = label(&report(downloading("0.2.0", 420, Some(1000))));
        assert_eq!((text.as_str(), on), ("Downloading lull 0.2.0… 42%", false));
        let (text, on) = label(&report(Status::Ready {
            version: "0.2.0".into(),
        }));
        assert_eq!((text.as_str(), on), ("Restart to update to lull 0.2.0", true));
        let (text, on) = label(&report(Status::Current));
        assert_eq!((text.as_str(), on), ("lull 0.1.0 is up to date", true));
    }

    #[test]
    fn a_missing_release_is_said_plainly() {
        assert_eq!(plain(&Error::ReleaseNotFound), NO_RELEASE);
    }
}
