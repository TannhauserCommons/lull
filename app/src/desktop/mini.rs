//! Mini player windows (DESKTOP.md 3): the device (`mini`) or the strip (`strip`). A bundled page
//! that mirrors `main`'s player over events; `main` hides while one is open.

use crate::desktop::main_window;
use crate::desktop::recovery;
use crate::desktop::settings::MiniStyle;
use crate::desktop::state::Desktop;
use crate::pages::{self, Mini};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow};

const LABELS: [&str; 2] = ["mini", "strip"];

pub fn window(app: &AppHandle) -> Option<WebviewWindow> {
    LABELS.iter().find_map(|label| app.get_webview_window(label))
}

pub fn is_label(label: &str) -> bool {
    LABELS.contains(&label)
}

pub fn toggle(app: &AppHandle) -> tauri::Result<()> {
    if window(app).is_some() { close(app) } else { open(app) }
}

pub fn open(app: &AppHandle) -> tauri::Result<()> {
    if let Some(mini) = window(app) {
        mini.show()?;
        return mini.set_focus();
    }
    let desktop = app.state::<Desktop>();
    let settings = desktop.settings();
    let style = settings.mini_style;
    let wayland_hint = desktop.platform.wayland && !settings.wayland_hint_shown;
    let url = pages::mini(
        Mini {
            view: style.label(),
            opaque: !desktop.platform.transparent,
            wayland_hint,
        },
        &settings.appearance,
    )?;
    // The opening size: the page then fits the height to its design's player (desktop.js, fitWindow).
    let (width, height) = match style {
        MiniStyle::Device => (340.0, 560.0),
        MiniStyle::Strip => (540.0, 90.0),
    };
    let mini = recovery::builder(app, style.label(), WebviewUrl::External(url))
        .title("lull mini player")
        .inner_size(width, height)
        .resizable(false)
        .maximizable(false)
        .decorations(false)
        // The device silhouette is the window's shape and its stack is the shadow.
        .transparent(desktop.platform.transparent)
        .shadow(false)
        .no_redirection_bitmap(desktop.platform.transparent)
        .always_on_top(settings.mini_on_top)
        .disable_drag_drop_handler()
        .build()?;
    recovery::watch(&mini);
    if wayland_hint {
        desktop.update(|s| s.wayland_hint_shown = true);
    }
    if let Some(main) = app.get_webview_window(main_window::LABEL) {
        main_window::hide(&main)?;
    }
    set_checked(app, true);
    Ok(())
}

/// Close the mini player and bring the full window back.
pub fn close(app: &AppHandle) -> tauri::Result<()> {
    for mini in LABELS.iter().filter_map(|label| app.get_webview_window(label)) {
        mini.destroy()?;
    }
    // A destroyed page may not get to say 'bye' itself: main stops its snapshots (desktop.js).
    app.emit_to(main_window::LABEL, "mini", serde_json::json!({ "action": "bye" }))?;
    set_checked(app, false);
    main_window::show(app)
}

/// "Mini player always on top" applies at once to an open mini player.
pub fn set_on_top(app: &AppHandle, on: bool) -> tauri::Result<()> {
    window(app).map_or(Ok(()), |mini| mini.set_always_on_top(on))
}

fn set_checked(app: &AppHandle, open: bool) {
    if let Some(tray) = app.state::<Desktop>().tray.get() {
        tray.set_mini(open);
    }
}
