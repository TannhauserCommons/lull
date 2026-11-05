//! Whether Soulseek peers share a song (`Transfers::song_available`), for the availability mark
//! on shelves (Home's picks). A song whose sources are known (streamed, prefetched, searched)
//! answers at once with no search; otherwise answers are remembered in `soulseek_songs` (found
//! for 7 days, not found for 1), and a song not remembered costs one search in the search
//! budget's `Availability` lane, behind playback, preloading and the listener's own searches,
//! one song at a time (docs/context/soulseek.md#albums).

use lull_core::Result;
use lull_core::db::Db;
use lull_core::ids::TrackId;
use lull_core::model::Track;
use rusqlite::{OptionalExtension, params};

use crate::Service;

/// As for albums: peers sharing a song rarely all leave within a week...
const FOUND_FOR_S: i64 = 7 * 24 * 3600;
/// ...and one nobody shares yet is asked about again after a day.
const MISSING_FOR_S: i64 = 24 * 3600;

/// The remembered or searched answer; `None` when Soulseek is offline and nothing is
/// remembered, or the search said nothing (`Searches::song_shared`).
pub async fn available(svc: &Service, track: &Track) -> Result<Option<bool>> {
    if svc.searches.knows(track) {
        return Ok(Some(true));
    }
    if let Some(known) = remembered(&svc.ctx.db, &track.id).await? {
        return Ok(Some(known));
    }
    if !svc.online() {
        return Ok(None);
    }
    let _one = svc.song_checks.lock().await;
    if let Some(known) = remembered(&svc.ctx.db, &track.id).await? {
        return Ok(Some(known));
    }
    if !svc.online() {
        return Ok(None);
    }
    let shared = match svc.searches.song_shared(track).await {
        Ok(shared) => shared,
        Err(e) => {
            tracing::debug!(error = %e, track = %track.id, "checking whether peers share a song");
            None
        }
    };
    if let Some(shared) = shared {
        remember(&svc.ctx.db, track.id.clone(), shared).await?;
    }
    Ok(shared)
}

fn fresh(available: bool, checked_at: i64, now: i64) -> bool {
    let keep = if available { FOUND_FOR_S } else { MISSING_FOR_S };
    now.saturating_sub(checked_at) < keep
}

async fn remembered(db: &Db, id: &TrackId) -> Result<Option<bool>> {
    let id = id.clone();
    let row: Option<(bool, i64)> = db
        .call(move |c| {
            Ok(c.query_row(
                "SELECT available, checked_at FROM soulseek_songs WHERE track_id = ?1",
                [id.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
        })
        .await?;
    Ok(row
        .filter(|(available, at)| fresh(*available, *at, lull_core::now()))
        .map(|(available, _)| available))
}

/// Stores an answer, and drops answers too old to be used again.
async fn remember(db: &Db, id: TrackId, available: bool) -> Result<()> {
    db.call(move |c| {
        let now = lull_core::now();
        c.execute(
            "INSERT INTO soulseek_songs (track_id, available, checked_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(track_id) DO UPDATE SET available = excluded.available, checked_at = excluded.checked_at",
            params![id.as_str(), available, now],
        )?;
        c.execute("DELETE FROM soulseek_songs WHERE checked_at < ?1", [now - FOUND_FOR_S])?;
        Ok(())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::fresh;

    const DAY: i64 = 24 * 3600;

    #[test]
    fn found_holds_for_a_week_and_not_found_for_a_day() {
        let now = 100 * DAY;
        assert!(fresh(true, now - 6 * DAY, now));
        assert!(!fresh(true, now - 7 * DAY, now));
        assert!(fresh(false, now - DAY + 60, now));
        assert!(
            !fresh(false, now - DAY, now),
            "a song nobody shared yet is checked again"
        );
    }
}
