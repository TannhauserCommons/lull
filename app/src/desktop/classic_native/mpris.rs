//! Linux's media keys and players' panel for Classic's own player: MPRIS on the session bus
//! (`org.mpris.MediaPlayer2.lull`), as a browser's Media Session gives the web UI; with no webview,
//! the player asks the desktop itself. The panel shows the song, its album and cover, and whether
//! it plays; its buttons and the keyboard's media keys come back as the tray's actions.

use super::hub::Hub;
use crate::desktop::state::NowPlaying;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use tauri::AppHandle;
use zbus::zvariant::{ObjectPath, OwnedValue, Value};

const PATH: &str = "/org/mpris/MediaPlayer2";
/// No cover is bigger than this.
const MAX_ART: u64 = 8 * 1024 * 1024;
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";

/// What the player shows: the song and whether it plays, and its cover's file.
#[derive(Default)]
struct Shown {
    now: Option<NowPlaying>,
    art: Option<String>,
}

struct Root {
    app: AppHandle,
}

#[zbus::interface(name = "org.mpris.MediaPlayer2")]
impl Root {
    fn raise(&self) {
        if let Err(e) = super::show(&self.app) {
            eprintln!("lull: media panel's raise: {e}");
        }
    }

    fn quit(&self) {}

    #[zbus(property)]
    fn can_quit(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn can_raise(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn has_track_list(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn identity(&self) -> String {
        "lull".into()
    }

    #[zbus(property)]
    fn supported_uri_schemes(&self) -> Vec<String> {
        Vec::new()
    }

    #[zbus(property)]
    fn supported_mime_types(&self) -> Vec<String> {
        Vec::new()
    }
}

struct Player {
    app: AppHandle,
    shown: Arc<Mutex<Shown>>,
}

impl Player {
    fn act(&self, action: &str) {
        if let Err(e) = super::act(&self.app, action) {
            eprintln!("lull: media key {action}: {e}");
        }
    }
}

#[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
impl Player {
    fn next(&self) {
        self.act("next");
    }

    fn previous(&self) {
        self.act("previous");
    }

    fn pause(&self) {
        self.act("pause");
    }

    fn play_pause(&self) {
        self.act("play-pause");
    }

    fn stop(&self) {
        self.act("stop");
    }

    fn play(&self) {
        self.act("play");
    }

    fn seek(&self, _offset: i64) {}

    fn set_position(&self, _track: ObjectPath<'_>, _position: i64) {}

    fn open_uri(&self, _uri: String) {}

    #[zbus(property)]
    fn playback_status(&self) -> String {
        status(&lock(&self.shown))
    }

    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, OwnedValue> {
        metadata(&lock(&self.shown))
    }

    #[zbus(property)]
    fn rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn minimum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn maximum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn volume(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn position(&self) -> i64 {
        0
    }

    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_play(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_pause(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_seek(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn can_control(&self) -> bool {
        true
    }
}

fn lock(shown: &Mutex<Shown>) -> std::sync::MutexGuard<'_, Shown> {
    shown.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// MPRIS's playback status: Stopped with nothing loaded.
fn status(shown: &Shown) -> String {
    match &shown.now {
        Some(now) if now.playing => "Playing",
        Some(_) => "Paused",
        None => "Stopped",
    }
    .into()
}

/// MPRIS's metadata for the song: its id as a D-Bus path, title, artist, album, cover.
fn metadata(shown: &Shown) -> HashMap<String, OwnedValue> {
    let mut map = HashMap::new();
    let Some(now) = &shown.now else {
        return map;
    };
    let id = plain(&now.id);
    let mut put = |key: &str, value: Value<'_>| {
        if let Ok(value) = OwnedValue::try_from(value) {
            map.insert(key.to_owned(), value);
        }
    };
    if let Ok(path) = ObjectPath::try_from(format!("/io/github/tannhausercommons/lull/track/{id}")) {
        put("mpris:trackid", Value::from(path));
    }
    put("xesam:title", Value::from(now.title.as_str()));
    put("xesam:artist", Value::from(vec![now.artist.as_str()]));
    put("xesam:album", Value::from(now.album.as_str()));
    if let Some(art) = &shown.art {
        put("mpris:artUrl", Value::from(art.as_str()));
    }
    map
}

#[derive(Clone)]
pub struct Overlay {
    connection: zbus::blocking::Connection,
    shown: Arc<Mutex<Shown>>,
}

impl Overlay {
    pub fn new(app: AppHandle) -> zbus::Result<Self> {
        let shown = Arc::new(Mutex::new(Shown::default()));
        let connection = zbus::blocking::connection::Builder::session()?
            .name("org.mpris.MediaPlayer2.lull")?
            .serve_at(PATH, Root { app: app.clone() })?
            .serve_at(
                PATH,
                Player {
                    app,
                    shown: shown.clone(),
                },
            )?
            .build()?;
        Ok(Self { connection, shown })
    }

    /// Tells the desktop what changed: the status, the song.
    fn changed(&self) -> zbus::Result<()> {
        let (status, metadata) = {
            let shown = lock(&self.shown);
            (status(&shown), metadata(&shown))
        };
        let mut changed: HashMap<&str, Value<'_>> = HashMap::new();
        changed.insert("PlaybackStatus", Value::from(status));
        changed.insert("Metadata", Value::from(metadata));
        self.connection.emit_signal(
            None::<()>,
            PATH,
            "org.freedesktop.DBus.Properties",
            "PropertiesChanged",
            &(PLAYER, changed, Vec::<String>::new()),
        )
    }

    /// What the panel shows: the song, and whether it plays. A new song's cover comes from the
    /// hub on a thread of its own, into a file the panel reads (MPRIS takes covers as URLs).
    pub fn show(&self, now: &NowPlaying, art: Option<&str>, hub: &Hub) -> zbus::Result<()> {
        let new = {
            let mut shown = lock(&self.shown);
            let new = shown.now.as_ref().is_none_or(|was| was.id != now.id);
            if new {
                shown.art = None;
            }
            shown.now = Some(now.clone());
            new
        };
        self.changed()?;
        let Some(url) = art.filter(|_| new) else {
            return Ok(());
        };
        let (overlay, id, url, hub) = (self.clone(), now.id.clone(), url.to_owned(), hub.clone());
        let spawned = std::thread::Builder::new().name("classic-art".into()).spawn(move || {
            let shown = cover(&hub, &url, &id).map_err(zbus::Error::Failure).and_then(|file| {
                {
                    let mut shown = lock(&overlay.shown);
                    if shown.now.as_ref().is_none_or(|now| now.id != id) {
                        return Ok(());
                    }
                    shown.art = Some(file);
                }
                overlay.changed()
            });
            if let Err(e) = shown {
                eprintln!("lull: no cover in the media panel: {e}");
            }
        });
        if let Err(e) = spawned {
            eprintln!("lull: no cover in the media panel: {e}");
        }
        Ok(())
    }
}

/// The cover from the hub, in a file of the system's temporary folder named for the song (the
/// last song's goes), as a `file://` URL.
fn cover(hub: &Hub, url: &str, id: &str) -> Result<String, String> {
    let mut bytes = Vec::new();
    hub.get(url)?
        .take(MAX_ART)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let folder = std::env::temp_dir();
    if let Ok(files) = std::fs::read_dir(&folder) {
        for file in files.flatten() {
            if file.file_name().to_string_lossy().starts_with(COVER) {
                let _ = std::fs::remove_file(file.path());
            }
        }
    }
    let path = folder.join(format!("{COVER}{}", plain(id)));
    let mut file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
    file.write_all(&bytes).map_err(|e| e.to_string())?;
    Ok(format!("file://{}", path.display()))
}

/// The start of the covers' files' names.
const COVER: &str = "lull-classic-cover-";

/// An id with only letters and digits, for a D-Bus path or a file's name.
fn plain(id: &str) -> String {
    id.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}
