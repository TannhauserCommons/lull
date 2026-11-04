//! What Classic does around its player for the window and the tray, so nothing waits on the web UI
//! while `main` is closed: Like and Save from the tray, what plays for the tray, the taskbar's
//! buttons and Discord (`set_now_playing`, player.rs) and Windows' media overlay (smtc.rs), and the
//! queue handed back to the page in its own saved shape when Classic closes. The player itself, its
//! listening history included, is the engine's (engine.rs).
//!
//! It lives on the main thread with the window, which calls `tick` on every frame.

use super::engine::{Command, Engine, Snapshot, Status, Track};
use super::hub::Hub;
use super::smtc::Overlay;
use crate::desktop::player;
use crate::desktop::state::{Desktop, NowPlaying};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};

/// What the windows show of the player now.
#[derive(Clone, Debug, Default)]
pub struct Now {
    pub index: usize,
    pub position: f64,
    pub duration: Option<f64>,
    pub playing: bool,
    /// Opening, or waiting for the song's sound.
    pub loading: bool,
    /// Winamp's Stop, or the queue's end: the time blank.
    pub stopped: bool,
    pub kbps: Option<u32>,
    pub khz: Option<u32>,
    pub stereo: bool,
    /// What the marquee says instead of the song: a failure, or where a song still arriving stands.
    pub said: Option<String>,
}

pub struct Session {
    app: AppHandle,
    hub: Hub,
    pub engine: Engine,
    /// The marquee's line for each song, as of the queue's revision.
    pub titles: Vec<String>,
    revision: Option<u64>,
    overlay: Option<Overlay>,
    reported: Option<(NowPlaying, i64)>,
}

/// What Classic hands back to the page when it closes (web/screens/classic.js `takeBack`).
pub struct Handback {
    pub value: Value,
    /// Calls still telling the hub about the last play and likes; the ops are complete once done.
    pub threads: Vec<JoinHandle<()>>,
    pub ops: Arc<Mutex<Vec<Value>>>,
}

