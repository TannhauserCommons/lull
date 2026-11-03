//! What Classic's own player does beyond the sound, so nothing waits on the web UI while `main` is
//! closed: it records listening history (a play when a song starts, how it ended when it goes, as
//! web/app.js `recordPlay`), Likes and Saves the song from the tray, and reports what plays to the
//! tray, the taskbar's buttons and Discord (`set_now_playing`, player.rs). Calls to the hub run on
//! threads of their own; what the hub couldn't take waits for the page's outbox.
//!
//! It lives on the main thread with the window, which calls `tick` on every frame.

use super::hub::{self, Failure, How, Hub};
use super::smtc::Overlay;
use super::sound::{Now, Player};
use crate::desktop::classic::{Equaliser, Track};
use crate::desktop::player;
use crate::desktop::state::{Desktop, NowPlaying};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};

/// A gap longer than this between two frames isn't listening (a stall, a sleep).
const LISTEN_GAP: f64 = 1.5;

pub struct Session {
    app: AppHandle,
    hub: Hub,
    pub player: Player,
    pub tracks: Vec<Track>,
    /// The marquee's line for each song.
    pub titles: Vec<String>,
    pub shuffle: bool,
    pub repeat: bool,
    pub eq: Equaliser,
    /// The outbox's ops for what the hub couldn't take, and whether it answered last time.
    ops: Arc<Mutex<Vec<Value>>>,
    reachable: Arc<AtomicBool>,
    /// Likes and Saves made here, as the threads that sent them leave them.
    marks: Arc<Mutex<Vec<(usize, Mark)>>>,
    play: Option<Play>,
    threads: Vec<JoinHandle<()>>,
    /// Windows' media overlay and keys, once the window has them.
    overlay: Option<Overlay>,
    /// What was last reported and when its song started (in 3 s steps), and the song's natural
    /// ends counted then.
    reported: Option<(NowPlaying, i64)>,
    ends: u32,
}

#[derive(Clone, Copy)]
enum Mark {
    Liked(bool),
    Saved,
}

/// The song being listened to: its place in the queue, the seconds heard, and the thread that
/// recorded its start and waits to record its end.
struct Play {
    index: usize,
    listened: f64,
    last: Option<f64>,
    end: Sender<(f64, How)>,
}

/// What Classic hands back to the page when it closes (web/screens/classic.js `takeBack`).
pub struct Handback {
    pub value: Value,
    /// Threads still telling the hub about the last play; the ops are complete once they're done.
    pub threads: Vec<JoinHandle<()>>,
    pub ops: Arc<Mutex<Vec<Value>>>,
}

impl Session {
    pub fn new(app: AppHandle, hub: Hub, player: Player, tracks: Vec<Track>, setup: Setup) -> Self {
        Self {
            app,
            hub,
            player,
            titles: titles(&tracks),
            tracks,
            shuffle: setup.shuffle,
            repeat: setup.repeat,
            eq: setup.eq,
            ops: Arc::default(),
            reachable: Arc::new(AtomicBool::new(true)),
            marks: Arc::default(),
            play: None,
            threads: Vec::new(),
            overlay: None,
            reported: None,
            ends: 0,
        }
    }

    /// Every frame: what the listener heard, a new song's play, and what the tray shows.
    pub fn tick(&mut self, now: &Now) {
        for (index, mark) in std::mem::take(&mut *lock(&self.marks)) {
            if let Some(track) = self.tracks.get_mut(index) {
                match mark {
                    Mark::Liked(on) => track.liked = on,
                    Mark::Saved => track.saved = true,
                }
            }
        }
        self.listen(now);
        self.report(now);
    }

