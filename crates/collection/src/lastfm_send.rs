//! History's "Send to Last.fm": plays that ended while Last.fm wasn't listening get one more
//! chance. The hub decides, not the button: only plays lull recorded itself, that Last.fm's rule
//! and age limit accept, that were never queued or sent, from a device that isn't listening
//! privately, in small batches at a limited rate. The play's own start is its timestamp.
//! Last.fm's side (account, rule, queue) is the `Scrobbles` seam.

use std::collections::VecDeque;
use std::sync::Arc;

use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, State};
use axum::routing::post;
use axum::{Json, Router};
use lull_core::devices::Device;
use lull_core::model::LatePlay;
use lull_core::seams::Scrobbles;
use lull_core::{Error, Result};
use rusqlite::{Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{Service, bad_json};

/// Plays in one request.
const MAX_PER_REQUEST: usize = 25;
/// Plays the hub sends in any one hour: a backlog of a few dozen clears, a script can't flood Last.fm.
const MAX_PER_HOUR: usize = 100;
const HOUR_S: i64 = 3600;

pub(crate) fn routes() -> Router<Arc<Service>> {
    Router::new().route("/api/history/lastfm", post(send))
}

/// When plays were sent in the last hour, oldest first.
#[derive(Default)]
pub(crate) struct Sent(VecDeque<i64>);

impl Sent {
    fn recent(&mut self, now: i64) -> usize {
        while self.0.front().is_some_and(|at| now - at >= HOUR_S) {
            self.0.pop_front();
        }
        self.0.len()
    }

    /// Minutes until enough has aged out for `wanted` more plays to fit.
    fn wait_minutes(&self, now: i64, wanted: usize) -> i64 {
        let must_leave = (self.0.len() + wanted).saturating_sub(MAX_PER_HOUR);
        let at = self.0.get(must_leave.saturating_sub(1)).copied().unwrap_or(now);
        ((at + HOUR_S - now).max(0) + 59) / 60
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Body {
    play_ids: Vec<String>,
}

/// `POST /api/history/lastfm { playIds }` -> `{ queued: [playId] }`, all or none.
async fn send(
    State(svc): State<Arc<Service>>,
    Extension(device): Extension<Device>,
    body: Result<Json<Body>, JsonRejection>,
) -> Result<Json<Value>> {
    let Json(body) = body.map_err(bad_json)?;
    if device.private {
        return Err(Error::Conflict(
            "Private listening is on for this device; switch it off to send plays to Last.fm.".into(),
        ));
    }
    let ids = parse_ids(&body.play_ids)?;
    let now = lull_core::now();
    svc.check_rate(now, ids.len())?;

    let scrobbles = svc.scrobbles.clone();
    let lookup = ids.clone();
    let plays = svc
        .ctx
        .db
        .call(move |conn| lookup.iter().map(|id| offered(conn, &*scrobbles, *id, now)).collect())
        .await?;
    svc.scrobbles.submit(plays).await?;
    svc.count_sent(now, ids.len());
    Ok(Json(json!({ "queued": body.play_ids })))
}

fn parse_ids(raw: &[String]) -> Result<Vec<i64>> {
    if raw.is_empty() || raw.len() > MAX_PER_REQUEST {
        return Err(Error::BadRequest(format!(
            "Send between 1 and {MAX_PER_REQUEST} plays at a time."
        )));
    }
    let mut ids = Vec::with_capacity(raw.len());
    for id in raw {
        let id: i64 = id
            .parse()
            .map_err(|_| Error::BadRequest(format!("{id:?} is not a play id.")))?;
        if ids.contains(&id) {
            return Err(Error::BadRequest(format!("Play {id} is listed twice.")));
        }
        ids.push(id);
    }
    Ok(ids)
}

/// A play as lull recorded it, with what decides whether it may go to Last.fm.
pub(crate) struct Stored {
    pub(crate) play: LatePlay,
    /// `play.ended` came in: lull knows how much was heard.
    pub(crate) finished: bool,
    pub(crate) imported: bool,
    /// Last.fm's verdict so far; `None` = never reported.
    pub(crate) scrobble: Option<String>,
}

impl Stored {
    /// Whether the play may be sent, or why not, as the answer to the listener. Says nothing
    /// about the account or the device, which `send` and Last.fm check.
    pub(crate) fn check(&self, scrobbles: &dyn Scrobbles, now: i64) -> Result<()> {
        let name = format!("\u{201c}{}\u{201d}", self.play.title);
        if self.imported {
            return Err(Error::BadRequest(format!(
                "{name} was imported, not played in lull; imported plays are never sent to Last.fm."
            )));
        }
        if !self.finished {
            return Err(Error::Conflict(format!(
                "{name} never finished, so lull can't say how much was heard."
            )));
        }
        match self.scrobble.as_deref() {
            None => {}
            Some("pending") => return Err(Error::Conflict(format!("{name} is already waiting to be sent."))),
            Some("done") => return Err(Error::Conflict(format!("{name} is already on Last.fm."))),
            Some(_) => return Err(Error::Conflict(format!("Last.fm already turned {name} down."))),
        }
        match scrobbles.refusal(&self.play, now) {
            None => Ok(()),
            Some(why) => Err(Error::BadRequest(format!("{name}: {why}"))),
        }
    }
}

fn offered(conn: &Connection, scrobbles: &dyn Scrobbles, id: i64, now: i64) -> Result<LatePlay> {
    let stored = conn
        .query_row(
            "SELECT p.artist, p.title, p.album, p.duration_s, p.played_s, p.started_at,
                    COALESCE(c.ended_at IS NOT NULL, 0), COALESCE(c.imported, 0), c.scrobble
             FROM plays p LEFT JOIN collection_plays c ON c.play_id = p.id WHERE p.id = ?1",
            [id],
            |r| {
                Ok(Stored {
                    play: LatePlay {
                        play_id: id,
                        artist: r.get(0)?,
                        title: r.get(1)?,
                        album: r.get(2)?,
                        duration_s: r.get(3)?,
                        played_s: r.get(4)?,
                        started_at: r.get(5)?,
                    },
                    finished: r.get(6)?,
                    imported: r.get(7)?,
                    scrobble: r.get(8)?,
                })
            },
        )
        .optional()?
        .ok_or_else(|| Error::NotFound(format!("No play {id}.")))?;
    stored.check(scrobbles, now)?;
    Ok(stored.play)
}

impl Service {
    fn check_rate(&self, now: i64, wanted: usize) -> Result<()> {
        let mut sent = self.sent.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if sent.recent(now) + wanted > MAX_PER_HOUR {
            return Err(Error::TooManyRequests(format!(
                "lull sends at most {MAX_PER_HOUR} plays to Last.fm an hour. Try again in {} minutes.",
                sent.wait_minutes(now, wanted)
            )));
        }
        Ok(())
    }

    fn count_sent(&self, now: i64, n: usize) {
        let mut sent = self.sent.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        sent.0.extend(std::iter::repeat_n(now, n));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use lull_core::devices::Device;
    use lull_core::model::PlayReport;
    use lull_core::seams::Collection;

    use super::*;
    use crate::testing;

    /// A finished play of a 200 s song that started `ago` seconds ago; its id.
    async fn played(svc: &Service, device: &Device, ago: i64, played_s: u32) -> Result<String> {
        let track = testing::known_song(svc, "Grouper", "Kelso").await?;
        svc.record_play(PlayReport {
            device: device.id.clone(),
            track_id: track,
            started_at: lull_core::now() - ago,
            played_s,
            completed: false,
        })
        .await?;
        let id = svc
            .ctx
            .db
            .call(|c| Ok(c.query_row("SELECT MAX(id) FROM plays", [], |r| r.get::<_, i64>(0))?))
            .await?;
        Ok(id.to_string())
    }

    async fn ask(svc: &Arc<Service>, device: &Device, ids: &[&str]) -> Result<Value> {
        let body = Body {
            play_ids: ids.iter().map(|id| (*id).to_owned()).collect(),
        };
        send(State(svc.clone()), Extension(device.clone()), Ok(Json(body)))
            .await
            .map(|Json(v)| v)
    }

    async fn sendable_in_history(svc: &Service, id: &str) -> Result<bool> {
        let (scrobbles, id) = (svc.scrobbles.clone(), id.parse::<i64>().unwrap_or_default());
        svc.ctx
            .db
            .call(move |c| offered(c, &*scrobbles, id, lull_core::now()).map(|_| true))
            .await
            .or_else(|e| match e {
                Error::BadRequest(_) | Error::Conflict(_) => Ok(false),
                e => Err(e),
            })
    }

    fn refused(r: Result<Value>) -> &'static str {
        match r {
            Err(Error::BadRequest(_)) => "400",
            Err(Error::NotFound(_)) => "404",
            Err(Error::Conflict(_)) => "409",
            Err(Error::TooManyRequests(_)) => "429",
            Err(_) => "other",
            Ok(_) => "sent",
        }
    }

    #[tokio::test]
    async fn a_play_lull_recorded_is_queued_once_and_keeps_its_own_start() -> Result<()> {
        let (svc, _rx, lastfm) = testing::service_with_lastfm().await?;
        let me = testing::device(&svc, false).await?;
        let id = played(&svc, &me, 3600, 150).await?;

        assert!(sendable_in_history(&svc, &id).await?);
        assert_eq!(ask(&svc, &me, &[&id]).await?, json!({ "queued": [id.clone()] }));
        let queued = lastfm.queued.lock().map(|q| q.clone()).unwrap_or_default();
        let started = lull_core::now() - 3600;
        assert_eq!(queued.len(), 1);
        assert!(
            (queued[0].started_at - started).abs() <= 5,
            "the timestamp is the play's own start, which the request can't set"
        );

        // Last.fm reports it queued; asking again is refused, here and by History.
        svc.ctx
            .db
            .call(|c| Ok(c.execute("UPDATE collection_plays SET scrobble = 'pending'", [])?))
            .await?;
        assert!(!sendable_in_history(&svc, &id).await?);
        assert_eq!(refused(ask(&svc, &me, &[&id]).await), "409");
        Ok(())
    }

    #[tokio::test]
    async fn only_plays_that_pass_every_rule_are_offered() -> Result<()> {
        let (svc, _rx, lastfm) = testing::service_with_lastfm().await?;
        let me = testing::device(&svc, false).await?;
        let good = played(&svc, &me, 3600, 100).await?;
        let imported = played(&svc, &me, 3600, 150).await?;
        svc.ctx
            .db
            .call({
                let id = imported.clone();
                move |c| Ok(c.execute("UPDATE collection_plays SET imported = 1 WHERE play_id = ?1", [id])?)
            })
            .await?;
        let too_old = played(&svc, &me, 15 * 86_400, 150).await?;
        let skipped_early = played(&svc, &me, 3600, 20).await?;
        let sent = played(&svc, &me, 3600, 150).await?;
        let rejected = played(&svc, &me, 3600, 150).await?;
        svc.ctx
            .db
            .call(|c| {
                c.execute(
                    "UPDATE collection_plays SET scrobble = 'done' WHERE play_id = (SELECT MAX(id) - 1 FROM plays)",
                    [],
                )?;
                c.execute(
                    "UPDATE collection_plays SET scrobble = 'short' WHERE play_id = (SELECT MAX(id) FROM plays)",
                    [],
                )?;
                Ok(())
            })
            .await?;

        for (what, id, want) in [
            ("imported", &imported, "400"),
            ("older than 14 days", &too_old, "400"),
            ("under half", &skipped_early, "400"),
            ("already sent", &sent, "409"),
            ("already turned down", &rejected, "409"),
            ("unknown", &"999999".to_owned(), "404"),
            ("not an id", &"abc".to_owned(), "400"),
        ] {
            assert_eq!(refused(ask(&svc, &me, &[id]).await), want, "{what}");
            assert!(!sendable_in_history(&svc, id).await.unwrap_or(false), "{what}");
        }
        // All or none: one bad play keeps the good one out too.
        assert_eq!(refused(ask(&svc, &me, &[&good, &imported]).await), "400");
        assert!(lastfm.queued.lock().map(|q| q.is_empty()).unwrap_or(false));
        assert_eq!(refused(ask(&svc, &me, &[&good]).await), "sent");
        assert_eq!(
            refused(ask(&svc, &me, &[&good, &good]).await),
            "400",
            "a play listed twice"
        );
        assert_eq!(refused(ask(&svc, &me, &[]).await), "400");
        Ok(())
    }

    #[tokio::test]
    async fn a_private_device_or_a_disconnected_last_fm_sends_nothing() -> Result<()> {
        let (svc, _rx, lastfm) = testing::service_with_lastfm().await?;
        let me = testing::device(&svc, false).await?;
        let private = testing::device(&svc, true).await?;
        let id = played(&svc, &me, 3600, 150).await?;

        assert_eq!(refused(ask(&svc, &private, &[&id]).await), "409");
        lastfm.disconnected.store(true, Ordering::Relaxed);
        assert_eq!(refused(ask(&svc, &me, &[&id]).await), "409");
        assert!(lastfm.queued.lock().map(|q| q.is_empty()).unwrap_or(false));
        Ok(())
    }

    #[tokio::test]
    async fn a_request_holds_few_plays_and_the_hub_sends_a_hundred_an_hour() -> Result<()> {
        let (svc, _rx, lastfm) = testing::service_with_lastfm().await?;
        let me = testing::device(&svc, false).await?;
        let mut ids = Vec::new();
        for _ in 0..=MAX_PER_HOUR {
            ids.push(played(&svc, &me, 3600, 150).await?);
        }
        let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        assert_eq!(
            refused(ask(&svc, &me, &refs[..=MAX_PER_REQUEST]).await),
            "400",
            "over the batch cap"
        );
        for chunk in refs[..MAX_PER_HOUR].chunks(MAX_PER_REQUEST) {
            assert_eq!(refused(ask(&svc, &me, chunk).await), "sent");
        }
        assert_eq!(refused(ask(&svc, &me, &refs[MAX_PER_HOUR..]).await), "429");
        assert_eq!(lastfm.queued.lock().map(|q| q.len()).unwrap_or(0), MAX_PER_HOUR);
        Ok(())
    }

    #[test]
    fn the_hour_window_frees_room_as_old_sends_age_out() {
        let mut sent = Sent::default();
        sent.0.extend([1_000, 1_000, 1_100]);
        assert_eq!(sent.recent(1_200), 3);
        assert_eq!(sent.recent(1_000 + HOUR_S), 1, "two aged out");
        assert_eq!(
            sent.wait_minutes(1_000 + HOUR_S, MAX_PER_HOUR),
            2,
            "100 more fit after the last one ages out"
        );
    }
}
