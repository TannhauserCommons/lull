//! Before Tauri's own build step:
//! - stages the web files the bundled pages need (welcome, connect, unreachable, mini player) into
//!   `dist/`, which `tauri.conf.json` embeds. Only these: `web/` also holds the full web UI, which the
//!   app loads from the hub, and untracked `node_modules`/`vendor` that must never end up in the binary;
//! - fetches the slskd sidecar for desktop targets: the pinned release zip, checked by sha256, then
//!   its binary as `binaries/slskd-<target>` (Tauri's sidecar naming) and its licence;
//! - fetches yt-dlp and Deno for the hub's YouTube source the same way, as `binaries/yt-dlp-<target>`
//!   and `binaries/deno-<target>` (docs/context/youtube.md).

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

/// The slskd the deploy kit runs too (ADR-0010), 0.26.2. Linux gets slskd's own release, checked by
/// the release's own hashes (`gh release view <v> --repo slskd/slskd --json assets`). Windows gets
/// lull's build of it (ADR-0062, `app/slskd/`): the same release with one line changed, so lull can
/// read a song while slskd downloads it; its hashes are what `.github/workflows/slskd-windows.yml`
/// builds, every time. Bump every entry together, and the deploy kit's image.
const SLSKD: &[Sidecar] = &[
    Sidecar {
        target: "x86_64-pc-windows-msvc",
        url: "https://github.com/TannhauserCommons/lull/releases/download/slskd-0.26.2-lull.1/slskd-0.26.2-lull.1-win-x64.zip",
        sha256: "40fe12e039047564865722671ff24a30e7513eec8aa4e7a5cb09819bbf9fbfe7",
    },
    Sidecar {
        target: "x86_64-unknown-linux-gnu",
        url: "https://github.com/slskd/slskd/releases/download/0.26.2/slskd-0.26.2-linux-x64.zip",
        sha256: "53102e91e0123f62d17627d5e02a24d9dcd08a165e75182c0f1d76d087510f37",
    },
    Sidecar {
        target: "aarch64-pc-windows-msvc",
        url: "https://github.com/TannhauserCommons/lull/releases/download/slskd-0.26.2-lull.1/slskd-0.26.2-lull.1-win-arm64.zip",
        sha256: "1d582f1845b0fb9e4580654595f9a610d945c43a06c16671b72aa57f74ad14dd",
    },
    Sidecar {
        target: "aarch64-unknown-linux-gnu",
        url: "https://github.com/slskd/slskd/releases/download/0.26.2/slskd-0.26.2-linux-arm64.zip",
        sha256: "b8acb302df08f7751f2068c779380dec136882adb5af82a081cd0ead8f028a51",
    },
];
const BINARIES: &str = "binaries";

struct Sidecar {
    target: &'static str,
    /// The release zip; tests/slskd-contract reads these three lines too.
    url: &'static str,
    sha256: &'static str,
}

/// yt-dlp and the JavaScript runtime it needs for YouTube, the versions the server image pins
/// (`Dockerfile`). Bump them there and here together; the hashes are the releases' own
/// (yt-dlp's `SHA2-256SUMS`; `gh release view v<version> --repo denoland/deno --json assets`).
const YTDLP_VERSION: &str = "2026.10.26";
const DENO_VERSION: &str = "2.9.8";
const YOUTUBE: &[Tool] = &[
    Tool {
        name: "yt-dlp",
        target: "x86_64-pc-windows-msvc",
        asset: "yt-dlp.exe",
        sha256: "f5389d3698ecde4ef06f4e6b1ce0294fe9ac1396228c8990b8d3416d6d177a7d",
        zipped: None,
    },
    Tool {
        name: "yt-dlp",
        target: "x86_64-unknown-linux-gnu",
        asset: "yt-dlp_linux",
        sha256: "767ba9dce7052aa196f8b12652f433d76de128ba168966fa84b6394d5e6c6795",
        zipped: None,
    },
    Tool {
        name: "deno",
        target: "x86_64-pc-windows-msvc",
        asset: "deno-x86_64-pc-windows-msvc.zip",
        sha256: "cba84b26e0df5ae7f74fc66b86a95b866872a86292ebd84fa1e959927edd93bf",
        zipped: Some("deno.exe"),
    },
    Tool {
        name: "deno",
        target: "x86_64-unknown-linux-gnu",
        asset: "deno-x86_64-unknown-linux-gnu.zip",
        sha256: "e890975bc2549807bbcc0b584ffafda72cc7c93349dd99c80483dd784f6c4609",
        zipped: Some("deno"),
    },
    Tool {
        name: "yt-dlp",
        target: "aarch64-pc-windows-msvc",
        asset: "yt-dlp_arm64.exe",
        sha256: "0066dbac3c124028e2dcae2a197fccee33cd91f456313d24f7af89fd54590e98",
        zipped: None,
    },
    Tool {
        name: "yt-dlp",
        target: "aarch64-unknown-linux-gnu",
        asset: "yt-dlp_linux_aarch64",
        sha256: "e87e9533e89fe5889abaa21f5114dfe7756d6aa6219b83551add04d48b18e483",
        zipped: None,
    },
    Tool {
        name: "deno",
        target: "aarch64-pc-windows-msvc",
        asset: "deno-aarch64-pc-windows-msvc.zip",
        sha256: "b413b92b735b50c198e8fe3b85af1aa50e29d5f72e229aab701d132286e14cef",
        zipped: Some("deno.exe"),
    },
    Tool {
        name: "deno",
        target: "aarch64-unknown-linux-gnu",
        asset: "deno-aarch64-unknown-linux-gnu.zip",
        sha256: "d49596c20b9788cf8f5d8ddf644103d4bde56ed239753f0bdb92ba9361d51905",
        zipped: Some("deno"),
    },
];

