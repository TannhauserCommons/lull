//! "This computer is my hub" (ADR-0002, ADR-0006): the hub runs in this process through
//! `lull_server::run`, with its data under the app's data dir and the bundled slskd beside it. The
//! app pairs itself: it creates its own device token before the hub starts and hands it to the web
//! UI as the device cookie.

use crate::desktop::slskd::{self, Fixed, Launch, Sidecar};
use crate::desktop::state::Desktop;
use crate::discovery::{self, Advert};
use crate::hub;
use lull_core::db::{self, Db};
use lull_core::devices::{self, Kind};
use lull_core::settings;
use std::collections::BTreeMap;
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, mpsc};
use std::time::{Duration, Instant};
use tauri::async_runtime::block_on;
use tauri::{AppHandle, Manager, Url};
use tokio::sync::oneshot;

/// The hub's port on every network this computer is on, so phones can reach it.
pub const PORT: u16 = 8484;

/// How long the hub may take from start to answering.
const START_TIMEOUT: Duration = Duration::from_secs(30);

/// How often the app reads the hub's `/health` for a wedged slskd, and how often at most it
/// restarts slskd for that (as the hub itself would with a web login, docs/context/soulseek.md).
const WEDGE_CHECK: Duration = Duration::from_secs(60);
const WEDGE_RESTART_EVERY: Duration = Duration::from_secs(30 * 60);

pub struct LocalHub {
    pub origin: Url,
    pub token: String,
    shutdown: Mutex<Option<oneshot::Sender<()>>>,
    sidecar: Sidecar,
    /// Answers mDNS while the hub runs; None if this network refused it (pairing by address still works).
    _advert: Option<Advert>,
}

impl LocalHub {
    /// At exit: stop answering and stop slskd. The hub isn't awaited: the webview's event stream
    /// would hold its graceful shutdown open, and the process is ending anyway.
    pub fn stop(&self) {
        if let Some(tx) = self.shutdown.lock().ok().and_then(|mut s| s.take()) {
            let _ = tx.send(());
        }
        self.sidecar.stop();
    }
}

/// Starts slskd and the hub and waits until the hub answers. Blocking: call it off the main thread.
pub fn start(app: &AppHandle) -> Result<LocalHub, String> {
    let paths = Paths::new(app)?;
    init_logs(&paths.log);
    let fixed = Fixed {
        exe: sidecar_exe()?,
        app_dir: paths.slskd.clone(),
        transfers: paths.transfers.clone(),
        port: free_port()?,
        api_key: slskd::secret()?,
        web_password: slskd::secret()?,
    };
    let mut env = BTreeMap::from([
        ("LULL_DATA_DIR".to_owned(), path_string(&paths.data)),
        ("LULL_LISTEN".to_owned(), format!("0.0.0.0:{PORT}")),
        ("LULL_SLSKD_URL".to_owned(), fixed.url()),
        ("LULL_SLSKD_API_KEY".to_owned(), fixed.api_key.clone()),
        ("LULL_TRANSFERS_DIR".to_owned(), path_string(&paths.transfers)),
        // The hub writes the Soulseek account from its settings here and logs slskd in again.
        ("LULL_SLSKD_CONFIG_FILE".to_owned(), path_string(&fixed.config_file())),
    ]);
    // YouTube's programs ship beside the app like slskd (docs/context/youtube.md). Without them
    // the hub looks on PATH, and Settings › Sources says when yt-dlp won't run.
    for (key, name) in [("LULL_YTDLP", "yt-dlp"), ("LULL_DENO", "deno")] {
        match beside_app(name) {
            Some(path) => {
                env.insert(key.to_owned(), path_string(&path));
            }
            None => tracing::warn!("{name} is missing next to the app; YouTube looks for it on PATH"),
        }
    }

    let db = Db::open(&paths.data).map_err(|e| e.to_string())?;
    db.migrate(&db::CORE).map_err(|e| e.to_string())?;
    block_on(seed_music_dir(&db, &env, app.path().audio_dir().ok()))?;
    block_on(follow_lan_address(&db))?;
    let token = block_on(own_token(app, &db))?;
    let launch = block_on(launch(&db, &env))?;

    let sidecar = slskd::start(fixed, launch)?;
    let watch_env = env.clone();
    let (shutdown, stopped) = oneshot::channel();
    let (done_tx, done) = mpsc::channel();
    tauri::async_runtime::spawn(async move {
        let result = lull_server::run(env, async {
            let _ = stopped.await;
        })
        .await;
        if let Err(e) = &result {
            tracing::error!("the hub stopped: {e}");
        }
        let _ = done_tx.send(result.map_err(|e| e.to_string()));
    });
    if let Err(e) = wait_until_up(&done) {
        sidecar.stop();
        return Err(e);
    }

    let origin = Url::parse(&format!("http://127.0.0.1:{PORT}")).map_err(|e| e.to_string())?;
    follow_restarts(origin.clone(), token.clone(), db, watch_env, sidecar.clone());
    restart_when_wedged(origin.clone(), sidecar.clone());
    let advert = discovery::advertise(&hub::device_name(), PORT)
        .map_err(|e| tracing::warn!("no mDNS on this network: {e}"))
        .ok();
    Ok(LocalHub {
        origin,
        token,
        shutdown: Mutex::new(Some(shutdown)),
        sidecar,
        _advert: advert,
    })
}

