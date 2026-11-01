//! Deezer (https://developers.deezer.com/api): catalog search with covers, artist photos and
//! popularity, no key needed. The limit is 50 requests per 5 s per IP; going over is answered
//! with HTTP 200 and `error.code` 4, which closes lull's window for 5 s and is tried once more.
//! Where Deezer isn't open (in some countries) track and album searches come back empty with a
//! `total` above zero: that is reported, not mistaken for "nothing found".

use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use lull_core::http::Http;
use lull_core::ids::normalize_artist;
use lull_core::{Error, Result};
use serde::Deserialize;
use serde_json::Value;
use tokio::time::Instant;

use super::cache::{Cache, DAY};
use crate::search::merge::{AlbumHit, ArtistHit, Found, SongHit, popularity};

const SERVICE: &str = "Deezer";
const API: &str = "https://api.deezer.com";
const CDN: &str = "https://cdn-images.dzcdn.net/images";
const WINDOW: Duration = Duration::from_secs(5);
const PER_WINDOW: usize = 50;
/// `error.code` for "Quota limit exceeded".
const QUOTA: u64 = 4;
/// `error.code` for "no data": an empty answer, not a failure.
const NO_DATA: u64 = 800;
/// Search answers are kept this long (seconds), and served stale when Deezer fails.
const KEEP_S: i64 = 600;
/// Rank, fans: the counts that score 1 on `popularity`'s scale.
const FULL_RANK: f64 = 1e6;
const FULL_FANS: f64 = 1e7;

pub struct Deezer {
    http: Http,
    cache: Cache,
    window: Mutex<Window>,
}

impl Deezer {
    pub fn new(http: Http, cache: Cache) -> Self {
        Deezer {
            http,
            cache,
            window: Mutex::default(),
        }
    }

    /// Tracks, albums and artists for `q`. A part that fails leaves the others, and is named in
    /// `problem`; all three failing is an error.
    pub async fn search(&self, q: &str) -> Result<Found> {
        self.cache
            .get(format!("dz:search:{q}"), KEEP_S, || async {
                let (tracks, albums, artists) = tokio::join!(
                    self.get("search/track", q, "25"),
                    self.get("search/album", q, "15"),
                    self.get("search/artist", q, "10"),
                );
                let tracks = tracks.and_then(songs);
                let albums = albums.and_then(albums_of);
                let artists = artists.and_then(artists_of);
                let problems: Vec<String> = [&tracks.as_ref().err(), &albums.as_ref().err(), &artists.as_ref().err()]
                    .into_iter()
                    .flatten()
                    .map(|e| e.to_string())
                    .collect();
                if problems.len() == 3 {
                    return Err(Error::upstream(SERVICE, problems.join("; ")));
                }
                Ok(Found {
                    songs: tracks.unwrap_or_default(),
                    albums: albums.unwrap_or_default(),
                    artists: artists.unwrap_or_default(),
                    problem: (!problems.is_empty()).then(|| problems.join("; ")),
                })
            })
            .await
    }

    /// Artists only: where Deezer's catalog is closed, its artist search still answers.
    pub async fn artists(&self, q: &str) -> Result<Found> {
        self.cache
            .get(format!("dz:artists:{q}"), KEEP_S, || async {
                Ok(Found {
                    artists: artists_of(self.get("search/artist", q, "10").await?)?,
                    ..Found::default()
                })
            })
            .await
    }

    /// Whether Deezer's catalog is open where lull runs (`/infos`).
    pub async fn is_open(&self) -> Result<bool> {
        self.wait_turn().await;
        open(&self.http.json(SERVICE, |c| c.get(format!("{API}/infos"))).await?)
    }

    /// The photo of the artist called `name` (exact name, ignoring case and a leading "the").
    pub async fn artist_image(&self, name: &str) -> Result<Option<String>> {
        self.cache
            .get(
                format!("dz:artist-image:{}", normalize_artist(name)),
                7 * DAY,
                || async {
                    let found = artists_of(self.get("search/artist", name, "5").await?)?;
                    Ok(found
                        .into_iter()
                        .find(|a| normalize_artist(&a.name) == normalize_artist(name))
                        .and_then(|a| a.image))
                },
            )
            .await
    }

    async fn wait_turn(&self) {
        let turn = self
            .window
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .reserve(Instant::now());
        tokio::time::sleep_until(turn).await;
    }

    async fn get(&self, path: &str, q: &str, limit: &str) -> Result<Value> {
        for attempt in 0..2 {
            self.wait_turn().await;
            let body: Value = self
                .http
                .json(SERVICE, |c| {
                    c.get(format!("{API}/{path}")).query(&[("q", q), ("limit", limit)])
                })
                .await?;
            match answer(body) {
                Err(Refusal::Quota) if attempt == 0 => {
                    tracing::info!("Deezer quota reached; waiting for the next window");
                    self.window
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .close(Instant::now());
                }
                Err(Refusal::Quota) => break,
                Err(Refusal::Other(message)) => return Err(Error::upstream(SERVICE, message)),
                Ok(body) => return Ok(body),
            }
        }
        Err(Error::upstream(
            SERVICE,
            "is limiting lull's requests; try again in a few seconds",
        ))
    }
}

/// When requests may go: at most `PER_WINDOW` in any `WINDOW`, and none for a window after
/// Deezer said the quota was reached.
#[derive(Debug, Default)]
pub struct Window {
    /// Send times handed out, oldest first.
    sent: VecDeque<Instant>,
    closed_until: Option<Instant>,
}

