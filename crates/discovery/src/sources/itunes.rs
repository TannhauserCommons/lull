//! Apple's iTunes Search (https://performance-partners.apple.com/search-api): the fast source
//! for tracks and albums where Deezer's catalog is closed (ADR-0008 update). One `entity=song`
//! call per query, in the store of `LULL_CATALOG_COUNTRY`; albums are derived from the songs.
//! Apple asks for about 20 calls a minute, so lull keeps a budget below that and answers
//! without iTunes, not with an error, when it is spent.

use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use lull_core::http::Http;
use lull_core::{Error, Result};
use serde::Deserialize;
use serde_json::Value;
use tokio::time::Instant;

use super::cache::Cache;
use super::year;
use crate::search::merge::{AlbumHit, Found, SongHit};

const SERVICE: &str = "iTunes";
const API: &str = "https://itunes.apple.com/search";
const BUDGET: usize = 18;
const BUDGET_WINDOW: Duration = Duration::from_secs(60);
/// Answers are kept this long (seconds).
const KEEP_S: i64 = 600;

pub struct Itunes {
    http: Http,
    cache: Cache,
    country: String,
    budget: Mutex<Budget>,
}

impl Itunes {
    pub fn new(http: Http, cache: Cache, country: String) -> Self {
        Itunes {
            http,
            cache,
            country,
            budget: Mutex::default(),
        }
    }

    pub fn country(&self) -> &str {
        &self.country
    }

    /// Songs for `q`, and the albums they are on. Empty when the budget is spent.
    pub async fn search(&self, q: &str) -> Result<Found> {
        let key = format!("it:search:{}:{q}", self.country);
        if let Some(found) = self.cache.fresh(key.clone(), KEEP_S).await? {
            return Ok(found);
        }
        if !self
            .budget
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take(Instant::now())
        {
            tracing::info!(query = q, "iTunes budget spent; searching without it");
            return Ok(Found::default());
        }
        let body: Value = self
            .http
            .json(SERVICE, |c| {
                c.get(API).query(&[
                    ("term", q),
                    ("entity", "song"),
                    ("limit", "25"),
                    ("country", self.country.as_str()),
                ])
            })
            .await?;
        let found = songs(body)?;
        self.cache.put(key, &found).await?;
        Ok(found)
    }
}

/// At most `BUDGET` calls in any `BUDGET_WINDOW`.
#[derive(Debug, Default)]
pub struct Budget {
    sent: VecDeque<Instant>,
}

impl Budget {
    /// Spends one call if the budget allows it.
    pub fn take(&mut self, now: Instant) -> bool {
        while self.sent.front().is_some_and(|t| *t + BUDGET_WINDOW <= now) {
            self.sent.pop_front();
        }
        let ok = self.sent.len() < BUDGET;
        if ok {
            self.sent.push_back(now);
        }
        ok
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Raw {
    track_name: Option<String>,
    artist_name: Option<String>,
    collection_name: Option<String>,
    collection_artist_name: Option<String>,
    track_time_millis: Option<u64>,
    release_date: Option<String>,
    artwork_url100: Option<String>,
}

#[derive(Deserialize)]
struct Page {
    results: Vec<Raw>,
}

/// An `entity=song` answer as rows. Apple orders by relevance and gives no counts, so the
/// order stands in for popularity.
pub fn songs(body: Value) -> Result<Found> {
    let page: Page =
        serde_json::from_value(body).map_err(|e| Error::upstream(SERVICE, format!("unexpected answer: {e}")))?;
    let n = page.results.len().max(1) as f64;
    let mut found = Found::default();
    for (i, r) in page.results.into_iter().enumerate() {
        let (Some(title), Some(artist)) = (r.track_name, r.artist_name) else {
            continue;
        };
        let (album, kind) = release(r.collection_name.as_deref().unwrap_or_default());
        let album_artist = r.collection_artist_name.filter(|a| *a != artist);
        let kind = match album_artist.as_deref() {
            Some("Various Artists") => "compilation".to_owned(),
            _ => kind.to_owned(),
        };
        let art = r.artwork_url100.as_deref().and_then(sized);
        let date = r.release_date.as_deref().and_then(|d| d.get(..10)).map(str::to_owned);
        let popularity = 0.7 * (1.0 - i as f64 / n);
        if !album.is_empty() {
            found.albums.push(AlbumHit {
                title: album.clone(),
                artist: album_artist.clone().unwrap_or_else(|| artist.clone()),
                kind: Some(kind.clone()),
                date: date.clone(),
                mbid: None,
                popularity: popularity * 0.5,
                art: art.clone(),
            });
        }
        found.songs.push(SongHit {
            title,
            artist,
            album,
            album_artist,
            album_kind: Some(kind),
            year: date.as_deref().and_then(year),
            duration: r.track_time_millis.map(|ms| (ms / 1000) as u32).filter(|s| *s > 0),
            popularity,
            art,
            ..SongHit::default()
        });
    }
    Ok(found)
}

/// "Creep - EP" → ("Creep", "EP"); "Song - Single" → ("Song", "single"); else an album.
fn release(collection: &str) -> (String, &'static str) {
    for (suffix, kind) in [(" - Single", "single"), (" - EP", "EP")] {
        if let Some(title) = collection.strip_suffix(suffix) {
            return (title.to_owned(), kind);
        }
    }
    (collection.to_owned(), "album")
}

/// Apple's artwork URLs name their size (`…/100x100bb.jpg`) and serve any other.
fn sized(url: &str) -> Option<String> {
    let (base, file) = url.rsplit_once('/')?;
    let ext = file.strip_prefix("100x100bb")?;
    Some(format!("{base}/{{size}}x{{size}}bb{ext}"))
}
