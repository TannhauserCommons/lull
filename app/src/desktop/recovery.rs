//! When a window's web content dies (docs/context/desktop.md, Crashes). The platform reports it
//! (WebView2's ProcessFailed, WebKitGTK's web-process-terminated: platform::watch_webview); this
//! logs it with what the app was doing to `logs/webview.log`, and brings the window back so the
//! listener never sees the engine's own error page:
//! - a renderer that hangs (WebView2 says so when the page ignores input) can't run a reload, so its
//!   process is ended first (platform::end_renderer); its exit then goes on as below.
//! - a renderer that exited: the window reloads. `main` then resumes the song, second and
//!   play state it last reported (`resume_after_crash`); the Classic window takes its handoff again,
//!   kept current as Webamp plays (classic.rs); the mini player asks `main` for the player again.
//! - the same window dying 3 times in 2 minutes is a loop: `main` shows lull's own page with a
//!   Reload button, the mini player closes, Classic mode hands the queue back to `main`.
//! - the browser process (every window's) exiting: lull restarts, once per 10 minutes at most.
//! - the GPU and helper processes come back on their own: logged only.

use crate::desktop::state::{Desktop, NowPlaying, lock};
use crate::desktop::{classic, main_window, mini, platform};
use crate::pages;
use serde::Serialize;
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, Wry};

/// This many deaths of one window's content within `LOOP_WINDOW` is a loop: reloading won't help.
const LOOP_COUNT: usize = 3;
const LOOP_WINDOW: Duration = Duration::from_secs(120);
/// lull restarts for a dead browser process at most this often.
const RESTART_GAP: Duration = Duration::from_secs(600);
/// The log keeps this much and one older file beside it.
const LOG_LIMIT: u64 = 256 * 1024;
const LOG: &str = "webview.log";

/// Which process died, as the platform says it. WebKitGTK only ever reports the page's own process:
/// the others are WebView2's.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub enum Process {
    /// The window's page (its renderer) exited or crashed.
    Renderer,
    /// The page stopped answering input.
    Unresponsive,
    /// The browser process every window shares.
    Browser,
    /// The GPU process, a frame's own renderer, a utility or helper process: back by itself.
    Helper(String),
}

#[derive(Debug, Clone)]
pub struct Failure {
    pub process: Process,
    /// The platform's reason ("crashed", "out of memory"…).
    pub reason: String,
    pub exit_code: Option<i32>,
    /// WebView2's description of the process, and the module the crash happened in, when known.
    pub description: String,
    pub module: String,
}

/// What `main`'s page resumes once it has reloaded after a crash: the song it had, where, playing.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Resume {
    pub id: String,
    pub position: f64,
    pub playing: bool,
}

#[derive(Default)]
pub struct Recovery {
    /// When each window's content last died, for loop detection.
    deaths: Mutex<HashMap<String, Vec<Instant>>>,
    resume: Mutex<Option<Resume>>,
    log: Mutex<Option<PathBuf>>,
    /// Windows whose hung renderer this app is ending, and what to do once it has gone.
    ending: Mutex<HashMap<String, Action>>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Action {
    Reload,
    GiveUp,
    /// The page hangs: a reload can't run in its busy renderer, so the renderer goes first.
    EndHung,
    Restart,
    Nothing,
}

/// One more death of this window's page: reload it, or give up when they come in a loop.
fn counted(recovery: &Recovery, label: &str) -> Action {
    let mut deaths = lock(&recovery.deaths);
    if a_loop(deaths.entry(label.to_owned()).or_default(), Instant::now()) {
        Action::GiveUp
    } else {
        Action::Reload
    }
}

/// Before the first window: the shared WebView2 environment (Windows), and a line in the log saying
/// which engine runs and where its crash dumps go.
pub fn start(app: &AppHandle) {
    app.manage(Recovery::default());
    let version = app.package_info().version.to_string();
    match platform::prepare_webviews(app) {
        Ok(engine) => log(app, &format!("lull {version} started; {engine}")),
        Err(e) => log(
            app,
            &format!("lull {version} started; no shared WebView2 environment, crash dumps go to Microsoft: {e}"),
        ),
    }
}

/// Every lull window is built here, so it uses the shared environment.
pub fn builder<'a>(app: &'a AppHandle, label: &str, url: WebviewUrl) -> WebviewWindowBuilder<'a, Wry, AppHandle> {
    platform::shared_environment(WebviewWindowBuilder::new(app, label, url))
}

