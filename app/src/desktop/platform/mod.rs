//! Where Windows and Linux differ: title bar, tray glyph, transparency, media keys.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::kill_with_app;

use crate::desktop::settings::Settings;
use crate::desktop::state::Platform;
use tauri::AppHandle;
use tauri::image::Image;

/// Read once at startup, on the main thread.
pub fn detect() -> Platform {
    #[cfg(target_os = "linux")]
    return Platform {
        tray: linux::tray_host(),
        transparent: linux::composited(),
        wayland: linux::wayland(),
        button_layout: linux::button_layout(),
    };
    #[cfg(not(target_os = "linux"))]
    Platform {
        tray: true,
        transparent: true,
        wayland: false,
        button_layout: None,
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

/// Keep the tray glyph matched to the taskbar when the user switches its theme.
pub fn follow_taskbar(app: &AppHandle) {
    #[cfg(windows)]
    {
        use crate::desktop::state::Desktop;
        use tauri::Manager;
        let app = app.clone();
        windows::watch_taskbar(move || {
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
