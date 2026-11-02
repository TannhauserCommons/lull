//! Discord Rich Presence, made to look like Spotify's: "Listening to <song>" in the member list,
//! and on the profile the song (linked to song.link), the artist, the album's cover with its name
//! on hover, lull's glass badge, the progress bar and up to two buttons. Over Discord's local IPC
//! pipe, as lull's own Discord application, so it sits beside a game's activity and never replaces
//! it. Off until the listener turns on "Show what I'm playing on Discord" (Settings > App), where
//! the rest of its look is chosen too.
//!
//! One thread owns the connection: it waits for the player to settle (skips and seeks come in
//! bursts), sends only when what Discord would show changed, connects when there is something to
//! show, tries again quietly while Discord isn't running, and hangs up when the setting goes off.

use crate::hub::{self, Share, ShareSource};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;
use tauri::Url;

/// lull's application on Discord (discord.com/developers/applications > General Information >
/// Application ID). Public, not a secret: every lull uses it, and each listener's own Discord
/// client shows their presence. None: Settings doesn't offer the switch.
pub const APPLICATION_ID: Option<&str> = Some("1566921302416171189");

/// The pictures, served from lull's public repository so nothing has to be uploaded to Discord,
/// which fetches them itself (it can't reach the hub). Sources and how they are made: `app/discord/`.
macro_rules! picture {
    ($name:literal) => {
        concat!(
            "https://raw.githubusercontent.com/TannhauserCommons/lull/main/app/discord/",
            $name
        )
    };
}
/// The small round badge: lull's glass orb, and its paused twin.
const BADGE: &str = picture!("badge.png");
const BADGE_PAUSED: &str = picture!("badge-paused.png");
/// The large picture for a song without a public cover, or with covers turned off.
const COVER: &str = picture!("cover.png");
/// The "Get lull" button.
const GET_LULL: &str = "https://github.com/TannhauserCommons/lull";

/// While Discord isn't running, look for it this often (only while there is something to show).
const RETRY: Duration = Duration::from_secs(20);
/// A burst of player changes (skipping through songs, dragging the seek bar) is over once it has
/// been quiet this long; Discord allows 5 activity changes per 20 s.
const SETTLE: Duration = Duration::from_millis(800);
/// A song whose start moved less than this (a tiny seek, timer jitter) shows the same.
const SAME_START_MS: i64 = 2000;

/// Discord drops texts shorter or longer than this, and links longer than 512.
const TEXT: std::ops::RangeInclusive<usize> = 2..=128;
const MAX_URL: usize = 512;

/// How the presence looks (Settings > App), kept with the desktop's settings.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Look {
    /// What follows "Listening to" in the member list.
    pub status: StatusLine,
    /// The album's cover as the large picture; off, lull's sky picture.
    pub cover: bool,
    /// "Open on song.link" and "Get lull" (others see them; Discord hides them from the listener).
    pub buttons: bool,
    pub paused: WhenPaused,
}

