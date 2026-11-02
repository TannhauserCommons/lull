//! Keep yt-dlp up to date (on by default): YouTube changes often and old yt-dlp releases stop
//! working. Once a day the hub asks GitHub for yt-dlp's newest stable release; a newer one is
//! downloaded into the data folder, checked against the release's published SHA2-256SUMS, and used
//! only once it runs. The pinned binary stays as the fallback (docs/context/youtube.md#yt-dlp).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::StreamExt;
use lull_core::{Error, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio::sync::Notify;

use crate::Service;
use crate::ytdlp::version_of;

const LATEST: &str = "https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest";
const SUMS: &str = "SHA2-256SUMS";
const FIRST_CHECK: Duration = Duration::from_secs(60);
const EVERY: Duration = Duration::from_secs(24 * 3600);
/// A release is about 35 MB.
const DOWNLOAD_TIME: Duration = Duration::from_secs(10 * 60);
/// The file in `<data>/youtube/bin` naming the updated binary in use.
const MARKER: &str = "current";

/// The last check, for `/api/youtube`.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Checked {
    /// yt-dlp's newest stable release, as GitHub said.
    pub latest: Option<String>,
    pub checked_at: Option<i64>,
    /// Why the last check or update failed, in plain words.
    pub error: Option<String>,
}

#[derive(Default)]
pub struct State {
    checked: Mutex<Checked>,
    wake: Notify,
}

impl State {
    /// The setting changed: pick the binary again, and check now when it is on.
    pub fn wake(&self) {
        self.wake.notify_one();
    }

    pub fn checked(&self) -> Checked {
        self.checked.lock().map(|c| c.clone()).unwrap_or_default()
    }

    fn record(&self, change: impl FnOnce(&mut Checked)) {
        if let Ok(mut c) = self.checked.lock() {
            change(&mut c);
        }
    }
}

pub async fn run(svc: Arc<Service>) {
    use_installed(&svc).await;
    let mut wait = FIRST_CHECK;
    loop {
        tokio::select! {
            () = tokio::time::sleep(wait) => {}
            () = svc.updates.wake.notified() => use_installed(&svc).await,
        }
        wait = EVERY;
        if svc.settings().auto_update {
            check(&svc).await;
        }
    }
}

/// Checks for a newer yt-dlp now and installs it; the outcome is in `State::checked`.
pub async fn check(svc: &Service) {
    let outcome = update(svc).await;
    svc.updates.record(|c| {
        c.checked_at = Some(lull_core::now());
        c.error = outcome.err().map(|e| e.to_string());
    });
    svc.report();
}

/// The updated binary while keeping up to date (and it runs and is newer), else the pinned one.
async fn use_installed(svc: &Service) {
    let installed = match svc.settings().auto_update {
        true => installed(&svc.dir.join("bin")).await,
        false => None,
    };
    svc.ytdlp.choose(installed).await;
    svc.report();
}

async fn installed(bin: &Path) -> Option<PathBuf> {
    let name = tokio::fs::read_to_string(bin.join(MARKER)).await.ok()?;
    let path = bin.join(name.trim());
    path.is_file().then_some(path)
}

async fn update(svc: &Service) -> Result<()> {
    let release: Release = svc
        .ctx
        .http
        .json("github", |c| {
            c.get(LATEST).header("Accept", "application/vnd.github+json")
        })
        .await?;
    let tag = release.tag_name.trim().to_owned();
    if !plain_version(&tag) {
        return Err(Error::upstream(
            "github",
            format!("yt-dlp's latest release is named {tag:?}"),
        ));
    }
    svc.updates.record(|c| c.latest = Some(tag.clone()));
    let have = [svc.ytdlp.current().version, svc.ytdlp.pinned_version()];
    if have.iter().flatten().any(|v| !newer(&tag, v)) {
        return Ok(());
    }
    let asset = asset().ok_or_else(|| Error::unavailable("youtube", "no yt-dlp release for this platform"))?;
    let url = |name: &str| {
        release
            .assets
            .iter()
            .find(|a| a.name == name)
            .map(|a| a.browser_download_url.clone())
            .ok_or_else(|| Error::upstream("github", format!("yt-dlp {tag} has no {name}")))
    };
    let (binary_url, sums_url) = (url(asset)?, url(SUMS)?);
    let sums = lull_core::http::ok("github", svc.ctx.http.send("github", |c| c.get(&sums_url)).await?)?
        .text()
        .await
        .map_err(|e| Error::upstream("github", format!("reading {SUMS}: {e}")))?;
    let expected = checksum(&sums, asset)
        .ok_or_else(|| Error::upstream("github", format!("{SUMS} of yt-dlp {tag} doesn't list {asset}")))?;

    let bin = svc.dir.join("bin");
    let file = bin.join(format!("yt-dlp-{tag}{}", std::env::consts::EXE_SUFFIX));
    let part = bin.join(format!("yt-dlp-{tag}.part"));
    let got = fetch(svc, &binary_url, &part).await;
    let got = match got {
        Ok(sum) if sum == expected => Ok(()),
        Ok(sum) => Err(Error::upstream(
            "github",
            format!("yt-dlp {tag} didn't match its published checksum ({sum}); not using it"),
        )),
        Err(e) => Err(e),
    };
    if let Err(e) = got {
        let _ = tokio::fs::remove_file(&part).await;
        return Err(e);
    }
    executable(&part).await?;
    tokio::fs::rename(&part, &file)
        .await
        .map_err(|e| Error::internal(format!("installing yt-dlp {tag}: {e}")))?;
    if version_of(&file, &svc.dir).await.as_deref() != Some(tag.as_str()) {
        let _ = tokio::fs::remove_file(&file).await;
        return Err(Error::unavailable(
            "youtube",
            format!("yt-dlp {tag} doesn't run here; keeping the one in use"),
        ));
    }
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    tokio::fs::write(bin.join(MARKER), &name)
        .await
        .map_err(|e| Error::internal(format!("recording yt-dlp {tag}: {e}")))?;
    svc.ytdlp.choose(Some(file.clone())).await;
    tracing::info!(version = tag, "yt-dlp updated");
    remove_others(&bin, &name).await;
    Ok(())
}