/// Recovers this window when its page's process dies; called once it is built.
pub fn watch(window: &WebviewWindow) {
    let app = window.app_handle().clone();
    let label = window.label().to_owned();
    let watching = platform::watch_webview(window, move |failure| {
        let (app, label) = (app.clone(), label.clone());
        // Off the engine's own event: recovering calls back into the window.
        std::thread::spawn(move || failed(&app, &label, failure));
    });
    if let Err(e) = watching {
        log(
            window.app_handle(),
            &format!("{}: can't hear its crashes: {e}", window.label()),
        );
    }
}

/// Called from the platform's handler, on a thread of its own (window calls go through Tauri).
pub fn failed(app: &AppHandle, label: &str, failure: Failure) {
    let recovery = app.state::<Recovery>();
    let action = match &failure.process {
        // A renderer this app ended because it hung: the hang counted, and decided what comes next.
        Process::Renderer => match lock(&recovery.ending).remove(label) {
            Some(decided) => decided,
            None => counted(&recovery, label),
        },
        Process::Unresponsive => {
            let decided = counted(&recovery, label);
            lock(&recovery.ending).insert(label.to_owned(), decided);
            Action::EndHung
        }
        Process::Browser => Action::Restart,
        Process::Helper(_) => Action::Nothing,
    };
    let context = context(app);
    let done = act(app, label, &action);
    log(
        app,
        &format!(
            "{label}: {} {}{}{}{} | {context} | {}",
            process_name(&failure.process),
            failure.reason,
            failure
                .exit_code
                .map(|c| format!(" exit={c} (0x{:08X})", c as u32))
                .unwrap_or_default(),
            if failure.description.is_empty() {
                String::new()
            } else {
                format!(" process=\"{}\"", failure.description)
            },
            if failure.module.is_empty() {
                String::new()
            } else {
                format!(" module=\"{}\"", failure.module)
            },
            done,
        ),
    );
}

/// Records one death; true once they come too often for a reload to help.
fn a_loop(deaths: &mut Vec<Instant>, now: Instant) -> bool {
    deaths.push(now);
    deaths.retain(|t| now.duration_since(*t) < LOOP_WINDOW);
    deaths.len() >= LOOP_COUNT
}

fn act(app: &AppHandle, label: &str, action: &Action) -> String {
    let result = match (action, label) {
        (Action::Nothing, _) => return "recovers by itself".into(),
        (Action::EndHung, _) => match app.get_webview_window(label) {
            Some(window) => platform::end_renderer(&window)
                .map(|()| "ending the hung renderer; the window comes back once it has gone".into()),
            None => Ok("the window is gone".into()),
        },
        (Action::Restart, _) => return restart(app),
        (Action::Reload, main_window::LABEL) => {
            let said = keep_resume_point(app);
            reload(app, label).map(|()| format!("reloaded{said}"))
        }
        (Action::Reload, _) => reload(app, label).map(|()| "reloaded".into()),
        // Its Reload loads the web UI again, which resumes the same way.
        (Action::GiveUp, main_window::LABEL) => {
            let said = keep_resume_point(app);
            main_window::open_page(app, pages::crashed(&app.state::<Desktop>().settings().appearance))
                .map(|()| format!("crashing in a loop: showed lull's page{said}"))
        }
        (Action::GiveUp, classic::LABEL) => {
            classic::hand_back(app).map(|()| "crashing in a loop: Classic mode handed the queue back".into())
        }
        (Action::GiveUp, _) => mini::close(app).map(|()| "crashing in a loop: closed it".into()),
    };
    result.unwrap_or_else(|e| format!("couldn't recover: {e}"))
}

