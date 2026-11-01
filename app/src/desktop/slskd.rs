//! The bundled slskd (ADR-0006, ADR-0010): a pinned release binary next to the app's own, which
//! `build.rs` fetched. The app owns its lifecycle, folders, port and API key, and restarts it when
//! what it shares changes. The hub writes the Soulseek account into its `slskd.yml`
//! (`LULL_SLSKD_CONFIG_FILE`, docs/context/soulseek.md). slskd's web UI stays off: lull is the UI.
//!
//! slskd reads, lowest first: env, `slskd.yml`, the command line. Fixed things go on the command
//! line, secrets in env (not in the process list).

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// How often the supervisor looks whether slskd is still running.
const WATCH: Duration = Duration::from_secs(5);

/// What slskd runs with for the whole life of the app's hub. The hub was started with this port and
/// key, so a restarted slskd keeps them.
pub struct Fixed {
    pub exe: PathBuf,
    /// slskd's own data, logs and `slskd.yml`.
    pub app_dir: PathBuf,
    /// `complete/` and `incomplete/` below it: `LULL_TRANSFERS_DIR` (docs/context/soulseek.md).
    pub transfers: PathBuf,
    pub port: u16,
    pub api_key: String,
    /// slskd's own web login, never used but never the default `slskd`/`slskd` either.
    pub web_password: String,
}

/// What lull's settings decide at slskd's start: its shares can't change while it runs. A change
/// restarts slskd.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub share: Option<PathBuf>,
}

impl Fixed {
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// The file the hub keeps the Soulseek account in; slskd reads it from its app dir.
    pub fn config_file(&self) -> PathBuf {
        self.app_dir.join("slskd.yml")
    }
}

enum Order {
    Apply(Launch),
    Stop(mpsc::Sender<()>),
}

/// The thread that owns the slskd process. On Linux the process is tied to the thread that spawned
/// it (PR_SET_PDEATHSIG), so one long-lived thread does every spawn.
#[derive(Clone)]
pub struct Sidecar {
    tx: mpsc::Sender<Order>,
}

/// Starts slskd and returns once the first spawn worked (or didn't).
pub fn start(fixed: Fixed, launch: Launch) -> Result<Sidecar, String> {
    let (tx, rx) = mpsc::channel();
    let (started_tx, started) = mpsc::channel();
    std::thread::Builder::new()
        .name("slskd".into())
        .spawn(move || supervise(&fixed, launch, &rx, &started_tx))
        .map_err(|e| e.to_string())?;
    started.recv().map_err(|_| "slskd's supervisor stopped".to_owned())??;
    Ok(Sidecar { tx })
}

impl Sidecar {
    /// Restarts slskd if `launch` differs from what it runs with.
    pub fn apply(&self, launch: Launch) {
        let _ = self.tx.send(Order::Apply(launch));
    }

    /// Stops slskd and waits until it has exited.
    pub fn stop(&self) {
        let (done, wait) = mpsc::channel();
        if self.tx.send(Order::Stop(done)).is_ok() {
            let _ = wait.recv();
        }
    }
}

fn supervise(
    fixed: &Fixed,
    mut launch: Launch,
    rx: &mpsc::Receiver<Order>,
    started: &mpsc::Sender<Result<(), String>>,
) {
    let mut child = match spawn(fixed, &launch) {
        Ok(child) => {
            let _ = started.send(Ok(()));
            Some(child)
        }
        Err(e) => {
            let _ = started.send(Err(e));
            return;
        }
    };
    loop {
        match rx.recv_timeout(WATCH) {
            Err(mpsc::RecvTimeoutError::Timeout) => report_exit(&mut child, &fixed.app_dir),
            Ok(Order::Apply(next)) if next != launch => {
                kill(&mut child);
                tracing::info!(?next, "restarting slskd with new settings");
                launch = next;
                child = spawn(fixed, &launch)
                    .map_err(|e| tracing::error!("slskd didn't restart: {e}"))
                    .ok();
            }
            Ok(Order::Apply(_)) => {}
            Ok(Order::Stop(done)) => {
                kill(&mut child);
                let _ = done.send(());
                return;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return kill(&mut child),
        }
    }
}

/// slskd stopping by itself is logged once, with where its own log is. lull's Soulseek status
/// already shows it down; the next settings change or launch starts it again.
fn report_exit(child: &mut Option<Child>, app_dir: &Path) {
    let Some(c) = child else { return };
    if let Ok(Some(status)) = c.try_wait() {
        let logs = app_dir.join("logs");
        tracing::error!(%status, logs = %logs.display(), "slskd stopped by itself");
        *child = None;
    }
}

fn spawn(fixed: &Fixed, launch: &Launch) -> Result<Child, String> {
    let complete = fixed.transfers.join("complete");
    let incomplete = fixed.transfers.join("incomplete");
    // slskd exits at start when a folder it is given doesn't exist.
    for dir in [&fixed.app_dir, &complete, &incomplete] {
        std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    }
    // The hub fills this file with the Soulseek account; slskd watches it from the start.
    let config = fixed.config_file();
    if !config.exists() {
        std::fs::write(&config, "").map_err(|e| format!("create {}: {e}", config.display()))?;
    }
    let mut cmd = Command::new(&fixed.exe);
    cmd.arg("--app-dir")
        .arg(&fixed.app_dir)
        .args(["--http-ip-address", "127.0.0.1", "--http-port", &fixed.port.to_string()])
        .args([
            "--no-https",
            "--headless",
            "--no-version-check",
            "--no-logo",
            "--no-color",
        ])
        // Headless still checks that its static folder exists, relative to slskd.exe: its own folder.
        .args(["--content-path", "."])
        .arg("--downloads")
        .arg(&complete)
        .arg("--incomplete")
        .arg(&incomplete)
        .env("SLSKD_API_KEY", &fixed.api_key)
        .env("SLSKD_USERNAME", "lull")
        .env("SLSKD_PASSWORD", &fixed.web_password)
        // slskd keeps its own log files in its app dir.
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(share) = &launch.share {
        cmd.arg("--shared").arg(share);
    }
    die_with_app::prepare(&mut cmd);
    let child = cmd
        .spawn()
        .map_err(|e| format!("can't start slskd ({}): {e}", fixed.exe.display()))?;
    die_with_app::adopt(&child);
    tracing::info!(pid = child.id(), port = fixed.port, "slskd started");
    Ok(child)
}

fn kill(child: &mut Option<Child>) {
    if let Some(mut c) = child.take() {
        let _ = c.kill();
        let _ = c.wait();
    }
}

/// slskd must never outlive the app, or it would keep the Soulseek login after a crash and log the
/// next launch out. Windows: a kill-on-close job object. Linux: a parent-death signal.
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

/// 32 random bytes as hex: slskd's API key and web password.
pub fn secret() -> Result<String, String> {
    let mut buf = [0u8; 32];
    getrandom::fill(&mut buf).map_err(|e| format!("random: {e}"))?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}