/// Streams the release to `part` and returns its SHA-256, lowercase hex.
async fn fetch(svc: &Service, url: &str, part: &Path) -> Result<String> {
    let resp = svc
        .ctx
        .http
        .client()
        .get(url)
        .timeout(DOWNLOAD_TIME)
        .send()
        .await
        .map_err(|e| Error::unavailable("github", format!("downloading yt-dlp: {e}")))?;
    let resp = lull_core::http::ok("github", resp)?;
    let mut out = tokio::fs::File::create(part)
        .await
        .map_err(|e| Error::internal(format!("writing {}: {e}", part.display())))?;
    let mut hash = Sha256::new();
    let mut body = resp.bytes_stream();
    while let Some(chunk) = body.next().await {
        let chunk = chunk.map_err(|e| Error::unavailable("github", format!("downloading yt-dlp: {e}")))?;
        hash.update(&chunk);
        out.write_all(&chunk)
            .await
            .map_err(|e| Error::internal(format!("writing {}: {e}", part.display())))?;
    }
    out.flush()
        .await
        .map_err(|e| Error::internal(format!("writing {}: {e}", part.display())))?;
    Ok(hex(&hash.finalize()))
}

#[cfg(unix)]
async fn executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .await
        .map_err(|e| Error::internal(format!("making yt-dlp executable: {e}")))
}

#[cfg(not(unix))]
async fn executable(_: &Path) -> Result<()> {
    Ok(())
}

/// Older updated binaries go once a newer one is in use.
async fn remove_others(bin: &Path, keep: &str) {
    let Ok(mut entries) = tokio::fs::read_dir(bin).await else {
        return;
    };
    while let Ok(Some(e)) = entries.next_entry().await {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with("yt-dlp-") && name != keep {
            let _ = tokio::fs::remove_file(e.path()).await;
        }
    }
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

/// The standalone release for this platform.
fn asset() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some("yt-dlp_linux"),
        ("linux", "aarch64") => Some("yt-dlp_linux_aarch64"),
        ("windows", "x86_64") => Some("yt-dlp.exe"),
        ("windows", "aarch64") => Some("yt-dlp_arm64.exe"),
        ("macos", _) => Some("yt-dlp_macos"),
        _ => None,
    }
}

/// The checksum `SHA2-256SUMS` lists for `asset` ("<hex>  <name>" lines).
fn checksum(sums: &str, asset: &str) -> Option<String> {
    sums.lines().find_map(|l| {
        let (sum, name) = l.trim().split_once(char::is_whitespace)?;
        (name.trim().trim_start_matches('*') == asset && sum.len() == 64).then(|| sum.to_ascii_lowercase())
    })
}

/// yt-dlp versions are dates, sometimes with a fourth part: "2026.10.26", "2026.10.26.1".
fn plain_version(v: &str) -> bool {
    !v.is_empty()
        && v.split('.')
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

/// Whether version `a` is newer than `b`.
pub fn newer(a: &str, b: &str) -> bool {
    let parts = |v: &str| v.split('.').map(|p| p.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    parts(a) > parts(b)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_as_dates_with_an_optional_fourth_part() {
        assert!(newer("2026.10.26", "2026.10.21"));
        assert!(newer("2026.10.26.1", "2026.10.26"));
        assert!(!newer("2026.10.26", "2026.10.26"));
        assert!(!newer("2026.9.30", "2026.10.1"));
        assert!(plain_version("2026.10.26") && !plain_version("../x") && !plain_version("nightly"));
    }

    #[test]
    fn the_published_checksum_is_read_for_this_asset_only() {
        let sums = "e51e69e89489f199530f13d32a4342da3ddb2cf34842f364603a066db05afbf3  yt-dlp\n\
                    767ba9dce7052aa196f8b12652f433d76de128ba168966fa84b6394d5e6c6795  yt-dlp_linux\n\
                    f34053415136e55073dd64ba5326d161ecc23ce0e98c081d8752867b37809450  yt-dlp_linux.zip\n";
        assert_eq!(
            checksum(sums, "yt-dlp_linux").as_deref(),
            Some("767ba9dce7052aa196f8b12652f433d76de128ba168966fa84b6394d5e6c6795")
        );
        assert_eq!(checksum(sums, "yt-dlp_macos"), None);
        assert_eq!(
            hex(&Sha256::digest(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
