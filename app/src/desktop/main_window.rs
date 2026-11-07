//! The `main` window: the hub's web UI, or a bundled page (welcome, connect, hub unreachable, hub
//! didn't start) when there is no hub yet or it doesn't answer. DESKTOP.md 1, 2 and 6; ADR-0006.

use crate::desktop::classic;
use crate::desktop::glass;
use crate::desktop::graphics;
use crate::desktop::links;
use crate::desktop::local_hub;
use crate::desktop::platform;
use crate::desktop::player;
use crate::desktop::recovery;
use crate::desktop::settings::HubChoice;
use crate::desktop::state::{Desktop, lock};
use crate::hub;
use crate::pages::{self, Unreachable};
use crate::reconnect;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::ipc::CapabilityBuilder;
use tauri::webview::PageLoadEvent;
use tauri::{AppHandle, Emitter, Manager, Url, WebviewUrl, WebviewWindow};

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
    "allow-classic-open",
    "allow-classic-back",
    "allow-app-update",
    "allow-app-update-check",
    "allow-app-update-install",
    "allow-resume-after-crash",
    "allow-window-glass",
    "allow-set-theme-package",
    "allow-window-front",
    "allow-sound-quiet",
    "allow-graphics-state",
    "allow-graphics-report",
    "allow-graphics-used",
];

/// What Webamp's page in the Classic window may call (classic.rs): fit and show its window (at its
/// size, `classic_fit`), move it by Webamp's title bars, take the handoff, and talk to `main`.
/// `allow-set-size` stays for pages that predate `classic_fit`.
const CLASSIC_PERMISSIONS: &[&str] = &[
    "core:window:default",
    "core:window:allow-set-size",
    "core:window:allow-show",
    "core:window:allow-set-focus",
    "core:window:allow-minimize",
    "core:window:allow-start-dragging",
    "core:event:default",
    "allow-window-action",
    "allow-classic-handoff",
    "allow-classic-fit",
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
            let probe: reconnect::Probe = {
                let (origin, token) = (origin.clone(), token.clone());
                std::sync::Arc::new(move || hub::health(&origin).and_then(|_| hub::paired(&origin, &token)))
            };
            match reconnect::once(&probe) {
                Ok(paired) => reached(app, &origin, &token, paired),
                // No answer: the unreachable page, and the app keeps trying by itself until there is one
                Err(error) => {
                    navigate(app, unreachable(&error)?)?;
                    reconnect::start(app, error, probe, move |app, paired| {
                        report(reached(app, &origin, &token, paired))
                    });
                    Ok(())
                }
            }
        }
    }
}

/// The hub answered: its web UI, or, unpaired on the hub (or never paired), pairing again with a new code.
fn reached(app: &AppHandle, origin: &Url, token: &str, paired: bool) -> tauri::Result<()> {
    if paired {
        return open_hub(app, origin, token);
    }
    let appearance = app.state::<Desktop>().settings().appearance;
    navigate(app, pages::connect(Some(origin), true, &appearance)?)
}

fn create(app: &AppHandle, url: Url) -> tauri::Result<WebviewWindow> {
    let desktop = app.state::<Desktop>();
    let settings = desktop.settings();
    let titlebar = platform::titlebar(&settings, &desktop.platform);
    // Show once painted (no white flash), unless the tray holds a connected lull ("Start in the tray"), or lull
    // started at login to keep running as this computer's hub ("Keep lull running when closed").
    let in_tray = settings.start_in_tray && desktop.has_tray() && settings.hub.is_some();
    let in_background = settings.keep_running && settings.hub == Some(HubChoice::Local) && started_at_login();
    let reveal = !(in_tray || in_background);
    let reveal = std::sync::atomic::AtomicBool::new(reveal);
    // A window shows the desktop only if it was made able to (glass.rs). On Windows `main` always is,
    // so glass follows the theme live: the window itself paints nothing, and a solid theme's opaque
    // page and webview cover it. Linux pays for an alpha channel, so only when the theme will use it.
    let alpha = cfg!(windows) || (desktop.platform.transparent && glass::wants_alpha(app));
    desktop.set_main_alpha(alpha);
    let main = recovery::builder(app, LABEL, WebviewUrl::External(url))
        .title("lull")
        .inner_size(1280.0, 800.0)
        .min_inner_size(480.0, 600.0)
        .center()
        .visible(false)
        .decorations(titlebar.is_none())
        .transparent(alpha)
        .shadow(true)
        // The web UI needs HTML5 drag and drop (rows, M3U and .wsz drops); the native handler blocks it.
        .disable_drag_drop_handler()
        .initialization_script(platform::init_script(titlebar.as_ref()))
        .on_navigation(links::on_navigation(app))
        .on_new_window(links::on_new_window(app))
        .on_page_load(move |window, payload| {
            if payload.event() != PageLoadEvent::Finished || payload.url().as_str() == BLANK {
                return;
            }
            loaded(&window, payload.url());
            graphics::page_loaded(window.app_handle());
            if reveal.swap(false, std::sync::atomic::Ordering::Relaxed) {
                report(show(window.app_handle()));
            }
        })
        .build()?;
    platform::add_thumb_buttons(&main);
    recovery::watch(&main);
    // See-through from its first frame when the theme in use asks (and may: glass.rs).
    glass::apply(app);
    Ok(main)
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
    player::refresh(window.app_handle());
    if on_hub {
        crate::desktop::open_links::deliver(window.app_handle());
    }
}

