//! The slskd a hub runs itself (ADR-0006, ADR-0010, ADR-0090): the desktop app's bundled one and the
//! Linux package's. Whoever runs the hub owns slskd's lifecycle, folders, port and API key. It is
//! restarted when what it shares changes, when it exits by itself (with backoff), and when the hub
//! says it's wedged (`restartSlskd`: headless slskd refuses the web login the hub would restart it
//! with). The hub writes the Soulseek account into its `slskd.yml` (`LULL_SLSKD_CONFIG_FILE`,
//! docs/context/soulseek.md). slskd's web UI stays off: lull is the UI.
//!
//! slskd reads, lowest first: env, `slskd.yml`, the command line. Fixed things go on the command
//! line, secrets in env (not in the process list).

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// How often the supervisor looks whether slskd is still running.
const WATCH: Duration = Duration::from_secs(5);

/// The wait before starting slskd again after it stopped by itself; doubles while it keeps stopping.
const RETRY_FIRST: Duration = Duration::from_secs(5);
const RETRY_MAX: Duration = Duration::from_secs(5 * 60);

/// slskd that ran this long before stopping counts as a fresh failure: the wait starts over.
const RAN_FINE: Duration = Duration::from_secs(10 * 60);

/// What slskd runs with for the whole life of the hub. The hub was started with this port and key,
/// so a restarted slskd keeps them.
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
    pub tie: Tie,
}

/// How slskd is kept from outliving whoever runs it: a leftover slskd would keep the Soulseek login
/// and log the next start out. The desktop app ties it to its own process (a kill-on-close job
/// object on Windows, a parent-death signal on Linux). A systemd service needs nothing: systemd
/// stops every process of the service with it.
#[derive(Clone, Copy)]
pub struct Tie {
    /// Before the spawn, on the command.
    pub prepare: fn(&mut Command),
    /// After the spawn, on the child.
    pub adopt: fn(&Child),
}

impl Tie {
    pub const NONE: Tie = Tie {
        prepare: |_| {},
        adopt: |_| {},
    };
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
    Restart,
    Stop(mpsc::Sender<()>),
}

/// The thread that owns the slskd process. On Linux a parent-death signal ties the process to the
/// thread that spawned it (PR_SET_PDEATHSIG), so one long-lived thread does every spawn.
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

    /// Restarts slskd whatever it runs with: the hub found it wedged.
    pub fn restart(&self) {
        let _ = self.tx.send(Order::Restart);
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
    let mut slskd = match spawn(fixed, &launch) {
        Ok(child) => {
            let _ = started.send(Ok(()));
            Process::running(child)
        }
        Err(e) => {
            let _ = started.send(Err(e));
            return;
        }
    };
    loop {
        match rx.recv_timeout(WATCH) {
            Err(mpsc::RecvTimeoutError::Timeout) => slskd.watch(fixed, &launch),
            Ok(Order::Apply(next)) if next != launch => {
                tracing::info!(?next, "restarting slskd with new settings");
                launch = next;
                slskd.restart(fixed, &launch);
            }
            Ok(Order::Apply(_)) => {}
            Ok(Order::Restart) => {
                tracing::warn!("the hub found slskd wedged; restarting it");
                slskd.restart(fixed, &launch);
            }
            Ok(Order::Stop(done)) => {
                slskd.kill();
                let _ = done.send(());
                return;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return slskd.kill(),
        }
    }
}

/// The slskd process, or when to try starting it again.
struct Process {
    child: Option<Child>,
    started: Instant,
    retry_at: Option<Instant>,
    /// The wait after its next stop.
    wait: Duration,
}

impl Process {
    fn running(child: Child) -> Self {
        Process {
            child: Some(child),
            started: Instant::now(),
            retry_at: None,
            wait: RETRY_FIRST,
        }
    }

    /// slskd stopping by itself is logged, with where its own log is, and it starts again after a
    /// wait that doubles while it keeps stopping. Meanwhile lull's Soulseek status shows it down.
    fn watch(&mut self, fixed: &Fixed, launch: &Launch) {
        if let Some(c) = &mut self.child
            && let Ok(Some(status)) = c.try_wait()
        {
            self.child = None;
            let wait = retry_wait(self.wait, self.started.elapsed());
            let logs = fixed.app_dir.join("logs");
            tracing::error!(%status, logs = %logs.display(), "slskd stopped by itself; starting it again in {} s", wait.as_secs());
            self.schedule(wait);
        }
        if self.retry_at.is_some_and(|at| Instant::now() >= at) {
            self.start(fixed, launch);
        }
    }

    fn restart(&mut self, fixed: &Fixed, launch: &Launch) {
        self.kill();
        self.wait = RETRY_FIRST;
        self.start(fixed, launch);
    }

    fn start(&mut self, fixed: &Fixed, launch: &Launch) {
        self.retry_at = None;
        match spawn(fixed, launch) {
            Ok(child) => {
                self.child = Some(child);
                self.started = Instant::now();
            }
            Err(e) => {
                tracing::error!("slskd didn't start: {e}; trying again in {} s", self.wait.as_secs());
                self.schedule(self.wait);
            }
        }
    }

    fn schedule(&mut self, wait: Duration) {
        self.retry_at = Some(Instant::now() + wait);
        self.wait = (wait * 2).min(RETRY_MAX);
    }

    fn kill(&mut self) {
        self.retry_at = None;
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

/// How long to wait before starting slskd again: `wait`, unless it ran fine for a while first.
fn retry_wait(wait: Duration, ran: Duration) -> Duration {
    if ran >= RAN_FINE { RETRY_FIRST } else { wait }
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
    let mut cmd = lull_core::process::command(&fixed.exe);
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
    (fixed.tie.prepare)(&mut cmd);
    let child = cmd
        .spawn()
        .map_err(|e| format!("can't start slskd ({}): {e}", fixed.exe.display()))?;
    (fixed.tie.adopt)(&child);
    tracing::info!(pid = child.id(), port = fixed.port, "slskd started");
    Ok(child)
}

/// 32 random bytes as hex: slskd's API key and web password.
pub fn secret() -> Result<String, String> {
    let mut buf = [0u8; 32];
    getrandom::fill(&mut buf).map_err(|e| format!("random: {e}"))?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}
