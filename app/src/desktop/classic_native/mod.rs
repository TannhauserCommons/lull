//! Classic mode without a webview (Windows; a test build chosen with `LULL_CLASSIC=native`): the
//! skin's main window drawn natively (window.rs, draw.rs, skin.rs) with its own player (sound.rs)
//! and what the web UI would do around it (session.rs: listening history, Like and Save, the tray,
//! the taskbar's buttons, Discord; smtc.rs: Windows' media overlay and keys), so `main`'s webview can
//! close and WebView2 with it while Classic plays. The window holds the main window, the equaliser
//! and the playlist (eqwin.rs, playlist.rs). Leaving Classic builds `main` again on the page it was
//! on, and that page takes the queue back (`classic_back`) as it takes it from Webamp's window.

mod align;
mod canvas;
mod deck;
mod draw;
mod engine;
mod eq;
mod eqwin;
mod events;
mod hub;
mod listen;
mod mixer;
mod output;
mod playlist;
mod queue;
mod remote;
mod session;
mod skin;
mod smtc;
#[cfg(test)]
mod testhub;
mod vis;
mod window;

use crate::desktop::classic::{Equaliser, Handoff};
use crate::desktop::main_window;
use crate::desktop::state::Desktop;
use base64::Engine as _;
use engine::Engine;
use hub::Hub;
use queue::{Entry, Queue, Repeat};
use serde_json::Value;
use session::{Handback, Session};
use skin::Skin;
use std::io::Read;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Manager, Url, WebviewWindow};

/// Webamp's default skin in the Webamp Skin Museum, through the hub: Classic wears it with no skin
/// chosen, and a skin's missing sheets come from it, as Winamp filled them from its own.
const BASE_SKIN: &str = "/api/skins/5e4f10275dcb1fb211d4a8b4f1bda236.wsz";
/// Classic's sizes (web/desktop/webamp-session.js SIZES).
const SIZES: [f64; 5] = [1.0, 1.5, 2.0, 2.5, 3.0];
/// No skin is bigger than this.
const MAX_SKIN: u64 = 16 * 1024 * 1024;

/// Open: readable from any thread (the window itself lives on the main thread).
static OPEN: AtomicBool = AtomicBool::new(false);
/// The queue Classic handed back, until `main`'s new page takes it.
static BACK: Mutex<Option<Value>> = Mutex::new(None);

/// Classic opens natively instead of in Webamp's window.
pub fn chosen() -> bool {
    std::env::var_os("LULL_CLASSIC").is_some_and(|v| v == "native")
}

pub fn is_open() -> bool {
    OPEN.load(Ordering::Relaxed)
}

/// Opens the native window with the queue `main` hands over, then closes `main`: its webview, and
/// WebView2 with it once no other window has one.
pub fn open(app: &AppHandle, handoff: Handoff, main: WebviewWindow) -> Result<(), String> {
    let (origin, token) = app
        .state::<Desktop>()
        .hub_access()
        .ok_or("Classic mode needs your hub")?;
    let source = match (&handoff.skin_data, &handoff.skin) {
        (Some(data), _) => Some(SkinSource::Bytes(
            base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(|e| format!("the skin's file is damaged: {e}"))?,
        )),
        (None, Some(skin)) => Some(SkinSource::Hub(skin.url.clone())),
        (None, None) => None,
    };
    let (skin, said) = load_skin(&origin, &token, source)?;
    let (size, dpi, at) = place(&main, handoff.size.unwrap_or(1.0))?;
    let favourites = handoff.skins.iter().map(|s| (s.name.clone(), s.url.clone())).collect();
    let eq = handoff.eq.clone().unwrap_or_else(Equaliser::flat).normalised();
    let hub = Hub::new(origin, token);
    let engine = Engine::start(hub.clone(), start(&handoff, eq))?;
    let session = Session::new(app.clone(), hub, engine);
    let page = handoff.page.clone();
    let app_back = app.clone();
    let (opened, result) = std::sync::mpsc::channel();
    app.run_on_main_thread(move || {
        let on_close: window::OnClose = Box::new(move |session, size| go_back(app_back, session, size, page));
        let window = window::Setup {
            skin,
            said,
            session,
            size,
            dpi,
            at,
            favourites,
        };
        let _ = opened.send(window::open(window, on_close));
    })
    .map_err(|e| e.to_string())?;
    result.recv().map_err(|e| e.to_string())??;
    OPEN.store(true, Ordering::Relaxed);
    main.destroy().map_err(|e| e.to_string())
}

