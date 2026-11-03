//! Running yt-dlp: the binary in use (the pinned one, or a newer one `update.rs` fetched), the
//! arguments every run gets (no config files, no cookies, the JS runtime, a cache folder in the
//! data dir), and its errors in plain words. YouTube's sign-in checks and 429s are told apart so
//! the service can back off (docs/context/youtube.md#politeness).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::RwLock;
use std::time::Duration;

use serde::Serialize;
use tokio::process::Command;

/// What yt-dlp's failure means for lull.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// YouTube wants a sign-in to prove this isn't a bot: back off.
    BotCheck,
    /// HTTP 429: back off.
    RateLimited,
    /// This upload can't be had (private, removed, age-gated, blocked here): try another.
    Gone(String),
    /// yt-dlp itself is missing or won't start.
    Broken(String),
    /// Anything else, in yt-dlp's own words (first error line).
    Other(String),
}

impl Problem {
    pub fn plain(&self) -> String {
        match self {
            Problem::BotCheck => "YouTube is asking for a sign-in check, try later".into(),
            Problem::RateLimited => "YouTube says lull asked too often, try later".into(),
            Problem::Gone(why) | Problem::Broken(why) | Problem::Other(why) => why.clone(),
        }
    }
}

/// The yt-dlp in use, as `/api/youtube` and the `youtube` status show it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Binary {
    #[serde(skip)]
    pub path: PathBuf,
    /// `None` when it doesn't run.
    pub version: Option<String>,
    /// Fetched by the hub (Keep yt-dlp up to date) rather than the pinned one.
    pub updated: bool,
}

pub struct Ytdlp {
    pinned: PathBuf,
    deno: PathBuf,
    /// `<data>/youtube`: yt-dlp's cache, Deno's cache, temporary files.
    dir: PathBuf,
    current: RwLock<Binary>,
    pinned_version: RwLock<Option<String>>,
}

impl Ytdlp {
    pub fn new(pinned: PathBuf, deno: PathBuf, dir: PathBuf) -> Self {
        Ytdlp {
            current: RwLock::new(Binary {
                path: pinned.clone(),
                version: None,
                updated: false,
            }),
            pinned_version: RwLock::new(None),
            pinned,
            deno,
            dir,
        }
    }

    pub fn current(&self) -> Binary {
        self.current
            .read()
            .map(|b| b.clone())
            .unwrap_or_else(|p| p.into_inner().clone())
    }

    pub fn pinned_version(&self) -> Option<String> {
        self.pinned_version.read().ok().and_then(|v| v.clone())
    }

    /// Learns the pinned version, and uses `updated` when it runs and is newer (an image built
    /// later may pin a newer one); the pinned one otherwise.
    pub async fn choose(&self, updated: Option<PathBuf>) {
        let pinned = version_of(&self.pinned, &self.dir).await;
        if let Ok(mut v) = self.pinned_version.write() {
            v.clone_from(&pinned);
        }
        let mut next = Binary {
            path: self.pinned.clone(),
            version: pinned.clone(),
            updated: false,
        };
        if let Some(path) = updated {
            match version_of(&path, &self.dir).await {
                Some(version) if pinned.as_deref().is_none_or(|p| crate::update::newer(&version, p)) => {
                    next = Binary {
                        path,
                        version: Some(version),
                        updated: true,
                    }
                }
                Some(_) => {}
                None => tracing::warn!(path = %path.display(), "the updated yt-dlp doesn't run; using the pinned one"),
            }
        }
        if let Ok(mut c) = self.current.write() {
            *c = next;
        }
    }

    /// Back to the pinned binary (the updated one broke).
    pub async fn fall_back(&self) {
        self.choose(None).await;
    }

    /// A yt-dlp command with the arguments every run gets.
    pub fn command(&self, args: &[&str]) -> Command {
        command(&self.current().path, &self.deno, &self.dir, args)
    }

    /// Runs yt-dlp to the end and returns its stdout. With `partial`, what it printed counts even
    /// when it then failed: a playlist run with `-i` prints the songs it could read and fails for
    /// the ones it couldn't (blocked in this country). A sign-in check, a 429 or a broken yt-dlp
    /// still fail.
    pub async fn output(&self, args: &[&str], limit: Duration, partial: bool) -> Result<Vec<u8>, Problem> {
        let mut cmd = self.command(args);
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(Stdio::null());
        let child = cmd.spawn().map_err(|e| broken(&self.current().path, &e))?;
        let out = tokio::time::timeout(limit, child.wait_with_output())
            .await
            .map_err(|_| Problem::Other(format!("yt-dlp took longer than {} s", limit.as_secs())))?
            .map_err(|e| Problem::Other(format!("yt-dlp: {e}")))?;
        if out.status.success() {
            return Ok(out.stdout);
        }
        match classify(&String::from_utf8_lossy(&out.stderr)) {
            Problem::Gone(_) | Problem::Other(_)
                if partial && out.stdout.trim_ascii() != b"null" && !out.stdout.is_empty() =>
            {
                Ok(out.stdout)
            }
            problem => Err(problem),
        }
    }
}