/// Load the hub's web UI in `main`, paired: the device cookie goes in before the first request.
pub fn open_hub(app: &AppHandle, origin: &Url, token: &str) -> tauri::Result<()> {
    reconnect::stop(app);
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

/// `main` shows something else: a reconnect for the page it showed stops.
fn navigate(app: &AppHandle, url: Url) -> tauri::Result<()> {
    reconnect::stop(app);
    let url = pages::fresh(url);
    match app.get_webview_window(LABEL) {
        Some(main) => main.navigate(url),
        None => create(app, url).map(drop),
    }
}

/// IPC for the hub's origin, added at runtime because the listener picks the hub: the web UI in
/// `main`, and Webamp's page in the Classic window. An origin used earlier in this run keeps its
/// grant until lull restarts (Tauri can't revoke a capability).
fn grant(app: &AppHandle, origin: &Url) -> tauri::Result<()> {
    let origin = origin.origin().ascii_serialization();
    if !app.state::<Desktop>().first_grant(&origin) {
        return Ok(());
    }
    let id: String = origin
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let capability = |name: String, window: &str, permissions: &[&str]| {
        permissions.iter().fold(
            CapabilityBuilder::new(name)
                .remote(format!("{origin}/*"))
                .window(window),
            |c, p| c.permission(*p),
        )
    };
    app.add_capability(capability(format!("hub-{id}"), LABEL, HUB_PERMISSIONS))?;
    app.add_capability(capability(format!("classic-{id}"), classic::LABEL, CLASSIC_PERMISSIONS))
}

/// Show `main`, un-minimised and focused.
pub fn show(app: &AppHandle) -> tauri::Result<()> {
    if let Some(main) = app.get_webview_window(LABEL) {
        platform::set_webview_awake(&main, true)?;
        main.show()?;
        main.unminimize()?;
        main.set_focus()?;
        graphics::shown(app);
    }
    Ok(())
}

/// Tray left click (Windows): hide a visible `main`, otherwise bring lull forward.
pub fn toggle(app: &AppHandle) -> tauri::Result<()> {
    match app.get_webview_window(LABEL) {
        Some(main) if main.is_visible()? && !main.is_minimized()? => hide(&main),
        _ => reveal(app),
    }
}

/// `main` out of sight (the tray, the mini player): hidden, and WebView2 told so, as `sleep` does for
/// Classic mode: no frames, the page's hidden-document rules (loops paused, no screen ticks) and a
/// low memory target. Playback goes on: an audible page isn't throttled. `show` wakes it.
pub fn hide(main: &WebviewWindow) -> tauri::Result<()> {
    main.hide()?;
    platform::set_webview_awake(main, false)
}

/// Minimised, `main` sleeps like a hidden one; back on screen (it takes focus), it wakes.
pub fn on_window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    let Some(main) = window.app_handle().get_webview_window(LABEL) else {
        return;
    };
    let result = match event {
        tauri::WindowEvent::Resized(_) if main.is_minimized().unwrap_or(false) => {
            platform::set_webview_awake(&main, false)
        }
        tauri::WindowEvent::Focused(true) => platform::set_webview_awake(&main, true),
        _ => Ok(()),
    };
    report(result);
}

/// A second launch, or the tray: the open Classic window or mini player if there is one, else `main`.
pub fn reveal(app: &AppHandle) -> tauri::Result<()> {
    #[cfg(any(windows, target_os = "linux"))]
    if crate::desktop::classic_native::is_open() {
        return crate::desktop::classic_native::show(app);
    }
    match classic::window(app).or_else(|| crate::desktop::mini::window(app)) {
        Some(other) => {
            other.show()?;
            other.unminimize()?;
            other.set_focus()
        }
        None => show(app),
    }
}

/// While Classic mode plays: `main` hides, tells WebView2 it isn't visible (no frames, throttled
/// timers) and asks it to use little memory. Its page keeps running for what Webamp hands back.
pub fn sleep(app: &AppHandle) -> tauri::Result<()> {
    let Some(main) = app.get_webview_window(LABEL) else {
        return Ok(());
    };
    main.hide()?;
    app.state::<Desktop>().set_main_asleep(true);
    platform::set_webview_awake(&main, false)
}

/// Classic mode closed: `main` as it was, in front.
pub fn wake(app: &AppHandle) -> tauri::Result<()> {
    if let Some(main) = app.get_webview_window(LABEL) {
        platform::set_webview_awake(&main, true)?;
        app.state::<Desktop>().set_main_asleep(false);
    }
    show(app)
}

/// The argument the login entry starts lull with (mod.rs).
pub const BACKGROUND: &str = "--background";

fn started_at_login() -> bool {
    std::env::args().any(|a| a == BACKGROUND)
}

/// The close button (ours or the OS's). With a tray, the web UI keeps playing hidden; the first
/// time, it asks (DESKTOP.md 1). Keeping lull running as this computer's hub hides it, tray or not.
/// A bundled page or no tray: close quits.
pub fn close_requested(window: &tauri::Window, api: &tauri::CloseRequestApi) {
    let desktop = window.state::<Desktop>();
    let settings = desktop.settings();
    if desktop.on_hub() && settings.keep_running && settings.hub == Some(HubChoice::Local) {
        api.prevent_close();
        report(
            window
                .app_handle()
                .get_webview_window(LABEL)
                .map_or(Ok(()), |main| hide(&main)),
        );
        return;
    }
    if !desktop.on_hub() || !desktop.has_tray() || !settings.close_to_tray {
        return;
    }
    api.prevent_close();
    let result = if settings.close_asked {
        window
            .app_handle()
            .get_webview_window(LABEL)
            .map_or(Ok(()), |main| hide(&main))
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
        Some(main) if keep => hide(&main),
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
