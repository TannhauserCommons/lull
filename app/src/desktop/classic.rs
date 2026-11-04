//! Classic mode on the desktop: Webamp alone in its own borderless, transparent window (`classic`),
//! like Winamp itself, while `main` hides and goes quiet. The web UI hands over its queue
//! (`classic_open`), the window's page takes it (`classic_handoff`), and when Webamp closes the page
//! hands the queue back to `main` and says `classic-done`. DESKTOP.md 9 (docs/context/desktop.md).

use crate::desktop::links;
use crate::desktop::main_window;
use crate::desktop::platform;
use crate::desktop::recovery;
use crate::desktop::state::{Desktop, lock};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Listener, Manager, PhysicalPosition, PhysicalSize, Url, WebviewUrl, WebviewWindow};

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
    pub(crate) tracks: Vec<Track>,
    pub(crate) index: usize,
    pub(crate) pos: f64,
    pub(crate) playing: bool,
    pub(crate) shuffle: bool,
    pub(crate) repeat: bool,
    pub(crate) volume: f64,
    /// The listener's Classic skin, and their favourites for Webamp's Skins menu.
    pub(crate) skin: Option<Skin>,
    pub(crate) skins: Vec<Skin>,
    /// The listener's Classic size, and the page Classic was opened from (`#/library…`): the
    /// native window (classic_native) takes its size here, and `main` comes back on that page.
    #[serde(default)]
    pub(crate) size: Option<f64>,
    #[serde(default)]
    pub(crate) page: Option<String>,
    /// The device's equaliser (`lull:eq`), which the native window plays through and hands back.
    #[serde(default)]
    pub(crate) eq: Option<Equaliser>,
    /// The skin's file, base64, when it is kept on the device (the page's storage) and not the hub.
    #[serde(default)]
    pub(crate) skin_data: Option<String>,
    /// The page's repeat (`off`, `all`, `one`; `repeat` above is Webamp's on or off), its radio and
    /// where the listening started, as it keeps them.
    #[serde(default)]
    pub(crate) repeat_mode: Option<String>,
    #[serde(default)]
    pub(crate) radio: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) context: Option<serde_json::Value>,
    /// Where native Classic's windows were when it last closed (classic_native's `Layout`), as the
    /// device keeps it; Webamp's window has no use for it.
    #[serde(default)]
    pub(crate) layout: Option<serde_json::Value>,
}

/// The web UI's equaliser settings (web/eq.js): on, ten bands in dB, and a hand-set preamp or
/// `None` for the automatic level.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) struct Equaliser {
    pub(crate) on: bool,
    pub(crate) bands: [f64; 10],
    pub(crate) preamp: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Track {
    pub(crate) id: String,
    pub(crate) url: String,
    pub(crate) title: String,
    pub(crate) artist: String,
    pub(crate) album: String,
    pub(crate) duration: Option<f64>,
    /// Cover art for the system's media overlay.
    pub(crate) art: Option<String>,
    /// What the native window reports and records (the tray, Discord, listening history).
    #[serde(default)]
    pub(crate) artist_id: String,
    #[serde(default)]
    pub(crate) album_id: String,
    #[serde(default)]
    pub(crate) liked: bool,
    #[serde(default)]
    pub(crate) saved: bool,
    /// Played from radio or a feed pick (listening history's hit rate).
    #[serde(default)]
    pub(crate) recommended: bool,
    /// Its queue entry as the page keeps it (`context`, `next`, `radio`; a picked copy; its place
    /// before shuffling) and its ReplayGain for Settings › Playback's mode: the native window plays
    /// the queue by the page's rules and hands it back in the page's shape.
    #[serde(default)]
    pub(crate) group: Option<String>,
    #[serde(default)]
    pub(crate) source: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) orig: Option<usize>,
    #[serde(default)]
    pub(crate) gain_db: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Skin {
    pub(crate) url: String,
    pub(crate) name: String,
}

/// The handoff while the Classic window is open. `taken` once its page has it; it stays, following
/// what Webamp plays, so a page that reloads after a crash starts where Webamp was (recovery.rs).
pub struct Held {
    handoff: Handoff,
    taken: bool,
    /// When `handoff.pos` was true: Webamp reports a position only as a song starts, pauses or seeks.
    at: Instant,
}

impl Held {
    fn new(handoff: Handoff) -> Self {
        Held {
            handoff,
            taken: false,
            at: Instant::now(),
        }
    }

    fn follow(&mut self, id: &str, playing: bool, pos: f64) {
        self.handoff.follow(id, playing, pos);
        self.at = Instant::now();
    }