fn reload(app: &AppHandle, label: &str) -> tauri::Result<()> {
    match app.get_webview_window(label) {
        Some(window) => window.reload(),
        None => Ok(()),
    }
}

/// Keeps what `main`'s next page resumes; says what, for the log.
fn keep_resume_point(app: &AppHandle) -> String {
    let resume = resume_point(app);
    let said = match &resume {
        Some(r) => format!(
            ", to resume {} at {:.0} s{}",
            r.id,
            r.position,
            if r.playing { ", playing" } else { "" }
        ),
        None => String::new(),
    };
    *lock(&app.state::<Recovery>().resume) = resume;
    said
}

/// `main` resumes only what it played itself: while Classic mode is open, Webamp plays.
fn resume_point(app: &AppHandle) -> Option<Resume> {
    if classic::window(app).is_some() {
        return None;
    }
    let (now, at) = app.state::<Desktop>().now_playing_at();
    resume_from(&now, at.elapsed())
}

fn resume_from(now: &NowPlaying, since: Duration) -> Option<Resume> {
    if now.id.is_empty() {
        return None;
    }
    let mut position = now.position;
    if now.playing {
        position += since.as_secs_f64();
        if now.duration > 0.0 {
            position = position.min(now.duration);
        }
    }
    Some(Resume {
        id: now.id.clone(),
        position,
        playing: now.playing,
    })
}

/// What `main`'s next page resumes when it isn't a crash that brings it back: Classic mode's native
/// window closed `main` and builds it again (classic_native).
#[cfg(windows)]
pub fn resume_on_load(app: &AppHandle, resume: Resume) {
    *lock(&app.state::<Recovery>().resume) = Some(resume);
}

/// `main`'s page, once it runs again: what to resume, once.
pub fn take_resume(app: &AppHandle) -> Option<Resume> {
    lock(&app.state::<Recovery>().resume).take()
}

/// The listener pressed Reload on the crash page: `main` gets its fresh chances again.
pub fn forget(app: &AppHandle, label: &str) {
    lock(&app.state::<Recovery>().deaths).remove(label);
}

/// The browser process is every window's: only a new one brings them back, so lull restarts. A
/// stamp beside the log keeps that from becoming a loop.
fn restart(app: &AppHandle) -> String {
    let Some(stamp) = log_path(app).map(|p| p.with_file_name("webview-restart")) else {
        return "lull can't keep a restart stamp: not restarting".into();
    };
    let recent = fs::metadata(&stamp)
        .and_then(|m| m.modified())
        .is_ok_and(|t| t.elapsed().is_ok_and(|e| e < RESTART_GAP));
    if recent {
        return "lull restarted for this less than 10 minutes ago: not again".into();
    }
    if let Err(e) = fs::write(&stamp, b"") {
        return format!("can't write {}: {e}; not restarting", stamp.display());
    }
    let app = app.clone();
    std::thread::spawn(move || app.restart());
    "restarting lull".into()
}

/// What the app was doing: Classic mode, `main` asleep, the mini player, a song.
fn context(app: &AppHandle) -> String {
    let desktop = app.state::<Desktop>();
    let (now, _) = desktop.now_playing_at();
    let classic = if classic::window(app).is_some() {
        "open"
    } else {
        "closed"
    };
    let asleep = if desktop.main_asleep() { "asleep" } else { "awake" };
    let mini = match mini::window(app) {
        Some(w) => w.label().to_owned(),
        None => "closed".into(),
    };
    let song = if now.id.is_empty() {
        "nothing".into()
    } else {
        format!(
            "{} {} at {:.0} s",
            now.id,
            if now.playing { "playing" } else { "paused" },
            now.position
        )
    };
    format!("classic={classic} main={asleep} mini={mini} song={song}")
}

