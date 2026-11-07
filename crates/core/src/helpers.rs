//! Which copy of each helper program a hub runs (ADR-0064): yt-dlp, Deno and slskd. The listener's
//! override wins; then a copy this computer has (its package manager's, or on `PATH`) that passes
//! its check; then lull's own copy. A computer's copy passes when `--version` answers within 5 s
//! with at least the version lull is tested with (`TESTED`); for yt-dlp the newer of this
//! computer's and lull's own wins, since YouTube breaks old yt-dlp first; for slskd a series lull
//! hasn't tested is still used (the owner's call), and says so.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

/// A helper program a hub runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Helper {
    Ytdlp,
    Deno,
    Slskd,
}

impl Helper {
    pub const ALL: [Helper; 3] = [Helper::Ytdlp, Helper::Deno, Helper::Slskd];

    /// The program's file name, without `.exe`.
    pub fn program(self) -> &'static str {
        match self {
            Helper::Ytdlp => "yt-dlp",
            Helper::Deno => "deno",
            Helper::Slskd => "slskd",
        }
    }

    /// Its name in a sentence.
    pub fn name(self) -> &'static str {
        match self {
            Helper::Ytdlp => "yt-dlp",
            Helper::Deno => "Deno",
            Helper::Slskd => "slskd",
        }
    }

    /// The listener's override (ADR-0064): `LULL_YTDLP`, `LULL_DENO`, `LULL_SLSKD`.
    pub fn override_key(self) -> &'static str {
        match self {
            Helper::Ytdlp => "LULL_YTDLP",
            Helper::Deno => "LULL_DENO",
            Helper::Slskd => "LULL_SLSKD",
        }
    }

    /// The version lull is tested with: what the app bundles (app/build.rs pins the same, checked
    /// by its tests) and the slskd contract runs against.
    pub fn tested(self) -> &'static str {
        match self {
            Helper::Ytdlp => "2026.10.26",
            Helper::Deno => "2.9.8",
            Helper::Slskd => "0.26.2",
        }
    }
}

/// slskd's series lull's contract (tests/slskd-contract) has passed. A newer series is used, and
/// Settings says lull hasn't been tested with it.
pub const SLSKD_TESTED_SERIES: &[(u64, u64)] = &[(0, 26)];

/// Where a hub's copy comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Source {
    /// `LULL_YTDLP`, `LULL_DENO`, `LULL_SLSKD`.
    Override,
    /// This computer's own copy.
    System,
    /// The copy lull ships.
    Own,
}

/// The copy chosen, with what Settings says about it.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Choice {
    pub helper: Helper,
    pub path: PathBuf,
    /// What `--version` said, when it answered.
    pub version: Option<String>,
    pub source: Source,
    /// The version lull is tested with (`Helper::tested`).
    pub tested: &'static str,
    /// A plain sentence when something deserves one: a computer's copy passed over as too old, or a
    /// slskd series lull hasn't been tested with.
    pub note: Option<String>,
}

/// A copy found and what it answered.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub path: PathBuf,
    pub version: Option<String>,
}

/// Chooses `helper`'s copy: the override if set, else this computer's (on `PATH`, and `/usr/bin`
/// and `/usr/local/bin`, which a desktop launcher's `PATH` can lack) if it passes, else `own`.
/// `prefer_own`: the listener asked for lull's own copies (Settings, when a computer's copy
/// misbehaves). None when there is no copy at all.
pub fn choose(
    helper: Helper,
    override_path: Option<PathBuf>,
    own: Option<PathBuf>,
    prefer_own: bool,
) -> Option<Choice> {
    if let Some(path) = override_path {
        let version = version_of(helper, &path);
        return Some(Choice {
            helper,
            path,
            version,
            source: Source::Override,
            tested: helper.tested(),
            note: None,
        });
    }
    let own = own.map(|path| Found {
        version: version_of(helper, &path),
        path,
    });
    let system = if prefer_own && own.is_some() {
        None
    } else {
        find_on_system(helper.program(), own.as_ref().map(|o| o.path.as_path())).map(|path| Found {
            version: version_of(helper, &path),
            path,
        })
    };
    decide(helper, system, own)
}