struct Paths {
    data: PathBuf,
    slskd: PathBuf,
    transfers: PathBuf,
    log: PathBuf,
}

impl Paths {
    fn new(app: &AppHandle) -> Result<Self, String> {
        let data = app.path().app_data_dir().map_err(|e| e.to_string())?;
        let logs = app.path().app_log_dir().map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&logs).map_err(|e| format!("create {}: {e}", logs.display()))?;
        Ok(Paths {
            data: data.join("hub"),
            slskd: data.join("slskd"),
            transfers: data.join("transfers"),
            log: logs.join("hub.log"),
        })
    }
}

/// The hub logs through `tracing`; here that goes to a file the listener can attach to a bug report.
/// Truncated at each start.
fn init_logs(path: &Path) {
    match std::fs::File::create(path) {
        Ok(file) => {
            let _ = tracing_subscriber::fmt()
                .with_env_filter(tracing_subscriber::EnvFilter::new("info"))
                .with_ansi(false)
                .with_writer(Mutex::new(file))
                .try_init();
        }
        Err(e) => eprintln!("lull: no hub log at {}: {e}", path.display()),
    }
}

/// slskd sits next to the app's own executable: Tauri puts sidecars there, in bundles and in dev.
fn sidecar_exe() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().ok_or("the app has no folder")?;
    let slskd = dir.join(format!("slskd{}", std::env::consts::EXE_SUFFIX));
    if slskd.is_file() {
        Ok(slskd)
    } else {
        Err(format!("slskd is missing from {}; reinstall lull", dir.display()))
    }
}

/// A program Tauri placed next to the app's own executable (a sidecar), when it is there.
fn beside_app(name: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let path = exe.parent()?.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    path.is_file().then_some(path)
}

/// A free port for slskd's API on loopback, chosen once per run (the hub keeps its URL).
fn free_port() -> Result<u16, String> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|e| e.to_string())?;
    listener.local_addr().map(|a| a.port()).map_err(|e| e.to_string())
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// The hub's default music folder is a container path; on a computer it's the listener's Music
/// folder. Stored, not set by env, so the setup screen can change it.
async fn seed_music_dir(db: &Db, env: &BTreeMap<String, String>, music: Option<PathBuf>) -> Result<(), String> {
    const KEY: &str = "LULL_MUSIC_DIR";
    let stored = settings::stored(db).await.map_err(|e| e.to_string())?;
    let Some(music) = music.filter(|_| !stored.contains_key(KEY) && !env.contains_key(KEY)) else {
        return Ok(());
    };
    let change = BTreeMap::from([(KEY.to_owned(), Some(path_string(&music)))]);
    settings::store(db, change).await.map_err(|e| e.to_string())
}

/// Pair codes carry `LULL_PUBLIC_URL`; without one they'd carry the webview's loopback address,
/// which no phone can open. So each start stores this computer's LAN address, unless the listener
/// set an https address (Tailscale or their own), which the app never touches.
async fn follow_lan_address(db: &Db) -> Result<(), String> {
    const KEY: &str = "LULL_PUBLIC_URL";
    let stored = settings::stored(db).await.map_err(|e| e.to_string())?;
    let ours = stored.get(KEY).is_none_or(|u| u.starts_with("http://"));
    let Some(lan) = lan_ip().filter(|_| ours) else {
        return Ok(());
    };
    let url = format!("http://{lan}:{PORT}");
    if stored.get(KEY) == Some(&url) {
        return Ok(());
    }
    settings::store(db, BTreeMap::from([(KEY.to_owned(), Some(url))]))
        .await
        .map_err(|e| e.to_string())
}

/// The address this computer uses on its default route, usually its LAN address. Connecting a UDP
/// socket only picks the route; nothing is sent.
fn lan_ip() -> Option<std::net::IpAddr> {
    let socket = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).ok()?;
    socket
        .local_addr()
        .ok()
        .map(|a| a.ip())
        .filter(|ip| !ip.is_loopback() && !ip.is_unspecified())
}

