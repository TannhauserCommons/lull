//! lull for Windows, Linux and Android (ADR-0006). Every platform shows the same web UI from the
//! listener's hub. Shared here: talking to a hub, finding one on the network, pairing, the bundled
//! pages and the settings file. The desktop app (`desktop/`) also runs the hub itself and adds a
//! tray and a mini player; the Android app (`mobile/`) adds a native player and downloads.

mod discovery;
mod hub;
mod pages;
mod pairing;
mod reconnect;
mod store;
mod token;
mod updates;

#[cfg(desktop)]
mod desktop;
#[cfg(target_os = "android")]
mod mobile;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(desktop)]
    desktop::run();
    #[cfg(target_os = "android")]
    mobile::run();
}
