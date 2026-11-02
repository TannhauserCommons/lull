//! One SQLite database (`<data_dir>/lull.db`, WAL). Each crate owns its tables and ships its
//! schema as ordered migration steps; `migrate` applies the steps a crate hasn't run yet.
//! A shipped step is never edited, only followed by a new one.

use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;

use crate::error::{Error, Result};

#[derive(Clone)]
pub struct Db(Arc<Mutex<Connection>>);

/// A crate's schema: `owner` is the crate name, `steps` run in order, once each.
pub struct Migrations {
    pub owner: &'static str,
    pub steps: &'static [&'static str],
}

/// Tables every crate may read. Writers: `plays` and `likes`/`follows` are written by `collection`;
/// `core_config` and `core_devices` only through `settings` and `devices`.
pub const CORE: Migrations = Migrations {
    owner: "core",
    steps: &[
        r#"
        CREATE TABLE plays (
            id          INTEGER PRIMARY KEY,
            track_id    TEXT NOT NULL,
            artist      TEXT NOT NULL,
            title       TEXT NOT NULL,
            album       TEXT NOT NULL,
            duration_s  INTEGER NOT NULL,
            started_at  INTEGER NOT NULL,          -- unix seconds
            played_s    INTEGER NOT NULL DEFAULT 0,
            completed   INTEGER NOT NULL DEFAULT 0, -- 1 = reached the end
            skipped     INTEGER NOT NULL DEFAULT 0  -- 1 = skipped before the scrobble point
        );
        CREATE INDEX plays_started ON plays(started_at);
        CREATE INDEX plays_track ON plays(track_id);
        CREATE TABLE likes (
            track_id  TEXT PRIMARY KEY,
            artist    TEXT NOT NULL,
            title     TEXT NOT NULL,
            album     TEXT NOT NULL,
            liked_at  INTEGER NOT NULL
        );
        CREATE TABLE follows (
            kind        TEXT NOT NULL CHECK (kind IN ('artist', 'tag')),
            id          TEXT NOT NULL,
            name        TEXT NOT NULL,
            followed_at INTEGER NOT NULL,
            PRIMARY KEY (kind, id)
        );
    "#,
        // ADR-0002: settings changed in the app, and paired devices.
        r#"
        CREATE TABLE core_config (
            key         TEXT PRIMARY KEY,   -- a LULL_* key, as in the environment
            value       TEXT NOT NULL,
            updated_at  INTEGER NOT NULL
        );
        CREATE TABLE core_devices (
            id          TEXT PRIMARY KEY,   -- "d-<16 hex>"
            name        TEXT NOT NULL,
            kind        TEXT NOT NULL CHECK (kind IN ('browser', 'app', 'subsonic')),
            token_hash  TEXT NOT NULL UNIQUE,  -- sha256 of the token, hex
            secret      TEXT,               -- subsonic only: the app password itself
            created_at  INTEGER NOT NULL,
            last_seen   INTEGER
        );
    "#,
        // Private listening: what a device plays leaves no trace (history, taste, Last.fm).
        r#"
        ALTER TABLE core_devices ADD COLUMN private INTEGER NOT NULL DEFAULT 0;
    "#,
    ],
};

impl Db {
    pub fn open(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir).map_err(|e| Error::internal(format!("create {}: {e}", dir.display())))?;
        let conn = Connection::open(dir.join("lull.db"))?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS schema (owner TEXT PRIMARY KEY, version INTEGER NOT NULL);",
        )?;
        Ok(Db(Arc::new(Mutex::new(conn))))
    }

    /// An empty in-memory database, for tests.
    pub fn memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("CREATE TABLE schema (owner TEXT PRIMARY KEY, version INTEGER NOT NULL);")?;
        Ok(Db(Arc::new(Mutex::new(conn))))
    }

    /// Runs `f` with the connection on the blocking pool, so async tasks never wait on disk.
    pub async fn call<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    {
        let db = self.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = db.lock().map_err(|_| Error::internal("database lock poisoned"))?;
            f(&mut conn)
        })
        .await
        .map_err(|e| Error::internal(format!("database task: {e}")))?
    }

    pub fn migrate(&self, m: &Migrations) -> Result<()> {
        let mut conn = self.0.lock().map_err(|_| Error::internal("database lock poisoned"))?;
        let tx = conn.transaction()?;
        let done: i64 = tx
            .query_row("SELECT version FROM schema WHERE owner = ?1", [m.owner], |r| r.get(0))
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(0),
                e => Err(e),
            })?;
        let done =
            usize::try_from(done).map_err(|_| Error::internal(format!("{} schema version is negative", m.owner)))?;
        if done > m.steps.len() {
            return Err(Error::internal(format!(
                "{} schema is at step {done}, newer than this build ({})",
                m.owner,
                m.steps.len()
            )));
        }
        for step in &m.steps[done..] {
            tx.execute_batch(step)?;
        }
        tx.execute(
            "INSERT INTO schema (owner, version) VALUES (?1, ?2)
             ON CONFLICT(owner) DO UPDATE SET version = excluded.version",
            rusqlite::params![m.owner, m.steps.len() as i64],
        )?;
        tx.commit()?;
        Ok(())
    }
}
