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
/// `<scheme>:` links start `exe` (the AppImage itself when run as one): a desktop entry in the
/// user's applications, made the scheme's handler with `xdg-mime`.
pub fn register_scheme(scheme: &str, exe: &std::path::Path) -> Result<(), String> {
    let exe = std::env::var_os("APPIMAGE").map_or_else(|| exe.to_path_buf(), std::path::PathBuf::from);
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share")))
        .ok_or("no home folder")?;
    let dir = data.join("applications");
    let name = format!("lull-{scheme}-links.desktop");
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName=lull\nExec=\"{}\" %u\nIcon=lull\nNoDisplay=true\nMimeType=x-scheme-handler/{scheme};\n",
        exe.display()
    );
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    std::fs::write(dir.join(&name), entry).map_err(|e| format!("{name}: {e}"))?;
    let status = std::process::Command::new("xdg-mime")
        .arg("default")
        .arg(&name)
        .arg(format!("x-scheme-handler/{scheme}"))
        .status()
        .map_err(|e| format!("xdg-mime: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("xdg-mime: {status}"))
    }
}

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

/// Hears WebKitGTK's web-process-terminated for this window: its page's process crashed, ran out of
/// memory or was stopped. `failed` runs on the GTK thread.
pub fn watch_webview(
    window: &tauri::WebviewWindow,
    failed: impl Fn(crate::desktop::recovery::Failure) + Send + 'static,
) -> tauri::Result<()> {
    use crate::desktop::recovery::{Failure, Process};
    use webkit2gtk::{WebProcessTerminationReason, WebViewExt};
    window.with_webview(move |webview| {
        webview.inner().connect_web_process_terminated(move |_, reason| {
            let reason = match reason {
                WebProcessTerminationReason::Crashed => "crashed".to_owned(),
                WebProcessTerminationReason::ExceededMemoryLimit => "out of memory".to_owned(),
                WebProcessTerminationReason::TerminatedByApi => "terminated".to_owned(),
                other => format!("{other:?}"),
            };
            failed(Failure {
                process: Process::Renderer,
                reason,
                exit_code: None,
                description: String::new(),
                module: String::new(),
            });
        });
    })
}
