//! Listening stats for a period, computed from plays: `MOCK.stats.byPeriod[p]`. The taste summary
//! on the same screen belongs to discovery's taste model.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use lull_core::ids::{ArtistId, TrackId};
use lull_core::model::Entities;
use lull_core::{Error, Result, catalog};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::follows::named_artist;
use crate::plays::tz_seconds;
use crate::{Service, bad_query};

pub(crate) fn routes() -> Router<Arc<Service>> {
    Router::new().route("/api/stats", get(stats))
}

const TOP: usize = 5;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StatsQuery {
    #[serde(default)]
    period: Period,
    /// Minutes east of UTC, for the hour-of-day clock.
    #[serde(default)]
    tz: i32,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
enum Period {
    #[default]
    #[serde(rename = "7d")]
    Week,
    #[serde(rename = "30d")]
    Month,
    #[serde(rename = "12m")]
    Year,
    #[serde(rename = "all")]
    All,
}

impl Period {
    /// First second inside the period.
    fn since(self, now: i64) -> Option<i64> {
        let days = match self {
            Period::Week => 7,
            Period::Month => 30,
            Period::Year => 365,
            Period::All => return None,
        };
        Some(now - days * 86_400)
    }
}

async fn stats(State(svc): State<Arc<Service>>, q: Result<Query<StatsQuery>, QueryRejection>) -> Result<Json<Value>> {
    let Query(q) = q.map_err(bad_query)?;
    let tz_offset = tz_seconds(q.tz)?;
    let since = q.period.since(lull_core::now());
    svc.ctx
        .db
        .call(move |conn| {
            let inputs = Inputs::load(conn, since, tz_offset)?;
            let stats = aggregate(&inputs);
            let mut entities = Entities::default();
            for (id, _) in &stats.top_tracks {
                if let Some(t) = catalog::track(conn, id)? {
                    entities.tracks.insert(id.clone(), t);
                }
            }
            for (id, _) in &stats.top_artists {
                if let Some(name) = inputs.artist_names.get(id) {
                    entities
                        .artists
                        .insert(id.clone(), named_artist(id.clone(), name.clone()));
                }
            }
            let mut body = serde_json::to_value(&stats).map_err(|e| Error::internal(format!("stats json: {e}")))?;
            body["entities"] = json!(entities);
            Ok(Json(body))
        })
        .await
}

#[derive(Debug, Clone)]
struct PlayRow {
    track_id: TrackId,
    artist_id: ArtistId,
    at: i64,
    played_s: i64,
    skipped: bool,
    recommended: bool,
}

/// Everything `aggregate` needs, read in one go.
#[derive(Debug, Default)]
struct Inputs {
    /// Plays inside the period.
    plays: Vec<PlayRow>,
    /// Each artist's first play ever (artists are told apart by id, so "The Cure" = "Cure").
    first_play: HashMap<ArtistId, i64>,
    artist_names: HashMap<ArtistId, String>,
    tags: HashMap<TrackId, Vec<String>>,
    liked: HashSet<TrackId>,
    /// Songs ever saved through lull.
    saved: HashSet<TrackId>,
    /// Saves inside the period.
    saves: u64,
    since: Option<i64>,
    tz_offset: i64,
}

impl Inputs {
    fn load(conn: &Connection, since: Option<i64>, tz_offset: i64) -> Result<Self> {
        let from = since.unwrap_or(i64::MIN);
        let plays = conn
            .prepare(
                "SELECT p.track_id, p.artist, p.started_at, p.played_s, p.skipped, COALESCE(c.recommended, 0)
                 FROM plays p LEFT JOIN collection_plays c ON c.play_id = p.id WHERE p.started_at >= ?1",
            )?
            .query_map([from], |r| {
                Ok((
                    TrackId(r.get(0)?),
                    r.get::<_, String>(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut inputs = Inputs {
            since,
            tz_offset,
            ..Default::default()
        };
        for (track_id, artist, at, played_s, skipped, recommended) in plays {
            let artist_id = ArtistId::of(&artist);
            inputs.artist_names.insert(artist_id.clone(), artist);
            inputs.plays.push(PlayRow {
                track_id,
                artist_id,
                at,
                played_s,
                skipped,
                recommended,
            });
        }

        let mut first = conn.prepare("SELECT artist, MIN(started_at) FROM plays GROUP BY artist")?;
        for row in first.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
            let (artist, at) = row?;
            let e = inputs.first_play.entry(ArtistId::of(&artist)).or_insert(at);
            *e = (*e).min(at);
        }

        for id in inputs.plays.iter().map(|p| &p.track_id).collect::<HashSet<_>>() {
            if let Some(t) = catalog::track(conn, id)? {
                inputs.tags.insert(id.clone(), t.tags);
            }
        }
        inputs.liked = ids(conn, "SELECT track_id FROM likes", [])?;
        inputs.saved = ids(conn, "SELECT track_id FROM collection_saves", [])?;
        let saves: i64 = conn.query_row(
            "SELECT COUNT(*) FROM collection_saves WHERE saved_at >= ?1",
            params![from],
            |r| r.get(0),
        )?;
        inputs.saves = saves.unsigned_abs();
        Ok(inputs)
    }
}

fn ids(conn: &Connection, sql: &str, p: impl rusqlite::Params) -> Result<HashSet<TrackId>> {
    Ok(conn
        .prepare(sql)?
        .query_map(p, |r| r.get(0).map(TrackId))?
        .collect::<rusqlite::Result<_>>()?)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Stats {
    minutes: i64,
    plays: u64,
    artists: u64,
    saves: u64,
    skip_rate: f64,
    new_artists: u64,
    hit_rate: HitRate,
    top_artists: Vec<(ArtistId, u64)>,
    top_tracks: Vec<(TrackId, u64)>,
    /// Share of tag mentions across the period's plays, in percent.
    top_tags: Vec<(String, u64)>,
    /// Plays per hour of the listener's day.
    clock: [u64; 24],
}

/// Of the songs started from a recommendation: how many the listener went on to like or save.
#[derive(Debug, Default, Serialize)]
struct HitRate {
    recommended: u64,
    liked: u64,
    saved: u64,
}

fn aggregate(i: &Inputs) -> Stats {
    let plays = i.plays.len() as u64;
    let mut by_artist: HashMap<&ArtistId, u64> = HashMap::new();
    let mut by_track: HashMap<&TrackId, u64> = HashMap::new();
    let mut by_tag: HashMap<&str, u64> = HashMap::new();
    let mut recommended: HashSet<&TrackId> = HashSet::new();
    let mut clock = [0u64; 24];
    for p in &i.plays {
        *by_artist.entry(&p.artist_id).or_default() += 1;
        *by_track.entry(&p.track_id).or_default() += 1;
        for tag in i.tags.get(&p.track_id).into_iter().flatten() {
            *by_tag.entry(tag).or_default() += 1;
        }
        if p.recommended {
            recommended.insert(&p.track_id);
        }
        clock[((p.at + i.tz_offset).rem_euclid(86_400) / 3600) as usize] += 1;
    }
    let tag_mentions: u64 = by_tag.values().sum();
    let skipped = i.plays.iter().filter(|p| p.skipped).count() as u64;
    let is_new = |a: &ArtistId| match i.since {
        Some(since) => i.first_play.get(a).is_some_and(|first| *first >= since),
        None => true,
    };
    Stats {
        minutes: i.plays.iter().map(|p| p.played_s).sum::<i64>() / 60,
        plays,
        artists: by_artist.len() as u64,
        saves: i.saves,
        skip_rate: if plays == 0 { 0.0 } else { skipped as f64 / plays as f64 },
        new_artists: by_artist.keys().filter(|a| is_new(a)).count() as u64,
        hit_rate: HitRate {
            recommended: recommended.len() as u64,
            liked: recommended.iter().filter(|t| i.liked.contains(**t)).count() as u64,
            saved: recommended.iter().filter(|t| i.saved.contains(**t)).count() as u64,
        },
        top_artists: top(by_artist).into_iter().map(|(a, n)| (a.clone(), n)).collect(),
        top_tracks: top(by_track).into_iter().map(|(t, n)| (t.clone(), n)).collect(),
        top_tags: top(by_tag)
            .into_iter()
            .map(|(tag, n)| (tag.to_owned(), (n * 100 + tag_mentions / 2) / tag_mentions))
            .collect(),
        clock,
    }
}

/// The `TOP` biggest counts, ties broken by key so the order is stable.
fn top<K: Ord>(counts: HashMap<K, u64>) -> Vec<(K, u64)> {
    let mut v: Vec<(K, u64)> = counts.into_iter().collect::<BTreeMap<_, _>>().into_iter().collect();
    v.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    v.truncate(TOP);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn play(artist: &str, track: &str, at: i64) -> PlayRow {
        PlayRow {
            track_id: TrackId(track.into()),
            artist_id: ArtistId::of(artist),
            at,
            played_s: 180,
            skipped: false,
            recommended: false,
        }
    }

    #[test]
    fn an_empty_period_reports_zeros_not_nan() -> serde_json::Result<()> {
        let json = serde_json::to_value(aggregate(&Inputs::default()))?;
        assert_eq!(json["skipRate"], 0.0);
        assert_eq!(json["topTags"], json!([]));
        assert_eq!(json["clock"].as_array().map(Vec::len), Some(24));
        Ok(())
    }

    #[test]
    fn new_artists_are_only_those_first_heard_in_the_period() {
        let since = 1_000_000;
        let inputs = Inputs {
            plays: vec![
                play("The Cure", "t-1", since + 10),
                play("Cure", "t-2", since + 20),
                play("Moonpail", "t-3", since + 30),
            ],
            // The Cure was heard before the period; "Cure" is the same artist.
            first_play: HashMap::from([
                (ArtistId::of("cure"), since - 5),
                (ArtistId::of("Moonpail"), since + 30),
            ]),
            since: Some(since),
            ..Default::default()
        };
        let s = aggregate(&inputs);
        assert_eq!((s.artists, s.new_artists), (2, 1));
        assert_eq!(s.top_artists[0], (ArtistId::of("The Cure"), 2));
    }

    #[test]
    fn clock_and_tags_follow_the_listener() {
        // 23:30 UTC: hour 0 in Berlin (UTC+1), hour 18 in New York (UTC-5).
        let at = 1_793_403_000;
        let inputs = |tz_offset| Inputs {
            plays: vec![play("A", "t-1", at), play("A", "t-1", at), play("B", "t-2", at)],
            tags: HashMap::from([
                (TrackId("t-1".into()), vec!["slowcore".into(), "dream-gaze".into()]),
                (TrackId("t-2".into()), vec!["slowcore".into()]),
            ]),
            tz_offset,
            ..Default::default()
        };
        assert_eq!(aggregate(&inputs(3600)).clock[0], 3);
        let s = aggregate(&inputs(-5 * 3600));
        assert_eq!(s.clock[18], 3);
        assert_eq!(
            s.top_tags,
            vec![("slowcore".to_owned(), 60), ("dream-gaze".to_owned(), 40)]
        );
    }
}
