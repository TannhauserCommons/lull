//! Linux: what the desktop session offers (DESKTOP.md 2.2, 3, 4, 5). Read on the main thread, where
//! GTK lives.

use crate::desktop::main_window::report;
use crate::desktop::player;
use gtk::gio::{self, prelude::*};
use gtk::glib;
use tauri::AppHandle;
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Shortcut, ShortcutState};

/// A tray shows only where a StatusNotifier host runs (KDE, Ubuntu's GNOME, GNOME with the
/// AppIndicator extension). Without one there is no tray, and Close quits.
pub fn tray_host() -> bool {
    let has_owner = || -> Option<bool> {
        let bus = gio::bus_get_sync(gio::BusType::Session, None::<&gio::Cancellable>).ok()?;
        let reply = bus
            .call_sync(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "NameHasOwner",
                Some(&("org.kde.StatusNotifierWatcher",).to_variant()),
                glib::VariantTy::new("(b)").ok(),
                gio::DBusCallFlags::NONE,
                1000,
                None::<&gio::Cancellable>,
            )
            .ok()?;
        reply.get::<(bool,)>().map(|(owned,)| owned)
    };
    has_owner().unwrap_or(false)
}

/// GNOME's window-button order, e.g. `appmenu:minimize,maximize,close`. None outside GNOME-schema
/// desktops (asking GSettings for a missing schema aborts, so look it up first).
pub fn button_layout() -> Option<String> {
    const SCHEMA: &str = "org.gnome.desktop.wm.preferences";
    gio::SettingsSchemaSource::default()?.lookup(SCHEMA, true)?;
    Some(gio::Settings::new(SCHEMA).string("button-layout").to_string())
}

/// Transparent windows need a compositor (X11 without one has no alpha visual).
pub fn composited() -> bool {
    gtk::gdk::Screen::default().is_some_and(|s| s.is_composited() && s.rgba_visual().is_some())
}

pub fn wayland() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
}

/// The fallback when WebKitGTK ships without Media Session: grab the media keys and forward them
/// like tray items. X11 only; Wayland has no global grabs and keeps the tray menu.
pub fn media_keys(app: &AppHandle) -> Result<(), String> {
    let keys = [
        (Code::MediaPlayPause, "play-pause"),
        (Code::MediaTrackNext, "next"),
        (Code::MediaTrackPrevious, "previous"),
        (Code::MediaStop, "stop"),
    ];
    let shortcuts = app.global_shortcut();
    for (code, action) in keys {
        let key = Shortcut::new(None, code);
        if shortcuts.is_registered(key) {
            continue;
        }
        shortcuts
            .on_shortcut(key, move |app, _, event| {
                if event.state == ShortcutState::Pressed {
                    report(player::forward(app, action));
                }
            })
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