    fn listen(&mut self, now: &Now) {
        let sounding = now.playing && !now.loading && !now.stopped;
        let natural = now.ends != self.ends;
        self.ends = now.ends;
        if self.play.as_ref().is_some_and(|p| p.index != now.index || now.stopped) {
            self.end_play(if natural { How::Completed } else { How::Skipped });
        }
        if sounding && self.play.is_none() {
            self.start_play(now.index);
        }
        if let Some(play) = self.play.as_mut() {
            if let Some(last) = play.last {
                let heard = now.position - last;
                if sounding && heard > 0.0 && heard < LISTEN_GAP {
                    play.listened += heard;
                }
            }
            play.last = Some(now.position);
        }
    }

    fn start_play(&mut self, index: usize) {
        let Some(track) = self.tracks.get(index) else {
            return;
        };
        let (end, ended) = channel::<(f64, How)>();
        let (hub, ops, reachable) = (self.hub.clone(), self.ops.clone(), self.reachable.clone());
        let (id, duration, recommended) = (track.id.clone(), track.duration, track.recommended);
        let started_at = unix_now();
        let spawned = std::thread::Builder::new().name("classic-play".into()).spawn(move || {
            let started = hub.start_play(&id, duration, recommended);
            reachable.store(!matches!(started, Err(Failure::Offline)), Ordering::Relaxed);
            let Ok((listened, how)) = ended.recv() else {
                return;
            };
            let body = hub::end_body(listened, how);
            match started {
                Ok(Some(play)) => {
                    if let Err(Failure::Refused(why)) = hub.end_play(&play, &body) {
                        eprintln!("lull: Classic play {play} not ended: {why}");
                    }
                }
                // Listening privately: the hub kept nothing, so there's nothing to end.
                Ok(None) => {}
                Err(Failure::Offline) => {
                    let mut op = json!({ "op": "play", "trackId": id, "startedAt": started_at });
                    if let (Some(op), Some(body)) = (op.as_object_mut(), body.as_object()) {
                        op.extend(body.clone());
                    }
                    lock(&ops).push(op);
                }
                Err(Failure::Refused(why)) => eprintln!("lull: Classic play not recorded: {why}"),
            }
        });
        match spawned {
            Ok(thread) => {
                self.threads.push(thread);
                self.play = Some(Play {
                    index,
                    listened: 0.0,
                    last: None,
                    end,
                });
            }
            Err(e) => eprintln!("lull: Classic play not recorded: {e}"),
        }
    }

    fn end_play(&mut self, how: How) {
        if let Some(play) = self.play.take() {
            let _ = play.end.send((play.listened, how));
        }
        self.threads.retain(|t| !t.is_finished());
    }

