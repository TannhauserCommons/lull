//! Checking for a newer lull (ADR-0006). The desktop app has Tauri's updater (desktop/updater.rs);
//! Android has none, so it reads the same `latest.json` and offers the release APK in the browser
//! (mobile/mod.rs). Shared here: what Settings > App is told, and reading the manifest on Android.

use serde::Serialize;

/// Where a check stands. Settings > App shows it as is; the desktop's tray item follows it too.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "kebab-case", rename_all_fields = "camelCase")]
pub enum Status {
    /// Not checked yet this run.
    Idle,
    Checking,
    /// This build is the newest release.
    Current,
    /// Desktop: the update downloads in the background; `total` is null while unknown.
    #[cfg_attr(all(mobile, not(test)), allow(dead_code))]
    Downloading {
        version: String,
        received: u64,
        total: Option<u64>,
    },
    /// Desktop: downloaded and checked, waiting for "Restart to update".
    #[cfg_attr(all(mobile, not(test)), allow(dead_code))]
    Ready {
        version: String,
    },
    /// Android: a newer APK, downloaded in the browser on the listener's click.
    #[cfg_attr(all(desktop, not(test)), allow(dead_code))]
    Available {
        version: String,
    },
    Failed {
        error: String,
    },
}

impl Status {
    /// A check or a download is running, or an update waits: another check would only repeat it.
    pub fn busy(&self) -> bool {
        matches!(
            self,
            Self::Checking | Self::Downloading { .. } | Self::Ready { .. } | Self::Available { .. }
        )
    }
}

/// What the web UI gets: this build's version beside where the check stands.
#[derive(Debug, Serialize)]
pub struct Report {
    pub current: String,
    #[serde(flatten)]
    pub status: Status,
}

/// The plain sentence for a manifest that isn't there: `latest/download/latest.json` answers 404
/// until the first `v*` release is published.
pub const NO_RELEASE: &str = "No lull release has been published yet";

/// The plain sentence when GitHub can't be reached at all.
pub const OFFLINE: &str = "Couldn't reach GitHub. Are you offline?";

#[cfg(any(mobile, test))]
pub use manifest::{Android, newer_apk};

#[cfg(any(mobile, test))]
mod manifest {
    use serde::Deserialize;

    /// The part of `latest.json` Android reads. The release job (`.github/workflows/app.yml`) adds
    /// `android` beside Tauri's own fields, which the desktop updater ignores.
    #[derive(Debug, Deserialize)]
    struct Manifest {
        version: String,
        android: Option<Apk>,
    }

    #[derive(Debug, Deserialize)]
    struct Apk {
        url: String,
    }

    /// A newer release's APK.
    #[derive(Debug, PartialEq)]
    pub struct Android {
        pub version: String,
        pub url: String,
    }

    /// `latest.json` against this build's version: the APK to offer, or None when this build is the
    /// newest. A manifest that can't be read, or a newer release without an APK, is an error.
    pub fn newer_apk(current: &str, manifest: &str) -> Result<Option<Android>, String> {
        let m: Manifest =
            serde_json::from_str(manifest).map_err(|e| format!("The release manifest can't be read: {e}"))?;
        let parse =
            |v: &str| semver::Version::parse(v.trim_start_matches('v')).map_err(|e| format!("Version {v:?}: {e}"));
        if parse(&m.version)? <= parse(current)? {
            return Ok(None);
        }
        let apk = m
            .android
            .ok_or_else(|| format!("lull {} has no Android download", m.version))?;
        if !apk.url.starts_with("https://") {
            return Err(format!("lull {}'s Android download isn't https", m.version));
        }
        Ok(Some(Android {
            version: m.version,
            url: apk.url,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const APK: &str = "https://github.com/o/lull/releases/download/v0.2.0/lull-0.2.0-android-arm64.apk";

    fn manifest(version: &str, android: bool) -> String {
        let android = if android {
            format!(r#","android":{{"url":"{APK}"}}"#)
        } else {
            String::new()
        };
        format!(r#"{{"version":"{version}","platforms":{{}}{android}}}"#)
    }

    #[test]
    fn a_newer_release_offers_its_apk() {
        let found = newer_apk("0.1.0", &manifest("0.2.0", true));
        assert_eq!(
            found,
            Ok(Some(Android {
                version: "0.2.0".into(),
                url: APK.into()
            }))
        );
    }

    #[test]
    fn versions_compare_as_numbers_not_text() {
        assert!(matches!(newer_apk("0.9.0", &manifest("0.10.0", true)), Ok(Some(_))));
        assert_eq!(newer_apk("0.10.0", &manifest("0.9.0", true)), Ok(None));
    }

    #[test]
    fn the_same_or_an_older_release_is_no_update() {
        assert_eq!(newer_apk("0.2.0", &manifest("0.2.0", true)), Ok(None));
        assert_eq!(newer_apk("0.3.0", &manifest("v0.2.0", false)), Ok(None));
    }

    #[test]
    fn a_newer_release_without_an_apk_is_an_error() {
        assert!(newer_apk("0.1.0", &manifest("0.2.0", false)).is_err());
    }

    #[test]
    fn an_unreadable_version_is_an_error() {
        assert!(newer_apk("0.1.0", &manifest("next", true)).is_err());
    }

    #[test]
    fn a_check_never_repeats_one_that_runs_or_an_update_that_waits() {
        let version = || "0.2.0".to_owned();
        for busy in [
            Status::Checking,
            Status::Downloading {
                version: version(),
                received: 0,
                total: None,
            },
            Status::Ready { version: version() },
            Status::Available { version: version() },
        ] {
            assert!(busy.busy(), "{busy:?}");
        }
        for free in [Status::Idle, Status::Current, Status::Failed { error: OFFLINE.into() }] {
            assert!(!free.busy(), "{free:?}");
        }
    }

    #[test]
    fn the_web_ui_gets_the_state_beside_this_builds_version() {
        let report = Report {
            current: "0.1.0".into(),
            status: Status::Downloading {
                version: "0.2.0".into(),
                received: 5,
                total: Some(10),
            },
        };
        assert_eq!(
            serde_json::to_value(report).ok(),
            Some(serde_json::json!({
                "current": "0.1.0", "state": "downloading", "version": "0.2.0", "received": 5, "total": 10
            }))
        );
    }
}
