//! Configuration the listener changes in the app (ADR-0002). It uses the same `LULL_*` keys as
//! the environment and fills in whatever the environment leaves unset: env, then stored, then
//! the defaults in `config`. The hub validates a change with `Config::from_vars` before storing it.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::db::Db;
use crate::error::{Error, Result};

/// Keys only the environment may set: the hub needs them before the database is open, or they
/// describe how lull is run (the setup code, the slskd.yml of an slskd the app runs).
pub const ENV_ONLY: &[&str] = &[
    "LULL_DATA_DIR",
    "LULL_LISTEN",
    "LULL_LOG",
    "LULL_SETUP_CODE",
    "LULL_SLSKD_CONFIG_FILE",
];

/// Every key the listener can set in the app, and whether it is a secret (the UI only learns
/// whether a secret is set). With `ENV_ONLY`, these are all the keys `Config` reads.
pub const KEYS: &[Key] = &[
    Key::plain("LULL_PUBLIC_URL"),
    Key::plain("LULL_SLSKD_URL"),
    Key::secret("LULL_SLSKD_API_KEY"),
    Key::plain("LULL_TRANSFERS_DIR"),
    Key::plain("LULL_PREFERRED_FORMATS"),
    Key::plain("LULL_SHARE"),
    Key::plain("LULL_SOULSEEK_USERNAME"),
    Key::secret("LULL_SOULSEEK_PASSWORD"),
    Key::plain("LULL_CACHE_MAX_GB"),
    Key::plain("LULL_LIBRARY"),
    Key::plain("LULL_JELLYFIN_URL"),
    Key::secret("LULL_JELLYFIN_API_KEY"),
    Key::plain("LULL_JELLYFIN_USER"),
    Key::plain("LULL_MUSIC_DIR"),
    Key::plain("LULL_NAMING"),
    Key::secret("LULL_LASTFM_API_KEY"),
    Key::secret("LULL_LASTFM_API_SECRET"),
    Key::plain("LULL_LISTENBRAINZ_USER"),
    Key::secret("LULL_LISTENBRAINZ_TOKEN"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Key {
    pub name: &'static str,
    pub secret: bool,
}

impl Key {
    const fn plain(name: &'static str) -> Key {
        Key { name, secret: false }
    }
    const fn secret(name: &'static str) -> Key {
        Key { name, secret: true }
    }
}

/// Where a key's value comes from, for the UI: an `Env` key is shown locked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Env,
    Stored,
    /// Neither set: `Config` uses its default.
    Default,
}

/// Every stored key and its value.
pub async fn stored(db: &Db) -> Result<BTreeMap<String, String>> {
    db.call(|conn| {
        let mut stmt = conn.prepare("SELECT key, value FROM core_config")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    })
    .await
}

/// Applies `changes` in one transaction: `Some` sets a key, `None` removes it (back to default).
pub async fn store(db: &Db, changes: BTreeMap<String, Option<String>>) -> Result<()> {
    if let Some(key) = changes.keys().find(|k| !KEYS.iter().any(|key| key.name == k.as_str())) {
        return Err(Error::BadRequest(format!("{key} can't be set in the app")));
    }
    let now = crate::now();
    db.call(move |conn| {
        let tx = conn.transaction()?;
        for (key, value) in &changes {
            match value {
                Some(v) => tx.execute(
                    "INSERT INTO core_config (key, value, updated_at) VALUES (?1, ?2, ?3)
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
                    rusqlite::params![key, v, now],
                )?,
                None => tx.execute("DELETE FROM core_config WHERE key = ?1", [key])?,
            };
        }
        tx.commit()?;
        Ok(())
    })
    .await
}

/// The settings the hub starts with, and where each came from. The environment wins.
pub fn merge(
    env: &BTreeMap<String, String>,
    stored: &BTreeMap<String, String>,
) -> (BTreeMap<String, String>, BTreeMap<String, Source>) {
    let mut vars = BTreeMap::new();
    let mut sources = BTreeMap::new();
    for (k, v) in stored.iter().filter(|(k, _)| !ENV_ONLY.contains(&k.as_str())) {
        vars.insert(k.clone(), v.clone());
        sources.insert(k.clone(), Source::Stored);
    }
    // An empty variable is unset: the deploy kit passes every key in its env file, empty or not.
    for (k, v) in env.iter().filter(|(_, v)| !v.is_empty()) {
        vars.insert(k.clone(), v.clone());
        sources.insert(k.clone(), Source::Env);
    }
    (vars, sources)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn env_wins_unless_empty_and_bootstrap_keys_stay_env_only() -> Result<()> {
        let db = Db::memory()?;
        db.migrate(&crate::db::CORE)?;
        assert!(
            store(&db, [("LULL_DATA_DIR".into(), Some("/x".into()))].into())
                .await
                .is_err()
        );
        store(
            &db,
            [
                ("LULL_MUSIC_DIR".into(), Some("/stored".into())),
                ("LULL_NAMING".into(), Some("{title}".into())),
            ]
            .into(),
        )
        .await?;
        store(&db, [("LULL_NAMING".into(), None)].into()).await?;
        store(&db, [("LULL_SHARE".into(), Some("none".into()))].into()).await?;
        let env = [
            ("LULL_MUSIC_DIR".to_string(), "/env".to_string()),
            ("LULL_SHARE".to_string(), String::new()),
        ]
        .into();
        let (vars, sources) = merge(&env, &stored(&db).await?);
        assert_eq!(vars.get("LULL_MUSIC_DIR").map(String::as_str), Some("/env"));
        assert_eq!(sources.get("LULL_MUSIC_DIR"), Some(&Source::Env));
        assert!(!vars.contains_key("LULL_NAMING"));
        assert_eq!(
            sources.get("LULL_SHARE"),
            Some(&Source::Stored),
            "an empty env value is unset"
        );
        Ok(())
    }

    /// The key lists, `Config` and `deploy/.env.example` describe the same settings.
    #[test]
    fn every_key_is_listed_documented_and_read() {
        let example = include_str!("../../../deploy/.env.example");
        let names: Vec<&str> = KEYS.iter().map(|k| k.name).chain(ENV_ONLY.iter().copied()).collect();
        for line in example.lines().filter(|l| l.starts_with("LULL_")) {
            let key = line.split('=').next().unwrap_or_default();
            assert!(
                names.contains(&key),
                "{key} is documented but not listed in KEYS or ENV_ONLY"
            );
        }
        for name in names {
            assert!(
                example.contains(&format!("\n{name}=")),
                "{name} is not in deploy/.env.example"
            );
            // An empty value means "use the default", so only an unknown key can fail here.
            let mut vars: BTreeMap<String, String> = [
                ("LULL_SLSKD_URL".into(), "http://slskd:5030".into()),
                ("LULL_SLSKD_API_KEY".into(), "k".into()),
            ]
            .into();
            vars.entry(name.into()).or_default();
            assert!(crate::config::Config::from_vars(vars).is_ok(), "Config rejects {name}");
        }
    }
}
