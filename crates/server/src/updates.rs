//! `GET /api/updates`: this hub's build, lull's newest release, and which of the two is newer. The newest
//! release is GitHub's "latest" for lull's repository, which leaves out drafts and pre-releases (slskd's
//! builds for the Windows app are pre-releases). Asked only when someone looks (Settings › App), kept for six
//! hours; `?check=true` asks again, at most once a minute.
//!
//! `POST /api/updates/apply` (ADR-0090, behind the `hub-self-update` switch): a hub installed as the Linux
//! package or the image updates itself to that release. It downloads the release's hub kit for this machine
//! (`kit.rs`), checks it, copies the database, and exits to start again on it (`launch.rs`). One action, no
//! parameters: the newest release, and only one newer than this build.

use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use lull_core::Error;
use lull_core::build::{self, Build, Install};
use lull_core::flags;
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;
use tokio::time::Instant;

use crate::hub::Hub;
use crate::kit::{self, Manifest};
use crate::launch;

const LATEST: &str = "https://api.github.com/repos/TannhauserCommons/lull/releases/latest";

/// Where the newest release is read. A debug build may be compiled with a stand-in (`LULL_DEV_RELEASES`) to try
/// an update end to end; a release build reads only GitHub.
fn latest_url() -> &'static str {
    #[cfg(debug_assertions)]
    if let Some(url) = option_env!("LULL_DEV_RELEASES") {
        return url;
    }
    LATEST
}
const KEEP: Duration = Duration::from_secs(6 * 3600);
const AGAIN: Duration = Duration::from_secs(60);