/// Brings the window forward: the tray, a second launch.
pub fn show(app: &AppHandle) -> tauri::Result<()> {
    app.run_on_main_thread(window::show)
}

/// Back to lull, as the window's close button does: the tray's Show lull.
pub fn close(app: &AppHandle) -> tauri::Result<()> {
    app.run_on_main_thread(window::close)
}

/// The tray's and the taskbar's buttons while Classic plays (player::forward).
pub fn act(app: &AppHandle, action: &'static str) -> tauri::Result<()> {
    app.run_on_main_thread(move || window::act(action))
}

/// `main`'s new page asks for the queue Classic handed back: once.
pub fn take_back() -> Option<Value> {
    BACK.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take()
}

/// The window has gone: once the hub has heard about the last play (or it's out of reach, and the
/// outbox will tell it), the queue waits for `main`'s new page, which opens on the page Classic was
/// opened from. Off the main thread: building a webview there would wait on itself.
fn go_back(app: AppHandle, session: Session, size: f64, page: Option<String>) {
    OPEN.store(false, Ordering::Relaxed);
    let (origin, token) = (session.origin().clone(), session.token().to_owned());
    let mut url = origin;
    url.set_fragment(Some(page.as_deref().filter(|p| p.starts_with('/')).unwrap_or("/home")));
    let spawned = std::thread::Builder::new().name("classic-back".into()).spawn(move || {
        if let Some(Handback {
            mut value,
            threads,
            ops,
        }) = session.hand_back()
        {
            for thread in threads {
                let _ = thread.join();
            }
            let ops = std::mem::take(&mut *ops.lock().unwrap_or_else(std::sync::PoisonError::into_inner));
            value["ops"] = Value::Array(ops);
            value["size"] = size.into();
            *BACK.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(value);
        }
        main_window::report(main_window::open_hub(&app, &url, &token));
    });
    if let Err(e) = spawned {
        eprintln!("lull: can't bring lull back from Classic: {e}");
    }
}

/// The player's start from the page's handoff: its queue by the page's rules, what it knows of each
/// song, where it was and how it sounded.
fn start(handoff: &Handoff, eq: Equaliser) -> engine::Start {
    let tracks = handoff
        .tracks
        .iter()
        .map(|t| engine::Track {
            id: t.id.clone(),
            title: t.title.clone(),
            artist: t.artist.clone(),
            album: t.album.clone(),
            artist_id: t.artist_id.clone(),
            album_id: t.album_id.clone(),
            duration: t.duration,
            saved: t.saved,
            liked: t.liked,
            art: t.art.clone(),
            gain_db: t.gain_db,
            recommended: t.recommended,
            entity: None,
        })
        .collect();
    let entries = handoff
        .tracks
        .iter()
        .map(|t| Entry {
            track_id: t.id.clone(),
            group: t.group.clone().unwrap_or_else(|| "context".into()),
            source: t.source.clone().and_then(|s| serde_json::from_value(s).ok()),
            orig: t.orig,
        })
        .collect();
    let repeat = match handoff.repeat_mode.as_deref() {
        Some("one") => Repeat::One,
        Some("all") => Repeat::All,
        Some(_) => Repeat::Off,
        None if handoff.repeat => Repeat::All,
        None => Repeat::Off,
    };
    let queue = Queue {
        queue: entries,
        index: handoff.index.min(handoff.tracks.len().saturating_sub(1)),
        shuffle: handoff.shuffle,
        repeat,
        radio: handoff.radio.clone().and_then(|r| serde_json::from_value(r).ok()),
        context: handoff.context.clone().unwrap_or(Value::Null),
    };
    engine::Start {
        queue,
        tracks,
        position: handoff.pos,
        playing: handoff.playing,
        volume: handoff.volume,
        eq,
    }
}

