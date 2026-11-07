//! Configuration the listener changes in the app (ADR-0002). It uses the same `LULL_*` keys as
//! the environment and fills in whatever the environment leaves unset: env, then stored, then
//! the defaults in `config`. The hub validates a change with `Config::from_vars` before storing it.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::db::Db;
use crate::error::{Error, Result};
use crate::secrets;

/// Keys only the environment may set: the hub needs them before the database is open, or they
/// describe how lull is run (the setup code, the slskd.yml of an slskd the app runs, the programs
/// the image, the app or the Linux package ships).
pub const ENV_ONLY: &[&str] = &[
    "LULL_DATA_DIR",
    "LULL_LISTEN",
    "LULL_LOG",
    "LULL_SETUP_CODE",
    "LULL_SLSKD_CONFIG_FILE",
    "LULL_SLSKD",
    "LULL_KIT",
    "LULL_INSTALL",
    "LULL_YTDLP",
    "LULL_DENO",
];

/// Every key the listener can set in the app, and whether it is a secret (the UI only learns
/// whether a secret is set). With `ENV_ONLY`, these are all the keys `Config` reads.
pub const KEYS: &[Key] = &[
    Key::plain("LULL_PUBLIC_URL"),
    Key::plain("LULL_SLSKD_URL"),
    Key::secret("LULL_SLSKD_API_KEY"),
    Key::plain("LULL_SLSKD_WEB_USERNAME"),
    Key::secret("LULL_SLSKD_WEB_PASSWORD"),
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
    Key::secret("LULL_ACOUSTID_KEY"),
    Key::secret("LULL_ACOUSTID_USER_KEY"),
    Key::plain("LULL_LISTENBRAINZ_USER"),
    Key::secret("LULL_LISTENBRAINZ_TOKEN"),
    Key::plain("LULL_CATALOG_COUNTRY"),
    Key::plain("LULL_SKINS_DIR"),
    Key::plain("LULL_FLAGS"),
    Key::plain("LULL_SELF_UPDATE"),
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

/// Every stored key and its value: the database's rows, and the secrets kept in a vault (ADR-0091). A
/// secret still in the database is newer than the vault's copy (an older lull wrote it), so it wins.
pub async fn stored(db: &Db) -> Result<BTreeMap<String, String>> {
    let mut stored: BTreeMap<String, String> = stored_rows(db).await?.into_iter().collect();
    for (name, value) in secrets::all(db) {
        if is_secret(&name) {
            stored.entry(name).or_insert(value);
        }
    }
    Ok(stored)
}

/// The secret settings kept in a vault that can't be read now (a locked keyring), and why.
pub fn withheld(db: &Db) -> BTreeMap<String, String> {
    let mut unreadable = secrets::unreadable(db);
    unreadable.retain(|name, _| is_secret(name));
    unreadable
}

fn is_secret(name: &str) -> bool {
    KEYS.iter().any(|k| k.secret && k.name == name)
}

/// Applies `changes`: `Some` sets a key, `None` removes it (back to default). With the `secrets` switch
/// on, a secret goes to its vault first, so a vault that refuses changes nothing; the rest is one
/// transaction.
pub async fn store(db: &Db, mut changes: BTreeMap<String, Option<String>>) -> Result<()> {
    if let Some(key) = changes.keys().find(|k| !KEYS.iter().any(|key| key.name == k.as_str())) {
        return Err(Error::BadRequest(format!("{key} can't be set in the app")));
    }
    let on = secrets::on(db);
    let secret: Vec<String> = changes.keys().filter(|k| is_secret(k)).cloned().collect();
    for key in secret {
        match changes.get(&key).cloned().flatten() {
            Some(value) if on => secrets::put(db, &key, &value).await?,
            Some(_) => continue,
            // Removed, wherever it is kept: a vault's copy would otherwise still count.
            None if secrets::kept(db, &key).is_some() => secrets::remove(db, &key).await?,
            None => continue,
        }
        // A row an older lull left goes too: the vault holds the setting now.
        changes.insert(key, None);
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

/// Secret settings still in the database (from before the `secrets` switch, or written by an older
/// lull) move to the vault. A row goes only once the vault has read its value back, and is
/// overwritten on disk as it goes (`secure_delete`, then the WAL emptied); one that can't move stays
/// for the next start, and only its name is logged.
pub(crate) async fn move_secrets(db: &Db) {
    let rows = match stored_rows(db).await {
        Ok(rows) => rows,
        Err(e) => return tracing::warn!("reading settings to move their secrets: {e}"),
    };
    let mut moved = Vec::new();
    for (key, value) in rows.into_iter().filter(|(k, _)| is_secret(k)) {
        match secrets::put(db, &key, &value).await {
            Ok(()) => moved.push(key),
            Err(e) => tracing::warn!("{key} stays in the database for now: {e}"),
        }
    }
    if moved.is_empty() {
        return;
    }
    let count = moved.len();
    let erased = db
        .call(move |conn| {
            conn.pragma_update(None, "secure_delete", true)?;
            let tx = conn.transaction()?;
            for key in &moved {
                tx.execute("DELETE FROM core_config WHERE key = ?1", [key])?;
            }
            tx.commit()?;
            conn.pragma_update(None, "secure_delete", false)?;
            conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
            Ok(())
        })
        .await;
    match erased {
        Ok(()) => tracing::info!(count, "secret settings moved out of the database"),
        Err(e) => tracing::warn!("secret settings are in the vault, but their old rows stay for now: {e}"),
    }
}

async fn stored_rows(db: &Db) -> Result<Vec<(String, String)>> {
    db.call(|conn| {
        let mut stmt = conn.prepare("SELECT key, value FROM core_config")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
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
