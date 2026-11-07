//! Hub kits (ADR-0090): what a hub installed as the Linux package or the image updates itself with. A release
//! publishes, for each architecture, `lull` (and the package's slskd, yt-dlp and Deno) gzipped, and a manifest
//! naming each file's SHA-256, signed with the desktop updater's minisign key. A hub downloads a kit into
//! `<data>/hub/<version>/` (`updates.rs`); the launcher (`launch.rs`) runs the newest one that verifies.
//!
//! Every start checks the signature and every file again, so a file put in the data folder some other way never
//! runs. A kit is only ever newer than the build installed, and a newer package or image wins over older kits.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The desktop updater's public key (`app/tauri.conf.json`, `plugins.updater.pubkey`), as Tauri writes it:
/// base64 of minisign's key file. A test holds the two equal.
pub const PUBLIC_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDc0QTU2NERGRTlCQ0FCNEUKUldST3E3enAzMlNsZENQaVduZno1S2FEbWZ0NVBGWHdnYldNaWZwVm1qSDF1SHRMd1NsSUsvdjcK";

/// How the launcher runs a kit: `<kit>/lull serve`, with the package's programs from the kit. A kit that needs
/// a newer way says so with a higher format, and isn't run until the package or image is updated.
pub const FORMAT: u32 = 1;

/// A kit's manifest (`manifest.json`, signed in `manifest.json.sig`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Manifest {
    pub version: String,
    pub format: u32,
    /// `x64` or `arm64`.
    pub arch: String,
    /// Each program's name (`lull`, `slskd`, `yt-dlp`, `deno`) and the SHA-256 of its unpacked bytes.
    pub files: BTreeMap<String, String>,
}

pub const MANIFEST: &str = "manifest.json";
pub const SIGNATURE: &str = "manifest.json.sig";
/// The files of the manifest a hub downloaded (an image keeps only `lull`): each must be in the signed
/// manifest with the same SHA-256, so this file needs no signature of its own.
pub const KEPT: &str = "kept.json";
/// Marks in a kit's folder: it ran healthy once, or it didn't start (the reason inside).
pub(crate) const GOOD: &str = "good";
const BAD: &str = "bad";

/// This machine's architecture as releases name it.
pub fn arch() -> Option<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Some("x64"),
        "aarch64" => Some("arm64"),
        _ => None,
    }
}

/// The folder kits live in.
pub fn root(data: &Path) -> PathBuf {
    data.join("hub")
}

/// A manifest's bytes and its signature as published: true when the desktop updater's key signed them.
pub fn signed(manifest: &[u8], signature: &str) -> bool {
    signed_by(key(), manifest, signature)
}

/// The key kits are checked with. A debug build may be compiled with another (`LULL_DEV_KIT_KEY`), to try an
/// update end to end with kits signed by a throwaway key; a release build has only lull's.
fn key() -> &'static str {
    #[cfg(debug_assertions)]
    if let Some(key) = option_env!("LULL_DEV_KIT_KEY") {
        return key;
    }
    PUBLIC_KEY
}

fn signed_by(public_key: &str, manifest: &[u8], signature: &str) -> bool {
    let decode = |b64: &str| {
        base64::engine::general_purpose::STANDARD
            .decode(b64.trim())
            .ok()
            .and_then(|b| String::from_utf8(b).ok())
    };
    let (Some(key), Some(sig)) = (decode(public_key), decode(signature)) else {
        return false;
    };
    let (Ok(key), Ok(sig)) = (
        minisign_verify::PublicKey::decode(&key),
        minisign_verify::Signature::decode(&sig),
    ) else {
        return false;
    };
    key.verify(manifest, &sig, false).is_ok()
}

pub fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// A kit on disk that verifies: its folder, its signed manifest and the files it holds.
#[derive(Debug, Clone)]
pub struct Kit {
    pub dir: PathBuf,
    pub manifest: Manifest,
    pub files: BTreeMap<String, String>,
}