impl Default for Look {
    fn default() -> Self {
        Look {
            status: StatusLine::Song,
            cover: true,
            buttons: true,
            paused: WhenPaused::Clear,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StatusLine {
    /// "Listening to Paper Moon Static"
    Song,
    /// "Listening to Velvet Orchard"
    Artist,
    /// "Listening to lull" (the application's name)
    Lull,
}

impl StatusLine {
    /// Discord's `status_display_type`: 0 the application's name, 1 `state`, 2 `details`.
    fn display_type(self) -> u8 {
        match self {
            StatusLine::Lull => 0,
            StatusLine::Artist => 1,
            StatusLine::Song => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WhenPaused {
    /// The presence goes away, as Spotify's does.
    Clear,
    /// It stays, with the paused badge and no progress bar.
    Show,
}

/// The song loaded now, as `player::refresh` sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct Listening {
    pub id: String,
    pub artist_id: String,
    pub album_id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub playing: bool,
    /// Unix milliseconds: when the song started (now minus the position), and when it will end.
    pub start_ms: i64,
    pub end_ms: Option<i64>,
}

/// What the thread should show.
enum Want {
    /// The setting is off: clear and hang up.
    Off,
    /// Nothing is loaded: clear, stay connected.
    Nothing,
    Song(Box<Request>),
}

/// A song, how to show it, and the hub (origin and token) to ask for its share links.
struct Request {
    song: Listening,
    look: Look,
    hub: Option<(Url, String)>,
}

pub struct Presence {
    tx: Option<Sender<Want>>,
}

impl Presence {
    /// Without an Application ID there is nothing to start.
    pub fn start() -> Self {
        let Some(client_id) = APPLICATION_ID else {
            return Presence { tx: None };
        };
        let (tx, rx) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("discord".into())
            .spawn(move || run(client_id, &rx));
        if let Err(e) = spawned {
            eprintln!("lull: Discord presence didn't start: {e}");
            return Presence { tx: None };
        }
        Presence { tx: Some(tx) }
    }

    /// Whether this build has a Discord application to show presence as.
    pub fn available() -> bool {
        APPLICATION_ID.is_some()
    }

    /// The latest state; the thread catches up with only the newest.
    pub fn show(&self, enabled: bool, look: Look, song: Option<Listening>, hub: Option<(Url, String)>) {
        let want = match (enabled, song) {
            (false, _) => Want::Off,
            (true, None) => Want::Nothing,
            (true, Some(song)) => Want::Song(Box::new(Request { song, look, hub })),
        };
        if let Some(tx) = &self.tx {
            let _ = tx.send(want);
        }
    }
}

fn run(client_id: &str, rx: &Receiver<Want>) {
    let mut want = Want::Off;
    let mut ipc: Option<Ipc> = None;
    // What Discord shows now (None: nothing).
    let mut shown: Option<Value> = None;
    let mut shares = Shares::default();
    loop {
        // Waiting for Discord to start: wake up now and then to look for it.
        let waiting = ipc.is_none() && matches!(want, Want::Song(..));
        let next = if waiting {
            rx.recv_timeout(RETRY)
        } else {
            rx.recv().map_err(|_| RecvTimeoutError::Disconnected)
        };
        match next {
            Ok(w) => want = w,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        // Let a burst settle: only its last state counts.
        loop {
            match rx.recv_timeout(SETTLE) {
                Ok(w) => want = w,
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
        let wanted = match &want {
            Want::Off => {
                if let Some(mut c) = ipc.take() {
                    let _ = c.set_activity(Value::Null);
                }
                shown = None;
                continue;
            }
            Want::Nothing => None,
            Want::Song(r) => activity(&r.song, r.look, &shares.links(r)),
        };
        if ipc.is_some() && same(shown.as_ref(), wanted.as_ref()) {
            continue;
        }
        if ipc.is_none() {
            if wanted.is_none() {
                continue; // nothing to clear on a Discord we aren't talking to
            }
            // Not running (or not yet): quietly, until next time.
            ipc = Ipc::connect(client_id).ok();
            shown = None;
        }
        if let Some(c) = ipc.as_mut() {
            let result = c.set_activity(wanted.clone().unwrap_or(Value::Null));
            send(&mut ipc, result);
            shown = wanted;
        }
    }
}

/// A failed write or read means Discord went away: drop the pipe and connect again next time.
fn send(ipc: &mut Option<Ipc>, result: io::Result<()>) {
    if let Err(e) = result {
        if e.kind() == io::ErrorKind::InvalidData {
            eprintln!("lull: Discord refused the activity: {e}");
        } else {
            *ipc = None;
        }
    }
}

/// Whether Discord would show the same: equal apart from a start or end moved by under 2 s.
fn same(a: Option<&Value>, b: Option<&Value>) -> bool {
    let (Some(a), Some(b)) = (a, b) else {
        return a.is_none() && b.is_none();
    };
    let without_times = |v: &Value| {
        let mut v = v.clone();
        if let Some(o) = v.as_object_mut() {
            o.remove("timestamps");
        }
        v
    };
    let near = |key: &str| {
        let (x, y) = (&a["timestamps"][key], &b["timestamps"][key]);
        match (x.as_i64(), y.as_i64()) {
            (Some(x), Some(y)) => (x - y).abs() < SAME_START_MS,
            _ => x == y,
        }
    };
    without_times(a) == without_times(b) && near("start") && near("end")
}

/// The public links for a song: its own (song.link), its artist's and its album's.
#[derive(Default)]
struct Links {
    track: Option<Share>,
    artist: Option<Share>,
    album: Option<Share>,
}

/// What Discord shows for a song, or None when it shows nothing (paused, set to clear). Type 2 is
/// "Listening to"; `status_display_type` picks the member list's words.
fn activity(song: &Listening, look: Look, links: &Links) -> Option<Value> {
    if !song.playing && look.paused == WhenPaused::Clear {
        return None;
    }
    let url = |share: &Option<Share>| share.as_ref().map(|s| s.url.clone()).filter(|u| fits(u));
    let cover = look
        .cover
        .then(|| links.track.as_ref().or(links.album.as_ref()))
        .flatten()
        .and_then(|s| s.artwork.clone())
        .filter(|u| fits(u));
    let mut assets = json!({
        "large_image": cover.as_deref().unwrap_or(COVER),
        "large_text": text(if song.album.is_empty() { "lull" } else { &song.album }),
        "small_image": if song.playing { BADGE } else { BADGE_PAUSED },
        "small_text": if song.playing { "lull" } else { "Paused on lull" },
    });
    if let Some(u) = url(&links.album) {
        assets["large_url"] = json!(u);
    }
    let mut activity = json!({
        "type": 2,
        "status_display_type": look.status.display_type(),
        "details": text(&song.title),
        "state": text(&song.artist),
        "assets": assets,
        "instance": false,
    });
    if let Some(u) = url(&links.track) {
        activity["details_url"] = json!(u);
    }
    if let Some(u) = url(&links.artist) {
        activity["state_url"] = json!(u);
    }
    if song.playing {
        let mut timestamps = json!({ "start": song.start_ms });
        if let Some(end) = song.end_ms {
            timestamps["end"] = json!(end);
        }
        activity["timestamps"] = timestamps;
    }
    if look.buttons {
        let mut buttons = Vec::new();
        if let (Some(share), Some(u)) = (&links.track, url(&links.track)) {
            let label = match share.source {
                ShareSource::Songlink => "Open on song.link",
                ShareSource::Lastfm => "Open on Last.fm",
            };
            buttons.push(json!({ "label": label, "url": u }));
        }
        buttons.push(json!({ "label": "Get lull", "url": GET_LULL }));
        activity["buttons"] = json!(buttons);
    }
    Some(activity)
}

fn fits(url: &str) -> bool {
    url.starts_with("https://") && url.len() <= MAX_URL
}

/// Clipped to Discord's limit, and padded with a blank Braille cell when a name is one letter.
fn text(s: &str) -> String {
    let mut out: String = s.chars().take(*TEXT.end()).collect();
    while out.chars().count() < *TEXT.start() {
        out.push('\u{2800}');
    }
    out
}

/// Share links by kind and id for this run. Unknown ids are remembered too; a hub that can't
/// answer now is asked again with the next change.
#[derive(Default)]
struct Shares(HashMap<(&'static str, String), Option<Share>>);

impl Shares {
    fn links(&mut self, r: &Request) -> Links {
        let Some((origin, token)) = &r.hub else {
            return Links::default();
        };
        let mut get = |kind: &'static str, id: &str| self.get(origin, token, kind, id);
        Links {
            track: get("track", &r.song.id),
            artist: get("artist", &r.song.artist_id),
            album: get("album", &r.song.album_id),
        }
    }

    fn get(&mut self, origin: &Url, token: &str, kind: &'static str, id: &str) -> Option<Share> {
        if id.is_empty() {
            return None;
        }
        let key = (kind, id.to_owned());
        if let Some(known) = self.0.get(&key) {
            return known.clone();
        }
        match hub::share(origin, token, kind, id) {
            Ok(share) => {
                self.0.insert(key, share.clone());
                share
            }
            Err(e) => {
                eprintln!("lull: no {kind} share link for Discord: {e}");
                None
            }
        }
    }
}

/// Discord's IPC: frames of an opcode and a JSON body, each length-prefixed (little endian).
struct Ipc {
    pipe: Box<dyn Pipe>,
    nonce: u64,
}

trait Pipe: Read + Write + Send {}
impl<T: Read + Write + Send> Pipe for T {}

const HANDSHAKE: u32 = 0;
const FRAME: u32 = 1;
/// Discord answers a handshake it refuses (an unknown application) with CLOSE.
const CLOSE: u32 = 2;
/// Discord's own frames are small; anything bigger is not Discord.
const MAX_FRAME: u32 = 64 * 1024;

impl Ipc {
    /// The first of Discord's pipes that answers the handshake.
    fn connect(client_id: &str) -> io::Result<Ipc> {
        let mut last = io::Error::new(io::ErrorKind::NotFound, "Discord isn't running");
        for pipe in open_pipes() {
            let mut ipc = Ipc { pipe, nonce: 0 };
            match ipc.handshake(client_id) {
                Ok(()) => return Ok(ipc),
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    fn handshake(&mut self, client_id: &str) -> io::Result<()> {
        self.write(HANDSHAKE, &json!({ "v": 1, "client_id": client_id }))?;
        let (op, body) = self.read()?;
        if op == FRAME && body["evt"] == "READY" {
            Ok(())
        } else {
            Err(io::Error::other(format!("Discord refused lull: {}", body["message"])))
        }
    }

    /// Null clears the activity. Discord's ERROR answer comes back as `InvalidData`.
    fn set_activity(&mut self, activity: Value) -> io::Result<()> {
        self.nonce += 1;
        let command = json!({
            "cmd": "SET_ACTIVITY",
            "args": { "pid": std::process::id(), "activity": activity },
            "nonce": self.nonce.to_string(),
        });
        self.write(FRAME, &command)?;
        let (op, body) = self.read()?;
        match (op, body["evt"].as_str()) {
            (CLOSE, _) => Err(io::Error::new(io::ErrorKind::ConnectionAborted, "Discord closed")),
            (_, Some("ERROR")) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                body["data"]["message"].to_string(),
            )),
            _ => Ok(()),
        }
    }

    fn write(&mut self, op: u32, body: &Value) -> io::Result<()> {
        let body = serde_json::to_vec(body)?;
        let len = u32::try_from(body.len()).map_err(io::Error::other)?;
        let mut frame = Vec::with_capacity(8 + body.len());
        frame.extend_from_slice(&op.to_le_bytes());
        frame.extend_from_slice(&len.to_le_bytes());
        frame.extend_from_slice(&body);
        self.pipe.write_all(&frame)?;
        self.pipe.flush()
    }

    fn read(&mut self) -> io::Result<(u32, Value)> {
        let mut header = [0u8; 8];
        self.pipe.read_exact(&mut header)?;
        let [a, b, c, d, e, f, g, h] = header;
        let (op, len) = (u32::from_le_bytes([a, b, c, d]), u32::from_le_bytes([e, f, g, h]));
        if len > MAX_FRAME {
            return Err(io::Error::other("Discord sent an oversized frame"));
        }
        let mut body = vec![0u8; len as usize];
        self.pipe.read_exact(&mut body)?;
        Ok((op, serde_json::from_slice(&body)?))
    }
}

/// Discord listens on `discord-ipc-0` to `-9`: the first free one per running client.
#[cfg(windows)]
fn open_pipes() -> impl Iterator<Item = Box<dyn Pipe>> {
    (0..10).filter_map(|n| {
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(format!(r"\\?\pipe\discord-ipc-{n}"))
            .ok()
            .map(|f| Box::new(f) as Box<dyn Pipe>)
    })
}

/// In the runtime dir, or a sandbox's corner of it (Flatpak, Snap).
#[cfg(unix)]
fn open_pipes() -> impl Iterator<Item = Box<dyn Pipe>> {
    use std::os::unix::net::UnixStream;
    let base = ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"]
        .iter()
        .find_map(|v| std::env::var(v).ok())
        .unwrap_or_else(|| "/tmp".to_owned());
    let dirs = ["", "app/com.discordapp.Discord/", "snap.discord/"];
    let paths: Vec<String> = dirs
        .iter()
        .flat_map(|dir| {
            (0..10)
                .map(|n| format!("{base}/{dir}discord-ipc-{n}"))
                .collect::<Vec<_>>()
        })
        .collect();
    paths
        .into_iter()
        .filter_map(|path| UnixStream::connect(path).ok().map(|s| Box::new(s) as Box<dyn Pipe>))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(playing: bool) -> Listening {
        Listening {
            id: "t1".into(),
            artist_id: "ar1".into(),
            album_id: "al1".into(),
            title: "Paper Moon Static".into(),
            artist: "Velvet Orchard".into(),
            album: "Hum".into(),
            playing,
            start_ms: 1_000_000,
            end_ms: Some(1_200_000),
        }
    }

    fn share(url: &str, artwork: Option<&str>) -> Option<Share> {
        Some(Share {
            url: url.into(),
            source: ShareSource::Songlink,
            artwork: artwork.map(Into::into),
        })
    }

    fn links() -> Links {
        Links {
            track: share("https://song.link/i/1", Some("https://art.example/1.jpg")),
            artist: share("https://song.link/a/2", None),
            album: share("https://album.link/i/3", None),
        }
    }

    #[test]
    fn a_playing_song_looks_like_spotifys() -> Result<(), String> {
        let a = activity(&song(true), Look::default(), &links()).ok_or("nothing shown")?;
        assert_eq!(a["type"], 2);
        assert_eq!(a["status_display_type"], 2, "the member list names the song");
        assert_eq!(a["details"], "Paper Moon Static");
        assert_eq!(a["details_url"], "https://song.link/i/1");
        assert_eq!(a["state_url"], "https://song.link/a/2");
        assert_eq!(a["assets"]["large_image"], "https://art.example/1.jpg");
        assert_eq!(a["assets"]["large_text"], "Hum");
        assert_eq!(a["assets"]["large_url"], "https://album.link/i/3");
        assert_eq!(a["assets"]["small_image"], BADGE);
        assert_eq!(a["timestamps"]["end"], 1_200_000);
        assert_eq!(a["buttons"][0]["url"], "https://song.link/i/1");
        assert_eq!(a["buttons"][1]["label"], "Get lull");
        Ok(())
    }

    #[test]
    fn the_look_follows_the_settings() -> Result<(), String> {
        let look = Look {
            status: StatusLine::Artist,
            cover: false,
            buttons: false,
            paused: WhenPaused::Show,
        };
        let a = activity(&song(true), look, &Links::default()).ok_or("nothing shown")?;
        assert_eq!(a["status_display_type"], 1);
        assert_eq!(a["assets"]["large_image"], COVER);
        assert!(a.get("buttons").is_none());
        assert!(a.get("details_url").is_none());
        let paused = activity(&song(false), look, &links()).ok_or("paused shows")?;
        assert_eq!(paused["assets"]["small_image"], BADGE_PAUSED);
        assert!(paused.get("timestamps").is_none(), "no progress bar while paused");
        assert!(
            activity(&song(false), Look::default(), &links()).is_none(),
            "paused clears by default"
        );
        Ok(())
    }

    #[test]
    fn texts_fit_discords_limits() {
        assert_eq!(text("X").chars().count(), 2);
        assert_eq!(text(&"a".repeat(300)).chars().count(), 128);
    }

    #[test]
    fn only_a_visible_change_is_sent() {
        let a = activity(&song(true), Look::default(), &links());
        let mut later = song(true);
        later.start_ms += 1500;
        later.end_ms = later.end_ms.map(|e| e + 1500);
        assert!(same(a.as_ref(), activity(&later, Look::default(), &links()).as_ref()));
        later.start_ms += 5000;
        assert!(!same(a.as_ref(), activity(&later, Look::default(), &links()).as_ref()));
        assert!(same(None, None));
        assert!(!same(a.as_ref(), None));
    }
}
