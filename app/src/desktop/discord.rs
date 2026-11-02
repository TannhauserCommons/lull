//! Discord Rich Presence: "Listening to lull" with the song, its artist, album, cover and progress,
//! over Discord's local IPC pipe. Off until the listener turns on "Show what I'm playing on
//! Discord" (Settings > App). One thread owns the connection: it connects when there is a song to
//! show, tries again quietly while Discord isn't running, and hangs up when the setting goes off.

use crate::hub::{self, Share, ShareSource};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;
use tauri::Url;

/// lull's application on Discord (discord.com/developers/applications > the app > General
/// Information > Application ID), e.g. `Some("1234567890123456789")`. Not a secret. None: Settings
/// doesn't offer the switch.
pub const APPLICATION_ID: Option<&str> = None;

/// The cover when the hub has no public one for a song: lull's icon, from its public repository
/// (Discord fetches images itself, and it can't reach the hub).
const FALLBACK_ART: &str =
    "https://raw.githubusercontent.com/TannhauserCommons/lull/main/web/assets/icons/icon-512.png";

/// While Discord isn't running, look for it this often (only while there is a song to show).
const RETRY: Duration = Duration::from_secs(20);

/// Discord drops texts shorter or longer than this.
const TEXT: std::ops::RangeInclusive<usize> = 2..=128;

/// A song playing now, as `player::refresh` sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct Listening {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    /// Unix milliseconds: when the song started (now minus the position), and when it will end.
    pub start_ms: i64,
    pub end_ms: Option<i64>,
}

/// What the thread should show.
enum Want {
    /// The setting is off: clear and hang up.
    Off,
    /// Nothing plays (or it's paused): clear, stay connected.
    Nothing,
    Song(Box<Request>),
}

/// A song, and the hub (origin and token) to ask for its share link.
struct Request {
    song: Listening,
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
    pub fn show(&self, enabled: bool, listening: Option<Listening>, hub: Option<(Url, String)>) {
        let want = match (enabled, listening) {
            (false, _) => Want::Off,
            (true, None) => Want::Nothing,
            (true, Some(song)) => Want::Song(Box::new(Request { song, hub })),
        };
        if let Some(tx) = &self.tx {
            let _ = tx.send(want);
        }
    }
}

fn run(client_id: &str, rx: &Receiver<Want>) {
    let mut want = Want::Off;
    let mut ipc: Option<Ipc> = None;
    let mut shown: Option<Listening> = None;
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
            Ok(w) => want = rx.try_iter().last().unwrap_or(w),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        match &want {
            Want::Off => {
                if let Some(mut c) = ipc.take() {
                    let _ = c.set_activity(Value::Null);
                }
                shown = None;
            }
            Want::Nothing => {
                if shown.take().is_some()
                    && let Some(c) = ipc.as_mut()
                {
                    let result = c.set_activity(Value::Null);
                    send(&mut ipc, result);
                }
            }
            Want::Song(request) => {
                let Request { song, hub } = request.as_ref();
                if shown.as_ref().is_some_and(|s| same(s, song)) && ipc.is_some() {
                    continue;
                }
                if ipc.is_none() {
                    // Not running (or not yet): quietly, until next time.
                    ipc = Ipc::connect(client_id).ok();
                }
                if let Some(c) = ipc.as_mut() {
                    let share = hub
                        .as_ref()
                        .and_then(|(origin, token)| shares.get(origin, token, &song.id));
                    let result = c.set_activity(activity(song, share.as_ref()));
                    send(&mut ipc, result);
                    shown = ipc.is_some().then(|| song.clone());
                }
            }
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

/// The same song, seeked by less than two seconds: nothing worth telling Discord (it limits how
/// often an activity may change).
fn same(a: &Listening, b: &Listening) -> bool {
    a.id == b.id
        && a.title == b.title
        && a.end_ms.is_some() == b.end_ms.is_some()
        && (a.start_ms - b.start_ms).abs() < 2000
}

/// The activity Discord shows: type 2 is "Listening to".
fn activity(song: &Listening, share: Option<&Share>) -> Value {
    let mut timestamps = json!({ "start": song.start_ms });
    if let Some(end) = song.end_ms {
        timestamps["end"] = json!(end);
    }
    let mut assets = json!({
        "large_image": share.and_then(|s| s.artwork.as_deref()).unwrap_or(FALLBACK_ART),
    });
    if !song.album.is_empty() {
        assets["large_text"] = json!(text(&song.album));
    }
    let mut activity = json!({
        "type": 2,
        "details": text(&song.title),
        "state": text(&song.artist),
        "timestamps": timestamps,
        "assets": assets,
        "instance": false,
    });
    if let Some(share) = share {
        let label = match share.source {
            ShareSource::Songlink => "Open on song.link",
            ShareSource::Lastfm => "Open on Last.fm",
        };
        activity["buttons"] = json!([{ "label": label, "url": share.url }]);
    }
    activity
}

/// Clipped to Discord's limit, and padded with a blank Braille cell when a name is one letter.
fn text(s: &str) -> String {
    let mut out: String = s.chars().take(*TEXT.end()).collect();
    while out.chars().count() < *TEXT.start() {
        out.push('\u{2800}');
    }
    out
}

/// Share links by song id for this run. Unknown ids are remembered too; a hub that can't answer
/// now is asked again with the next song change.
#[derive(Default)]
struct Shares(HashMap<String, Option<Share>>);

impl Shares {
    fn get(&mut self, origin: &Url, token: &str, id: &str) -> Option<Share> {
        if id.is_empty() {
            return None;
        }
        if let Some(known) = self.0.get(id) {
            return known.clone();
        }
        match hub::share(origin, token, "track", id) {
            Ok(share) => {
                self.0.insert(id.to_owned(), share.clone());
                share
            }
            Err(e) => {
                eprintln!("lull: no share link for Discord: {e}");
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

    fn song() -> Listening {
        Listening {
            id: "t1".into(),
            title: "Paper Moon Static".into(),
            artist: "Velvet Orchard".into(),
            album: "Hum".into(),
            start_ms: 1_000_000,
            end_ms: Some(1_200_000),
        }
    }

    #[test]
    fn activity_is_listening_with_progress_cover_and_link() {
        let share = Share {
            url: "https://song.link/x".into(),
            source: ShareSource::Songlink,
            artwork: Some("https://art.example/1.jpg".into()),
        };
        let a = activity(&song(), Some(&share));
        assert_eq!(a["type"], 2);
        assert_eq!(a["details"], "Paper Moon Static");
        assert_eq!(a["state"], "Velvet Orchard");
        assert_eq!(a["timestamps"]["end"], 1_200_000);
        assert_eq!(a["assets"]["large_image"], "https://art.example/1.jpg");
        assert_eq!(a["buttons"][0]["url"], "https://song.link/x");
        let bare = activity(&song(), None);
        assert_eq!(bare["assets"]["large_image"], FALLBACK_ART);
        assert!(bare.get("buttons").is_none());
    }

    #[test]
    fn texts_fit_discords_limits() {
        assert_eq!(text("X").chars().count(), 2);
        assert_eq!(text(&"a".repeat(300)).chars().count(), 128);
    }

    #[test]
    fn a_small_seek_is_the_same_song() {
        let mut b = song();
        b.start_ms += 1500;
        assert!(same(&song(), &b));
        b.start_ms += 5000;
        assert!(!same(&song(), &b));
    }
}
