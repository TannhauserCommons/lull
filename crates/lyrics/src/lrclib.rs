//! The LRCLIB API (https://lrclib.net/docs): an exact lookup by signature first, then a search
//! when the album or duration is unknown or the exact lookup misses.

use std::time::Duration;

use lull_core::http::{self, Http};
use lull_core::model::{Lyrics, LyricsKind, Track};
use lull_core::{Error, Result};
use reqwest::StatusCode;
use serde::Deserialize;

use crate::lrc;

pub const SOURCE: &str = "LRCLIB";
const BASE: &str = "https://lrclib.net/api";
/// A search hit further than this from the track's length is another recording.
const MAX_DURATION_GAP_S: f64 = 5.0;
/// LRCLIB asks for sequential calls a few hundred milliseconds apart.
const PACE: Duration = Duration::from_millis(300);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    #[serde(default)]
    instrumental: bool,
    plain_lyrics: Option<String>,
    synced_lyrics: Option<String>,
    duration: Option<f64>,
}

impl Record {
    pub fn into_lyrics(self) -> Lyrics {
        let lines = self.synced_lyrics.as_deref().map(lrc::parse).unwrap_or_default();
        let plain = self.plain_lyrics.filter(|p| !p.trim().is_empty());
        let kind = if self.instrumental {
            LyricsKind::Instrumental
        } else if !lines.is_empty() {
            LyricsKind::Synced
        } else if plain.is_some() {
            LyricsKind::Plain
        } else {
            LyricsKind::None
        };
        Lyrics {
            kind,
            lines,
            plain,
            source: SOURCE.to_owned(),
        }
    }
}

/// The best LRCLIB record for `track`, or `None` when LRCLIB has none.
pub async fn find(http: &Http, track: &Track) -> Result<Option<Record>> {
    if let Some(duration) = track.duration
        && !track.album.is_empty()
        && let Some(hit) = get(http, track, duration).await?
    {
        return Ok(Some(hit));
    }
    search(http, track).await
}

async fn get(http: &Http, track: &Track, duration: u32) -> Result<Option<Record>> {
    let duration = duration.to_string();
    http.pace(SOURCE, PACE).await;
    let resp = http
        .send(SOURCE, |c| {
            c.get(format!("{BASE}/get")).query(&[
                ("artist_name", track.artist.as_str()),
                ("track_name", track.title.as_str()),
                ("album_name", track.album.as_str()),
                ("duration", duration.as_str()),
            ])
        })
        .await?;
    if resp.status() == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    decode(http::ok(SOURCE, resp)?).await.map(Some)
}

async fn search(http: &Http, track: &Track) -> Result<Option<Record>> {
    http.pace(SOURCE, PACE).await;
    let resp = http
        .send(SOURCE, |c| {
            c.get(format!("{BASE}/search")).query(&[
                ("artist_name", track.artist.as_str()),
                ("track_name", track.title.as_str()),
            ])
        })
        .await?;
    let hits: Vec<Record> = decode(http::ok(SOURCE, resp)?).await?;
    Ok(best(hits, track.duration))
}

/// Closest in length, preferring synced lyrics; without a known length, the first synced hit.
fn best(hits: Vec<Record>, duration: Option<u32>) -> Option<Record> {
    let gap = |r: &Record| match (r.duration, duration) {
        (Some(a), Some(b)) => (a - f64::from(b)).abs(),
        _ => 0.0,
    };
    hits.into_iter()
        .filter(|r| gap(r) <= MAX_DURATION_GAP_S)
        .min_by(|a, b| {
            let rank = |r: &Record| (r.synced_lyrics.is_none(), gap(r));
            rank(a).partial_cmp(&rank(b)).unwrap_or(std::cmp::Ordering::Equal)
        })
}

async fn decode<T: serde::de::DeserializeOwned>(resp: reqwest::Response) -> Result<T> {
    resp.json()
        .await
        .map_err(|e| Error::upstream(SOURCE, format!("bad JSON: {e}")))
}