/// Where a skin comes from: its file on the hub, or its bytes (one kept on the device).
pub enum SkinSource {
    Hub(String),
    Bytes(Vec<u8>),
}

/// The skin, or Webamp's base skin with none chosen; a skin missing some sheets has them filled
/// from the base skin. A skin that fails leaves the base skin on, and the failure says why (Webamp
/// kept the skin it had). Without even the base skin there's nothing to draw.
fn load_skin(origin: &Url, token: &str, source: Option<SkinSource>) -> Result<(Skin, Option<String>), String> {
    let base = || {
        let url = origin.join(BASE_SKIN).map_err(|e| e.to_string())?;
        fetch(url.as_str(), token).and_then(|bytes| Skin::from_archive(&bytes, None))
    };
    let chosen = source.map(|source| {
        let bytes = match source {
            SkinSource::Hub(url) => fetch(&url, token)?,
            SkinSource::Bytes(bytes) => bytes,
        };
        match Skin::from_archive(&bytes, None) {
            Ok(skin) if skin.complete() => Ok(skin),
            _ => Skin::from_archive(&bytes, Some(&base()?)),
        }
    });
    match chosen {
        Some(Ok(skin)) => Ok((skin, None)),
        Some(Err(why)) => Ok((base()?, Some(format!("Couldn't load the skin: {why}")))),
        None => Ok((base()?, None)),
    }
}

/// Another skin from the Options menu: fetched off the main thread, then on.
pub fn change_skin(app: &AppHandle, origin: Url, token: String, url: String) {
    let app = app.clone();
    let spawned = std::thread::Builder::new().name("classic-skin".into()).spawn(move || {
        let skin = load_skin(&origin, &token, Some(SkinSource::Hub(url)));
        let shown = app.run_on_main_thread(move || window::set_skin(skin));
        if let Err(e) = shown {
            eprintln!("lull: Classic skin: {e}");
        }
    });
    if let Err(e) = spawned {
        eprintln!("lull: Classic skin: {e}");
    }
}

fn fetch(url: &str, token: &str) -> Result<Vec<u8>, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .build(),
        )
        .build()
        .into();
    let response = agent
        .get(url)
        .header("Authorization", format!("Bearer {token}"))
        .call()
        .map_err(|e| format!("can't fetch the skin: {e}"))?;
    let mut bytes = Vec::new();
    response
        .into_body()
        .into_reader()
        .take(MAX_SKIN + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("can't fetch the skin: {e}"))?;
    if bytes.len() as u64 > MAX_SKIN {
        return Err("the skin is bigger than 16 MB".into());
    }
    Ok(bytes)
}

/// The size and place of the window: the listener's size, or the largest smaller one that fits the
/// work area of the monitor `main` is on (classic::fit's rule), centred there; the monitor's own
/// scale goes on top. The windows stacked are three of the main one's height.
fn place(main: &WebviewWindow, want: f64) -> Result<(f64, f64, (i32, i32)), String> {
    let monitor = main
        .current_monitor()
        .map_err(|e| e.to_string())?
        .ok_or("lull's window is on no monitor")?;
    let area = *monitor.work_area();
    let dpi = monitor.scale_factor();
    let physical = |size: f64| {
        (
            f64::from(skin::WIDTH) * size * dpi,
            f64::from(3 * skin::HEIGHT) * size * dpi,
        )
    };
    let fits = |&size: &f64| {
        let (w, h) = physical(size);
        w <= f64::from(area.size.width) && h <= f64::from(area.size.height)
    };
    let size = SIZES
        .iter()
        .rev()
        .filter(|&&s| s <= want)
        .find(|s| fits(s))
        .copied()
        .unwrap_or(1.0);
    let (w, h) = physical(size);
    let x = area.position.x + ((f64::from(area.size.width) - w) / 2.0) as i32;
    let y = area.position.y + ((f64::from(area.size.height) - h) / 2.0) as i32;
    Ok((size, dpi, (x, y)))
}
