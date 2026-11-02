//! Listening: the UI starts a play when a song starts and ends it once with how much was heard.
//! Each publishes a `play` event (lastfm scrobbles, discovery learns taste); the history reads
//! the plays back grouped by the listener's local day.

use std::sync::Arc;

use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Extension, Path, Query, State};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use lull_core::devices::{self, Device};
use lull_core::ids::TrackId;
use lull_core::model::{LatePlay, Track};
use lull_core::{Error, Result};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::lastfm_send::Stored;
use crate::{Events, Service, bad_json, bad_query, entities, known_track};

pub(crate) fn routes() -> Router<Arc<Service>> {
    Router::new()
        .route("/api/plays", post(start))
        .route("/api/plays/{play_id}", patch(end))
        .route("/api/history", get(history))
}

const MAX_SECONDS: u32 = 24 * 3600;

/// Last.fm's verdict on a play, reported by lastfm's `scrobble` event.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ScrobbleStatus {
    Pending,
    Done,
    /// Not eligible: too short, or skipped before the scrobble point.
    Short,
}

impl ScrobbleStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            ScrobbleStatus::Pending => "pending",
            ScrobbleStatus::Done => "done",
            ScrobbleStatus::Short => "short",
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StartBody {
    track_id: TrackId,
    /// Seconds, as the player measured it; the catalog's duration when absent.
    duration: Option<u32>,
    /// Started from a recommendation (feed pick, radio): counts towards the hit rate on Stats.
    #[serde(default)]
    recommended: bool,
}

