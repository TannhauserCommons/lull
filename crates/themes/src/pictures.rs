//! The pictures lull makes of a listener's theme for Settings › Appearance (ADR-0080): drawn by a
//! page, kept here beside the package (never in it, so a picture never fires `theme.changed` or
//! travels in an export), each named by the `rev` of the files it shows. A theme's `rev` changes
//! with any of its files and with lull's version, so a picture of an older theme is never shown.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io;
use std::path::{Path, PathBuf};

use lull_core::{Error, Result};

use crate::package::Mode;
use crate::watch;

/// The picker's picture size, the same as a package's own `assets/preview-<mode>.webp`.
pub const WIDTH: u32 = 320;
pub const HEIGHT: u32 = 200;
const MAX_BYTES: usize = 256 * 1024;

pub struct Pictures {
    /// `<data dir>/theme-pictures/`, one folder per theme id
    pub dir: PathBuf,
}

/// What a theme looks like now: its files (as the live-reload watcher sees them) and this lull.
pub fn rev(theme_dir: &Path) -> String {
    let mut h = DefaultHasher::new();
    watch::fingerprint(theme_dir, &mut h);
    env!("CARGO_PKG_VERSION").hash(&mut h);
    format!("{:016x}", h.finish())
}

/// `<rev>-<mode>.webp`, as the routes name a picture.
pub fn parse_name(name: &str) -> Option<(&str, Mode)> {
    let (rev, mode) = name.strip_suffix(".webp")?.split_once('-')?;
    let mode = match mode {
        "day" => Mode::Day,
        "night" => Mode::Night,
        _ => return None,
    };
    (rev.len() == 16 && rev.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))).then_some((rev, mode))
}

impl Pictures {
    /// The modes with a picture made for `rev`.
    pub fn made(&self, id: &str, rev: &str) -> Vec<Mode> {
        [Mode::Day, Mode::Night]
            .into_iter()
            .filter(|m| self.path(id, rev, *m).is_file())
            .collect()
    }

    pub fn file(&self, id: &str, rev: &str, mode: Mode) -> Result<Vec<u8>> {
        fs::read(self.path(id, rev, mode)).map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => Error::NotFound(format!("no picture of theme {id} for {rev}")),
            _ => io_err(e),
        })
    }

    /// Stores a checked picture and drops the theme's pictures of older revs.
    pub fn put(&self, id: &str, rev: &str, mode: Mode, webp: &[u8]) -> Result<()> {
        check_webp(webp)?;
        let dir = self.dir.join(id);
        fs::create_dir_all(&dir).map_err(io_err)?;
        let target = self.path(id, rev, mode);
        let tmp = dir.join(format!(".{rev}-{}.webp", mode.word()));
        fs::write(&tmp, webp)
            .and_then(|()| fs::rename(&tmp, &target))
            .map_err(io_err)?;
        for entry in fs::read_dir(&dir).map_err(io_err)?.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if parse_name(&name).is_none_or(|(r, _)| r != rev) {
                let _ = fs::remove_file(entry.path());
            }
        }
        Ok(())
    }

    /// A deleted theme's pictures go with it.
    pub fn delete(&self, id: &str) -> Result<()> {
        match fs::remove_dir_all(self.dir.join(id)) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(io_err(e)),
            _ => Ok(()),
        }
    }

    fn path(&self, id: &str, rev: &str, mode: Mode) -> PathBuf {
        self.dir.join(id).join(format!("{rev}-{}.webp", mode.word()))
    }
}

/// A WebP of the picker's size (lossy, lossless or extended), read from its header alone.
fn check_webp(b: &[u8]) -> Result<()> {
    if b.len() > MAX_BYTES {
        return Err(Error::BadRequest(format!(
            "a theme's picture is at most {} KB",
            MAX_BYTES / 1024
        )));
    }
    let size = webp_size(b).ok_or_else(|| Error::BadRequest("a theme's picture is a WebP".into()))?;
    if size != (WIDTH, HEIGHT) {
        return Err(Error::BadRequest(format!(
            "a theme's picture is {WIDTH}×{HEIGHT}, not {}×{}",
            size.0, size.1
        )));
    }
    Ok(())
}