pub fn router() -> Router<Arc<Hub>> {
    Router::new()
        .route("/api/updates", get(updates))
        .route("/api/updates/apply", post(apply))
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Latest {
    pub version: String,
    pub url: String,
    pub published_at: Option<String>,
    /// The release's files, by name: where a hub kit comes from.
    #[serde(skip)]
    pub assets: Vec<Asset>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Standing {
    /// The newest release, and nothing on top.
    Current,
    /// A newer release is out.
    Available,
    /// Everything in the newest release and changes since: it follows lull's main.
    Ahead,
    /// The newest release isn't known (GitHub didn't answer).
    Unknown,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    version: String,
    build: Build,
    latest: Option<Latest>,
    state: Standing,
    checked_at: Option<i64>,
    error: Option<String>,
    /// Behind the `hub-self-update` switch: whether this hub can update itself now, and how it went.
    #[serde(skip_serializing_if = "Option::is_none")]
    self_update: Option<SelfUpdate>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SelfUpdate {
    /// Update can be pressed.
    can: bool,
    /// Why not, in a plain sentence, when it can't and a newer release is out.
    why: Option<String>,
    /// What the update is doing now.
    step: Option<String>,
    /// Why the last press didn't get as far as the restart.
    error: Option<String>,
    /// An update that didn't start, which the hub went back from (`launch.rs`).
    failed: Option<serde_json::Value>,
}

#[derive(Clone)]
struct Checked {
    at: Instant,
    unix: i64,
    latest: Result<Latest, String>,
}

static LAST: Mutex<Option<Checked>> = Mutex::new(None);

#[derive(Deserialize)]
struct Ask {
    #[serde(default)]
    check: bool,
}

async fn updates(State(hub): State<Arc<Hub>>, Query(ask): Query<Ask>) -> Json<Report> {
    Json(report(&hub, ask.check).await)
}

async fn checked(hub: &Hub, fresh: bool) -> Checked {
    let last = LAST.lock().ok().and_then(|l| l.clone());
    let wait = if fresh { AGAIN } else { KEEP };
    match last {
        Some(c) if c.at.elapsed() < wait => c,
        _ => {
            let ctx = hub.ctx().await;
            let c = Checked {
                at: Instant::now(),
                unix: lull_core::now(),
                latest: newest(&ctx.http).await.map_err(|e| e.to_string()),
            };
            if let Ok(mut l) = LAST.lock() {
                *l = Some(c.clone());
            }
            c
        }
    }
}

async fn report(hub: &Hub, fresh: bool) -> Report {
    let checked = checked(hub, fresh).await;
    let (latest, error) = match checked.latest {
        Ok(l) => (Some(l), None),
        Err(e) => (None, Some(e)),
    };
    let state = standing(
        build::RELEASE,
        build::CHANGES,
        latest.as_ref().map(|l| l.version.as_str()),
    );
    let ctx = hub.ctx().await;
    let self_update = ctx.config.flags.on(&flags::HUB_SELF_UPDATE).then(|| {
        let why = blocked(&ctx.config, state, latest.as_ref()).err();
        let progress = PROGRESS.lock().map(|p| p.clone()).unwrap_or_default();
        SelfUpdate {
            can: why.is_none() && progress.step.is_none(),
            why: why.filter(|_| state == Standing::Available),
            step: progress.step,
            error: progress.error,
            failed: failed(&ctx.config.data_dir),
        }
    });
    Report {
        version: build::version(),
        build: build::build(),
        state,
        latest,
        checked_at: Some(checked.unix),
        error,
        self_update,
    }
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
    published_at: Option<String>,
    #[serde(default)]
    assets: Vec<Asset>,
}

async fn newest(http: &lull_core::http::Http) -> lull_core::Result<Latest> {
    let r: Release = http
        .json("github", |c| {
            c.get(latest_url()).header("Accept", "application/vnd.github+json")
        })
        .await?;
    let version = r.tag_name.trim().trim_start_matches('v').to_owned();
    if build::parse_release(&version).is_none() {
        return Err(Error::upstream(
            "github",
            format!("lull's newest release is named {:?}", r.tag_name),
        ));
    }
    Ok(Latest {
        version,
        url: r.html_url,
        published_at: r.published_at,
        assets: r.assets,
    })
}

/// What the update is doing; one at a time.
#[derive(Debug, Clone, Default)]
struct Progress {
    step: Option<String>,
    error: Option<String>,
}

static PROGRESS: Mutex<Progress> = Mutex::new(Progress {
    step: None,
    error: None,
});
static RESTARTING: AtomicBool = AtomicBool::new(false);
static RESTART: Notify = Notify::const_new();

/// Resolves once an update is ready and the hub should exit to start again on it (`lull serve` then exits
/// with `launch::UPDATE`).
pub async fn restart_requested() {
    RESTART.notified().await;
}

/// Whether the hub is exiting to update.
pub fn restarting() -> bool {
    RESTARTING.load(Ordering::SeqCst)
}

/// The kit's files a hub of this install uses: the package's programs, or the image's `lull` alone (the image
/// brings its own yt-dlp and Deno, and compose runs slskd).
fn wanted(install: Install, name: &str) -> bool {
    match install {
        Install::Docker => name == "lull",
        _ => true,
    }
}

/// Why this hub can't update itself to `latest`, or `Ok`.
fn blocked(config: &lull_core::config::Config, state: Standing, latest: Option<&Latest>) -> Result<String, String> {
    match build::install() {
        Install::Package | Install::Docker => {}
        Install::Desktop => return Err("It runs inside the desktop app and updates with it.".into()),
        Install::Server => {
            return Err("It wasn't installed as lull's package or image: update it the way it was installed.".into());
        }
    }
    if !config.self_update {
        return Err("Updating from here is off on this server (LULL_SELF_UPDATE=off).".into());
    }
    let Some(latest) = latest.filter(|_| state == Standing::Available) else {
        return Err("No newer release.".into());
    };
    let Some(arch) = kit::arch() else {
        return Err(format!(
            "lull has no update for this machine ({}).",
            std::env::consts::ARCH
        ));
    };
    if find(latest, &manifest_name(&latest.version, arch)).is_none() {
        return Err(format!(
            "{} has no update for this server yet: update it the way it was installed.",
            latest.version
        ));
    }
    Ok(latest.version.clone())
}

fn manifest_name(version: &str, arch: &str) -> String {
    format!("lull-hub-{version}-linux-{arch}.json")
}

fn find<'a>(latest: &'a Latest, name: &str) -> Option<&'a Asset> {
    latest.assets.iter().find(|a| a.name == name)
}

fn failed(data: &Path) -> Option<serde_json::Value> {
    let text = std::fs::read_to_string(kit::root(data).join(launch::FAILED)).ok()?;
    serde_json::from_str(&text).ok()
}

/// `POST /api/updates/apply`: starts the update and answers at once; `GET /api/updates` follows it.
async fn apply(State(hub): State<Arc<Hub>>) -> lull_core::Result<(StatusCode, Json<Report>)> {
    let ctx = hub.ctx().await;
    if !ctx.config.flags.on(&flags::HUB_SELF_UPDATE) {
        return Err(Error::NotFound("updating the hub from here isn't switched on".into()));
    }
    let checked = checked(&hub, true).await;
    let latest = checked.latest.map_err(|e| Error::upstream("github", e))?;
    let state = standing(build::RELEASE, build::CHANGES, Some(&latest.version));
    blocked(&ctx.config, state, Some(&latest)).map_err(Error::Conflict)?;
    {
        let mut progress = PROGRESS.lock().map_err(|_| Error::internal("update state poisoned"))?;
        if progress.step.is_some() {
            return Err(Error::Conflict("An update is already running.".into()));
        }
        *progress = Progress {
            step: Some(format!("Downloading lull {}", latest.version)),
            error: None,
        };
    }
    let worker = hub.clone();
    tokio::spawn(async move {
        let outcome = download(&worker, &latest).await;
        let mut progress = PROGRESS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        match outcome {
            Ok(()) => {
                progress.step = Some(format!("Restarting on lull {}", latest.version));
                drop(progress);
                tracing::info!(version = %latest.version, "updating: restarting on the new version");
                RESTARTING.store(true, Ordering::SeqCst);
                RESTART.notify_waiters();
            }
            Err(e) => {
                tracing::warn!(version = %latest.version, "the update stopped: {e}");
                *progress = Progress {
                    step: None,
                    error: Some(e.to_string()),
                };
            }
        }
    });
    Ok((StatusCode::ACCEPTED, Json(report(&hub, false).await)))
}

/// The release's kit into `<data>/hub/<version>/`, checked as the launcher will check it, then the copy of the
/// database the launcher goes back to if the new version doesn't start.
async fn download(hub: &Hub, latest: &Latest) -> lull_core::Result<()> {
    let ctx = hub.ctx().await;
    let arch = kit::arch().ok_or_else(|| Error::internal("no update for this machine"))?;
    let name = manifest_name(&latest.version, arch);
    let fetch = |name: String| {
        let http = ctx.http.client().clone();
        let url = find(latest, &name).map(|a| a.browser_download_url.clone());
        async move {
            let url = url.ok_or_else(|| Error::upstream("github", format!("the release has no {name}")))?;
            let resp = http
                .get(&url)
                .timeout(Duration::from_secs(30 * 60))
                .send()
                .await
                .map_err(|e| Error::upstream("github", format!("{name}: {e}")))?;
            let resp = lull_core::http::ok("github", resp)?;
            resp.bytes()
                .await
                .map_err(|e| Error::upstream("github", format!("{name}: {e}")))
        }
    };
    let manifest_bytes = fetch(name.clone()).await?;
    let signature = fetch(format!("{name}.sig")).await?;
    let signature = String::from_utf8_lossy(&signature).into_owned();
    if !kit::signed(&manifest_bytes, &signature) {
        return Err(Error::upstream(
            "github",
            "the update isn't signed by lull, so it wasn't installed",
        ));
    }
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|e| Error::upstream("github", format!("the update's manifest: {e}")))?;
    if manifest.version != latest.version || manifest.arch != arch {
        return Err(Error::upstream("github", "the update's manifest names another release"));
    }
    if manifest.format > kit::FORMAT {
        return Err(Error::Conflict(format!(
            "lull {} needs the new lull package or image: install it once where your server runs.",
            manifest.version
        )));
    }

    let root = kit::root(&ctx.config.data_dir);
    let partial = root.join(format!("{}.partial", manifest.version));
    let _ = std::fs::remove_dir_all(&partial);
    std::fs::create_dir_all(&partial).map_err(|e| Error::internal(format!("{}: {e}", partial.display())))?;
    let mut kept = Manifest {
        files: Default::default(),
        ..manifest.clone()
    };
    for (file, sha) in manifest.files.iter().filter(|(f, _)| wanted(build::install(), f)) {
        set_step(format!("Downloading lull {}: {file}", manifest.version));
        let gz = fetch(format!("lull-hub-{}-linux-{arch}-{file}.gz", manifest.version)).await?;
        let bytes = gunzip(&gz).map_err(|e| Error::upstream("github", format!("{file}: {e}")))?;
        if &kit::sha256(&bytes) != sha {
            return Err(Error::upstream(
                "github",
                format!("{file} isn't the file the update names"),
            ));
        }
        write_program(&partial.join(file), &bytes)?;
        kept.files.insert(file.clone(), sha.clone());
    }
    // The launcher checks the manifest's signature over its exact bytes, and every file it names: an image's
    // kit keeps only lull, so the files it didn't download are left out of what it checks.
    std::fs::write(partial.join(kit::MANIFEST), &manifest_bytes).map_err(io)?;
    std::fs::write(partial.join(kit::SIGNATURE), signature.as_bytes()).map_err(io)?;
    std::fs::write(
        partial.join(kit::KEPT),
        serde_json::to_vec(&kept).map_err(|e| Error::internal(e.to_string()))?,
    )
    .map_err(io)?;
    let dir = root.join(&manifest.version);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::rename(&partial, &dir).map_err(io)?;

    set_step("Saving a copy of your library".into());
    let backup = launch::backup_path(&ctx.config.data_dir, &manifest.version);
    if let Some(parent) = backup.parent() {
        lull_core::private::create_dir_all(parent).map_err(io)?;
    }
    let _ = std::fs::remove_file(&backup);
    // The copy holds what the database holds, so it is lull's alone from the start (ADR-0091): SQLite
    // fills an empty file it is given.
    lull_core::private::create_new(&backup).map_err(io)?;
    let to = backup.to_string_lossy().into_owned();
    ctx.db
        .call(move |conn| Ok(conn.execute("VACUUM INTO ?1", [to]).map(drop)?))
        .await?;
    Ok(())
}

