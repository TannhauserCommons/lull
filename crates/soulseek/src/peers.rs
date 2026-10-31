//! Memory of peers that stalled on us, so ranking can demote them for a while. A finished
//! transfer from a peer clears its record: it works again.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use lull_core::Result;
use lull_core::db::Db;
use rusqlite::params;

/// A stall counts against a peer for this long.
const MEMORY_SECS: i64 = 7 * 24 * 3600;

#[derive(Clone)]
pub struct Peers {
    db: Db,
    /// user → unix time of their last stall; mirrors `soulseek_peers`.
    stalled: Arc<RwLock<HashMap<String, i64>>>,
}

impl Peers {
    pub async fn load(db: Db) -> Result<Self> {
        let since = lull_core::now() - MEMORY_SECS;
        let rows = db
            .call(move |c| {
                c.execute("DELETE FROM soulseek_peers WHERE stalled_at < ?1", [since])?;
                let mut stmt = c.prepare("SELECT user, stalled_at FROM soulseek_peers")?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
                Ok(rows.collect::<Result<HashMap<String, i64>, _>>()?)
            })
            .await?;
        Ok(Peers {
            db,
            stalled: Arc::new(RwLock::new(rows)),
        })
    }

    pub fn stalled_before(&self, user: &str) -> bool {
        let since = lull_core::now() - MEMORY_SECS;
        self.stalled
            .read()
            .is_ok_and(|m| m.get(user).is_some_and(|&at| at >= since))
    }

    pub async fn stalled(&self, user: &str) -> Result<()> {
        let now = lull_core::now();
        if let Ok(mut m) = self.stalled.write() {
            m.insert(user.to_owned(), now);
        }
        let user = user.to_owned();
        self.db
            .call(move |c| {
                c.execute(
                    "INSERT INTO soulseek_peers (user, stalls, stalled_at) VALUES (?1, 1, ?2)
                     ON CONFLICT(user) DO UPDATE SET stalls = stalls + 1, stalled_at = excluded.stalled_at",
                    params![user, now],
                )?;
                Ok(())
            })
            .await
    }

    pub async fn delivered(&self, user: &str) -> Result<()> {
        let known = self.stalled.write().is_ok_and(|mut m| m.remove(user).is_some());
        if !known {
            return Ok(());
        }
        let user = user.to_owned();
        self.db
            .call(move |c| {
                c.execute("DELETE FROM soulseek_peers WHERE user = ?1", [user])?;
                Ok(())
            })
            .await
    }
}