impl Kit {
    /// The kit in `dir`, if its manifest is signed and names this machine and a format the launcher knows, and
    /// every file it holds is one the manifest names, there with its SHA-256.
    pub fn verified(dir: &Path) -> Result<Kit, String> {
        let bytes = std::fs::read(dir.join(MANIFEST)).map_err(|e| format!("no manifest: {e}"))?;
        let signature = std::fs::read_to_string(dir.join(SIGNATURE)).map_err(|e| format!("no signature: {e}"))?;
        if !signed(&bytes, &signature) {
            return Err("its manifest isn't signed by lull".into());
        }
        let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|e| format!("its manifest: {e}"))?;
        if Some(manifest.arch.as_str()) != arch() {
            return Err(format!("it is for {}", manifest.arch));
        }
        if manifest.format > FORMAT {
            return Err("it needs a newer package or image".into());
        }
        let files = match std::fs::read(dir.join(KEPT)) {
            Ok(kept) => {
                serde_json::from_slice::<Manifest>(&kept)
                    .map_err(|e| format!("{KEPT}: {e}"))?
                    .files
            }
            Err(_) => manifest.files.clone(),
        };
        for (name, sha) in &files {
            if manifest.files.get(name) != Some(sha) {
                return Err(format!("{name} isn't in its signed manifest"));
            }
            let file = std::fs::read(dir.join(name)).map_err(|e| format!("{name}: {e}"))?;
            if &sha256(&file) != sha {
                return Err(format!("{name} isn't the file its manifest names"));
            }
        }
        if !files.contains_key("lull") {
            return Err("it has no lull".into());
        }
        Ok(Kit {
            dir: dir.to_owned(),
            manifest,
            files,
        })
    }

    pub fn program(&self, name: &str) -> Option<PathBuf> {
        self.files.contains_key(name).then(|| self.dir.join(name))
    }

    pub fn is_good(&self) -> bool {
        self.dir.join(GOOD).exists()
    }

    pub fn mark_good(&self) {
        let _ = std::fs::write(self.dir.join(GOOD), "");
    }

    pub fn mark_bad(&self, why: &str) {
        let _ = std::fs::write(self.dir.join(BAD), why);
    }
}

/// The newest kit in `<data>/hub` that verifies, is newer than this build's release and didn't fail before.
/// Kits this build is as new as are deleted: a newer package or image wins.
pub fn newest(data: &Path, own: &str) -> Option<Kit> {
    let own = lull_core::build::parse_release(own)?;
    let entries = std::fs::read_dir(root(data)).ok()?;
    let mut best: Option<Kit> = None;
    for entry in entries.flatten() {
        let dir = entry.path();
        let Some(version) = dir
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(lull_core::build::parse_release)
        else {
            continue;
        };
        if version <= own {
            let _ = std::fs::remove_dir_all(&dir);
            continue;
        }
        if dir.join(BAD).exists() {
            continue;
        }
        match Kit::verified(&dir) {
            Ok(kit) if Some(version) == lull_core::build::parse_release(&kit.manifest.version) => {
                let newer = best
                    .as_ref()
                    .and_then(|b| lull_core::build::parse_release(&b.manifest.version))
                    .is_none_or(|b| version > b);
                if newer {
                    best = Some(kit);
                }
            }
            Ok(_) => tracing::warn!(dir = %dir.display(), "a hub kit's folder and manifest name different versions"),
            Err(why) => tracing::warn!(dir = %dir.display(), "a hub kit isn't used: {why}"),
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_is_the_desktop_updaters() -> Result<(), Box<dyn std::error::Error>> {
        let conf: serde_json::Value = serde_json::from_str(include_str!("../../../app/tauri.conf.json"))?;
        assert_eq!(conf["plugins"]["updater"]["pubkey"], PUBLIC_KEY);
        Ok(())
    }

    /// A throwaway key (`tauri signer generate`) and its signature of `{"version":"9.9.9"}`.
    const TEST_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEZCNEIwRDIyRjcwRDc3RjgKUldUNGR3MzNJZzFMKzI1QzE4YlhoR3ZBLzV4aWQxRFJaUnRwQk9nUDJZd1hGNGF2TXd1bnVJTlAK";
    const TEST_SIGNATURE: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVUNGR3MzNJZzFMKytQbzhLR1VrWElpNUR5dkR3Rk4wUVJCSW5Helpydk8wVllmOSs0V3kyQk9SeWhvWnFnSzc3QXJZQ21FV1oyN3U5US93MEIyZlE3NE9UYUJhNkVQL2dRPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkzOTcxNDExCWZpbGU6bS5qc29uCnlWOUg1Y012YXU4RkdlZTFhRGcrc0hpbEQrOGJVTEFUUTNQRWt4eWtyZ29WVUtsWFlGU1hrNi83L0pqQ0VtWWFpa3JXZWFTRDlSMC9lTWJaSVF4M0RRPT0K";

    #[test]
    fn only_the_bytes_the_key_signed_pass() {
        let manifest = br#"{"version":"9.9.9"}"#;
        assert!(signed_by(TEST_KEY, manifest, TEST_SIGNATURE));
        assert!(
            !signed_by(TEST_KEY, br#"{"version":"9.9.8"}"#, TEST_SIGNATURE),
            "altered bytes"
        );
        assert!(!signed(manifest, TEST_SIGNATURE), "another key's signature");
        assert!(!signed(manifest, ""));
        assert!(!signed(manifest, "not base64"));
    }
}
