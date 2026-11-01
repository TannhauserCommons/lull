//! Before Tauri's own build step:
//! - stages the web files the bundled pages need (welcome, connect, unreachable, mini player) into
//!   `dist/`, which `tauri.conf.json` embeds. Only these: `web/` also holds the full web UI, which the
//!   app loads from the hub, and untracked `node_modules`/`vendor` that must never end up in the binary;
//! - fetches the slskd sidecar for desktop targets: the pinned release zip, checked by sha256, then
//!   its binary as `binaries/slskd-<target>` (Tauri's sidecar naming) and its licence.

use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Read};
use std::path::Path;

const WEB: &str = "../web";
const DIST: &str = "dist";
const STAGED: &[&str] = &[
    "tokens.css",
    "base.css",
    "components.css",
    "icons.js",
    "fonts",
    "designs",
    "desktop",
    "assets",
];

/// The slskd the deploy kit runs too (ADR-0010). Bump the version and every hash together; the
/// hashes are the release's own (`gh release view <v> --repo slskd/slskd --json assets`).
const SLSKD_VERSION: &str = "0.26.2";
const SLSKD: &[Sidecar] = &[
    Sidecar {
        target: "x86_64-pc-windows-msvc",
        asset: "win-x64",
        sha256: "ffe186ecbedce15bf19b6a690c6213c669f34acc793ef53aa92390a5cd9c48f0",
    },
    Sidecar {
        target: "x86_64-unknown-linux-gnu",
        asset: "linux-x64",
        sha256: "53102e91e0123f62d17627d5e02a24d9dcd08a165e75182c0f1d76d087510f37",
    },
];
const BINARIES: &str = "binaries";

struct Sidecar {
    target: &'static str,
    asset: &'static str,
    sha256: &'static str,
}

fn main() -> io::Result<()> {
    stage_web()?;
    let target = std::env::var("TARGET").map_err(io::Error::other)?;
    // Phones get no hub, so no slskd (ADR-0002).
    if !target.contains("android") {
        fetch_slskd(&target)?;
    }
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        // App commands get permissions (allow-<name>) so capabilities can grant them one by one.
        tauri_build::AppManifest::new().commands(&[
            "discover_hubs",
            "find_hub",
            "window_action",
            "set_now_playing",
            "desktop_settings",
            "set_desktop_settings",
            "tailscale_status",
            "tailscale_serve",
        ]),
    ))
    .map_err(io::Error::other)
}

fn stage_web() -> io::Result<()> {
    let dist = Path::new(DIST);
    if dist.exists() {
        fs::remove_dir_all(dist)?;
    }
    fs::create_dir_all(dist)?;
    for entry in STAGED {
        let from = Path::new(WEB).join(entry);
        println!("cargo:rerun-if-changed={}", from.display());
        copy(&from, &dist.join(entry))?;
    }
    Ok(())
}

fn copy(from: &Path, to: &Path) -> io::Result<()> {
    if from.is_dir() {
        fs::create_dir_all(to)?;
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            copy(&entry.path(), &to.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        fs::copy(from, to).map(drop)
    }
}

/// The zip is kept in `binaries/` once verified, so only the first build downloads it. The binary is
/// unpacked again whenever its stamp doesn't name the pinned zip.
fn fetch_slskd(target: &str) -> io::Result<()> {
    let sidecar = SLSKD
        .iter()
        .find(|s| s.target == target)
        .ok_or_else(|| io::Error::other(format!("no slskd build is pinned for {target}; add it to build.rs")))?;
    let dir = Path::new(BINARIES);
    fs::create_dir_all(dir)?;
    let name = format!("slskd-{SLSKD_VERSION}-{}.zip", sidecar.asset);
    let zip = dir.join(&name);
    let exe_suffix = if target.contains("windows") { ".exe" } else { "" };
    let exe = dir.join(format!("slskd-{target}{exe_suffix}"));
    let stamp = dir.join(format!("slskd-{target}.sha256"));
    println!("cargo:rerun-if-changed={}", exe.display());
    if fs::read_to_string(&stamp).is_ok_and(|s| s == sidecar.sha256) && exe.is_file() {
        return Ok(());
    }

    let bytes = match fs::read(&zip) {
        Ok(bytes) if sha256(&bytes) == sidecar.sha256 => bytes,
        _ => {
            let url = format!("https://github.com/slskd/slskd/releases/download/{SLSKD_VERSION}/{name}");
            let bytes = download(&url)?;
            let got = sha256(&bytes);
            if got != sidecar.sha256 {
                return Err(io::Error::other(format!(
                    "{url} has sha256 {got}, not the pinned {}",
                    sidecar.sha256
                )));
            }
            fs::write(&zip, &bytes)?;
            bytes
        }
    };

    let mut archive = zip::ZipArchive::new(io::Cursor::new(bytes)).map_err(io::Error::other)?;
    extract(&mut archive, &format!("slskd{exe_suffix}"), &exe)?;
    extract(&mut archive, "LICENSE", &dir.join("slskd-LICENSE.txt"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&exe, fs::Permissions::from_mode(0o755))?;
    }
    fs::write(&stamp, sidecar.sha256)
}

fn download(url: &str) -> io::Result<Vec<u8>> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(600)))
        .build()
        .into();
    let mut bytes = Vec::new();
    agent
        .get(url)
        .call()
        .map_err(|e| io::Error::other(format!("{url}: {e}")))?
        .into_body()
        .into_with_config()
        .limit(256 * 1024 * 1024)
        .reader()
        .read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn extract(archive: &mut zip::ZipArchive<io::Cursor<Vec<u8>>>, entry: &str, to: &Path) -> io::Result<()> {
    let mut file = archive
        .by_name(entry)
        .map_err(|e| io::Error::other(format!("{entry}: {e}")))?;
    let mut out = fs::File::create(to)?;
    io::copy(&mut file, &mut out).map(drop)
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}