impl Session {
    pub fn new(app: AppHandle, hub: Hub, engine: Engine) -> Self {
        Self {
            app,
            hub,
            engine,
            titles: Vec::new(),
            revision: None,
            overlay: None,
            reported: None,
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        self.engine.snapshot()
    }

    /// The player as the windows draw it.
    pub fn now(&self, snapshot: &Snapshot) -> Now {
        let said = match &snapshot.status {
            Status::Failed(why) => Some(why.clone()),
            Status::Finding => Some("Finding a copy on Soulseek...".into()),
            Status::Queued(0) => Some("Queued at the peer...".into()),
            Status::Queued(n) => Some(format!("Queued at the peer, {n} ahead...")),
            Status::Buffering if snapshot.playing => Some("Buffering...".into()),
            Status::Stalled => Some("The peer stalled: waiting...".into()),
            Status::Choose(_) => Some("No copy passes your minimum: click here to choose one".into()),
            _ => snapshot.notice.clone(),
        };
        Now {
            index: snapshot.index,
            position: snapshot.position,
            duration: snapshot.duration,
            playing: snapshot.playing,
            loading: snapshot.waiting || matches!(snapshot.status, Status::Loading),
            stopped: snapshot.stopped || snapshot.status == Status::Ended,
            kbps: snapshot.facts.kbps,
            khz: snapshot.facts.khz,
            stereo: snapshot.facts.stereo,
            said,
        }
    }

    /// Every frame: the titles after a queue change, and what plays for the tray and the overlay.
    pub fn tick(&mut self, snapshot: &Snapshot) {
        if self.revision != Some(snapshot.revision) {
            self.revision = Some(snapshot.revision);
            self.titles = snapshot
                .queue
                .iter()
                .enumerate()
                .map(|(i, e)| {
                    let t = self.engine.track(&e.track_id).unwrap_or_default();
                    title_line(i, &t.artist, &t.title, t.duration)
                })
                .collect();
        }
        self.report(snapshot);
    }

    /// The playlist's lines: `1. Artist - Title` and the length, for each song.
    pub fn lines(&self, snapshot: &Snapshot) -> (Vec<String>, Vec<Option<f64>>) {
        snapshot
            .queue
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let t = self.engine.track(&e.track_id).unwrap_or_default();
                (title_line(i, &t.artist, &t.title, None), t.duration)
            })
            .unzip()
    }

    fn playing_track(&self, snapshot: &Snapshot) -> Option<Track> {
        snapshot
            .queue
            .get(snapshot.index)
            .and_then(|e| self.engine.track(&e.track_id))
    }

    /// The tray, the taskbar's buttons, Discord and the overlay show what plays: reported when it
    /// changes, not as the seconds pass (a seek is a change: the song's start moves).
    fn report(&mut self, snapshot: &Snapshot) {
        let Some(track) = self.playing_track(snapshot) else {
            return;
        };
        let report = NowPlaying {
            title: track.title.clone(),
            artist: track.artist.clone(),
            playing: snapshot.playing,
            liked: track.liked,
            saved: track.saved,
            connected: snapshot.reachable,
            id: track.id.clone(),
            artist_id: track.artist_id.clone(),
            album_id: track.album_id.clone(),
            album: track.album.clone(),
            position: snapshot.position,
            duration: snapshot.duration.unwrap_or(0.0),
        };
        let started = ((unix_now() - report.position) / 3.0).round() as i64;
        let same = self.reported.as_ref().is_some_and(|(was, at)| {
            let moved = NowPlaying {
                position: report.position,
                ..was.clone()
            };
            moved == report && (!report.playing || *at == started)
        });
        if same {
            return;
        }
        self.app.state::<Desktop>().set_now_playing(report.clone());
        player::refresh(&self.app);
        if let Some(overlay) = &self.overlay
            && let Err(e) = overlay.show(&report, track.art.as_deref(), &self.hub)
        {
            eprintln!("lull: media overlay: {e}");
        }
        self.reported = Some((report, started));
    }

    /// The tray's and the taskbar's buttons, and the system's media keys.
    pub fn act(&mut self, action: &str) {
        let snapshot = self.snapshot();
        let command = match action {
            "play-pause" if snapshot.playing => Command::Pause,
            "play-pause" | "play" => Command::Play,
            "pause" => Command::Pause,
            "stop" => Command::Stop,
            "next" => Command::Next,
            "previous" => Command::Previous,
            "like" => Command::Like,
            "save" => Command::Save,
            other => return eprintln!("lull: Classic has no {other}"),
        };
        self.engine.send(command);
    }

    /// Windows' media overlay and keys for the window.
    pub fn attach_overlay(&mut self, overlay: Overlay) {
        self.overlay = Some(overlay);
    }

    pub fn app(&self) -> &AppHandle {
        &self.app
    }

    pub fn origin(&self) -> &tauri::Url {
        self.hub.origin()
    }

    pub fn token(&self) -> &str {
        self.hub.token()
    }

    /// Classic closes: the player stops (the play ends, stopped) and the queue goes back in the
    /// page's saved shape (app.js `persist`), with what it plays and the outbox's ops.
    pub fn hand_back(self) -> Option<Handback> {
        let back = self.engine.finish()?;
        let queue = &back.queue;
        let value = json!({
            "player": {
                "volume": back.volume,
                "muted": false,
                "shuffle": queue.shuffle,
                "repeat": queue.repeat,
                "radio": queue.radio,
                "context": queue.context,
                "index": queue.index,
                "pos": back.position,
                "queue": queue.queue,
                "tracks": back.tracks,
            },
            "playing": back.playing,
            "eq": back.eq,
        });
        Some(Handback {
            value,
            threads: back.threads,
            ops: back.ops,
        })
    }
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

fn unix_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
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