/// `{ playId }`, or `{ playId: null, private: true }` from a device in private listening: nothing
/// was recorded or published, so there is no play to end.
async fn start(
    State(svc): State<Arc<Service>>,
    Extension(device): Extension<Device>,
    body: Result<Json<StartBody>, JsonRejection>,
) -> Result<Json<Value>> {
    let Json(body) = body.map_err(bad_json)?;
    let play_id = svc
        .write(move |tx, events| {
            if devices::is_private(tx, &device.id)? {
                return Ok(None);
            }
            begin(
                tx,
                events,
                &body.track_id,
                body.duration,
                body.recommended,
                lull_core::now(),
            )
            .map(Some)
        })
        .await?;
    Ok(Json(match play_id {
        Some(id) => json!({ "playId": id.to_string() }),
        None => json!({ "playId": null, "private": true }),
    }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct EndBody {
    pub(crate) played_s: u32,
    pub(crate) completed: bool,
    pub(crate) skipped: bool,
}

async fn end(
    State(svc): State<Arc<Service>>,
    Extension(device): Extension<Device>,
    Path(play_id): Path<String>,
    body: Result<Json<EndBody>, JsonRejection>,
) -> Result<Json<Value>> {
    let Json(body) = body.map_err(bad_json)?;
    let id: i64 = play_id
        .parse()
        .map_err(|_| Error::NotFound(format!("No play {play_id}.")))?;
    svc.write(move |tx, events| finish(tx, events, &device.id, id, &body))
        .await?;
    Ok(Json(json!({ "playId": play_id })))
}

/// Starts a play at `at` and publishes `play.started`. `duration` defaults to the catalog's.
fn begin(
    conn: &Connection,
    events: &mut Events,
    track_id: &TrackId,
    duration: Option<u32>,
    recommended: bool,
    at: i64,
) -> Result<i64> {
    if duration.is_some_and(|d| d > MAX_SECONDS) {
        return Err(Error::BadRequest("duration is longer than a day.".into()));
    }
    let track = known_track(conn, track_id)?;
    let duration = duration.or(track.duration).unwrap_or(0);
    let play_id = insert(conn, &track, duration, at)?;
    conn.execute(
        "INSERT INTO collection_plays (play_id, recommended) VALUES (?1, ?2)",
        params![play_id, recommended],
    )?;
    events.push(
        "play.started",
        json!({
            "playId": play_id.to_string(), "trackId": track.id, "at": at, "duration": duration,
            "artist": track.artist, "title": track.title, "album": track.album,
        }),
    );
    Ok(play_id)
}

/// Ends a play, once: a second end would scrobble twice, so it is a conflict. A device in
/// private listening ends nothing: a play it started before switching it on stays as it was.
fn finish(conn: &Connection, events: &mut Events, device: &str, id: i64, body: &EndBody) -> Result<()> {
    check_end(body)?;
    if devices::is_private(conn, device)? {
        return Ok(());
    }
    let play = conn
        .query_row(
            "SELECT p.track_id, p.artist, p.title, p.album, p.duration_s, p.started_at, c.ended_at
             FROM plays p JOIN collection_plays c ON c.play_id = p.id WHERE p.id = ?1",
            [id],
            |r| {
                Ok((
                    Ended {
                        play_id: id.to_string(),
                        track_id: TrackId(r.get(0)?),
                        artist: r.get(1)?,
                        title: r.get(2)?,
                        album: r.get(3)?,
                        duration: r.get(4)?,
                        at: r.get(5)?,
                        played_s: body.played_s,
                        completed: body.completed,
                        skipped: body.skipped,
                    },
                    r.get::<_, Option<i64>>(6)?.is_some(),
                ))
            },
        )
        .optional()?;
    let Some((play, ended)) = play else {
        return Err(Error::NotFound(format!("No play {id}.")));
    };
    if ended {
        return Err(Error::Conflict(format!("Play {id} has already ended.")));
    }
    conn.execute(
        "UPDATE plays SET played_s = ?2, completed = ?3, skipped = ?4 WHERE id = ?1",
        params![id, body.played_s, body.completed, body.skipped],
    )?;
    conn.execute(
        "UPDATE collection_plays SET ended_at = ?2 WHERE play_id = ?1",
        params![id, lull_core::now()],
    )?;
    events.push("play.ended", json!(play));
    Ok(())
}

/// Records a play that already happened elsewhere (the offline outbox, a Subsonic app): history,
/// taste and scrobbling see it as the web player's own, timed by its real start. Only
/// `play.ended` goes out: "now playing" for a song that already finished would be wrong.
/// A private device's play is dropped, whatever it was queued as: `None`.
pub(crate) fn record(
    conn: &Connection,
    events: &mut Events,
    device: &str,
    track_id: &TrackId,
    started_at: i64,
    body: &EndBody,
) -> Result<Option<i64>> {
    check_end(body)?;
    if devices::is_private(conn, device)? {
        return Ok(None);
    }
    let now = lull_core::now();
    if started_at <= 0 {
        return Err(Error::BadRequest("startedAt is not a time.".into()));
    }
    // A device clock a little ahead must not put a play in the future.
    let at = started_at.min(now);
    let track = known_track(conn, track_id)?;
    let duration = track.duration.unwrap_or(0);
    let play_id = insert(conn, &track, duration, at)?;
    conn.execute(
        "UPDATE plays SET played_s = ?2, completed = ?3, skipped = ?4 WHERE id = ?1",
        params![play_id, body.played_s, body.completed, body.skipped],
    )?;
    conn.execute(
        "INSERT INTO collection_plays (play_id, ended_at) VALUES (?1, ?2)",
        params![play_id, now],
    )?;
    let ended = Ended {
        play_id: play_id.to_string(),
        track_id: track.id,
        at,
        played_s: body.played_s,
        completed: body.completed,
        skipped: body.skipped,
        duration: i64::from(duration),
        artist: track.artist,
        title: track.title,
        album: track.album,
    };
    events.push("play.ended", json!(ended));
    Ok(Some(play_id))
}

fn insert(conn: &Connection, track: &Track, duration: u32, at: i64) -> Result<i64> {
    conn.execute(
        "INSERT INTO plays (track_id, artist, title, album, duration_s, started_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![track.id.as_str(), track.artist, track.title, track.album, duration, at],
    )?;
    Ok(conn.last_insert_rowid())
}

fn check_end(body: &EndBody) -> Result<()> {
    if body.played_s > MAX_SECONDS {
        return Err(Error::BadRequest("playedS is longer than a day.".into()));
    }
    if body.completed && body.skipped {
        return Err(Error::BadRequest("A play can't be both completed and skipped.".into()));
    }
    Ok(())
}

/// The `play.ended` payload (api.md#events); `at` is the play's start, Last.fm's timestamp.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Ended {
    play_id: String,
    track_id: TrackId,
    at: i64,
    played_s: u32,
    completed: bool,
    skipped: bool,
    duration: i64,
    artist: String,
    title: String,
    album: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HistoryQuery {
    /// Cursor: plays that started before this play (the previous page's `next`). History is in
    /// order of start, not of arrival: synced and imported plays arrive late.
    before: Option<i64>,
    limit: Option<u32>,
    /// The listener's offset from UTC in minutes (east positive), so days split at their midnight.
    #[serde(default)]
    tz: i32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HistoryItem {
    play_id: String,
    track_id: TrackId,
    at: i64,
    played_s: i64,
    completed: bool,
    skipped: bool,
    scrobble: Option<String>,
    /// "Send to Last.fm" would be accepted for this play (`lastfm_send`), given an account that
    /// is connected and scrobbling and a device that isn't private, which only the UI knows.
    sendable: bool,
}

/// `{ days: [{ day: "YYYY-MM-DD", items }], next, entities }`, newest first. A day can continue
/// on the next page; the UI merges groups with the same `day`.
async fn history(
    State(svc): State<Arc<Service>>,
    q: Result<Query<HistoryQuery>, QueryRejection>,
) -> Result<Json<Value>> {
    let Query(q) = q.map_err(bad_query)?;
    let limit = q.limit.unwrap_or(100);
    if limit == 0 || limit > 500 {
        return Err(Error::BadRequest("limit must be between 1 and 500.".into()));
    }
    let tz_offset = tz_seconds(q.tz)?;
    let scrobbles = svc.scrobbles.clone();
    svc.ctx
        .db
        .call(move |conn| {
            let now = lull_core::now();
            let items = conn
                .prepare(
                    "SELECT p.id, p.track_id, p.started_at, p.played_s, p.completed, p.skipped, c.scrobble,
                            p.artist, p.title, p.album, p.duration_s,
                            COALESCE(c.ended_at IS NOT NULL, 0), COALESCE(c.imported, 0)
                     FROM plays p LEFT JOIN collection_plays c ON c.play_id = p.id
                     WHERE ?1 IS NULL OR (p.started_at, p.id) < (SELECT started_at, id FROM plays WHERE id = ?1)
                     ORDER BY p.started_at DESC, p.id DESC LIMIT ?2",
                )?
                .query_map(params![q.before, limit], |r| {
                    let id: i64 = r.get(0)?;
                    let stored = Stored {
                        play: LatePlay {
                            play_id: id,
                            artist: r.get(7)?,
                            title: r.get(8)?,
                            album: r.get(9)?,
                            duration_s: r.get(10)?,
                            played_s: r.get(3)?,
                            started_at: r.get(2)?,
                        },
                        finished: r.get(11)?,
                        imported: r.get(12)?,
                        scrobble: r.get(6)?,
                    };
                    Ok(HistoryItem {
                        play_id: id.to_string(),
                        track_id: TrackId(r.get(1)?),
                        at: stored.play.started_at,
                        played_s: r.get(3)?,
                        completed: r.get(4)?,
                        skipped: r.get(5)?,
                        sendable: stored.check(&*scrobbles, now).is_ok(),
                        scrobble: stored.scrobble,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let next = (items.len() == limit as usize).then(|| items.last().map(|i| i.play_id.clone()));
            let entities = entities(conn, items.iter().map(|i| &i.track_id))?;
            let days = group_by_day(items, tz_offset);
            Ok(Json(
                json!({ "days": days, "next": next.flatten(), "entities": entities }),
            ))
        })
        .await
}

fn group_by_day(items: Vec<HistoryItem>, tz_offset: i64) -> Vec<Value> {
    let mut days: Vec<(String, Vec<HistoryItem>)> = Vec::new();
    for item in items {
        let day = local_day(item.at, tz_offset);
        match days.last_mut() {
            Some((d, list)) if *d == day => list.push(item),
            _ => days.push((day, vec![item])),
        }
    }
    days.into_iter()
        .map(|(day, items)| json!({ "day": day, "items": items }))
        .collect()
}

/// `?tz=` minutes to seconds; real offsets run from UTC-12 to UTC+14.
pub(crate) fn tz_seconds(minutes: i32) -> Result<i64> {
    if !(-12 * 60..=14 * 60).contains(&minutes) {
        return Err(Error::BadRequest(format!(
            "tz {minutes} is not a UTC offset in minutes."
        )));
    }
    Ok(i64::from(minutes) * 60)
}

/// "YYYY-MM-DD" of a unix time at a UTC offset (Hinnant's civil-from-days).
fn local_day(at: i64, tz_offset: i64) -> String {
    let days = (at + tz_offset).div_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_split_at_the_listeners_midnight() {
        // 2026-10-30 23:30 UTC is already the 31st in Berlin (UTC+1) and still the 30th in New York.
        let at = 1_793_403_000;
        assert_eq!(local_day(at, 0), "2026-10-30");
        assert_eq!(local_day(at, 3600), "2026-10-31");
        assert_eq!(local_day(at, -4 * 3600), "2026-10-30");
        assert_eq!(local_day(951_782_400, 0), "2000-02-29");
    }
}