impl Window {
    /// The time the next request may be sent, booked.
    pub fn reserve(&mut self, now: Instant) -> Instant {
        while self.sent.front().is_some_and(|t| *t + WINDOW <= now) {
            self.sent.pop_front();
        }
        let mut at = self.closed_until.filter(|u| *u > now).unwrap_or(now);
        if self.sent.len() >= PER_WINDOW {
            at = at.max(self.sent[self.sent.len() - PER_WINDOW] + WINDOW);
        }
        let i = self.sent.partition_point(|t| *t <= at);
        self.sent.insert(i, at);
        at
    }

    pub fn close(&mut self, now: Instant) {
        self.closed_until = Some(now + WINDOW);
    }
}

/// `/infos`'s `open`: whether Deezer's catalog is available in the caller's country.
pub fn open(infos: &Value) -> Result<bool> {
    infos["open"]
        .as_bool()
        .ok_or_else(|| Error::upstream(SERVICE, "its /infos answer has no `open`"))
}

#[derive(Debug, PartialEq)]
pub enum Refusal {
    Quota,
    Other(String),
}

/// Deezer reports errors, its quota included, as HTTP 200 with an `error` object.
pub fn answer(body: Value) -> Result<Value, Refusal> {
    let Some(error) = body.get("error") else {
        return Ok(body);
    };
    match error["code"].as_u64() {
        Some(QUOTA) => Err(Refusal::Quota),
        Some(NO_DATA) => Ok(serde_json::json!({ "data": [], "total": 0 })),
        code => Err(Refusal::Other(format!(
            "error {}: {}",
            code.map_or_else(|| "?".into(), |c| c.to_string()),
            error["message"].as_str().unwrap_or("no message")
        ))),
    }
}

#[derive(Deserialize)]
struct Page<T> {
    data: Vec<T>,
    #[serde(default)]
    total: u64,
}

impl<T: serde::de::DeserializeOwned> Page<T> {
    fn of(body: Value, what: &str) -> Result<Vec<T>> {
        let page: Page<T> = serde_json::from_value(body)
            .map_err(|e| Error::upstream(SERVICE, format!("unexpected {what} answer: {e}")))?;
        if page.data.is_empty() && page.total > 0 {
            // The same words every time, so the status tray doesn't flicker with each count.
            return Err(Error::upstream(
                SERVICE,
                format!("withholds its {what} here: its catalog isn't open in this country"),
            ));
        }
        Ok(page.data)
    }
}

#[derive(Deserialize)]
struct Named {
    name: String,
}

#[derive(Deserialize)]
struct RawTrack {
    title: String,
    duration: Option<u32>,
    rank: Option<u64>,
    isrc: Option<String>,
    artist: Named,
    album: RawTrackAlbum,
}

#[derive(Deserialize)]
struct RawTrackAlbum {
    title: String,
    md5_image: Option<String>,
}

#[derive(Deserialize)]
struct RawAlbum {
    title: String,
    record_type: Option<String>,
    md5_image: Option<String>,
    artist: Named,
}

#[derive(Deserialize)]
struct RawArtist {
    name: String,
    picture_xl: Option<String>,
    nb_fan: Option<u64>,
}

pub fn songs(body: Value) -> Result<Vec<SongHit>> {
    Ok(Page::<RawTrack>::of(body, "tracks")?
        .into_iter()
        .map(|t| SongHit {
            title: t.title,
            artist: t.artist.name,
            album: t.album.title,
            duration: t.duration.filter(|d| *d > 0),
            isrcs: t.isrc.into_iter().collect(),
            popularity: t.rank.map_or(0.0, |r| popularity(r, FULL_RANK)),
            art: t.album.md5_image.as_deref().and_then(|m| picture("cover", m)),
            ..SongHit::default()
        })
        .collect())
}

pub fn albums_of(body: Value) -> Result<Vec<AlbumHit>> {
    let raw = Page::<RawAlbum>::of(body, "albums")?;
    let n = raw.len().max(1) as f64;
    // Album answers carry no count, so Deezer's own order (relevance, then popularity) stands in.
    Ok(raw
        .into_iter()
        .enumerate()
        .map(|(i, a)| AlbumHit {
            title: a.title,
            artist: a.artist.name,
            kind: a.record_type.as_deref().map(kind),
            popularity: 0.6 * (1.0 - i as f64 / n),
            art: a.md5_image.as_deref().and_then(|m| picture("cover", m)),
            ..AlbumHit::default()
        })
        .collect())
}

pub fn artists_of(body: Value) -> Result<Vec<ArtistHit>> {
    Ok(Page::<RawArtist>::of(body, "artists")?
        .into_iter()
        .map(|a| ArtistHit {
            image: a
                .picture_xl
                .as_deref()
                .and_then(artist_md5)
                .and_then(|m| picture("artist", m)),
            popularity: a.nb_fan.map_or(0.0, |f| popularity(f, FULL_FANS)),
            audience: a.nb_fan,
            lastfm_only: false,
            name: a.name,
            mbid: None,
        })
        .collect())
}

/// Deezer's `record_type` in the words the UI shows.
fn kind(record_type: &str) -> String {
    match record_type {
        "ep" => "EP".into(),
        "compile" => "compilation".into(),
        other => other.to_owned(),
    }
}

/// A CDN picture at any size; Deezer's placeholder (an empty hash) is no picture.
fn picture(kind: &str, md5: &str) -> Option<String> {
    (!md5.is_empty() && md5.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| format!("{CDN}/{kind}/{md5}/{{size}}x{{size}}-000000-80-0-0.jpg"))
}

/// The hash in `…/images/artist/<md5>/1000x1000-….jpg`.
fn artist_md5(url: &str) -> Option<&str> {
    url.split("/images/artist/").nth(1)?.split('/').next()
}