    /// The handoff as Webamp has it now: a playing song has moved on since its last report.
    fn now(&self) -> Handoff {
        self.handoff.since(self.at.elapsed())
    }
}

/// What the Classic window's page tells `main` it plays (`web/desktop/winamp.js`).
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum Report {
    Now {
        id: Option<String>,
        playing: bool,
        pos: f64,
    },
    #[serde(other)]
    Other,
}

impl Handoff {
    /// Webamp moved on: the song it plays (the first time it is in the list), where, and whether.
    fn follow(&mut self, id: &str, playing: bool, pos: f64) {
        if let Some(index) = self.tracks.iter().position(|t| t.id == id) {
            self.index = index;
        }
        self.pos = pos;
        self.playing = playing;
    }

    /// Where a playing song has got to `elapsed` after its position was reported, never past its end.
    fn since(&self, elapsed: Duration) -> Handoff {
        let mut now = self.clone();
        if now.playing {
            now.pos += elapsed.as_secs_f64();
            if let Some(length) = now.tracks.get(now.index).and_then(|t| t.duration) {
                now.pos = now.pos.min(length);
            }
        }
        now
    }

    /// The queue as `main` takes it back from Webamp (`web/screens/classic.js`, takeBack).
    fn back(&self) -> serde_json::Value {
        serde_json::json!({
            "ids": self.tracks.iter().map(|t| &t.id).collect::<Vec<_>>(),
            "index": self.index,
            "pos": self.pos,
            "playing": self.playing,
            "shuffle": self.shuffle,
            "repeat": self.repeat,
            "local": 0,
        })
    }

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
    #[cfg(any(windows, target_os = "linux"))]
    if crate::desktop::classic_native::is_open() {
        return crate::desktop::classic_native::show(app).map_err(|e| e.to_string());
    }
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
    #[cfg(any(windows, target_os = "linux"))]
    if crate::desktop::classic_native::chosen() {
        return crate::desktop::classic_native::open(app, handoff, main);
    }
    let page = Url::parse(&hub.ascii_serialization())
        .and_then(|origin| origin.join(PAGE))
        .map_err(|e| e.to_string())?;
    *lock(&desktop.classic) = Some(Held::new(handoff));
    build(app, page).map_err(|e| e.to_string())?;
    main_window::sleep(app).map_err(|e| e.to_string())
}

fn build(app: &AppHandle, page: Url) -> tauri::Result<()> {
    let (width, height) = FIRST_SIZE;
    let classic = recovery::builder(app, LABEL, WebviewUrl::External(page))
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
    recovery::watch(&classic);
    Ok(())
}

/// Keeps the held handoff where Webamp is, from the reports its page sends `main`.
pub fn follow(app: &AppHandle) {
    let handle = app.clone();
    app.listen_any("classic", move |event| {
        let Ok(Report::Now {
            id: Some(id),
            playing,
            pos,
        }) = serde_json::from_str(event.payload())
        else {
            return;
        };
        if let Some(held) = lock(&handle.state::<Desktop>().classic).as_mut() {
            held.follow(&id, playing, pos);
        }
    });
}

