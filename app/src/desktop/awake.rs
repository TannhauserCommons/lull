//! "Keep lull running when closed" (ADR-0090): while this computer is the hub and the switch is on, it doesn't
//! sleep when idle on mains power, so phones can still reach lull. On battery the OS decides as usual, and a
//! closed laptop lid still sleeps. Windows: `SetThreadExecutionState` from one thread that keeps it. Linux:
//! logind's sleep and idle inhibitor, held as long as its file descriptor is open.

use crate::desktop::settings::HubChoice;
use crate::desktop::state::Desktop;
use std::time::Duration;
use tauri::{AppHandle, Manager};

/// How often the switch and the power source are looked at again.
const EVERY: Duration = Duration::from_secs(60);

/// Starts the thread that holds the computer awake while it should, for the life of the app.
pub fn start(app: AppHandle) {
    let watch = move || {
        let mut held: Option<Hold> = None;
        loop {
            let s = app.state::<Desktop>().settings();
            let want = s.keep_running && s.hub == Some(HubChoice::Local) && on_mains();
            match (&held, want) {
                (None, true) => match Hold::take() {
                    Ok(hold) => {
                        tracing::info!("keeping this computer awake on mains power while lull serves");
                        held = Some(hold);
                    }
                    Err(e) => tracing::warn!("can't keep this computer awake: {e}"),
                },
                (Some(_), false) => {
                    tracing::info!("this computer may sleep again");
                    held = None;
                }
                _ => {}
            }
            std::thread::sleep(EVERY);
        }
    };
    if let Err(e) = std::thread::Builder::new().name("keep-awake".into()).spawn(watch) {
        tracing::warn!("this computer may sleep while lull serves: {e}");
    }
}

/// A computer with no battery runs on mains power.
#[cfg(windows)]
#[allow(unsafe_code)]
fn on_mains() -> bool {
    use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
    // SAFETY: a zeroed status struct is valid; the call fills it in.
    let mut status: SYSTEM_POWER_STATUS = unsafe { std::mem::zeroed() };
    // SAFETY: a valid pointer to that struct.
    if unsafe { GetSystemPowerStatus(&raw mut status) } == 0 {
        return true;
    }
    // ACLineStatus: 0 offline, 1 online, 255 unknown (a desktop often says unknown).
    status.ACLineStatus != 0
}

#[cfg(target_os = "linux")]
fn on_mains() -> bool {
    let Ok(supplies) = std::fs::read_dir("/sys/class/power_supply") else {
        return true;
    };
    let read = |dir: &std::path::Path, file: &str| std::fs::read_to_string(dir.join(file)).unwrap_or_default();
    let mut battery = false;
    for supply in supplies.flatten().map(|e| e.path()) {
        match read(&supply, "type").trim() {
            "Mains" | "USB" if read(&supply, "online").trim() == "1" => return true,
            "Battery" => battery = true,
            _ => {}
        }
    }
    !battery
}

#[cfg(windows)]
struct Hold;

#[cfg(windows)]
impl Hold {
    /// Holds the system awake from this thread (the watch thread, which never ends while the app runs).
    #[allow(unsafe_code)]
    fn take() -> Result<Hold, String> {
        use windows_sys::Win32::System::Power::{ES_CONTINUOUS, ES_SYSTEM_REQUIRED, SetThreadExecutionState};
        // SAFETY: plain flags; the state belongs to the calling thread until changed.
        if unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED) } == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(Hold)
    }
}

#[cfg(windows)]
impl Drop for Hold {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        use windows_sys::Win32::System::Power::{ES_CONTINUOUS, SetThreadExecutionState};
        // SAFETY: plain flags.
        unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
    }
}

/// logind's inhibitor lock: sleep and idle stay blocked while the descriptor is open.
#[cfg(target_os = "linux")]
struct Hold(#[allow(dead_code)] zbus::zvariant::OwnedFd);

#[cfg(target_os = "linux")]
impl Hold {
    fn take() -> Result<Hold, String> {
        let bus = zbus::blocking::Connection::system().map_err(|e| e.to_string())?;
        let reply = bus
            .call_method(
                Some("org.freedesktop.login1"),
                "/org/freedesktop/login1",
                Some("org.freedesktop.login1.Manager"),
                "Inhibit",
                &(
                    "sleep:idle",
                    "lull",
                    "lull serves this computer's music to your devices",
                    "block",
                ),
            )
            .map_err(|e| e.to_string())?;
        let fd: zbus::zvariant::OwnedFd = reply.body().deserialize().map_err(|e| e.to_string())?;
        Ok(Hold(fd))
    }
}
