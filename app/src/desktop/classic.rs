//! Classic mode on the desktop: Webamp alone in its own borderless, transparent window (`classic`),
//! like Winamp itself, while `main` hides and goes quiet. The web UI hands over its queue
//! (`classic_open`), the window's page takes it (`classic_handoff`), and when Webamp closes the page
//! hands the queue back to `main` and says `classic-done`. DESKTOP.md 9 (docs/context/desktop.md).

use crate::desktop::links;
use crate::desktop::main_window;
use crate::desktop::platform;
use crate::desktop::state::{Desktop, lock};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, Url, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

pub const LABEL: &str = "classic";

/// The hub serves this page beside the web UI (`web/desktop/winamp.html`).
const PAGE: &str = "desktop/winamp.html";

/// Webamp's three windows stacked (275 x 116 each); the page fits the window to them once it runs.
const FIRST_SIZE: (f64, f64) = (275.0, 348.0);

/// The queue and player state `main` hands to Webamp. Built by `web/screens/classic.js`; every URL
/// must be on the hub `main` shows, since the page plays them with its device cookie.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Handoff {
    tracks: Vec<Track>,
    index: usize,
    pos: f64,
    playing: bool,
    shuffle: bool,
    repeat: bool,
    volume: f64,
    /// The listener's Classic skin, and their favourites for Webamp's Skins menu.
    skin: Option<Skin>,
    skins: Vec<Skin>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Track {
    id: String,
    url: String,
    title: String,
    artist: String,
    album: String,
    duration: Option<f64>,
    /// Cover art for the system's media overlay.
    art: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Skin {
    url: String,
    name: String,
}

impl Handoff {
    fn urls(&self) -> impl Iterator<Item = &str> {
        let tracks = self.tracks.iter().flat_map(|t| std::iter::once(&t.url).chain(&t.art));
        let skins = self.skin.iter().chain(&self.skins).map(|s| &s.url);
        tracks.chain(skins).map(String::as_str)
    }
}

pub fn window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(LABEL)
}

/// Open Webamp's window with the queue `main` hands over, then hide `main`. Asked again while it is
/// open, it comes to the front.
pub fn open(app: &AppHandle, handoff: Handoff) -> Result<(), String> {
    if let Some(classic) = window(app) {
        return classic
            .show()
            .and_then(|()| classic.set_focus())
            .map_err(|e| e.to_string());
    }
    let desktop = app.state::<Desktop>();
    let main = app
        .get_webview_window(main_window::LABEL)
        .filter(|_| desktop.on_hub())
        .ok_or("Classic mode needs lull's own page")?;
    let hub = main.url().map_err(|e| e.to_string())?.origin();
    if let Some(url) = handoff
        .urls()
        .find(|u| Url::parse(u).map(|u| u.origin()) != Ok(hub.clone()))
    {
        return Err(format!("Classic mode plays only from your hub, not {url}"));
    }
    let page = Url::parse(&hub.ascii_serialization())
        .and_then(|origin| origin.join(PAGE))
        .map_err(|e| e.to_string())?;
    *lock(&desktop.classic) = Some(handoff);
    build(app, page).map_err(|e| e.to_string())?;
    main_window::sleep(app).map_err(|e| e.to_string())
}

fn build(app: &AppHandle, page: Url) -> tauri::Result<()> {
    let (width, height) = FIRST_SIZE;
    let classic = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::External(page))
        .title("lull classic")
        .inner_size(width, height)
        .center()
        .resizable(false)
        .maximizable(false)
        .decorations(false)
        // Only Webamp's own windows show: the page is transparent around them.
        .transparent(true)
        .shadow(false)
        .no_redirection_bitmap(true)
        // The page shows the window once Webamp has drawn and the window fits it.
        .visible(false)
        // Webamp takes dropped skins and songs itself.
        .disable_drag_drop_handler()
        .on_navigation(links::on_navigation(app))
        .on_new_window(links::on_new_window(app))
        .build()?;
    platform::add_thumb_buttons(&classic);
    Ok(())
}

/// The window's page, once: what to play.
pub fn take_handoff(app: &AppHandle) -> Option<Handoff> {
    lock(&app.state::<Desktop>().classic).take()
}

/// Back to lull (its close button, Alt+F4, the tray's Show lull): the page hands the queue back and
/// then says `classic-done`. A page that never took its handoff has nothing to hand back.
pub fn close(app: &AppHandle) -> tauri::Result<()> {
    let Some(classic) = window(app) else {
        return Ok(());
    };
    if lock(&app.state::<Desktop>().classic).is_some() {
        return done(app);
    }
    classic.emit_to(LABEL, "classic-close", ())
}

/// The page handed the queue back: Webamp goes, `main` wakes.
pub fn done(app: &AppHandle) -> tauri::Result<()> {
    lock(&app.state::<Desktop>().classic).take();
    if let Some(classic) = window(app) {
        classic.destroy()?;
    }
    main_window::wake(app)
}
