//! Politeness for every outside source: answers are kept in `discovery_cache` for a
//! source-specific time, and an expired answer is still served when the source fails. Calls
//! are spaced out by `Http::pace`, shared with every other lull crate.

use std::future::Future;

use lull_core::db::Db;
use lull_core::{Error, Result};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use serde::de::DeserializeOwned;

pub const HOUR: i64 = 3600;
pub const DAY: i64 = 24 * HOUR;

/// Rows unused for this long are dropped at start-up.
const FORGET_AFTER: i64 = 90 * DAY;

#[derive(Clone)]
pub struct Cache {
    db: Db,
}

impl Cache {
    pub fn new(db: Db) -> Self {
        Cache { db }
    }

    pub async fn prune(&self) -> Result<()> {
        let cutoff = lull_core::now() - FORGET_AFTER;
        self.db
            .call(move |c| {
                c.execute("DELETE FROM discovery_cache WHERE fetched_at < ?1", [cutoff])?;
                Ok(())
            })
            .await
    }

    /// The cached answer for `key` if younger than `ttl` seconds; else `fetch`, stored. When
    /// `fetch` fails, an expired answer is better than none.
    pub async fn get<T, F>(&self, key: String, ttl: i64, fetch: impl FnOnce() -> F) -> Result<T>
    where
        T: Serialize + DeserializeOwned + Send + 'static,
        F: Future<Output = Result<T>>,
    {
        self.get_with(key, |_| ttl, fetch).await
    }

    /// `get`, keeping each answer for `ttl(answer)` seconds: "not found" can be kept shorter
    /// than a find, so a source that learns of it later is asked again.
    pub async fn get_with<T, F>(&self, key: String, ttl: impl Fn(&T) -> i64, fetch: impl FnOnce() -> F) -> Result<T>
    where
        T: Serialize + DeserializeOwned + Send + 'static,
        F: Future<Output = Result<T>>,
    {
        let stored = self.read(key.clone()).await?;
        if let Some((value, at)) = &stored
            && let Ok(answer) = serde_json::from_value::<T>(value.clone())
            && lull_core::now() - at < ttl(&answer)
        {
            return Ok(answer);
        }
        match fetch().await {
            Ok(fresh) => {
                self.write(key, &fresh).await?;
                Ok(fresh)
            }
            Err(e) => match stored {
                Some((value, _)) => {
                    tracing::info!(error = %e, key, "source failed; serving the stale answer");
                    serde_json::from_value(value).map_err(|e| Error::internal(format!("cache {key}: {e}")))
                }
                None => Err(e),
            },
        }
    }

    /// The cached answer for `key` if younger than `ttl` seconds (for batch lookups that fetch
    /// only what is missing, then `put` each answer).
    pub async fn fresh<T: DeserializeOwned>(&self, key: String, ttl: i64) -> Result<Option<T>> {
        Ok(self
            .read(key)
            .await?
            .filter(|(_, at)| lull_core::now() - at < ttl)
            .and_then(|(value, _)| serde_json::from_value(value).ok()))
    }

    pub async fn put<T: Serialize>(&self, key: String, value: &T) -> Result<()> {
        self.write(key, value).await
    }

    async fn read(&self, key: String) -> Result<Option<(serde_json::Value, i64)>> {
        let row: Option<(String, i64)> = self
            .db
            .call(move |c| {
                Ok(c.query_row(
                    "SELECT json, fetched_at FROM discovery_cache WHERE key = ?1",
                    [key],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?)
            })
            .await?;
        // A row written by an older build that no longer parses is simply fetched again.
        Ok(row.and_then(|(json, at)| serde_json::from_str(&json).ok().map(|v| (v, at))))
    }

    async fn write<T: Serialize>(&self, key: String, value: &T) -> Result<()> {
        let json = serde_json::to_string(value).map_err(|e| Error::internal(format!("cache json: {e}")))?;
        self.db
            .call(move |c| {
                c.execute(
                    "INSERT INTO discovery_cache (key, json, fetched_at) VALUES (?1, ?2, ?3)
                     ON CONFLICT(key) DO UPDATE SET json = excluded.json, fetched_at = excluded.fetched_at",
                    params![key, json, lull_core::now()],
                )?;
                Ok(())
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use lull_core::Result;
    use lull_core::db::Db;

    use super::{Cache, DAY};

    #[tokio::test]
    async fn not_finding_is_kept_shorter_than_finding() -> Result<()> {
        // Live, an album MusicBrainz didn't have yet stayed "not found" for 30 days.
        let db = Db::memory()?;
        db.migrate(&crate::MIGRATIONS)?;
        let cache = Cache::new(db.clone());
        let asked = AtomicU32::new(0);
        let ttl = |found: &Option<String>| if found.is_some() { 30 * DAY } else { DAY };
        let lookup = |answer: Option<&'static str>| {
            let asked = &asked;
            move || async move {
                asked.fetch_add(1, Ordering::Relaxed);
                Ok(answer.map(str::to_owned))
            }
        };
        let age = |days: i64| {
            let db = db.clone();
            async move {
                db.call(move |c| {
                    c.execute("UPDATE discovery_cache SET fetched_at = fetched_at - ?1", [days * DAY])?;
                    Ok(())
                })
                .await
            }
        };
        cache.get_with("missing".into(), ttl, lookup(None)).await?;
        age(2).await?;
        let again = cache.get_with("missing".into(), ttl, lookup(Some("found"))).await?;
        assert_eq!(again.as_deref(), Some("found"), "asked again after a day");
        age(2).await?;
        cache.get_with("missing".into(), ttl, lookup(None)).await?;
        assert_eq!(asked.load(Ordering::Relaxed), 2, "a find is kept a month");
        Ok(())
    }
}
