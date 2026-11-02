//! Whether Soulseek peers share an album (`Transfers::album_available`), for shelves that show
//! only what can play (new releases). Answers are remembered in `soulseek_albums` (found for 7
//! days, not found for 1) and answered from there at once; otherwise one early-stopped search,
//! behind playback and preloading, one album at a time (docs/context/soulseek.md#albums).

use lull_core::Result;
use lull_core::db::Db;
use lull_core::ids::{loose_artist, loose_title};
use rusqlite::{OptionalExtension, params};

use crate::Service;

/// Peers sharing an album rarely all leave within a week.
const FOUND_FOR_S: i64 = 7 * 24 * 3600;
/// A fresh release nobody shares yet often appears within a day.
const MISSING_FOR_S: i64 = 24 * 3600;

/// The remembered or searched answer; `None` when Soulseek is offline and nothing is
/// remembered, or the search said nothing (see `Searches::album_shared`).
pub async fn available(svc: &Service, artist: &str, album: &str) -> Result<Option<bool>> {
    let key = (loose_artist(artist), loose_title(album));
    if let Some(known) = remembered(&svc.ctx.db, &key).await? {
        return Ok(Some(known));
    }
    if !svc.online() {
        return Ok(None);
    }
    let _one = svc.album_checks.lock().await;
    // Another caller may have asked about the same album while this one waited.
    if let Some(known) = remembered(&svc.ctx.db, &key).await? {
        return Ok(Some(known));
    }
    if !svc.online() {
        return Ok(None);
    }
    let shared = match svc.searches.album_shared(artist, album).await {
        Ok(shared) => shared,
        Err(e) => {
            tracing::debug!(error = %e, artist, album, "checking whether peers share an album");
            None
        }
    };
    if let Some(shared) = shared {
        remember(&svc.ctx.db, key, shared).await?;
    }
    Ok(shared)
}

/// Whether an answer checked at `checked_at` still holds at `now`.
fn fresh(available: bool, checked_at: i64, now: i64) -> bool {
    let keep = if available { FOUND_FOR_S } else { MISSING_FOR_S };
    now.saturating_sub(checked_at) < keep
}

async fn remembered(db: &Db, key: &(String, String)) -> Result<Option<bool>> {
    let (artist, album) = key.clone();
    let row: Option<(bool, i64)> = db
        .call(move |c| {
            Ok(c.query_row(
                "SELECT available, checked_at FROM soulseek_albums WHERE artist = ?1 AND album = ?2",
                params![artist, album],
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
async fn remember(db: &Db, (artist, album): (String, String), available: bool) -> Result<()> {
    db.call(move |c| {
        let now = lull_core::now();
        c.execute(
            "INSERT INTO soulseek_albums (artist, album, available, checked_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(artist, album) DO UPDATE SET available = excluded.available, checked_at = excluded.checked_at",
            params![artist, album, available, now],
        )?;
        c.execute("DELETE FROM soulseek_albums WHERE checked_at < ?1", [now - FOUND_FOR_S])?;
        Ok(())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 24 * 3600;

    #[test]
    fn found_holds_for_a_week_and_not_found_for_a_day() {
        let now = 100 * DAY;
        assert!(fresh(true, now - 6 * DAY, now));
        assert!(!fresh(true, now - 7 * DAY, now));
        assert!(fresh(false, now - DAY + 60, now));
        assert!(
            !fresh(false, now - DAY, now),
            "a release nobody shared yet is checked again"
        );
    }

    #[tokio::test]
    async fn an_answer_is_remembered_by_loose_names() -> Result<()> {
        let db = Db::memory()?;
        db.migrate(&crate::MIGRATIONS)?;
        let key = |artist: &str, album: &str| (loose_artist(artist), loose_title(album));
        assert_eq!(remembered(&db, &key("Sigur Rós", "Ágætis byrjun")).await?, None);
        remember(&db, key("Sigur Rós", "Ágætis byrjun"), true).await?;
        assert_eq!(
            remembered(&db, &key("Sigur Ros", "Agaetis Byrjun (Remastered)")).await?,
            Some(true)
        );
        Ok(())
    }
}