    /// The tray, the taskbar's buttons and Discord show what plays: reported when it changes, not
    /// as the seconds pass (a seek is a change: the song's start moves).
    fn report(&mut self, now: &Now) {
        let Some(track) = self.tracks.get(now.index) else {
            return;
        };
        let report = NowPlaying {
            title: track.title.clone(),
            artist: track.artist.clone(),
            playing: now.playing && !now.stopped,
            liked: track.liked,
            saved: track.saved,
            connected: self.reachable.load(Ordering::Relaxed),
            id: track.id.clone(),
            artist_id: track.artist_id.clone(),
            album_id: track.album_id.clone(),
            album: track.album.clone(),
            position: now.position,
            duration: now.duration.unwrap_or(0.0),
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
        let now = self.player.now();
        match action {
            "play-pause" if now.playing => self.player.pause(),
            "play-pause" | "play" => self.player.play(),
            "pause" | "stop" => self.player.pause(),
            "next" => self.player.next(),
            "previous" => self.player.previous(),
            "like" => self.like(now.index),
            "save" => self.save(now.index),
            other => eprintln!("lull: Classic has no {other}"),
        }
    }

    /// Like, or unlike, the song: the hub at once, or the outbox when it's out of reach.
    fn like(&mut self, index: usize) {
        let Some(track) = self.tracks.get_mut(index) else {
            return;
        };
        let on = !track.liked;
        track.liked = on;
        let (hub, ops, marks, id) = (self.hub.clone(), self.ops.clone(), self.marks.clone(), track.id.clone());
        self.spawn(move || match hub.like(&id, on) {
            Ok(()) => {}
            Err(Failure::Offline) => lock(&ops).push(json!({ "op": "like", "trackId": id, "liked": on })),
            Err(Failure::Refused(why)) => {
                eprintln!("lull: Classic like refused: {why}");
                lock(&marks).push((index, Mark::Liked(!on)));
            }
        });
    }

    /// Save the song into the library. The page's outbox doesn't keep saves, so neither does this.
    fn save(&mut self, index: usize) {
        let Some(track) = self.tracks.get(index).filter(|t| !t.saved) else {
            return;
        };
        let (hub, marks, id) = (self.hub.clone(), self.marks.clone(), track.id.clone());
        self.spawn(move || match hub.save(&id) {
            Ok(()) => lock(&marks).push((index, Mark::Saved)),
            Err(Failure::Offline) => eprintln!("lull: Classic save: the hub is out of reach"),
            Err(Failure::Refused(why)) => eprintln!("lull: Classic save refused: {why}"),
        });
    }

    fn spawn(&mut self, work: impl FnOnce() + Send + 'static) {
        match std::thread::Builder::new().name("classic-hub".into()).spawn(work) {
            Ok(thread) => self.threads.push(thread),
            Err(e) => eprintln!("lull: Classic can't reach the hub: {e}"),
        }
    }

    /// Takes songs out of the queue (the playlist's REM menu, Delete): `keep` says, for each, whether
    /// it stays. The playing song always does, and its play goes on.
    pub fn keep(&mut self, keep: &[bool]) {
        let index = self.player.now().index;
        let stays = |i: usize| i == index || keep.get(i).copied().unwrap_or(true);
        let moved = (0..index).filter(|&i| stays(i)).count();
        let mut i = 0;
        self.tracks.retain(|_| {
            i += 1;
            stays(i - 1)
        });
        self.titles = titles(&self.tracks);
        if let Some(play) = self.play.as_mut().filter(|p| p.index == index) {
            play.index = moved;
        }
        self.player.keep(keep);
    }

    /// The playlist's lines: `1. Artist - Title` and the length, for each song.
    pub fn lines(&self) -> (Vec<String>, Vec<Option<f64>>) {
        self.tracks
            .iter()
            .enumerate()
            .map(|(i, t)| (title_line(i, &t.artist, &t.title, None), t.duration))
            .unzip()
    }

    pub fn set_modes(&mut self, shuffle: bool, repeat: bool) {
        (self.shuffle, self.repeat) = (shuffle, repeat);
        self.player.set_modes(shuffle, repeat);
    }

    pub fn set_equaliser(&mut self, eq: Equaliser) {
        self.eq = eq.normalised();
        self.player.set_equaliser(self.eq.clone());
    }

    /// Classic closes: the play ends (stopped), and the queue goes back as the page takes it.
    pub fn hand_back(mut self) -> Handback {
        let now = self.player.now();
        self.end_play(How::Stopped);
        let value = json!({
            "ids": self.tracks.iter().map(|t| &t.id).collect::<Vec<_>>(),
            "index": now.index,
            "pos": now.position,
            "playing": now.playing && !now.stopped,
            "shuffle": self.shuffle,
            "repeat": self.repeat,
            "local": 0,
            "eq": self.eq,
        });
        Handback {
            value,
            threads: std::mem::take(&mut self.threads),
            ops: self.ops.clone(),
        }
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
}

/// What the session starts with besides the queue.
pub struct Setup {
    pub shuffle: bool,
    pub repeat: bool,
    pub eq: Equaliser,
}

fn titles(tracks: &[Track]) -> Vec<String> {
    tracks
        .iter()
        .enumerate()
        .map(|(i, t)| title_line(i, &t.artist, &t.title, t.duration))
        .collect()
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

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
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