/// Webamp's page asks for its size: Webamp measures `width` x `height` at 1x, and `zooms` are the
/// sizes it may be drawn at, the listener's own first. The window takes the first that fits the work
/// area of the monitor it is on (the last if none does), zooms its webview to it (page zoom, so
/// Webamp's pointer math and its pixelated skin stay right), fits Webamp exactly and stays inside
/// that work area. Returns the zoom it took.
pub fn fit(window: &WebviewWindow, width: f64, height: f64, zooms: &[f64]) -> Result<f64, String> {
    let valid = |v: f64| v.is_finite() && v > 0.0;
    if !valid(width) || !valid(height) || width > 10_000.0 || height > 10_000.0 {
        return Err(format!("Webamp can't be {width} x {height}"));
    }
    if zooms.is_empty() || zooms.iter().any(|z| !(0.5..=4.0).contains(z)) {
        return Err(format!("unknown Classic sizes {zooms:?}"));
    }
    let monitor = window.current_monitor().map_err(|e| e.to_string())?;
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let physical = |zoom: f64| ((width * zoom * scale).ceil(), (height * zoom * scale).ceil());
    let area = monitor.map(|m| *m.work_area());
    let fits = |zoom: f64| {
        area.is_none_or(|a| {
            let (w, h) = physical(zoom);
            w <= f64::from(a.size.width) && h <= f64::from(a.size.height)
        })
    };
    let zoom = zooms
        .iter()
        .copied()
        .find(|&z| fits(z))
        .or(zooms.last().copied())
        .ok_or("no Classic size")?;
    let (w, h) = physical(zoom);
    window.set_zoom(zoom).map_err(|e| e.to_string())?;
    window
        .set_size(PhysicalSize::new(w as u32, h as u32))
        .map_err(|e| e.to_string())?;
    if let Some(a) = area {
        let at = window.outer_position().map_err(|e| e.to_string())?;
        let inside = |pos: i32, len: f64, start: i32, room: u32| {
            let end = start + i32::try_from(room).unwrap_or(i32::MAX) - len as i32;
            pos.min(end).max(start)
        };
        let x = inside(at.x, w, a.position.x, a.size.width);
        let y = inside(at.y, h, a.position.y, a.size.height);
        if (x, y) != (at.x, at.y) {
            window
                .set_position(PhysicalPosition::new(x, y))
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(zoom)
}

/// The window's page: what to play. Again, as Webamp last had it, if the page reloaded.
pub fn take_handoff(app: &AppHandle) -> Option<Handoff> {
    lock(&app.state::<Desktop>().classic).as_mut().map(|held| {
        held.taken = true;
        held.now()
    })
}

/// Back to lull (its close button, Alt+F4, the tray's Show lull): the page hands the queue back and
/// then says `classic-done`. A page that never took its handoff has nothing to hand back.
pub fn close(app: &AppHandle) -> tauri::Result<()> {
    let Some(classic) = window(app) else {
        return Ok(());
    };
    if lock(&app.state::<Desktop>().classic)
        .as_ref()
        .is_some_and(|held| !held.taken)
    {
        return done(app);
    }
    classic.emit_to(LABEL, "classic-close", ())
}

/// The window's page keeps crashing: `main` takes the queue back as Webamp last had it, as the page
/// itself would have handed it, then Webamp goes.
pub fn hand_back(app: &AppHandle) -> tauri::Result<()> {
    let back = lock(&app.state::<Desktop>().classic)
        .as_ref()
        .map(|held| held.now().back());
    if let Some(back) = back {
        app.emit_to(
            main_window::LABEL,
            "classic",
            serde_json::json!({ "type": "back", "back": back }),
        )?;
    }
    done(app)
}

/// The page handed the queue back: Webamp goes, `main` wakes.
pub fn done(app: &AppHandle) -> tauri::Result<()> {
    lock(&app.state::<Desktop>().classic).take();
    if let Some(classic) = window(app) {
        classic.destroy()?;
    }
    main_window::wake(app)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handoff(playing: bool) -> Handoff {
        let track = |id: &str| Track {
            id: id.into(),
            url: format!("http://hub/{id}"),
            title: id.into(),
            artist: String::new(),
            album: String::new(),
            duration: Some(200.0),
            art: None,
            artist_id: String::new(),
            album_id: String::new(),
            liked: false,
            saved: false,
            recommended: false,
            group: None,
            source: None,
            orig: None,
            gain_db: None,
        };
        Handoff {
            tracks: vec![track("a"), track("b")],
            index: 0,
            pos: 10.0,
            playing,
            shuffle: false,
            repeat: false,
            volume: 1.0,
            skin: None,
            skins: Vec::new(),
            size: None,
            page: None,
            eq: None,
            skin_data: None,
            repeat_mode: None,
            radio: None,
            context: None,
            layout: None,
        }
    }

    #[test]
    fn a_reloaded_classic_window_starts_on_the_song_webamp_reported_and_where_it_has_got_to() {
        let mut h = handoff(true);
        h.follow("b", true, 50.0);
        let now = h.since(Duration::from_secs(30));
        assert_eq!((now.index, now.pos, now.playing), (1, 80.0, true));
    }

    #[test]
    fn a_paused_song_stays_where_it_stopped_and_a_playing_one_never_runs_past_its_end() {
        assert_eq!(handoff(false).since(Duration::from_secs(30)).pos, 10.0);
        assert_eq!(handoff(true).since(Duration::from_secs(600)).pos, 200.0);
    }

    #[test]
    fn main_takes_back_the_whole_list_at_webamps_song() {
        let mut h = handoff(true);
        h.follow("b", false, 42.0);
        assert_eq!(
            h.back(),
            serde_json::json!({ "ids": ["a", "b"], "index": 1, "pos": 42.0, "playing": false, "shuffle": false, "repeat": false, "local": 0 })
        );
    }
}
