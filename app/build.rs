//! Stages the web files the bundled pages need (connect, unreachable, mini player) into `dist/`,
//! which `tauri.conf.json` embeds. Only these: `web/` also holds the full web UI, which the wrapper
//! loads from the server, and untracked `node_modules`/`vendor` that must never end up in the binary.

use std::fs;
use std::io;
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

fn main() -> io::Result<()> {
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
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        // App commands get permissions (allow-<name>) so capabilities can grant them one by one.
        tauri_build::AppManifest::new().commands(&[
            "test_server",
            "window_action",
            "set_now_playing",
            "desktop_settings",
            "set_desktop_settings",
        ]),
    ))
    .map_err(io::Error::other)
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
