//! Settings from `LULL_*` environment variables, optionally preloaded from the file named by
//! `LULL_CONFIG` (`KEY=value` lines; the environment wins). Every key is documented in
//! `deploy/.env.example`. Unknown `LULL_*` keys are an error, so a typo never passes silently.

use std::collections::BTreeMap;
use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    pub listen: SocketAddr,
    /// Base URL the browser uses to reach lull; needed for the Last.fm auth callback.
    pub public_url: Option<String>,
    /// Fixes the code that pairs the first device; without it the hub prints a random one while
    /// no device is paired (ADR-0002).
    pub setup_code: Option<Secret>,
    /// Irreplaceable state: the SQLite database.
    pub data_dir: PathBuf,
    pub log: String,
    pub soulseek: Soulseek,
    pub cache: Cache,
    pub jellyfin: Option<Jellyfin>,
    pub library: Library,
    pub lastfm: Option<LastFm>,
    pub discovery: Discovery,
}

#[derive(Debug, Clone)]
pub struct Soulseek {
    pub slskd_url: String,
    pub slskd_api_key: Secret,
    /// slskd's download directory as lull sees it; incomplete files are read from here while they grow.
    pub transfers_dir: PathBuf,
    /// Lowercase extensions, most preferred first.
    pub preferred_formats: Vec<String>,
    pub share: Share,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Share {
    /// Share the saved music library read-only (Soulseek etiquette expects sharing).
    Library,
    None,
}

#[derive(Debug, Clone)]
pub struct Cache {
    /// Upper bound for streamed-but-unsaved audio; the oldest unused files are evicted first.
    pub max_bytes: u64,
}

#[derive(Debug, Clone)]
pub struct Jellyfin {
    pub url: String,
    pub api_key: Secret,
    /// Jellyfin user whose library and playback state lull uses; the first admin when unset.
    pub user: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Library {
    /// Where the saved library comes from (ADR-0003).
    pub mode: LibraryMode,
    /// The music folder as lull sees it: indexed in folder mode, Jellyfin's music library in
    /// Jellyfin mode. Saved files are written here in both.
    pub music_dir: PathBuf,
    /// Path template for saved files, relative to `music_dir`.
    pub naming: String,
}

/// `LULL_LIBRARY`: `jellyfin` when Jellyfin is configured, else `folder`, unless set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryMode {
    Folder,
    Jellyfin,
}

impl LibraryMode {
    pub fn as_str(self) -> &'static str {
        match self {
            LibraryMode::Folder => "folder",
            LibraryMode::Jellyfin => "jellyfin",
        }
    }
}

#[derive(Debug, Clone)]
pub struct LastFm {
    pub api_key: Secret,
    pub api_secret: Secret,
}

impl LastFm {
    /// lull's own Last.fm API account, compiled in from the build environment
    /// (`LULL_LASTFM_API_KEY` and `LULL_LASTFM_API_SECRET` at `cargo build` time), so a
    /// listener just presses Connect. Absent from builds made without them, such as a
    /// contributor's. Settings that name a key override it.
    fn built_in() -> Option<Self> {
        Self::from_build(
            option_env!("LULL_LASTFM_API_KEY"),
            option_env!("LULL_LASTFM_API_SECRET"),
        )
    }

