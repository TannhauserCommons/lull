//! What this build of the hub is, said honestly between releases: the release it holds, and how many
//! changes landed on top of it. The queue builds main's image with `LULL_BUILD_RELEASE`, `LULL_BUILD_CHANGES`
//! and `LULL_BUILD_COMMIT`, worked out from git (`scripts/queue.sh`'s `build_version`). Any other build,
//! a release's included, is the version in `Cargo.toml` with no changes on top.
//!
//! `LULL_BUILD_INSTALL` says how the hub was installed (`docker` for the image); the desktop app's
//! built-in hub says so with [`set_install`] before it starts.

use std::sync::OnceLock;

use serde::Serialize;

/// The release this build holds everything of.
pub const RELEASE: &str = or(option_env!("LULL_BUILD_RELEASE"), env!("CARGO_PKG_VERSION"));
/// Changes landed on top of [`RELEASE`] (0 for a release).
pub const CHANGES: u32 = count(option_env!("LULL_BUILD_CHANGES"));
/// The commit it was built from, when the build said.
pub const COMMIT: Option<&str> = match option_env!("LULL_BUILD_COMMIT") {
    Some(c) if !c.is_empty() => Some(c),
    _ => None,
};

/// How the hub was installed, which decides how it is updated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Install {
    /// The desktop app's built-in hub: it updates with the app.
    Desktop,
    /// lull's image, run by Docker or compose.
    Docker,
    /// Built and run some other way.
    Server,
}

static INSTALL: OnceLock<Install> = OnceLock::new();

/// The desktop app calls this before it starts its built-in hub.
pub fn set_install(install: Install) {
    let _ = INSTALL.set(install);
}

pub fn install() -> Install {
    *INSTALL.get_or_init(|| match option_env!("LULL_BUILD_INSTALL") {
        Some("docker") => Install::Docker,
        _ => Install::Server,
    })
}

/// The version as one string: `0.2.4` for a release, `0.2.4+12.abc1234` for a build with changes on top
/// (semver's build metadata, so it still reads as 0.2.4 to anything that compares versions).
pub fn version() -> String {
    match (CHANGES, COMMIT) {
        (0, _) => RELEASE.to_owned(),
        (n, Some(c)) => format!("{RELEASE}+{n}.{}", short(c)),
        (n, None) => format!("{RELEASE}+{n}"),
    }
}

/// What `/health` says about the build: the web UI words it ("0.2.4 + 12 changes").
#[derive(Debug, Clone, Serialize)]
pub struct Build {
    pub release: &'static str,
    pub changes: u32,
    pub commit: Option<&'static str>,
    pub install: Install,
}

pub fn build() -> Build {
    Build {
        release: RELEASE,
        changes: CHANGES,
        commit: COMMIT.map(short),
        install: install(),
    }
}

fn short(commit: &str) -> &str {
    commit.get(..7).unwrap_or(commit)
}

const fn or(value: Option<&'static str>, default: &'static str) -> &'static str {
    match value {
        Some(v) if !v.is_empty() => v,
        _ => default,
    }
}

/// A count from the build's environment; anything but digits is 0, so a bad value can't stop a build.
const fn count(value: Option<&str>) -> u32 {
    let Some(v) = value else { return 0 };
    let b = v.as_bytes();
    let (mut i, mut n) = (0, 0u32);
    while i < b.len() {
        if !b[i].is_ascii_digit() {
            return 0;
        }
        n = n.saturating_mul(10).saturating_add((b[i] - b'0') as u32);
        i += 1;
    }
    n
}

/// A release's number as comparable parts, or None for anything that isn't `X.Y.Z` (a `v` in front is fine).
pub fn parse_release(v: &str) -> Option<(u64, u64, u64)> {
    let mut parts = v.trim().trim_start_matches('v').split('.');
    let mut next = || parts.next()?.parse::<u64>().ok();
    let r = (next()?, next()?, next()?);
    parts.next().is_none().then_some(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_only_digits() {
        assert_eq!(count(None), 0);
        assert_eq!(count(Some("")), 0);
        assert_eq!(count(Some("12")), 12);
        assert_eq!(count(Some("1x")), 0);
    }

    #[test]
    fn releases_parse_as_three_numbers() {
        assert_eq!(parse_release("v0.2.5"), Some((0, 2, 5)));
        assert_eq!(parse_release("0.10.0"), Some((0, 10, 0)));
        assert!(parse_release("0.2.5") > parse_release("0.2.4"));
        assert_eq!(parse_release("slskd-0.26.2-lull.1"), None);
        assert_eq!(parse_release("0.2"), None);
        assert_eq!(parse_release("0.2.4.1"), None);
    }

    #[test]
    fn short_commit() {
        assert_eq!(short("f64cb1327a"), "f64cb13");
        assert_eq!(short("abc"), "abc");
    }
}