/// A program shipped beside the app: one release asset, the binary itself or a zip holding it.
struct Tool {
    name: &'static str,
    target: &'static str,
    asset: &'static str,
    sha256: &'static str,
    zipped: Option<&'static str>,
}

impl Tool {
    fn version(&self) -> &'static str {
        match self.name {
            "deno" => DENO_VERSION,
            _ => YTDLP_VERSION,
        }
    }

    fn url(&self) -> String {
        match self.name {
            "deno" => format!(
                "https://github.com/denoland/deno/releases/download/v{DENO_VERSION}/{}",
                self.asset
            ),
            _ => format!(
                "https://github.com/yt-dlp/yt-dlp/releases/download/{YTDLP_VERSION}/{}",
                self.asset
            ),
        }
    }
}

fn main() -> io::Result<()> {
    stage_web()?;
    let target = std::env::var("TARGET").map_err(io::Error::other)?;
    // Phones get no hub, so no slskd (ADR-0002).
    if !target.contains("android") {
        fetch_slskd(&target)?;
        for tool in YOUTUBE.iter().filter(|t| t.target == target) {
            fetch_tool(tool)?;
        }
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
            "classic_open",
            "classic_handoff",
            "classic_fit",
            "classic_back",
            "app_update",
            "app_update_check",
            "app_update_install",
            "resume_after_crash",
            "window_glass",
            "set_theme_package",
            "theme_package",
            "window_front",
            "sound_quiet",
            "reconnect_status",
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
    let name = sidecar.url.rsplit('/').next().unwrap_or(sidecar.url);
    let zip = dir.join(name);
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
            let url = sidecar.url;
            let bytes = download(url)?;
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

/// Like slskd: the verified asset is kept in `binaries/`, and the binary is put in place again
/// whenever its stamp doesn't name the pinned asset.
fn fetch_tool(tool: &Tool) -> io::Result<()> {
    let dir = Path::new(BINARIES);
    fs::create_dir_all(dir)?;
    let suffix = if tool.target.contains("windows") { ".exe" } else { "" };
    let exe = dir.join(format!("{}-{}{suffix}", tool.name, tool.target));
    let stamp = dir.join(format!("{}-{}.sha256", tool.name, tool.target));
    println!("cargo:rerun-if-changed={}", exe.display());
    if fs::read_to_string(&stamp).is_ok_and(|s| s == tool.sha256) && exe.is_file() {
        return Ok(());
    }
    // A zip is kept to unpack again; a bare binary is the file itself.
    let kept = tool
        .zipped
        .map(|_| dir.join(format!("{}-{}-{}", tool.name, tool.version(), tool.asset)));
    let bytes = match kept.as_ref().map(fs::read) {
        Some(Ok(bytes)) if sha256(&bytes) == tool.sha256 => bytes,
        _ => {
            let url = tool.url();
            let bytes = download(&url)?;
            let got = sha256(&bytes);
            if got != tool.sha256 {
                return Err(io::Error::other(format!(
                    "{url} has sha256 {got}, not the pinned {}",
                    tool.sha256
                )));
            }
            if let Some(kept) = &kept {
                fs::write(kept, &bytes)?;
            }
            bytes
        }
    };
    match tool.zipped {
        Some(entry) => {
            let mut archive = zip::ZipArchive::new(io::Cursor::new(bytes)).map_err(io::Error::other)?;
            extract(&mut archive, entry, &exe)?;
        }
        None => fs::write(&exe, &bytes)?,
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&exe, fs::Permissions::from_mode(0o755))?;
    }
    fs::write(&stamp, tool.sha256)
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
