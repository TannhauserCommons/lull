//! lull for Windows, Linux and Android (ADR-0006). Every platform shows the same web UI from the
//! listener's hub. Shared here: talking to a hub and finding one on the network. The desktop app
//! (`desktop/`) also runs the hub itself and adds a tray and a mini player.

mod discovery;
mod hub;

#[cfg(desktop)]
mod desktop;

pub fn run() {
    #[cfg(desktop)]
    desktop::run();
}