/// The rules, on what was found (`choose` finds and asks; tests call this directly).
pub fn decide(helper: Helper, system: Option<Found>, own: Option<Found>) -> Option<Choice> {
    let pick = |found: Found, source, note| Choice {
        helper,
        path: found.path,
        version: found.version,
        source,
        tested: helper.tested(),
        note,
    };
    let Some(system) = system else {
        return own.map(|o| pick(o, Source::Own, None));
    };
    let Some(version) = system.version.clone() else {
        let note = format!(
            "This computer's {} didn't say its version, so lull uses its own.",
            helper.name()
        );
        return match own {
            Some(o) => Some(pick(o, Source::Own, Some(note))),
            // nothing else to run: this computer's, as is
            None => Some(pick(system, Source::System, None)),
        };
    };
    if at_least(&version, helper.tested()) {
        // yt-dlp: the newer of the two, YouTube breaks old ones first; a tie goes to this computer's
        if helper == Helper::Ytdlp
            && let Some(o) = own
                .as_ref()
                .filter(|o| o.version.as_deref().is_some_and(|v| newer(v, &version)))
        {
            return Some(pick(o.clone(), Source::Own, None));
        }
        let note = (helper == Helper::Slskd && !slskd_series_tested(&version)).then(|| {
            format!(
                "slskd {version} is newer than lull has been tested with ({}).",
                Helper::Slskd.tested()
            )
        });
        return Some(pick(system, Source::System, note));
    }
    let note = format!(
        "This computer's {} {version} is older than lull is tested with ({}), so lull uses its own.",
        helper.name(),
        helper.tested()
    );
    match own {
        Some(o) => Some(pick(o, Source::Own, Some(note))),
        None => Some(pick(
            system,
            Source::System,
            Some(format!(
                "This computer's {} {version} is older than lull is tested with ({}).",
                helper.name(),
                helper.tested()
            )),
        )),
    }
}

/// The first `program` in `PATH`, then `/usr/bin` and `/usr/local/bin`, that isn't in lull's own
/// folder (`own`'s).
fn find_on_system(program: &str, own: Option<&Path>) -> Option<PathBuf> {
    let name = format!("{program}{}", std::env::consts::EXE_SUFFIX);
    let own_dir = own.and_then(Path::parent).and_then(|d| d.canonicalize().ok());
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    if cfg!(unix) {
        dirs.extend(["/usr/bin", "/usr/local/bin"].map(PathBuf::from));
    }
    dirs.into_iter()
        .filter(|dir| dir.canonicalize().ok() != own_dir || own_dir.is_none())
        .map(|dir| dir.join(&name))
        .find(|path| path.is_file())
}

/// What `<path> --version` says, within 5 s: the first thing on its first line that looks like a
/// version (yt-dlp `2026.10.26`, Deno `deno 2.9.8 (stable, …)`, slskd `0.26.2`).
pub fn version_of(_helper: Helper, path: &Path) -> Option<String> {
    let mut child = crate::process::command(path)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut out = String::new();
    child.stdout.take()?.read_to_string(&mut out).ok()?;
    version_in(&out)
}

/// The first word of `text` that starts with a digit and has a dot: `2026.10.26`, `2.9.8`,
/// `0.26.2.0`, `v1.2.3` (the `v` dropped).
pub fn version_in(text: &str) -> Option<String> {
    text.lines().next()?.split_whitespace().find_map(|w| {
        let w = w
            .trim_start_matches('v')
            .trim_end_matches(|c: char| !c.is_ascii_alphanumeric());
        (w.starts_with(|c: char| c.is_ascii_digit()) && w.contains('.')).then(|| w.to_owned())
    })
}

/// Dotted numbers compared part by part (a missing part is 0; `-rc1` and the like are dropped).
fn parts(version: &str) -> Vec<u64> {
    version
        .split(['.', '-', '+'])
        .map_while(|p| p.parse::<u64>().ok())
        .collect()
}

fn compare(a: &str, b: &str) -> std::cmp::Ordering {
    let (a, b) = (parts(a), parts(b));
    (0..a.len().max(b.len()))
        .map(|i| a.get(i).unwrap_or(&0).cmp(b.get(i).unwrap_or(&0)))
        .find(|o| o.is_ne())
        .unwrap_or(std::cmp::Ordering::Equal)
}

pub fn at_least(version: &str, min: &str) -> bool {
    compare(version, min).is_ge()
}

pub fn newer(a: &str, b: &str) -> bool {
    compare(a, b).is_gt()
}

