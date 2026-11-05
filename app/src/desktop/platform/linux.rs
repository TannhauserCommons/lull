//! Linux: what the desktop session offers (DESKTOP.md 2.2, 3, 4, 5), and what the UI sounds need
//! (ADR-0081). Read on the main thread, where GTK lives.

use crate::desktop::glass::Material;
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
    let status = lull_core::process::command("xdg-mime")
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

/// GNOME's event sounds (`org.gnome.desktop.sound event-sounds`). Kept for the life of the app on
/// the GTK thread: GSettings tells only a live object about changes.
const SOUND: &str = "org.gnome.desktop.sound";
thread_local! {
    static SOUND_SETTINGS: std::cell::OnceCell<gio::Settings> = const { std::cell::OnceCell::new() };
}

/// Calls `changed` with "GNOME's event sounds are off" now, then again whenever that switch changes:
/// the listener asked for quiet, and lull's UI sounds rest. On the main thread. A desktop without
/// GNOME's schema has no such switch: never quiet (asking GSettings for a missing schema aborts, so
/// look it up first).
pub fn follow_event_sounds(changed: impl Fn(bool) + 'static) {
    const KEY: &str = "event-sounds";
    if gio::SettingsSchemaSource::default()
        .and_then(|source| source.lookup(SOUND, true))
        .is_none()
    {
        changed(false);
        return;
    }
    let settings = gio::Settings::new(SOUND);
    changed(!settings.boolean(KEY));
    settings.connect_changed(Some(KEY), move |settings, key| changed(!settings.boolean(key)));
    SOUND_SETTINGS.with(|kept| {
        let _ = kept.set(settings);
    });
}

/// One of lull's own windows has the focus: any of this process's GTK windows is active (`main`, the
/// mini player, the Classic window, native Classic's windows). Asks the GTK thread, so never call it
/// from there.
pub fn any_window_active(app: &AppHandle) -> bool {
    let (answer, answered) = std::sync::mpsc::channel();
    let asked = app.run_on_main_thread(move || {
        use gtk::prelude::*;
        let active = gtk::Window::list_toplevels()
            .iter()
            .filter_map(|widget| widget.downcast_ref::<gtk::Window>())
            .any(|window| window.is_active());
        let _ = answer.send(active);
    });
    asked.is_ok() && answered.recv().unwrap_or(false)
}

/// Transparent windows need a compositor (X11 without one has no alpha visual).
pub fn composited() -> bool {
    gtk::gdk::Screen::default().is_some_and(|s| s.is_composited() && s.rgba_visual().is_some())
}

pub fn wayland() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
}

/// What a see-through `main` can show here: the desktop as it is wherever a compositor gives windows
/// an alpha channel, and blurred where KWin is asked on X11 (`_KDE_NET_WM_BLUR_BEHIND_REGION`).
/// GNOME has no blur an app can ask for, and Wayland's KWin takes it only over its own protocol.
pub fn glass_materials() -> Vec<Material> {
    if !composited() {
        return Vec::new();
    }
    let kde = std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.split(':').any(|d| d == "KDE"));
    let blur = kde && !wayland();
    [(Material::Blur, blur), (Material::Transparent, true)]
        .into_iter()
        .filter_map(|(m, here)| here.then_some(m))
        .collect()
}

/// `main` shows `material` behind its page, or is solid again. Only a window made with an alpha
/// channel can (glass.rs decides that when it makes `main`); the webview then paints nothing where
/// the page paints nothing, and KWin blurs behind the whole window when asked.
pub fn set_glass(window: &tauri::WebviewWindow, material: Option<Material>, _dark: bool) -> tauri::Result<()> {
    use gtk::prelude::WidgetExt;
    let webview: &tauri::Webview = window.as_ref();
    let clear = tauri::window::Color(0, 0, 0, 0);
    let solid = tauri::window::Color(255, 255, 255, 255);
    webview.set_background_color(Some(if material.is_some() { clear } else { solid }))?;
    let target = window.clone();
    window.run_on_main_thread(move || {
        let Some(gdk_window) = target.gtk_window().ok().and_then(|w| w.window()) else {
            return;
        };
        let blur = gtk::gdk::Atom::intern("_KDE_NET_WM_BLUR_BEHIND_REGION");
        if material == Some(Material::Blur) {
            // No rectangles: the whole window.
            let cardinal = gtk::gdk::Atom::intern("CARDINAL");
            gtk::gdk::property_change(
                &gdk_window,
                &blur,
                &cardinal,
                32,
                gtk::gdk::PropMode::Replace,
                gtk::gdk::ChangeData::ULongs(&[]),
            );
        } else {
            gtk::gdk::property_delete(&gdk_window, &blur);
        }
    })
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
