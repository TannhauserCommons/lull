//! Where Windows and Linux differ: title bar, tray glyph, transparency and glass, media keys,
//! taskbar buttons, a sleeping webview, opening links.

#[cfg(windows)]
mod glyphs;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
mod taskbar;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::kill_with_app;

use crate::desktop::glass::Material;
use crate::desktop::settings::Settings;
use crate::desktop::state::Platform;
use tauri::image::Image;
use tauri::{AppHandle, Url, WebviewWindow};

/// What the taskbar's thumbnail buttons show (player.rs).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThumbButtons {
    /// Play/Pause shows Pause.
    pub playing: bool,
    /// Something is loaded and the hub answers: the buttons work.
    pub usable: bool,
}

/// Windows: Previous, Play/Pause and Next under the window's taskbar thumbnail. Linux has none.
pub fn add_thumb_buttons(window: &WebviewWindow) {
    #[cfg(windows)]
    taskbar::attach(window);
    #[cfg(not(windows))]
    let _ = window;
}

/// The same under a window that isn't Tauri's (Classic's native one), from the main thread.
#[cfg(windows)]
pub fn add_thumb_buttons_to(app: &AppHandle, hwnd: isize) {
    taskbar::attach_hwnd(app, hwnd);
}

pub fn show_thumb_buttons(app: &AppHandle, buttons: ThumbButtons) {
    #[cfg(windows)]
    taskbar::show(app, buttons);
    #[cfg(not(windows))]
    let _ = (app, buttons);
}

/// `main` shows `material` behind its page, or is solid (glass.rs). `dark`: Mica's dark variant.
/// Linux: only a window made with an alpha channel shows anything.
pub fn set_glass(window: &WebviewWindow, material: Option<Material>, dark: bool) -> tauri::Result<()> {
    #[cfg(windows)]
    return windows::set_glass(window, material, dark);
    #[cfg(target_os = "linux")]
    return linux::set_glass(window, material, dark);
}

/// Calls `on_change`, from a thread of its own, each time the network changes (a VPN such as
/// Tailscale coming up, Wi-Fi back), so a hub that didn't answer is tried again at once
/// (reconnect.rs). Windows: its IPv4 addresses changed. Linux follows the backoff alone.
pub fn follow_network(on_change: impl Fn() + Send + 'static) {
    #[cfg(windows)]
    windows::follow_network(on_change);
    #[cfg(not(windows))]
    let _ = on_change;
}

/// The system lets windows be see-through: Windows' "Transparency effects" (Settings ›
/// Personalisation › Colours). Linux has no such switch.
pub fn transparency_effects() -> bool {
    #[cfg(windows)]
    return windows::transparency_effects();
    #[cfg(not(windows))]
    true
}

/// One of lull's own windows has the focus (sounds.rs): `main`, the mini player, the Classic window
/// or native Classic's. Never on the main thread (Linux asks GTK's).
pub fn lull_in_front(app: &AppHandle) -> bool {
    #[cfg(windows)]
    {
        let _ = app;
        windows::foreground_is_ours()
    }
    #[cfg(target_os = "linux")]
    return linux::any_window_active(app);
}

/// Calls `changed` with whether the system asks for quiet now, then again whenever that changes
/// (sounds.rs): Windows' "No Sounds" scheme, GNOME's event sounds off. On the main thread.
pub fn follow_quiet(changed: impl Fn(bool) + Send + 'static) {
    #[cfg(windows)]
    windows::follow_sound_scheme(changed);
    #[cfg(target_os = "linux")]
    linux::follow_event_sounds(changed);
}

/// A hidden `main` while Classic mode plays (main_window::sleep). On Linux, WebKitGTK already
/// stops drawing a hidden window.
pub fn set_webview_awake(window: &WebviewWindow, awake: bool) -> tauri::Result<()> {
    #[cfg(windows)]
    return windows::set_webview_awake(window, awake);
    #[cfg(not(windows))]
    {
        let _ = (window, awake);
        Ok(())
    }
}

/// Before the first window (recovery.rs): on Windows, a dump of lull's own process if it crashes
/// (`logs\lull.dmp`), and one WebView2 environment for every window, whose crash dumps stay on this
/// computer. What it says goes in the webview log.
pub fn prepare_webviews(app: &AppHandle) -> Result<String, String> {
    #[cfg(windows)]
    {
        use tauri::Manager;
        if let Ok(logs) = app.path().app_log_dir() {
            windows::dump_on_crash(&logs.join("lull.dmp"));
        }
        let data = app.path().app_local_data_dir().map_err(|e| e.to_string())?;
        windows::create_environment(&data)
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        Ok("WebKitGTK".into())
    }
}

/// A window builder that uses the environment `prepare_webviews` made.
pub fn shared_environment<'a>(
    builder: tauri::WebviewWindowBuilder<'a, tauri::Wry, AppHandle>,
) -> tauri::WebviewWindowBuilder<'a, tauri::Wry, AppHandle> {
    #[cfg(windows)]
    return windows::shared_environment(builder);
    #[cfg(not(windows))]
    builder
}

/// Ends the hung renderer of this window's page, so it can load again (recovery.rs). Only WebView2
/// reports a hang.
pub fn end_renderer(window: &WebviewWindow) -> tauri::Result<()> {
    #[cfg(windows)]
    return windows::end_renderer(window);
    #[cfg(not(windows))]
    {
        let _ = window;
        Ok(())
    }
}