fn webp_size(b: &[u8]) -> Option<(u32, u32)> {
    if b.len() < 30 || &b[0..4] != b"RIFF" || &b[8..12] != b"WEBP" {
        return None;
    }
    let le = |s: &[u8]| s.iter().rev().fold(0u32, |n, &x| n << 8 | u32::from(x));
    match &b[12..16] {
        b"VP8 " if b[23..26] == [0x9d, 0x01, 0x2a] => Some((le(&b[26..28]) & 0x3fff, le(&b[28..30]) & 0x3fff)),
        b"VP8L" if b[20] == 0x2f => {
            let bits = le(&b[21..25]);
            Some(((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1))
        }
        b"VP8X" => Some((le(&b[24..27]) + 1, le(&b[27..30]) + 1)),
        _ => None,
    }
}

fn io_err(e: io::Error) -> Error {
    Error::internal(format!("theme pictures folder: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The header of a lossy WebP of that size, as a canvas encodes one (the image data doesn't matter here).
    fn webp(w: u16, h: u16) -> Vec<u8> {
        let mut b = b"RIFF\0\0\0\0WEBPVP8 \0\0\0\0\0\0\0\x9d\x01\x2a".to_vec();
        b.extend(w.to_le_bytes());
        b.extend(h.to_le_bytes());
        b.extend([0; 16]);
        b
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lull-theme-pictures-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        assert!(fs::create_dir_all(&dir).is_ok());
        dir
    }

    #[test]
    fn a_theme_gets_a_new_rev_when_any_of_its_files_changes() {
        let theme = scratch("rev").join("halcyon");
        assert!(fs::create_dir_all(theme.join("assets")).is_ok());
        assert!(fs::write(theme.join("theme.css"), "a{}").is_ok());
        let first = rev(&theme);
        assert_eq!(rev(&theme), first, "an untouched theme keeps its rev");
        assert!(fs::write(theme.join("assets/sky.svg"), "<svg/>").is_ok());
        assert_ne!(rev(&theme), first, "a file added in a subfolder changes it");
    }

    #[test]
    fn a_new_picture_replaces_the_older_revs_and_only_its_own_rev_is_listed() {
        let p = Pictures { dir: scratch("put") };
        let (old, new) = ("00000000000000aa", "00000000000000bb");
        assert!(p.put("relay", old, Mode::Day, &webp(320, 200)).is_ok());
        assert!(p.put("relay", old, Mode::Night, &webp(320, 200)).is_ok());
        assert_eq!(p.made("relay", old), [Mode::Day, Mode::Night]);
        assert!(p.put("relay", new, Mode::Night, &webp(320, 200)).is_ok());
        assert_eq!(p.made("relay", new), [Mode::Night]);
        assert!(p.made("relay", old).is_empty(), "pictures of the older files are gone");
        assert!(p.delete("relay").is_ok());
        assert!(p.made("relay", new).is_empty());
    }

    #[test]
    fn only_a_webp_of_the_pickers_size_is_kept() {
        let p = Pictures { dir: scratch("check") };
        let rev = "0123456789abcdef";
        assert!(p.put("x", rev, Mode::Day, &webp(640, 400)).is_err());
        assert!(
            p.put(
                "x",
                rev,
                Mode::Day,
                b"\x89PNG\r\n\x1a\n not a webp at all, but long enough"
            )
            .is_err()
        );
        assert!(
            p.put("x", rev, Mode::Day, &[webp(320, 200), vec![0; MAX_BYTES]].concat())
                .is_err()
        );
        assert!(p.made("x", rev).is_empty());
    }

    #[test]
    fn a_picture_name_is_a_rev_and_a_mode() {
        assert_eq!(
            parse_name("0123456789abcdef-night.webp"),
            Some(("0123456789abcdef", Mode::Night))
        );
        for bad in [
            "0123456789abcdef-dusk.webp",
            "../../etc-day.webp",
            "0123456789ABCDEF-day.webp",
            "0123456789abcdef-day.png",
        ] {
            assert_eq!(parse_name(bad), None, "{bad}");
        }
    }
}
