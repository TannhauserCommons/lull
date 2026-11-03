//! Classic mode without a webview (Windows; a test build chosen with `LULL_CLASSIC=native`): the
//! skin's main window drawn natively (window.rs, draw.rs, skin.rs) and its own player (sound.rs),
//! so `main`'s webview can close and WebView2 with it while Classic plays. Leaving Classic builds
//! `main` again on the page it was on, and the page resumes the song where Classic left it
//! (`resume_after_crash`, as after a crash). The equaliser and playlist windows, menus, shade,
//! plays, likes, media keys, the taskbar buttons and Discord aren't here yet.

mod draw;
mod skin;
mod sound;
mod vis;
mod window;

use crate::desktop::classic::Handoff;
use crate::desktop::main_window;
use crate::desktop::recovery::{self, Resume};
use crate::desktop::state::Desktop;
use skin::Skin;
use sound::{Player, Song, Start};
use std::io::Read;
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
    let skin = load_skin(&origin, &token, handoff.skin.as_ref().map(|s| s.url.as_str()))?;
    let (scale, at) = place(&main, handoff.size.unwrap_or(1.0))?;
    let titles = handoff
        .tracks
        .iter()
        .enumerate()
        .map(|(i, t)| title_line(i, &t.artist, &t.title, t.duration))
        .collect();
    let songs = handoff
        .tracks
        .iter()
        .map(|t| Song {
            url: t.url.clone(),
            duration: t.duration,
        })
        .collect();
    let start = Start {
        index: handoff.index.min(handoff.tracks.len().saturating_sub(1)),
        position: handoff.pos,
        playing: handoff.playing,
        volume: handoff.volume,
        repeat: handoff.repeat,
    };
    let player = Player::start(token.clone(), songs, start)?;
    let setup = window::Setup {
        skin,
        player,
        titles,
        scale,
        at,
        shuffle: handoff.shuffle,
        repeat: handoff.repeat,
    };
    let back = Back {
        app: app.clone(),
        origin,
        token,
        page: handoff.page.clone(),
        ids: handoff.tracks.iter().map(|t| t.id.clone()).collect(),
    };
    let (opened, result) = std::sync::mpsc::channel();
    app.run_on_main_thread(move || {
        let on_close: window::OnClose = Box::new(move |player| back.go(player));
        let _ = opened.send(window::open(setup, on_close));
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

/// The way back to lull: its hub, the page Classic was opened from, and the queue's songs.
struct Back {
    app: AppHandle,
    origin: Url,
    token: String,
    page: Option<String>,
    ids: Vec<String>,
}

impl Back {
    /// The window has gone: `main` comes back on its page, and its page resumes the song where
    /// Classic had it. Off the main thread: building a webview there would wait on itself.
    fn go(self, player: &Player) {
        OPEN.store(false, Ordering::Relaxed);
        let now = player.now();
        if let Some(id) = self.ids.get(now.index) {
            let resume = Resume {
                id: id.clone(),
                position: now.position,
                playing: now.playing,
            };
            recovery::resume_on_load(&self.app, resume);
        }
        let mut url = self.origin.clone();
        url.set_fragment(Some(
            self.page.as_deref().filter(|p| p.starts_with('/')).unwrap_or("/home"),
        ));
        let spawned = std::thread::Builder::new()
            .name("classic-back".into())
            .spawn(move || main_window::report(main_window::open_hub(&self.app, &url, &self.token)));
        if let Err(e) = spawned {
            eprintln!("lull: can't bring lull back from Classic: {e}");
        }
    }
}

/// The listener's skin, or Webamp's base skin; a skin missing some of the main window's sheets has
/// them filled from the base skin.
fn load_skin(origin: &Url, token: &str, url: Option<&str>) -> Result<Skin, String> {
    let base_url = origin.join(BASE_SKIN).map_err(|e| e.to_string())?;
    let base = || fetch(base_url.as_str(), token).and_then(|bytes| Skin::from_archive(&bytes, None));
    let Some(url) = url else {
        return base();
    };
    let bytes = fetch(url, token)?;
    let skin = Skin::from_archive(&bytes, None);
    match skin {
        Ok(skin) if skin.complete() => Ok(skin),
        _ => Skin::from_archive(&bytes, Some(&base()?)),
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
/// work area of the monitor `main` is on (classic::fit's rule), centred there. In physical pixels,
/// with the monitor's own scale on top of Classic's size.
fn place(main: &WebviewWindow, want: f64) -> Result<(f64, (i32, i32)), String> {
    let monitor = main
        .current_monitor()
        .map_err(|e| e.to_string())?
        .ok_or("lull's window is on no monitor")?;
    let area = *monitor.work_area();
    let dpi = monitor.scale_factor();
    let physical = |size: f64| {
        (
            f64::from(skin::WIDTH) * size * dpi,
            f64::from(skin::HEIGHT) * size * dpi,
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
    Ok((size * dpi, (x, y)))
}

/// Winamp's marquee line: `3. Artist - Title (4:05)`.
fn title_line(index: usize, artist: &str, title: &str, duration: Option<f64>) -> String {
    let name = if artist.is_empty() {
        title.to_owned()
    } else {
        format!("{artist} - {title}")
    };
    match duration {
        Some(d) => format!("{}. {name} ({}:{:02})", index + 1, d as u32 / 60, d as u32 % 60),
        None => format!("{}. {name}", index + 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_marquee_names_the_song_the_way_winamp_did() {
        assert_eq!(
            title_line(2, "Air", "La Femme d'Argent", Some(431.0)),
            "3. Air - La Femme d'Argent (7:11)"
        );
        assert_eq!(title_line(0, "", "Untitled", None), "1. Untitled");
    }
}