/// The app's own device on its hub: the stored token while the hub still knows it, else a new one.
async fn own_token(app: &AppHandle, db: &Db) -> Result<String, String> {
    let desktop = app.state::<Desktop>();
    if let Some(token) = desktop.settings().token
        && devices::verify(db, &token).await.map_err(|e| e.to_string())?.is_some()
    {
        return Ok(token);
    }
    let (_, token) = devices::create(db, &hub::device_name(), Kind::App)
        .await
        .map_err(|e| e.to_string())?;
    let token = token.expose().to_owned();
    desktop.update(|s| s.token = Some(token.clone()));
    Ok(token)
}

/// What slskd shares, from lull's settings as the hub sees them (env over stored).
async fn launch(db: &Db, env: &BTreeMap<String, String>) -> Result<Launch, String> {
    let stored = settings::stored(db).await.map_err(|e| e.to_string())?;
    let (vars, _) = settings::merge(env, &stored);
    let get = |k: &str| vars.get(k).filter(|v| !v.is_empty()).cloned();
    let share = match get("LULL_SHARE").as_deref() {
        Some("none") => None,
        _ => get("LULL_MUSIC_DIR").map(PathBuf::from),
    };
    Ok(Launch { share })
}

/// The hub restarts its crates when a setting changes (`hub.restarting`), and what slskd shares may
/// be among them: read the settings again and let the sidecar restart if they differ. The event
/// stream drops during each restart, so it reconnects.
fn follow_restarts(origin: Url, token: String, db: Db, env: BTreeMap<String, String>, sidecar: Sidecar) {
    let watch = move || {
        loop {
            let streamed = hub::events(&origin, &token, |topic, _| {
                if topic == "hub.restarting" {
                    match block_on(launch(&db, &env)) {
                        Ok(next) => sidecar.apply(next),
                        Err(e) => tracing::warn!("can't read the settings for slskd: {e}"),
                    }
                }
                true
            });
            if let Err(e) = streamed {
                tracing::debug!("hub events: {e}");
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    };
    if let Err(e) = std::thread::Builder::new().name("hub-events".into()).spawn(watch) {
        tracing::warn!("slskd won't follow setting changes until lull restarts: {e}");
    }
}

/// The hub tells a wedged slskd that fresh logins didn't cure by `/health`
/// `services.soulseek.extra.restartSlskd`. The bundled slskd is headless and refuses the web login
/// the hub would restart it with, so the app restarts the process, at most once per 30 minutes.
fn restart_when_wedged(origin: Url, sidecar: Sidecar) {
    let watch = move || {
        let mut restarted: Option<Instant> = None;
        loop {
            std::thread::sleep(WEDGE_CHECK);
            let wedged = match hub::health(&origin) {
                Ok(health) => health
                    .services
                    .get("soulseek")
                    .is_some_and(|s| s.extra["restartSlskd"] == true),
                Err(e) => {
                    tracing::debug!("hub health: {e}");
                    false
                }
            };
            if wedged && restarted.is_none_or(|at| at.elapsed() >= WEDGE_RESTART_EVERY) {
                restarted = Some(Instant::now());
                sidecar.restart();
            }
        }
    };
    if let Err(e) = std::thread::Builder::new().name("slskd-wedge".into()).spawn(watch) {
        tracing::warn!("a wedged slskd needs restarting lull: {e}");
    }
}

/// Until the hub accepts connections, or fails to start.
fn wait_until_up(done: &mpsc::Receiver<Result<(), String>>) -> Result<(), String> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, PORT));
    let deadline = Instant::now() + START_TIMEOUT;
    while Instant::now() < deadline {
        match done.try_recv() {
            Ok(Err(e)) => return Err(port_hint(e)),
            Ok(Ok(())) | Err(mpsc::TryRecvError::Disconnected) => return Err("The hub stopped as it started".into()),
            Err(mpsc::TryRecvError::Empty) => {}
        }
        if TcpStream::connect_timeout(&addr, Duration::from_millis(250)).is_ok() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err(format!("The hub didn't answer within {} s", START_TIMEOUT.as_secs()))
}

fn port_hint(error: String) -> String {
    if error.contains("10048") || error.to_lowercase().contains("address already in use") {
        format!("Port {PORT} is already used by another program, so the hub can't start")
    } else {
        error
    }
}

/// Pair codes carry the hub's public address (`LULL_PUBLIC_URL`). Set through the hub's own settings
/// API, which validates it and restarts what needs it.
pub fn set_public_url((origin, token): &(Url, String), url: &str) -> Result<(), String> {
    hub::patch_config(origin, token, serde_json::json!({ "LULL_PUBLIC_URL": url }))
}