fn process_name(process: &Process) -> String {
    match process {
        Process::Renderer => "renderer exited:".into(),
        Process::Unresponsive => "renderer unresponsive:".into(),
        Process::Browser => "browser process exited:".into(),
        Process::Helper(kind) => format!("{kind} exited:"),
    }
}

/// One line to `logs/webview.log`, UTC-stamped. Past `LOG_LIMIT` it becomes `webview.log.1`.
pub fn log(app: &AppHandle, line: &str) {
    let Some(path) = log_path(app) else {
        return eprintln!("lull: {line}");
    };
    if fs::metadata(&path).is_ok_and(|m| m.len() > LOG_LIMIT) {
        let _ = fs::rename(&path, path.with_extension("log.1"));
    }
    let written = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut f| writeln!(f, "{} {line}", utc_now()));
    if let Err(e) = written {
        eprintln!("lull: {} ({e}): {line}", path.display());
    }
}

fn log_path(app: &AppHandle) -> Option<PathBuf> {
    let recovery = app.state::<Recovery>();
    let mut path = lock(&recovery.log);
    if path.is_none() {
        let dir = app.path().app_log_dir().ok()?;
        fs::create_dir_all(&dir).ok()?;
        *path = Some(dir.join(LOG));
    }
    path.clone()
}

fn utc_now() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    utc(secs)
}

/// `2026-11-03T09:41:07Z` from unix seconds (Howard Hinnant's days-to-civil).
fn utc(secs: u64) -> String {
    let (days, rest) = ((secs / 86_400) as i64, secs % 86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3_600,
        rest % 3_600 / 60,
        rest % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn playing(position: f64, duration: f64) -> NowPlaying {
        NowPlaying {
            id: "t1".into(),
            playing: true,
            position,
            duration,
            ..NowPlaying::default()
        }
    }

    #[test]
    fn a_playing_song_resumes_where_it_had_got_to_since_its_last_report() {
        let r = resume_from(&playing(60.0, 300.0), Duration::from_secs(15));
        assert_eq!(
            r,
            Some(Resume {
                id: "t1".into(),
                position: 75.0,
                playing: true
            })
        );
    }

    #[test]
    fn a_paused_song_resumes_where_it_stopped_and_stays_paused() {
        let now = NowPlaying {
            playing: false,
            ..playing(60.0, 300.0)
        };
        let r = resume_from(&now, Duration::from_secs(600));
        assert_eq!(r.map(|r| (r.position, r.playing)), Some((60.0, false)));
    }

    #[test]
    fn the_resume_point_never_runs_past_the_song() {
        let r = resume_from(&playing(290.0, 300.0), Duration::from_secs(60));
        assert_eq!(r.map(|r| r.position), Some(300.0));
    }

    #[test]
    fn nothing_playing_resumes_nothing() {
        assert_eq!(resume_from(&NowPlaying::default(), Duration::ZERO), None);
    }

    #[test]
    fn a_third_death_within_two_minutes_is_a_loop_and_older_ones_are_forgotten() {
        let start = Instant::now();
        let mut deaths = Vec::new();
        assert!(!a_loop(&mut deaths, start));
        assert!(!a_loop(&mut deaths, start + Duration::from_secs(30)));
        assert!(a_loop(&mut deaths, start + Duration::from_secs(60)));
        // Two minutes after the first two, only the last one and this one count.
        let mut deaths = vec![start, start + Duration::from_secs(10)];
        assert!(!a_loop(&mut deaths, start + Duration::from_secs(200)));
    }

    #[test]
    fn log_lines_carry_utc_time() {
        assert_eq!(utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc(1_793_659_267), "2026-11-02T22:41:07Z");
        assert_eq!(utc(951_782_400), "2000-02-29T00:00:00Z");
    }
}
