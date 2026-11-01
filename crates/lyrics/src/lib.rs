//! Lyrics from LRCLIB (synced, plain or instrumental), fetched and cached server-side.
//! `GET /api/lyrics/{trackId}`: the track is resolved through the catalog, LRCLIB is asked once
//! and the answer (a miss included) is kept, so a song is looked up rarely. When LRCLIB fails or
//! rate-limits, lull backs off for a minute and answers 503 (or the stale copy it has).

mod lrc;
mod lrclib;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use lull_core::db::Migrations;
use lull_core::ids::TrackId;
use lull_core::model::{Lyrics, LyricsKind};
use lull_core::{Ctx, Error, Result, catalog, seams};
use rusqlite::{OptionalExtension, params};

pub const MIGRATIONS: Migrations = Migrations {
    owner: "lyrics",
    steps: &[r#"
        CREATE TABLE lyrics_cache (
            track_id    TEXT PRIMARY KEY,
            json        TEXT NOT NULL,    -- Lyrics as served
            fetched_at  INTEGER NOT NULL  -- unix seconds
        );
    "#],
};

/// After LRCLIB fails or rate-limits, lull leaves it alone this long.
const COOLDOWN: Duration = Duration::from_secs(60);

/// How long an answer stays fresh. Synced and instrumental answers are final; a plain or
/// missing one is asked again later, since LRCLIB grows from its users' submissions.
fn ttl_secs(kind: LyricsKind) -> i64 {
    const DAY: i64 = 86_400;
    match kind {
        LyricsKind::Synced | LyricsKind::Instrumental => 365 * DAY,
        LyricsKind::Plain => 30 * DAY,
        LyricsKind::None => 7 * DAY,
    }
}

pub struct Service {
    ctx: Ctx,
    /// Until when LRCLIB is left alone.
    cooldown: Mutex<Option<Instant>>,
}

pub async fn start(ctx: &Ctx) -> Result<Arc<Service>> {
    Ok(Arc::new(Service {
        ctx: ctx.clone(),
        cooldown: Mutex::new(None),
    }))
}

/// `/api/lyrics/{track_id}`.
pub fn router(svc: Arc<Service>) -> Router {
    Router::new()
        .route("/api/lyrics/{track_id}", get(lyrics))
        .with_state(svc)
}

async fn lyrics(State(svc): State<Arc<Service>>, Path(track_id): Path<String>) -> Result<Json<Lyrics>> {
    svc.lookup(TrackId(track_id)).await.map(Json)
}

/// The same answer as `/api/lyrics`, for Subsonic.
#[async_trait]
impl seams::Lyrics for Service {
    async fn lyrics(&self, track: &TrackId) -> Result<Lyrics> {
        self.lookup(track.clone()).await
    }
}

impl Service {
    async fn lookup(&self, id: TrackId) -> Result<Lyrics> {
        let (track, cached) = self
            .ctx
            .db
            .call({
                let id = id.clone();
                move |conn| Ok((catalog::track(conn, &id)?, cached(conn, &id)?))
            })
            .await?;
        let track = track.ok_or_else(|| Error::NotFound(format!("Track {id} is unknown to lull.")))?;

        if let Some((lyrics, fetched_at)) = &cached
            && lull_core::now() - fetched_at < ttl_secs(lyrics.kind)
        {
            return Ok(lyrics.clone());
        }
        if self.cooling_down() {
            return cached.map(|(l, _)| l).ok_or_else(busy);
        }
        match lrclib::find(&self.ctx.http, &track).await {
            Ok(found) => {
                let lyrics = found.map(lrclib::Record::into_lyrics).unwrap_or_else(none);
                self.store(id, lyrics.clone()).await?;
                Ok(lyrics)
            }
            Err(e) => {
                tracing::warn!(error = %e, track = %id, "LRCLIB lookup failed; backing off");
                self.cool_down();
                cached.map(|(l, _)| l).ok_or_else(busy)
            }
        }
    }

    async fn store(&self, id: TrackId, lyrics: Lyrics) -> Result<()> {
        let json = serde_json::to_string(&lyrics).map_err(|e| Error::internal(format!("lyrics json: {e}")))?;
        self.ctx
            .db
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO lyrics_cache (track_id, json, fetched_at) VALUES (?1, ?2, ?3)
                     ON CONFLICT(track_id) DO UPDATE SET json = excluded.json, fetched_at = excluded.fetched_at",
                    params![id.as_str(), json, lull_core::now()],
                )?;
                Ok(())
            })
            .await
    }

    fn cooling_down(&self) -> bool {
        self.cooldown
            .lock()
            .map(|until| until.is_some_and(|t| Instant::now() < t))
            .unwrap_or(false)
    }

    fn cool_down(&self) {
        if let Ok(mut until) = self.cooldown.lock() {
            *until = Some(Instant::now() + COOLDOWN);
        }
    }
}

fn cached(conn: &rusqlite::Connection, id: &TrackId) -> Result<Option<(Lyrics, i64)>> {
    let row: Option<(String, i64)> = conn
        .query_row(
            "SELECT json, fetched_at FROM lyrics_cache WHERE track_id = ?1",
            [id.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    // A row this build can't read is treated as missing and fetched again.
    Ok(row.and_then(|(json, at)| serde_json::from_str(&json).ok().map(|l| (l, at))))
}

fn none() -> Lyrics {
    Lyrics {
        kind: LyricsKind::None,
        lines: Vec::new(),
        plain: None,
        source: lrclib::SOURCE.to_owned(),
    }
}

fn busy() -> Error {
    Error::unavailable(
        "LRCLIB",
        "it is busy or limiting lull right now; lyrics come back in a minute",
    )
}
