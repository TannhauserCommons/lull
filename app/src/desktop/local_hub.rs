//! "This computer is my hub" (ADR-0002, ADR-0006): the hub runs in this process through
//! `lull_server::run_with`, with its data under the app's data dir and the bundled slskd beside it,
//! the same hub with its own slskd as the Linux package's (`lull_server::managed`, ADR-0090). The
//! app pairs itself: it creates its own device token before the hub starts and hands it to the web
//! UI as the device cookie.

use crate::desktop::state::Desktop;
use crate::hub;
use lull_core::db::{self, Db};
use lull_core::devices::{self, Kind};
use lull_server::managed::advert::{self, Advert};
use lull_server::managed::{self, Fixed, Sidecar, Tie, slskd};
use std::collections::BTreeMap;
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
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
        port: managed::free_port().map_err(|e| e.to_string())?,
        api_key: slskd::secret()?,
        web_password: slskd::secret()?,
        tie: TIE,
    };
    let mut env = BTreeMap::from([
        ("LULL_DATA_DIR".to_owned(), path_string(&paths.data)),
        ("LULL_LISTEN".to_owned(), format!("0.0.0.0:{PORT}")),
    ]);
    // Where slskd answers, its key and folders, and the slskd.yml the hub writes the account into.
    env.extend(managed::vars(&fixed));
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

    // The system keyring holds the listener's credentials (ADR-0091); the hub falls back to its own file
    // where there is none.
    let keyring = app.state::<Desktop>().keyring.clone();
    let db = Db::open(&paths.data)
        .map_err(|e| e.to_string())?
        .with_keyring(keyring.clone());
    db.migrate(&db::CORE).map_err(|e| e.to_string())?;
    if let Ok(music) = app.path().audio_dir() {
        block_on(managed::seed_music_dir(&db, &env, &music)).map_err(|e| e.to_string())?;
    }
    block_on(managed::follow_lan_address(&db, PORT)).map_err(|e| e.to_string())?;
    let token = block_on(own_token(app, &db))?;
    let launch = block_on(managed::launch(&env, &db)).map_err(|e| e.to_string())?;

    let sidecar = slskd::start(fixed, launch)?;
    let (shutdown, stopped) = oneshot::channel();
    let (done_tx, done) = mpsc::channel();
    // It updates with the app: Settings › App says so instead of offering a server update.
    lull_core::build::set_install(lull_core::build::Install::Desktop);
    let follows = sidecar.clone();
    tauri::async_runtime::spawn(async move {
        let result = lull_server::run_with(env, Some(follows), Some(keyring), async {
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
    let advert = advert::advertise(&hub::device_name(), PORT)
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

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// The app's own device on its hub: the stored token while the hub still knows it, else a new one.
async fn own_token(app: &AppHandle, db: &Db) -> Result<String, String> {
    let desktop = app.state::<Desktop>();
    if let Some(token) = desktop.token()
        && devices::verify(db, &token).await.map_err(|e| e.to_string())?.is_some()
    {
        return Ok(token);
    }
    let (_, token) = devices::create(db, &hub::device_name(), Kind::App)
        .await
        .map_err(|e| e.to_string())?;
    let token = token.expose().to_owned();
    desktop.set_token(Some(token.clone()));
    Ok(token)
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

/// slskd must never outlive the app, or it would keep the Soulseek login after a crash and log the
/// next launch out. Windows: a kill-on-close job object. Linux: a parent-death signal.
const TIE: Tie = Tie {
    prepare: die_with_app::prepare,
    adopt: die_with_app::adopt,
};

mod die_with_app {
    use std::process::{Child, Command};

    #[cfg(target_os = "linux")]
    #[allow(unsafe_code)]
    pub fn prepare(cmd: &mut Command) {
        use std::os::unix::process::CommandExt;
        // SAFETY: prctl is async-signal-safe and touches only the child between fork and exec.
        unsafe {
            cmd.pre_exec(|| {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }

    #[cfg(not(target_os = "linux"))]
    pub fn prepare(_: &mut Command) {}

    #[cfg(windows)]
    pub fn adopt(child: &Child) {
        if let Err(e) = crate::desktop::platform::kill_with_app(child) {
            tracing::warn!("slskd may outlive a crash of lull: {e}");
        }
    }

    #[cfg(not(windows))]
    pub fn adopt(_: &Child) {}
}
