//! One SQLite database (`<data_dir>/lull.db`, WAL). Each crate owns its tables and ships its
//! schema as ordered migration steps; `migrate` applies the steps a crate hasn't run yet.
//! A shipped step is never edited, only followed by a new one.

use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;

use crate::error::{Error, Result};
use crate::secrets::{FileVault, Vault, Vaults};

/// The database, and the vaults the credentials it doesn't hold are kept in (ADR-0091, `secrets`).
#[derive(Clone)]
pub struct Db {
    conn: Arc<Mutex<Connection>>,
    vaults: Arc<Vaults>,
}

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
    /// Opens `<dir>/lull.db`, readable by lull's own user alone (ADR-0091): a new folder is 0700, a
    /// new database 0600 (SQLite gives its `-wal` and `-shm` the database's mode), and a database an
    /// older lull made readable to others is narrowed.
    pub fn open(dir: &Path) -> Result<Self> {
        let fail = |what: &Path, e: std::io::Error| Error::internal(format!("{}: {e}", what.display()));
        crate::private::create_dir_all(dir).map_err(|e| fail(dir, e))?;
        let path = dir.join("lull.db");
        match crate::private::create_new(&path) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                // A folder lull doesn't own (a share that ignores modes, a volume another user made) may
                // refuse: the hub still starts, and says so.
                for file in [path.clone(), dir.join("lull.db-wal"), dir.join("lull.db-shm")] {
                    if let Err(e) = crate::private::restrict(&file) {
                        tracing::warn!("can't make {} readable by lull alone: {e}", file.display());
                    }
                }
            }
            Err(e) => return Err(fail(&path, e)),
        }
        let mut conn = Connection::open(&path)?;
        writes_first(&mut conn);
        conn.execute_batch(
            "PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS schema (owner TEXT PRIMARY KEY, version INTEGER NOT NULL);",
        )?;
        Ok(Db {
            conn: Arc::new(Mutex::new(conn)),
            vaults: Arc::new(Vaults::new(Some(FileVault::new(dir)))),
        })
    }

    /// This database with the system keyring as the vault credentials go to (the desktop app's hub).
    /// Before the hub starts: the vaults read at start are the keyring and the file.
    #[must_use]
    pub fn with_keyring(self, keyring: Arc<dyn Vault>) -> Db {
        Db {
            vaults: Arc::new(self.vaults.with_keyring(keyring)),
            conn: self.conn,
        }
    }

    pub(crate) fn vaults(&self) -> &Vaults {
        &self.vaults
    }

    /// An empty in-memory database, for tests. It has no vault: tests of credentials open a folder.
    pub fn memory() -> Result<Self> {
        let mut conn = Connection::open_in_memory()?;
        writes_first(&mut conn);
        conn.execute_batch("CREATE TABLE schema (owner TEXT PRIMARY KEY, version INTEGER NOT NULL);")?;
        Ok(Db {
            conn: Arc::new(Mutex::new(conn)),
            vaults: Arc::new(Vaults::new(None)),
        })
    }

    /// Runs `f` with the connection on the blocking pool, so async tasks never wait on disk.
    pub async fn call<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    {
        let db = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = db.lock().map_err(|_| Error::internal("database lock poisoned"))?;
            f(&mut conn)
        })
        .await
        .map_err(|e| Error::internal(format!("database task: {e}")))?
    }

    pub fn migrate(&self, m: &Migrations) -> Result<()> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| Error::internal("database lock poisoned"))?;
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

/// Every transaction here writes, so it takes the write lock as it begins (`BEGIN IMMEDIATE`) and a busy database
/// makes it wait, up to rusqlite's 5 s. A deferred one that reads first and writes after another connection wrote
/// (the desktop app's own connection beside its hub's, a test's beside a running hub) gets SQLite's
/// "database is locked" at once: in WAL mode that upgrade never waits.
fn writes_first(conn: &mut Connection) {
    conn.set_transaction_behavior(rusqlite::TransactionBehavior::Immediate);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A transaction holds the write lock from its start, so a second connection writing meanwhile waits for it
    /// instead of making it fail later (the cause of a flaky queue run, 42538849524).
    #[tokio::test]
    async fn a_transaction_takes_the_write_lock_as_it_begins() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("lull-db-immediate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let db = Db::open(&dir)?;
        let path = dir.join("lull.db");
        let other_wrote = db
            .call(move |conn| {
                let tx = conn.transaction()?;
                // Another connection that doesn't wait: it can begin writing only if this one holds no lock.
                let other = Connection::open(&path)?;
                other.busy_timeout(std::time::Duration::ZERO)?;
                let wrote = other.execute_batch("BEGIN IMMEDIATE; ROLLBACK;").is_ok();
                tx.rollback()?;
                Ok(wrote)
            })
            .await?;
        assert!(!other_wrote, "the open transaction already holds the write lock");
        let _ = std::fs::remove_dir_all(dir);
        Ok(())
    }
}