fn slskd_series_tested(version: &str) -> bool {
    let p = parts(version);
    SLSKD_TESTED_SERIES
        .iter()
        .any(|&(major, minor)| p.first() == Some(&major) && p.get(1) == Some(&minor))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(path: &str, version: Option<&str>) -> Option<Found> {
        Some(Found {
            path: PathBuf::from(path),
            version: version.map(str::to_owned),
        })
    }

    fn chosen(helper: Helper, system: Option<Found>, own: Option<Found>) -> (Source, String, Option<String>) {
        let Some(c) = decide(helper, system, own) else {
            return (Source::Own, "none".into(), None);
        };
        (c.source, c.path.display().to_string(), c.note)
    }

    #[test]
    fn versions_are_read_from_what_each_program_prints() {
        assert_eq!(version_in("2026.10.26\n").as_deref(), Some("2026.10.26"));
        assert_eq!(
            version_in("deno 2.9.8 (stable, release, x86_64-unknown-linux-gnu)\nv8 13.0\n").as_deref(),
            Some("2.9.8")
        );
        assert_eq!(version_in("0.26.2.0\n").as_deref(), Some("0.26.2.0"));
        assert_eq!(version_in("slskd v0.27.1+abc\n").as_deref(), Some("0.27.1+abc"));
        assert_eq!(version_in("usage: nothing here\n"), None);
    }

    #[test]
    fn a_computers_copy_at_or_above_the_tested_version_is_used() {
        let (s, p, n) = chosen(
            Helper::Deno,
            found("/usr/bin/deno", Some("2.9.8")),
            found("/opt/lull/deno", Some("2.9.8")),
        );
        assert_eq!((s, p.as_str(), n), (Source::System, "/usr/bin/deno", None));
        let (s, _, _) = chosen(
            Helper::Deno,
            found("/usr/bin/deno", Some("2.10.0")),
            found("/opt/lull/deno", Some("2.9.8")),
        );
        assert_eq!(s, Source::System);
    }

    #[test]
    fn an_older_copy_is_passed_over_for_lulls_own_and_said() {
        let (s, p, n) = chosen(
            Helper::Deno,
            found("/usr/bin/deno", Some("1.46.3")),
            found("/opt/lull/deno", Some("2.9.8")),
        );
        assert_eq!((s, p.as_str()), (Source::Own, "/opt/lull/deno"));
        assert!(n.is_some_and(|n| n.contains("Deno 1.46.3 is older")));
    }

    #[test]
    fn yt_dlp_takes_the_newer_of_the_two() {
        let (s, _, _) = chosen(
            Helper::Ytdlp,
            found("/usr/bin/yt-dlp", Some("2026.10.26")),
            found("/o/yt-dlp", Some("2026.11.02")),
        );
        assert_eq!(s, Source::Own);
        let (s, _, _) = chosen(
            Helper::Ytdlp,
            found("/usr/bin/yt-dlp", Some("2026.11.02")),
            found("/o/yt-dlp", Some("2026.10.26")),
        );
        assert_eq!(s, Source::System);
        let (s, _, _) = chosen(
            Helper::Ytdlp,
            found("/usr/bin/yt-dlp", Some("2026.10.26")),
            found("/o/yt-dlp", Some("2026.10.26")),
        );
        assert_eq!(s, Source::System, "a tie goes to this computer's");
    }

    #[test]
    fn a_newer_slskd_series_is_used_and_said() {
        let (s, _, n) = chosen(
            Helper::Slskd,
            found("/usr/bin/slskd", Some("0.27.0")),
            found("/o/slskd", Some("0.26.2")),
        );
        assert_eq!(s, Source::System);
        assert!(n.is_some_and(|n| n.contains("newer than lull has been tested with")));
        let (_, _, n) = chosen(Helper::Slskd, found("/usr/bin/slskd", Some("0.26.3")), None);
        assert_eq!(n, None, "a tested series says nothing");
    }

    #[test]
    fn with_no_copy_of_lulls_own_the_computers_runs_whatever_it_is() {
        let (s, _, n) = chosen(Helper::Slskd, found("/usr/bin/slskd", Some("0.20.0")), None);
        assert_eq!(s, Source::System);
        assert!(n.is_some_and(|n| n.contains("older")));
        let (s, _, _) = chosen(Helper::Deno, found("/usr/bin/deno", None), None);
        assert_eq!(s, Source::System);
        assert!(decide(Helper::Deno, None, None).is_none());
    }

    #[test]
    fn versions_compare_by_their_numbers() {
        assert!(at_least("2026.10.26", "2026.10.26"));
        assert!(at_least("2026.11.2", "2026.10.26"));
        assert!(!at_least("2026.9.30", "2026.10.26"));
        assert!(at_least("0.26.2.0", "0.26.2"));
        assert!(!at_least("0.26.1", "0.26.2"));
        assert!(newer("2.10.0", "2.9.8"));
    }
}
