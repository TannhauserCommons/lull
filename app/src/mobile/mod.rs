//! lull on Android (ADR-0006, docs/context/android.md): the hub's web UI in the app's WebView, a
//! native player for background playback and the lock screen, and downloads to app storage. A phone
//! is never a hub: it pairs with one by QR code or pair code.

mod update;

use crate::hub;
use crate::pages::{self, Appearance, Unreachable};
use crate::pairing;
use crate::store::Store;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::{Mutex, MutexGuard, PoisonError};
use tauri::async_runtime::spawn_blocking;
use tauri::ipc::CapabilityBuilder;
use tauri::{App, AppHandle, Manager, Url, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_lull_player::PlayerExt;

const LABEL: &str = "main";

/// `window.LullNative`, defined before the web UI's scripts run (docs/context/web.md).
const NATIVE: &str = include_str!("native.js");

/// What the web UI on the hub may call: the native player, downloads and this app's updates. Granted to
/// its origin only.
const HUB_PERMISSIONS: &[&str] = &[
    "lull-player:default",
    "allow-app-update",
    "allow-app-update-check",
    "allow-app-update-install",
];

/// The phone's own settings: which hub, and its device token there (ADR-0002).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Settings {
    hub: Option<String>,
    token: Option<String>,
}

struct Phone {
    store: Store<Settings>,
    settings: Mutex<Settings>,
    /// Hub origins already given their IPC capability this run (Tauri can't revoke one).
    granted: Mutex<HashSet<String>>,
}

impl Phone {
    fn settings(&self) -> Settings {
        lock(&self.settings).clone()
    }

    fn update(&self, change: impl FnOnce(&mut Settings)) {
        let mut settings = lock(&self.settings);
        change(&mut settings);
        self.store.save(&settings);
    }
}

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_lull_player::init())
        .plugin(tauri_plugin_opener::init())
        .manage(update::Updates::default())
        .invoke_handler(tauri::generate_handler![
            pairing::discover_hubs,
            pairing::find_hub,
            window_action,
            update::app_update,
            update::app_update_check,
            update::app_update_install
        ])
        .setup(setup)
        .build(tauri::generate_context!());
    match app {
        Ok(app) => app.run(|_, _| {}),
        Err(e) => eprintln!("lull: {e}"),
    }
}

fn setup(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    let store = Store::new(&app.path().app_config_dir()?);
    let settings = store.load();
    app.manage(Phone {
        store,
        settings: Mutex::new(settings),
        granted: Mutex::new(HashSet::new()),
    });
    let blank = Url::parse("about:blank")?;
    WebviewWindowBuilder::new(app, LABEL, WebviewUrl::External(blank))
        .initialization_script(NATIVE)
        // https://tauri.localhost and https://asset.localhost: covers of downloads (asset protocol)
        // must not be mixed content on an https hub's page.
        .use_https_scheme(true)
        .build()?;
    open_chosen(app.handle());
    Ok(())
}

/// The paired hub's web UI, or the page that says why not. Off the main thread: it probes the hub.
fn open_chosen(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        if let Err(e) = resolve(&app) {
            eprintln!("lull: {e}");
        }
    });
}

fn resolve(app: &AppHandle) -> Result<(), String> {
    let settings = app.state::<Phone>().settings();
    let appearance = Appearance::default();
    let (Some(origin), Some(token)) = (settings.hub, settings.token) else {
        return navigate(app, pages::connect(None, false, &appearance));
    };
    let origin = Url::parse(&origin).map_err(|e| e.to_string())?;
    match hub::health(&origin).and_then(|_| hub::paired(&origin, &token)) {
        Ok(true) => open_hub(app, &origin, &token),
        Ok(false) => navigate(app, pages::connect(Some(&origin), true, &appearance)),
        // Away from the hub: an https hub's web UI still opens from its service worker, with the
        // downloads. A plain-http hub can't have one, so it gets the unreachable page.
        Err(_) if origin.scheme() == "https" => open_hub(app, &origin, &token),
        Err(error) => {
            let page = Unreachable {
                hub: &origin,
                error: &error,
                last_connected: None,
            };
            navigate(app, pages::unreachable(page, &appearance))
        }
    }
}

/// Load the hub's web UI, paired: Kotlin puts the device cookie in the WebView and the token on its
/// own requests before the first page request.
fn open_hub(app: &AppHandle, origin: &Url, token: &str) -> Result<(), String> {
    app.player()
        .configure(origin.origin().ascii_serialization().as_str(), token)?;
    grant(app, origin).map_err(|e| e.to_string())?;
    navigate(app, Ok(origin.clone()))
}

fn navigate(app: &AppHandle, url: tauri::Result<Url>) -> Result<(), String> {
    let main = app.get_webview_window(LABEL).ok_or("The window is gone")?;
    main.navigate(url.map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

fn grant(app: &AppHandle, origin: &Url) -> tauri::Result<()> {
    let origin = origin.origin().ascii_serialization();
    if !lock(&app.state::<Phone>().granted).insert(origin.clone()) {
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

/// What the bundled pages ask of the phone (the desktop's `window_action`, phone actions only).
#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "kebab-case", deny_unknown_fields)]
pub enum WindowAction {
    /// Connect page: pair with the hub at `url` using a pair code (typed, or from its QR code).
    Pair { url: String, code: String },
    /// Unreachable page: open the paired hub again.
    Retry,
    /// Unreachable page: pair with another hub.
    ChangeServer,
}

#[tauri::command]
pub async fn window_action(app: AppHandle, request: WindowAction) -> Result<(), String> {
    match request {
        WindowAction::Pair { url, code } => {
            let origin = hub::origin(&url)?;
            let o = origin.clone();
            let token = spawn_blocking(move || hub::pair(&o, &code, &hub::device_name()))
                .await
                .map_err(|e| e.to_string())??;
            app.state::<Phone>().update(|s| {
                s.hub = Some(origin.to_string());
                s.token = Some(token.clone());
            });
            open_hub(&app, &origin, &token)
        }
        WindowAction::Retry => {
            open_chosen(&app);
            Ok(())
        }
        WindowAction::ChangeServer => navigate(&app, pages::connect(None, false, &Appearance::default())),
    }
}

/// A poisoned lock only means another thread panicked mid-update; the data is still usable.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}