fn set_step(step: String) {
    if let Ok(mut p) = PROGRESS.lock() {
        p.step = Some(step);
    }
}

fn gunzip(gz: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(gz).read_to_end(&mut out)?;
    Ok(out)
}

fn write_program(path: &Path, bytes: &[u8]) -> lull_core::Result<()> {
    std::fs::write(path, bytes).map_err(io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).map_err(io)?;
    }
    Ok(())
}

fn io(e: std::io::Error) -> Error {
    Error::internal(e.to_string())
}

/// Where a build stands against the newest release.
pub fn standing(release: &str, changes: u32, latest: Option<&str>) -> Standing {
    let (Some(have), Some(newest)) = (build::parse_release(release), latest.and_then(build::parse_release)) else {
        return Standing::Unknown;
    };
    if newest > have {
        Standing::Available
    } else if changes > 0 {
        Standing::Ahead
    } else {
        Standing::Current
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_build_stands_against_the_newest_release() {
        assert_eq!(standing("0.2.4", 0, Some("0.2.4")), Standing::Current);
        assert_eq!(standing("0.2.4", 0, Some("0.2.5")), Standing::Available);
        assert_eq!(standing("0.2.4", 12, Some("0.2.4")), Standing::Ahead);
        // main from before 0.2.5: behind it, changes or not
        assert_eq!(standing("0.2.4", 12, Some("0.2.5")), Standing::Available);
        // a build newer than any release (the release commit landed before the release was published)
        assert_eq!(standing("0.2.5", 0, Some("0.2.4")), Standing::Current);
        assert_eq!(standing("0.2.4", 0, None), Standing::Unknown);
        assert_eq!(standing("0.2.4", 0, Some("nightly")), Standing::Unknown);
    }
}
