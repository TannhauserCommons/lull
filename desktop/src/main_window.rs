//! The `main` window: the server's web UI, or a bundled page (connect, server unreachable) when
//! there is no server yet or it doesn't answer. DESKTOP.md 1, 2 and 6.

use crate::pages::{self, Unreachable};
use crate::platform;
use crate::server;
use crate::state::Desktop;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::ipc::CapabilityBuilder;
use tauri::webview::PageLoadEvent;
use tauri::{AppHandle, Emitter, Manager, Url, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

pub const LABEL: &str = "main";

/// What the web UI on the server may call (DESKTOP.md 2.1 and 4). Granted to its origin only.
const SERVER_PERMISSIONS: &[&str] = &[
    "core:window:default",
    "core:window:allow-minimize",
    "core:window:allow-toggle-maximize",
    "core:window:allow-internal-toggle-maximize",
    "core:window:allow-close",
    "core:window:allow-start-dragging",
    "core:event:default",
    "allow-window-action",
    "allow-set-now-playing",
    "allow-desktop-settings",
    "allow-set-desktop-settings",
];

/// First window of the run: the web UI if the saved server answers, else the right bundled page.
/// The probe runs off the main thread; `main` appears once its page has loaded.
pub fn launch(app: &AppHandle) {
    let desktop = app.state::<Desktop>();
    let settings = desktop.settings();
    let Some(origin) = settings.server.as_deref().and_then(|s| Url::parse(s).ok()) else {
        let page = pages::connect(None, &settings.appearance);
        return report(page.and_then(|url| create(app, url)));
    };
    let app = app.clone();
    std::thread::spawn(move || {
        let target = match server::health(&origin) {
            Ok(_) => grant(&app, &origin).map(|()| origin),
            Err(error) => pages::unreachable(
                Unreachable {
                    server: &origin,
                    error: &error,
                    last_connected: settings.last_connected,
                },
                &settings.appearance,
            ),
        };
        report(target.and_then(|url| create(&app, url)));
    });
}

fn create(app: &AppHandle, url: Url) -> tauri::Result<()> {
    let desktop = app.state::<Desktop>();
    let settings = desktop.settings();
    let titlebar = platform::titlebar(&settings, &desktop.platform);
    // Show once painted (no white flash), unless the tray holds a connected lull ("Start in the tray").
    let reveal = !(settings.start_in_tray && desktop.has_tray() && settings.server.is_some());
    let reveal = std::sync::atomic::AtomicBool::new(reveal);
    WebviewWindowBuilder::new(app, LABEL, WebviewUrl::External(url))
        .title("lull")
        .inner_size(1280.0, 800.0)
        .min_inner_size(480.0, 600.0)
        .center()
        .visible(false)
        .decorations(titlebar.is_none())
        .shadow(true)
        // The web UI needs HTML5 drag and drop (rows, M3U and .wsz drops); the native handler blocks it.
        .disable_drag_drop_handler()
        .initialization_script(platform::init_script(titlebar.as_ref()))
        .on_page_load(move |window, payload| {
            if payload.event() != PageLoadEvent::Finished {
                return;
            }
            loaded(&window, payload.url());
            if reveal.swap(false, std::sync::atomic::Ordering::Relaxed) {
                report(show(window.app_handle()));
            }
        })
        .build()
        .map(drop)
}

/// Keeps "is the web UI up" true to the page actually loaded, whoever navigated there.
fn loaded(window: &WebviewWindow, url: &Url) {
    let desktop = window.state::<Desktop>();
    let on_server = !pages::is_local(url);
    if on_server {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        desktop.update(|s| s.last_connected = Some(now));
    }
    desktop.set_on_server(on_server);
}

/// Load the web UI from `origin` in `main`.
pub fn open_server(app: &AppHandle, origin: &Url) -> tauri::Result<()> {
    grant(app, origin)?;
    navigate(app, origin.clone())
}

pub fn open_connect(app: &AppHandle) -> tauri::Result<()> {
    let settings = app.state::<Desktop>().settings();
    let server = settings.server.as_deref().and_then(|s| Url::parse(s).ok());
    navigate(app, pages::connect(server.as_ref(), &settings.appearance)?)
}

fn navigate(app: &AppHandle, url: Url) -> tauri::Result<()> {
    match app.get_webview_window(LABEL) {
        Some(main) => main.navigate(url),
        None => create(app, url),
    }
}

/// IPC for the server's origin, added at runtime because the user picks the server. An origin used
/// earlier in this run keeps its grant until lull restarts (Tauri can't revoke a capability).
fn grant(app: &AppHandle, origin: &Url) -> tauri::Result<()> {
    let origin = origin.origin().ascii_serialization();
    if !app.state::<Desktop>().first_grant(&origin) {
        return Ok(());
    }
    let id: String = origin
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let capability = SERVER_PERMISSIONS.iter().fold(
        CapabilityBuilder::new(format!("server-{id}"))
            .remote(format!("{origin}/*"))
            .window(LABEL),
        |c, p| c.permission(*p),
    );
    app.add_capability(capability)
}

/// Show `main`, un-minimised and focused.
pub fn show(app: &AppHandle) -> tauri::Result<()> {
    if let Some(main) = app.get_webview_window(LABEL) {
        main.show()?;
        main.unminimize()?;
        main.set_focus()?;
    }
    Ok(())
}

/// Tray left click (Windows): hide a visible `main`, otherwise bring lull forward.
pub fn toggle(app: &AppHandle) -> tauri::Result<()> {
    match app.get_webview_window(LABEL) {
        Some(main) if main.is_visible()? && !main.is_minimized()? => main.hide(),
        _ => reveal(app),
    }
}

/// A second launch, or the tray: the open mini player if there is one, else `main`.
pub fn reveal(app: &AppHandle) -> tauri::Result<()> {
    match crate::mini::window(app) {
        Some(mini) => {
            mini.show()?;
            mini.set_focus()
        }
        None => show(app),
    }
}

/// The close button (ours or the OS's). With a tray, the web UI keeps playing hidden; the first
/// time, it asks (DESKTOP.md 1). A bundled page or no tray: close quits.
pub fn close_requested(window: &tauri::Window, api: &tauri::CloseRequestApi) {
    let desktop = window.state::<Desktop>();
    let settings = desktop.settings();
    if !desktop.on_server() || !desktop.has_tray() || !settings.close_to_tray {
        return;
    }
    api.prevent_close();
    let result = if settings.close_asked {
        window.hide()
    } else {
        window.emit_to(LABEL, "ask-close", ())
    };
    report(result);
}

/// The answer to "keep playing in the tray?".
pub fn close_choice(app: &AppHandle, keep: bool, remember: bool) -> tauri::Result<()> {
    if remember {
        app.state::<Desktop>().update(|s| {
            s.close_asked = true;
            s.close_to_tray = keep;
        });
    }
    match app.get_webview_window(LABEL) {
        Some(main) if keep => main.hide(),
        _ => {
            app.exit(0);
            Ok(())
        }
    }
}

/// Failures here are window-system hiccups with nobody to answer them: log and carry on.
pub fn report(result: tauri::Result<()>) {
    if let Err(e) = result {
        eprintln!("lull: {e}");
    }
}