/// `--version` of a yt-dlp binary, or `None` when it doesn't run.
pub async fn version_of(path: &Path, dir: &Path) -> Option<String> {
    let mut cmd = Command::new(path);
    cmd.arg("--version")
        .envs(env(dir))
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .stdin(Stdio::null())
        .kill_on_drop(true);
    no_window(&mut cmd);
    let out = tokio::time::timeout(Duration::from_secs(60), cmd.output())
        .await
        .ok()?
        .ok()?;
    let v = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (out.status.success() && !v.is_empty()).then_some(v)
}

fn command(exe: &Path, deno: &Path, dir: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new(exe);
    // No config files, no cookies, no accounts; few retries, so a 429 isn't hammered.
    cmd.args(["--ignore-config", "--color", "never", "--no-update"])
        .args(["--socket-timeout", "15", "--retries", "3", "--extractor-retries", "1"])
        .arg("--cache-dir")
        .arg(dir.join("cache"))
        .arg("--no-js-runtimes")
        .arg("--js-runtimes")
        .arg(js_runtime(deno))
        .args(args)
        .envs(env(dir))
        .kill_on_drop(true);
    no_window(&mut cmd);
    cmd
}

/// `deno` alone is looked up on PATH by yt-dlp; a path is passed as one.
fn js_runtime(deno: &Path) -> String {
    match deno.components().count() {
        1 => "deno".to_owned(),
        _ => format!("deno:{}", deno.display()),
    }
}

/// The standalone yt-dlp unpacks itself into TMPDIR on every run, and Deno keeps a cache: both go
/// to the data folder, which is always writable (the image's root file system may not be).
fn env(dir: &Path) -> [(&'static str, PathBuf); 4] {
    [
        ("TMPDIR", dir.join("tmp")),
        ("TEMP", dir.join("tmp")),
        ("TMP", dir.join("tmp")),
        ("DENO_DIR", dir.join("deno")),
    ]
}

/// A console program started by the desktop app must not open a console window.
fn no_window(cmd: &mut Command) {
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    #[cfg(not(windows))]
    let _ = cmd;
}

fn broken(path: &Path, e: &std::io::Error) -> Problem {
    Problem::Broken(match e.kind() {
        std::io::ErrorKind::NotFound => format!(
            "yt-dlp isn't installed here ({} not found); set LULL_YTDLP",
            path.display()
        ),
        _ => format!("yt-dlp won't start: {e}"),
    })
}

/// yt-dlp's stderr in lull's terms. Only `ERROR:` lines count; warnings are noise here.
pub fn classify(stderr: &str) -> Problem {
    let errors: Vec<&str> = stderr
        .lines()
        .filter_map(|l| l.trim().strip_prefix("ERROR:"))
        .map(str::trim)
        .collect();
    let all = errors.join(" ").to_ascii_lowercase();
    if all.contains("confirm your age") || all.contains("age-restricted") {
        return Problem::Gone("the upload is age-restricted".into());
    }
    if all.contains("not a bot") || all.contains("sign in to confirm") {
        return Problem::BotCheck;
    }
    if all.contains("429") || all.contains("too many requests") {
        return Problem::RateLimited;
    }
    let first = errors.first().map(|e| plain(e)).unwrap_or_else(|| {
        let last = stderr
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("no output");
        plain(last.trim())
    });
    if [
        "video unavailable",
        "private video",
        "not available",
        "removed",
        "blocked",
        "copyright",
    ]
    .iter()
    .any(|w| all.contains(w))
    {
        return Problem::Gone(first);
    }
    Problem::Other(first)
}

/// "[youtube] abc: Video unavailable. This video …" → "Video unavailable. This video …", short.
fn plain(line: &str) -> String {
    let line = match line.strip_prefix('[') {
        Some(rest) => rest.split_once("] ").map_or(rest, |(_, r)| r),
        None => line,
    };
    let line = match line.split_once(": ") {
        Some((id, rest)) if id.len() == 11 && !id.contains(' ') => rest,
        _ => line,
    };
    let mut s: String = line.chars().take(160).collect();
    if line.chars().count() > 160 {
        s.push('…');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_in_checks_and_429s_are_told_apart_from_a_missing_video() {
        let bot = "WARNING: [youtube] x\nERROR: [youtube] LUjGtyYEi90: Sign in to confirm you’re not a bot. Use --cookies-from-browser or --cookies for the authentication.";
        assert_eq!(classify(bot), Problem::BotCheck);
        assert_eq!(
            classify("ERROR: unable to download video data: HTTP Error 429: Too Many Requests"),
            Problem::RateLimited
        );
        assert_eq!(
            classify("ERROR: [youtube] LUjGtyYEi90: Video unavailable. This video has been removed by the uploader"),
            Problem::Gone("Video unavailable. This video has been removed by the uploader".into())
        );
        assert!(matches!(
            classify("ERROR: [youtube] LUjGtyYEi90: Sign in to confirm your age. This video may be inappropriate"),
            Problem::Gone(_)
        ));
        assert_eq!(
            Problem::BotCheck.plain(),
            "YouTube is asking for a sign-in check, try later"
        );
    }

    #[test]
    fn a_bare_deno_is_found_on_path_and_a_path_is_passed_as_one() {
        assert_eq!(js_runtime(Path::new("deno")), "deno");
        assert_eq!(js_runtime(Path::new("/usr/local/bin/deno")), "deno:/usr/local/bin/deno");
    }
}