    /// An empty value counts as unset: CI passes an empty variable when a secret is missing.
    fn from_build(key: Option<&str>, secret: Option<&str>) -> Option<Self> {
        let (key, secret) = (key.filter(|k| !k.is_empty())?, secret.filter(|s| !s.is_empty())?);
        Some(LastFm {
            api_key: Secret::new(key),
            api_secret: Secret::new(secret),
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct Discovery {
    pub listenbrainz_user: Option<String>,
    pub listenbrainz_token: Option<Secret>,
}

/// A credential: never printed, never sent to the UI.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Secret(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("{0} is required")]
    Missing(&'static str),
    #[error("{key} is invalid: {reason}")]
    Invalid { key: &'static str, reason: String },
    #[error("{0} and {1} must be set together")]
    Pair(&'static str, &'static str),
    #[error("unknown setting {0} (see deploy/.env.example)")]
    Unknown(String),
    #[error("cannot read config file {path}: {source}")]
    File { path: String, source: std::io::Error },
}

impl Config {
    /// Reads the process environment (and `LULL_CONFIG` if set).
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_vars(env_vars()?)
    }

    pub fn from_vars(vars: BTreeMap<String, String>) -> Result<Self, ConfigError> {
        let mut v = Vars(vars);
        let jellyfin = {
            let user = v.opt("LULL_JELLYFIN_USER");
            v.pair("LULL_JELLYFIN_URL", "LULL_JELLYFIN_API_KEY")?
                .map(|(url, key)| Jellyfin {
                    url: url.trim_end_matches('/').to_owned(),
                    api_key: Secret(key),
                    user,
                })
        };
        let mode = match (v.opt("LULL_LIBRARY").as_deref(), &jellyfin) {
            (None, Some(_)) | (Some("jellyfin"), Some(_)) => LibraryMode::Jellyfin,
            (None, None) | (Some("folder"), _) => LibraryMode::Folder,
            (Some("jellyfin"), None) => {
                return Err(invalid(
                    "LULL_LIBRARY",
                    "jellyfin needs LULL_JELLYFIN_URL and LULL_JELLYFIN_API_KEY".into(),
                ));
            }
            (Some(other), _) => return Err(invalid("LULL_LIBRARY", format!("{other:?} is not folder or jellyfin"))),
        };
        let config = Config {
            listen: v.parse("LULL_LISTEN", "0.0.0.0:8080")?,
            public_url: v.opt("LULL_PUBLIC_URL").map(|u| u.trim_end_matches('/').to_owned()),
            setup_code: v.opt("LULL_SETUP_CODE").map(Secret),
            data_dir: v.or("LULL_DATA_DIR", "/data").into(),
            log: v.or("LULL_LOG", "info"),
            soulseek: Soulseek {
                slskd_url: v.req("LULL_SLSKD_URL")?.trim_end_matches('/').to_owned(),
                slskd_api_key: Secret(v.req("LULL_SLSKD_API_KEY")?),
                transfers_dir: v.or("LULL_TRANSFERS_DIR", "/transfers").into(),
                preferred_formats: v
                    .or("LULL_PREFERRED_FORMATS", "flac,mp3,opus,ogg,m4a")
                    .split(',')
                    .map(|f| f.trim().to_ascii_lowercase())
                    .filter(|f| !f.is_empty())
                    .collect(),
                share: match v.or("LULL_SHARE", "library").as_str() {
                    "library" => Share::Library,
                    "none" => Share::None,
                    other => return Err(invalid("LULL_SHARE", format!("{other:?} is not library or none"))),
                },
            },
            cache: Cache {
                max_bytes: {
                    let gb: f64 = v.parse("LULL_CACHE_MAX_GB", "10")?;
                    if gb.is_nan() || gb <= 0.0 {
                        return Err(invalid("LULL_CACHE_MAX_GB", "must be above 0".into()));
                    }
                    (gb * 1_000_000_000.0) as u64
                },
            },
            jellyfin,
            library: Library {
                mode,
                music_dir: v.or("LULL_MUSIC_DIR", "/music").into(),
                naming: v.or(
                    "LULL_NAMING",
                    "{albumartist}/{album} ({year})/{disc}{track:02} - {title}",
                ),
            },
            lastfm: v
                .pair("LULL_LASTFM_API_KEY", "LULL_LASTFM_API_SECRET")?
                .map(|(key, secret)| LastFm {
                    api_key: Secret(key),
                    api_secret: Secret(secret),
                })
                .or_else(LastFm::built_in),
            discovery: Discovery {
                listenbrainz_user: v.opt("LULL_LISTENBRAINZ_USER"),
                listenbrainz_token: v.opt("LULL_LISTENBRAINZ_TOKEN").map(Secret),
            },
        };
        match v.0.into_keys().next() {
            Some(unknown) => Err(ConfigError::Unknown(unknown)),
            None => Ok(config),
        }
    }
}

/// The `LULL_*` settings from `LULL_CONFIG` (if set) and the process environment, which wins.
/// The hub merges these over the settings stored in the app (`settings::merge`).
pub fn env_vars() -> Result<BTreeMap<String, String>, ConfigError> {
    let mut vars: BTreeMap<String, String> = BTreeMap::new();
    if let Ok(path) = std::env::var("LULL_CONFIG") {
        let text = std::fs::read_to_string(&path).map_err(|source| ConfigError::File { path, source })?;
        // The deploy kit's env file also holds settings for compose and slskd; only
        // `LULL_*` lines are lull's, as with the environment.
        vars.extend(parse_env_file(&text).filter(|(k, _)| k.starts_with("LULL_")));
    }
    vars.extend(std::env::vars().filter(|(k, _)| k.starts_with("LULL_")));
    vars.remove("LULL_CONFIG");
    Ok(vars)
}

fn invalid(key: &'static str, reason: String) -> ConfigError {
    ConfigError::Invalid { key, reason }
}

/// `KEY=value` lines; blank lines and `#` comments are skipped, surrounding quotes removed.
fn parse_env_file(text: &str) -> impl Iterator<Item = (String, String)> + '_ {
    text.lines().filter_map(|line| {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return None;
        }
        let (k, v) = line.split_once('=')?;
        let v = v.trim();
        let v = v.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(v);
        Some((k.trim().to_owned(), v.to_owned()))
    })
}

/// Settings not yet consumed; whatever is left at the end is unknown.
struct Vars(BTreeMap<String, String>);

impl Vars {
    fn opt(&mut self, key: &str) -> Option<String> {
        self.0.remove(key).filter(|v| !v.is_empty())
    }
    fn or(&mut self, key: &str, default: &str) -> String {
        self.opt(key).unwrap_or_else(|| default.to_owned())
    }
    fn req(&mut self, key: &'static str) -> Result<String, ConfigError> {
        self.opt(key).ok_or(ConfigError::Missing(key))
    }
    fn parse<T: std::str::FromStr>(&mut self, key: &'static str, default: &str) -> Result<T, ConfigError>
    where
        T::Err: fmt::Display,
    {
        let raw = self.or(key, default);
        raw.parse().map_err(|e: T::Err| invalid(key, e.to_string()))
    }
    fn pair(&mut self, a: &'static str, b: &'static str) -> Result<Option<(String, String)>, ConfigError> {
        match (self.opt(a), self.opt(b)) {
            (Some(x), Some(y)) => Ok(Some((x, y))),
            (None, None) => Ok(None),
            _ => Err(ConfigError::Pair(a, b)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(extra: &[(&str, &str)]) -> BTreeMap<String, String> {
        [("LULL_SLSKD_URL", "http://slskd:5030"), ("LULL_SLSKD_API_KEY", "k")]
            .iter()
            .chain(extra)
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn the_built_in_last_fm_key_needs_both_halves_and_settings_override_it() {
        assert!(LastFm::from_build(Some("k"), Some("s")).is_some());
        for (key, secret) in [
            (None, None),
            (Some("k"), None),
            (Some(""), Some("s")),
            (Some("k"), Some("")),
        ] {
            assert!(LastFm::from_build(key, secret).is_none(), "{key:?} {secret:?}");
        }
        let config = Config::from_vars(vars(&[
            ("LULL_LASTFM_API_KEY", "mine"),
            ("LULL_LASTFM_API_SECRET", "mine-s"),
        ]));
        assert_eq!(
            config
                .ok()
                .and_then(|c| c.lastfm)
                .map(|l| l.api_key.expose().to_owned()),
            Some("mine".to_owned())
        );
    }

    #[test]
    fn rejects_what_would_otherwise_pass_silently() {
        for (extra, ok) in [
            (&[][..], true),
            (&[("LULL_CAHCE_MAX_GB", "5")][..], false),
            (&[("LULL_JELLYFIN_URL", "http://jf")][..], false),
            (&[("LULL_SHARE", "everything")][..], false),
            (&[("LULL_CACHE_MAX_GB", "0")][..], false),
            (&[("LULL_LIBRARY", "jellyfin")][..], false),
            (&[("LULL_LIBRARY", "navidrome")][..], false),
        ] {
            assert_eq!(Config::from_vars(vars(extra)).is_ok(), ok, "{extra:?}");
        }
    }

    #[test]
    fn library_mode_follows_jellyfin_unless_set() -> Result<(), ConfigError> {
        let jf = [("LULL_JELLYFIN_URL", "http://jf"), ("LULL_JELLYFIN_API_KEY", "k")];
        let mode = |extra: &[(&str, &str)]| Config::from_vars(vars(extra)).map(|c| c.library.mode);
        assert_eq!(mode(&[])?, LibraryMode::Folder);
        assert_eq!(mode(&jf)?, LibraryMode::Jellyfin);
        assert_eq!(mode(&[jf[0], jf[1], ("LULL_LIBRARY", "folder")])?, LibraryMode::Folder);
        Ok(())
    }
}
