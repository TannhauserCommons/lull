//! The scrobble queue: every eligible play is written here first and deleted only once Last.fm
//! has taken it, so scrobbles survive restarts and offline spells. A failed batch waits with
//! exponential backoff (30 s doubling, at most an hour) and is retried for as long as it takes.

use rusqlite::{Connection, params};

use crate::api::Play;
use lull_core::Result;

const FIRST_BACKOFF_S: i64 = 30;
const MAX_BACKOFF_S: i64 = 3600;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Queued {
    pub play_id: i64,
    pub play: Play,
    pub attempts: u32,
}

/// Queues a play once; a repeated `play.ended` for the same play changes nothing.
pub fn push(conn: &Connection, play_id: i64, play: &Play, now: i64) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO lastfm_queue (play_id, artist, title, album, duration_s, started_at, next_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            play_id,
            play.artist,
            play.title,
            play.album,
            play.duration_s,
            play.started_at,
            now
        ],
    )?;
    Ok(())
}

/// Up to `limit` plays whose wait is over, oldest play first (Last.fm wants them in order).
pub fn due(conn: &Connection, now: i64, limit: usize) -> Result<Vec<Queued>> {
    let mut stmt = conn.prepare_cached(
        "SELECT play_id, artist, title, album, duration_s, started_at, attempts FROM lastfm_queue
         WHERE next_at <= ?1 ORDER BY started_at LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![now, limit as i64], |r| {
        Ok(Queued {
            play_id: r.get(0)?,
            play: Play {
                artist: r.get(1)?,
                title: r.get(2)?,
                album: r.get(3)?,
                duration_s: r.get(4)?,
                started_at: r.get(5)?,
            },
            attempts: r.get(6)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn remove(conn: &Connection, play_ids: &[i64]) -> Result<()> {
    let mut stmt = conn.prepare_cached("DELETE FROM lastfm_queue WHERE play_id = ?1")?;
    for id in play_ids {
        stmt.execute([id])?;
    }
    Ok(())
}

/// Records a failed attempt and schedules the next one.
pub fn defer(conn: &Connection, batch: &[Queued], now: i64, error: &str) -> Result<()> {
    let mut stmt =
        conn.prepare_cached("UPDATE lastfm_queue SET attempts = ?2, next_at = ?3, last_error = ?4 WHERE play_id = ?1")?;
    for q in batch {
        let attempts = q.attempts + 1;
        stmt.execute(params![q.play_id, attempts, now + backoff_s(attempts), error])?;
    }
    Ok(())
}

/// "Retry now": every waiting play becomes due.
pub fn wake_all(conn: &Connection) -> Result<()> {
    conn.execute("UPDATE lastfm_queue SET next_at = 0", [])?;
    Ok(())
}

pub fn clear(conn: &Connection) -> Result<()> {
    conn.execute("DELETE FROM lastfm_queue", [])?;
    Ok(())
}

/// How many plays wait, and when the next one is due.
pub fn summary(conn: &Connection) -> Result<(u64, Option<i64>)> {
    Ok(
        conn.query_row("SELECT COUNT(*), MIN(next_at) FROM lastfm_queue", [], |r| {
            Ok((r.get::<_, i64>(0)?.max(0) as u64, r.get(1)?))
        })?,
    )
}

/// The most recent failure, if any play is waiting after one.
pub fn last_error(conn: &Connection) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT last_error FROM lastfm_queue WHERE last_error IS NOT NULL ORDER BY next_at DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            e => Err(e),
        })?)
}

fn backoff_s(attempts: u32) -> i64 {
    let doublings = attempts.saturating_sub(1).min(16);
    (FIRST_BACKOFF_S << doublings).min(MAX_BACKOFF_S)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lull_core::db::Db;

    fn play(title: &str, started_at: i64) -> Play {
        Play {
            artist: "Grouper".into(),
            title: title.into(),
            album: "Shade".into(),
            duration_s: 200,
            started_at,
        }
    }

    fn open(dir: &std::path::Path) -> Result<Db> {
        let db = Db::open(dir)?;
        db.migrate(&crate::MIGRATIONS)?;
        Ok(db)
    }

    #[tokio::test]
    async fn queued_plays_survive_a_restart_and_wait_out_their_backoff() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("lull-lastfm-queue-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let db = open(&dir)?;
        db.call(|c| {
            push(c, 2, &play("Kelso", 1_100), 2_000)?;
            push(c, 1, &play("Followed the Ocean", 1_000), 2_000)?;
            push(c, 1, &play("Followed the Ocean", 1_000), 2_000)?; // same play again: ignored
            let batch = due(c, 2_000, 50)?;
            assert_eq!(
                batch.iter().map(|q| q.play_id).collect::<Vec<_>>(),
                [1, 2],
                "oldest first"
            );
            defer(c, &batch, 2_000, "Last.fm is unavailable")
        })
        .await?;
        drop(db);

        let db = open(&dir)?; // the server restarted
        let (waiting, retry_at, early, on_time) = db
            .call(|c| {
                let (waiting, retry_at) = summary(c)?;
                Ok((waiting, retry_at, due(c, 2_029, 50)?, due(c, 2_030, 50)?))
            })
            .await?;
        assert_eq!((waiting, retry_at), (2, Some(2_030)));
        assert!(early.is_empty(), "still backing off");
        assert_eq!(on_time.len(), 2);
        assert_eq!(on_time[0].attempts, 1);
        assert_eq!(on_time[0].play, play("Followed the Ocean", 1_000));
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn backoff_doubles_up_to_an_hour() {
        let waits: Vec<i64> = (1..=9).map(backoff_s).collect();
        assert_eq!(waits, [30, 60, 120, 240, 480, 960, 1920, 3600, 3600]);
        assert_eq!(backoff_s(u32::MAX), 3600);
    }
}
