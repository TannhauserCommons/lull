//! The launcher (ADR-0090): the `lull` a package or image installed starts the newest hub kit in its data
//! folder (`kit.rs`) instead of itself, and watches it.
//! - A kit that never ran healthy gets 2 minutes to answer `/health` as its own version. Then it is marked
//!   good; otherwise it is stopped, marked bad, the database goes back to the copy taken before the update,
//!   and the next kit down (or the installed build) runs.
//! - A kit that exits to update ([`UPDATE`]) is followed by the newer one it downloaded.
//! - Any other exit ends the launcher with the same code, and systemd or Docker starts it again.
//! - SIGTERM is handed on: Docker signals only PID 1.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use lull_core::db::Db;
use lull_core::settings;

use crate::BoxError;
use crate::kit::{self, Kit};

/// The exit code that asks to start again on a newer kit (`updates.rs` exits with it).
pub const UPDATE: u8 = 75;
/// Set in a kit's environment: it runs itself and launches nothing.
const KIT: &str = "LULL_KIT";
/// How long a new kit may take to answer healthy.
const WINDOW: Duration = Duration::from_secs(120);

/// The programs a kit brings, by the setting that points at each.
const PROGRAMS: [(&str, &str); 3] = [("LULL_SLSKD", "slskd"), ("LULL_YTDLP", "yt-dlp"), ("LULL_DENO", "deno")];

/// Runs kits while there is one to run. `None`: none is (or self-update is off), so the installed build serves.
pub async fn kits(env: &BTreeMap<String, String>) -> Result<Option<ExitCode>, BoxError> {
    if env.contains_key(KIT) || !on(env).await? {
        return Ok(None);
    }
    let data = data_dir(env);
    let port = port(env);
    loop {
        let Some(kit) = kit::newest(&data, lull_core::build::RELEASE) else {
            return Ok(None);
        };
        let version = kit.manifest.version.clone();
        tracing::info!(%version, "starting the hub's update");
        let mut child = spawn(&kit, env)?;
        if !kit.is_good() {
            match healthy(&mut child, port, &version).await {
                Ok(()) => kit.mark_good(),
                Err(why) => {
                    tracing::error!(%version, "the update didn't start: {why}; going back");
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                    kit.mark_bad(&why);
                    restore(&data, &version)?;
                    record_failure(&data, &version, &why);
                    continue;
                }
            }
        }
        match wait(&mut child).await? {
            Some(UPDATE) => continue,
            code => return Ok(Some(ExitCode::from(code.unwrap_or(1)))),
        }
    }
}

/// `LULL_SELF_UPDATE`, from the environment or the stored settings: on unless `off`.
async fn on(env: &BTreeMap<String, String>) -> Result<bool, BoxError> {
    if let Some(v) = env.get("LULL_SELF_UPDATE").filter(|v| !v.is_empty()) {
        return Ok(v != "off");
    }
    let root = kit::root(&data_dir(env));
    if !root.exists() {
        return Ok(true); // no kit to run either way; nothing to open
    }
    let db = Db::open(&data_dir(env))?;
    let stored = settings::stored(&db).await?;
    Ok(stored.get("LULL_SELF_UPDATE").is_none_or(|v| v != "off"))
}

fn data_dir(env: &BTreeMap<String, String>) -> PathBuf {
    PathBuf::from(env.get("LULL_DATA_DIR").map_or("/data", String::as_str))
}

fn port(env: &BTreeMap<String, String>) -> u16 {
    env.get("LULL_LISTEN")
        .and_then(|l| l.rsplit(':').next())
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080)
}

/// `<kit>/lull serve`, with the package's programs from the kit where it brings them.
fn spawn(kit: &Kit, env: &BTreeMap<String, String>) -> Result<tokio::process::Child, BoxError> {
    let lull = kit.program("lull").ok_or("the kit has no lull")?;
    let mut cmd = tokio::process::Command::from(lull_core::process::command(&lull));
    // The kit's lull was built as the package's; it runs as whatever this lull was installed as.
    cmd.arg("serve")
        .env(KIT, &kit.dir)
        .env("LULL_INSTALL", lull_core::build::install().name())
        .kill_on_drop(true);
    for (key, name) in PROGRAMS {
        if env.contains_key(key)
            && let Some(path) = kit.program(name)
        {
            cmd.env(key, path);
        }
    }
    Ok(cmd.spawn()?)
}

/// Until the kit answers `/health` as its own version, for at most [`WINDOW`]; an exit before then fails it.
async fn healthy(child: &mut tokio::process::Child, port: u16, version: &str) -> Result<(), String> {
    let http = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{port}/health");
    let until = tokio::time::Instant::now() + WINDOW;
    while tokio::time::Instant::now() < until {
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!("it stopped ({status})"));
        }
        let answer = http.get(&url).timeout(Duration::from_secs(3)).send().await;
        if let Ok(resp) = answer
            && let Ok(health) = resp.json::<serde_json::Value>().await
            && health["version"] == version
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    Err(format!("it didn't answer within {} s", WINDOW.as_secs()))
}

/// Until the kit exits, handing SIGTERM on. Its exit code, or `None` when a signal ended it.
async fn wait(child: &mut tokio::process::Child) -> Result<Option<u8>, BoxError> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut term = signal(SignalKind::terminate())?;
        let mut int = signal(SignalKind::interrupt())?;
        loop {
            tokio::select! {
                status = child.wait() => return Ok(status?.code().and_then(|c| u8::try_from(c).ok())),
                _ = term.recv() => forward(child, nix::sys::signal::Signal::SIGTERM),
                _ = int.recv() => forward(child, nix::sys::signal::Signal::SIGTERM),
            }
        }
    }
    #[cfg(not(unix))]
    Ok(child.wait().await?.code().and_then(|c| u8::try_from(c).ok()))
}

#[cfg(unix)]
fn forward(child: &tokio::process::Child, signal: nix::sys::signal::Signal) {
    if let Some(pid) = child.id().and_then(|p| i32::try_from(p).ok()) {
        let _ = nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), signal);
    }
}

/// The database as it was before the update to `version` (`updates.rs` took the copy). An older lull refuses a
/// database a newer one migrated, so going back is only safe with it.
fn restore(data: &Path, version: &str) -> Result<(), BoxError> {
    let backup = backup_path(data, version);
    if !backup.exists() {
        return Err(format!("no copy of the database from before {version} to go back to").into());
    }
    let db = data.join("lull.db");
    for side in ["lull.db-wal", "lull.db-shm"] {
        match std::fs::remove_file(data.join(side)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
    }
    std::fs::copy(&backup, &db)?;
    Ok(())
}

/// Copies an older lull took before ADR-0091 were readable by others: lull's alone from now on.
pub fn restrict_backups(data: &Path) {
    let Ok(entries) = std::fs::read_dir(data.join("backups")) else {
        return;
    };
    for path in entries.flatten().map(|e| e.path()) {
        if let Err(e) = lull_core::private::restrict(&path) {
            tracing::warn!("can't make {} readable by lull alone: {e}", path.display());
        }
    }
}

/// Where the copy of the database from before the update to `version` is kept.
pub fn backup_path(data: &Path, version: &str) -> PathBuf {
    data.join("backups").join(format!("before-{version}.db"))
}

/// What Settings › App says after going back: `<data>/hub/failed.json`.
fn record_failure(data: &Path, version: &str, why: &str) {
    let note = serde_json::json!({ "version": version, "why": why, "at": lull_core::now() });
    let _ = std::fs::write(kit::root(data).join(FAILED), note.to_string());
}

pub const FAILED: &str = "failed.json";