/// Calls `failed` when a process behind this window's page dies (recovery.rs).
pub fn watch_webview(
    window: &WebviewWindow,
    failed: impl Fn(crate::desktop::recovery::Failure) + Send + 'static,
) -> tauri::Result<()> {
    #[cfg(windows)]
    return windows::watch_webview(window, failed);
    #[cfg(target_os = "linux")]
    return linux::watch_webview(window, failed);
}

/// The listener's default browser (links.rs).
pub fn open_in_browser(url: &Url) -> Result<(), String> {
    #[cfg(windows)]
    return windows::open_in_browser(url);
    #[cfg(not(windows))]
    lull_core::process::command("xdg-open")
        .arg(url.as_str())
        .spawn()
        .map(drop)
        .map_err(|e| format!("xdg-open: {e}"))
}

/// `<scheme>:` links start this app (open_links.rs), for this user: the registry's
/// `Software\Classes` on Windows, a desktop entry and `xdg-mime` on Linux.
pub fn register_scheme(scheme: &str) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("where lull is: {e}"))?;
    #[cfg(windows)]
    return windows::register_scheme(scheme, &exe);
    #[cfg(target_os = "linux")]
    return linux::register_scheme(scheme, &exe);
}

/// Read once at startup, on the main thread.
pub fn detect() -> Platform {
    #[cfg(target_os = "linux")]
    return Platform {
        tray: linux::tray_host(),
        transparent: linux::composited(),
        wayland: linux::wayland(),
        button_layout: linux::button_layout(),
        glass: linux::glass_materials(),
    };
    #[cfg(not(target_os = "linux"))]
    Platform {
        tray: true,
        transparent: true,
        wayland: false,
        button_layout: None,
        glass: windows::glass_materials(),
    }
}

/// The title bar the web UI draws itself (DESKTOP.md 2): always on Windows, on Linux only with
/// "Use lull title bar". None: the OS decorates the window.
pub enum Titlebar {
    Win,
    Gnome { layout: Option<String> },
}

pub fn titlebar(settings: &Settings, platform: &Platform) -> Option<Titlebar> {
    if cfg!(windows) {
        Some(Titlebar::Win)
    } else if cfg!(target_os = "linux") && settings.own_titlebar {
        Some(Titlebar::Gnome {
            layout: platform.button_layout.clone(),
        })
    } else {
        None
    }
}

/// Runs before every page in `main`, the server's web UI included: `desktop.js` reads it and
/// attaches the title bar. Plain data, no behaviour.
pub fn init_script(titlebar: Option<&Titlebar>) -> String {
    let (name, layout) = match titlebar {
        Some(Titlebar::Win) => (Some("win"), None),
        Some(Titlebar::Gnome { layout }) => (Some("gnome"), layout.as_deref()),
        None => (None, None),
    };
    let config = serde_json::json!({ "os": std::env::consts::OS, "titlebar": name, "layout": layout });
    format!("window.__LULL_DESKTOP__ = Object.freeze({config});")
}

/// Monochrome tray mark. Windows: the variant for the taskbar's theme, at the primary monitor's
/// scale (16/20/24/32 = 100/125/150/200 %). Linux: the light glyph, panels are dark by default
/// (Tauri hands AppIndicator a file, so the themed `lull-tray-symbolic` name can't be used).
pub fn tray_icon(app: &AppHandle) -> tauri::Result<Image<'static>> {
    #[cfg(windows)]
    {
        let scale = app.primary_monitor()?.map_or(1.0, |m| m.scale_factor());
        Image::from_bytes(windows_tray_png(windows::light_taskbar(), scale))
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        Image::from_bytes(include_bytes!("../../../../web/assets/icons/tray-dark-32.png"))
    }
}

#[cfg(windows)]
fn windows_tray_png(light_taskbar: bool, scale: f64) -> &'static [u8] {
    macro_rules! png {
        ($theme:literal, $size:literal) => {
            include_bytes!(concat!(
                "../../../../web/assets/icons/tray-",
                $theme,
                "-",
                $size,
                ".png"
            ))
        };
    }
    // tray-light = dark glyph for a light taskbar.
    match (light_taskbar, scale) {
        (true, s) if s <= 1.0 => png!("light", "16"),
        (true, s) if s <= 1.25 => png!("light", "20"),
        (true, s) if s <= 1.5 => png!("light", "24"),
        (true, _) => png!("light", "32"),
        (false, s) if s <= 1.0 => png!("dark", "16"),
        (false, s) if s <= 1.25 => png!("dark", "20"),
        (false, s) if s <= 1.5 => png!("dark", "24"),
        (false, _) => png!("dark", "32"),
    }
}

/// Windows' personalisation changed: the tray glyph follows the taskbar's theme, and a see-through
/// `main` follows "Transparency effects".
pub fn follow_personalize(app: &AppHandle) {
    #[cfg(windows)]
    {
        use crate::desktop::state::Desktop;
        use tauri::Manager;
        let app = app.clone();
        // The same key holds "Transparency effects": a see-through `main` follows it too.
        windows::watch_taskbar(move || {
            crate::desktop::glass::refresh(&app);
            if let Some(tray) = app.state::<Desktop>().tray.get() {
                match tray_icon(&app) {
                    Ok(image) => tray.set_icon(image),
                    Err(e) => eprintln!("lull: tray icon: {e}"),
                }
            }
        });
    }
    #[cfg(not(windows))]
    let _ = app;
}

/// Media keys without Media Session (Linux only, DESKTOP.md 5). Windows has SMTC through WebView2,
/// and grabbing the keys there would fight it.
pub fn media_keys(app: &AppHandle) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    return linux::media_keys(app);
    #[cfg(not(target_os = "linux"))]
    {
        let _ = app;
        Ok(())
    }
}
