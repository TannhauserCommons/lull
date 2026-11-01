//! The `main` window: the hub's web UI, or a bundled page (welcome, connect, hub unreachable, hub
//! didn't start) when there is no hub yet or it doesn't answer. DESKTOP.md 1, 2 and 6; ADR-0006.

use crate::desktop::local_hub;
use crate::desktop::pages::{self, Unreachable};
use crate::desktop::platform;
use crate::desktop::settings::HubChoice;
use crate::desktop::state::{Desktop, lock};
use crate::hub;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::ipc::CapabilityBuilder;
use tauri::webview::PageLoadEvent;
use tauri::{AppHandle, Emitter, Manager, Url, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

pub const LABEL: &str = "main";

/// Where `main` starts while the device cookie is put in: the cookie needs a webview first.
const BLANK: &str = "about:blank";

/// What the web UI on the hub may call (DESKTOP.md 2.1 and 4). Granted to its origin only.
const HUB_PERMISSIONS: &[&str] = &[
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
    "allow-tailscale-status",
    "allow-tailscale-serve",
];

/// First window of the run. No hub chosen yet: the welcome page. Otherwise the hub is started or
/// probed off the main thread, and `main` appears once its page has loaded.
pub fn launch(app: &AppHandle) {
    let settings = app.state::<Desktop>().settings();
    if settings.hub.is_none() {
        return report(pages::welcome(&settings.appearance).and_then(|url| create(app, url).map(drop)));
    }
    report(open_chosen(app));
}

/// The chosen hub's web UI, or the page that says why not. Runs on its own thread: starting the
/// hub or probing one takes a moment.
pub fn open_chosen(app: &AppHandle) -> tauri::Result<()> {
    let app = app.clone();
    std::thread::Builder::new()
        .name("open-hub".into())
        .spawn(move || report(resolve(&app)))
        .map(drop)
        .map_err(tauri::Error::Io)
}

fn resolve(app: &AppHandle) -> tauri::Result<()> {
    let desktop = app.state::<Desktop>();
    let settings = desktop.settings();
    match settings.hub {
        None => navigate(app, pages::welcome(&settings.appearance)?),
        Some(HubChoice::Local) => {
            let _starting = lock(&desktop.starting);
            if let Some(local) = desktop.local_hub() {
                return open_hub(app, &local.origin, &local.token);
            }
            match local_hub::start(app) {
                Ok(local) => {
                    let (origin, token) = (local.origin.clone(), local.token.clone());
                    desktop.set_local_hub(local);
                    open_hub(app, &origin, &token)
                }
                Err(error) => navigate(app, pages::hub_failed(&error, &settings.appearance)?),
            }
        }
        Some(HubChoice::Remote { origin }) => {
            let origin = Url::parse(&origin).map_err(tauri::Error::InvalidUrl)?;
            let token = settings.token.unwrap_or_default();
            let unreachable = |error: &str| {
                let page = Unreachable {
                    hub: &origin,
                    error,
                    last_connected: settings.last_connected,
                };
                pages::unreachable(page, &settings.appearance)
            };
            match hub::health(&origin).and_then(|_| hub::paired(&origin, &token)) {
                Ok(true) => open_hub(app, &origin, &token),
                // Unpaired on the hub (or never paired): pair again with a new code.
                Ok(false) => navigate(app, pages::connect(Some(&origin), true, &settings.appearance)?),
                Err(error) => navigate(app, unreachable(&error)?),
            }
        }
    }
}

fn create(app: &AppHandle, url: Url) -> tauri::Result<WebviewWindow> {
    let desktop = app.state::<Desktop>();
    let settings = desktop.settings();
    let titlebar = platform::titlebar(&settings, &desktop.platform);
    // Show once painted (no white flash), unless the tray holds a connected lull ("Start in the tray").
    let reveal = !(settings.start_in_tray && desktop.has_tray() && settings.hub.is_some());
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
            if payload.event() != PageLoadEvent::Finished || payload.url().as_str() == BLANK {
                return;
            }
            loaded(&window, payload.url());
            if reveal.swap(false, std::sync::atomic::Ordering::Relaxed) {
                report(show(window.app_handle()));
            }
        })
        .build()
}

/// Keeps "is the web UI up" true to the page actually loaded, whoever navigated there.
fn loaded(window: &WebviewWindow, url: &Url) {
    let desktop = window.state::<Desktop>();
    let on_hub = !pages::is_local(url);
    if on_hub {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        desktop.update(|s| s.last_connected = Some(now));
    }
    desktop.set_on_hub(on_hub);
}

/// Load the hub's web UI in `main`, paired: the device cookie goes in before the first request.
pub fn open_hub(app: &AppHandle, origin: &Url, token: &str) -> tauri::Result<()> {
    let main = match app.get_webview_window(LABEL) {
        Some(main) => main,
        None => create(app, Url::parse(BLANK).map_err(tauri::Error::InvalidUrl)?)?,
    };
    main.set_cookie(hub::device_cookie(origin, token))?;
    grant(app, origin)?;
    main.navigate(origin.clone())
}

pub fn open_page(app: &AppHandle, url: tauri::Result<Url>) -> tauri::Result<()> {
    navigate(app, url?)
}

fn navigate(app: &AppHandle, url: Url) -> tauri::Result<()> {
    match app.get_webview_window(LABEL) {
        Some(main) => main.navigate(url),
        None => create(app, url).map(drop),
    }
}

/// IPC for the hub's origin, added at runtime because the listener picks the hub. An origin used
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
    let capability = HUB_PERMISSIONS.iter().fold(
        CapabilityBuilder::new(format!("hub-{id}"))
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
    match crate::desktop::mini::window(app) {
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
    if !desktop.on_hub() || !desktop.has_tray() || !settings.close_to_tray {
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
