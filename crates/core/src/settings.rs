//! Configuration the listener changes in the app (ADR-0002). It uses the same `LULL_*` keys as
//! the environment and fills in whatever the environment leaves unset: env, then stored, then
//! the defaults in `config`. The hub validates a change with `Config::from_vars` before storing it.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::db::Db;
use crate::error::{Error, Result};

/// Keys only the environment may set: the hub needs them before the database is open.
pub const ENV_ONLY: &[&str] = &["LULL_DATA_DIR", "LULL_LISTEN", "LULL_LOG"];

/// Where a key's value comes from, for the UI: an `Env` key is shown locked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Env,
    Stored,
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
    if let Some(key) = changes
        .keys()
        .find(|k| !k.starts_with("LULL_") || ENV_ONLY.contains(&k.as_str()))
    {
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
    for (k, v) in env {
        vars.insert(k.clone(), v.clone());
        sources.insert(k.clone(), Source::Env);
    }
    (vars, sources)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn env_wins_and_bootstrap_keys_stay_env_only() -> Result<()> {
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
        let env = [("LULL_MUSIC_DIR".to_string(), "/env".to_string())].into();
        let (vars, sources) = merge(&env, &stored(&db).await?);
        assert_eq!(vars.get("LULL_MUSIC_DIR").map(String::as_str), Some("/env"));
        assert_eq!(sources.get("LULL_MUSIC_DIR"), Some(&Source::Env));
        assert!(!vars.contains_key("LULL_NAMING"));
        Ok(())
    }
}
