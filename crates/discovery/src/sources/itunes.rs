//! Apple's iTunes Search (https://performance-partners.apple.com/search-api): the fast source
//! for tracks and albums where Deezer's catalog is closed (ADR-0008 update). Per query, in the
//! store of `LULL_CATALOG_COUNTRY`: an `entity=song` call, whose songs also bring the albums
//! they are on, and an `entity=album` call beside it, so a soundtrack or compilation is found
//! by its own name. Apple asks for about 20 calls a minute, so lull keeps a budget below that:
//! album searches use only what songs leave spare, and with it spent search answers without
//! iTunes, not with an error.

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
/// Calls of the budget album searches leave to songs.
const SONGS_RESERVE: usize = 6;
const BUDGET_WINDOW: Duration = Duration::from_secs(60);
/// Songs and albums asked for per query.
const SONGS: &str = "25";
const ALBUMS: &str = "15";
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

    /// Songs for `q` with the albums they are on, and albums by their own name. Empty when the
    /// budget is spent; without albums when only songs fit in it. Album search failing leaves
    /// the songs, and is named in `problem`.
    pub async fn search(&self, q: &str) -> Result<Found> {
        let (songs, albums) = tokio::join!(self.find(q, Entity::Song), self.find(q, Entity::Album));
        let mut found = songs?;
        match albums {
            Ok(albums) => found.albums.extend(albums.albums),
            Err(e) => found.problem = Some(format!("its album search failed: {e}")),
        }
        Ok(found)
    }

    /// One search call, cached.
    async fn find(&self, q: &str, entity: Entity) -> Result<Found> {
        let (name, limit) = match entity {
            Entity::Song => ("song", SONGS),
            Entity::Album => ("album", ALBUMS),
        };
        let key = format!("it:search-{name}:{}:{q}", self.country);
        if let Some(found) = self.cache.fresh(key.clone(), KEEP_S).await? {
            return Ok(found);
        }
        let allowed = {
            let mut budget = self.budget.lock().unwrap_or_else(PoisonError::into_inner);
            match entity {
                Entity::Song => budget.take(Instant::now()),
                Entity::Album => budget.take_spare(Instant::now()),
            }
        };
        if !allowed {
            tracing::info!(query = q, entity = name, "iTunes budget spent; searching without it");
            return Ok(Found::default());
        }
        let body: Value = self
            .http
            .json(SERVICE, |c| {
                c.get(API).query(&[
                    ("term", q),
                    ("entity", name),
                    ("limit", limit),
                    ("country", self.country.as_str()),
                ])
            })
            .await?;
        let found = match entity {
            Entity::Song => songs(body)?,
            Entity::Album => albums(body)?,
        };
        self.cache.put(key, &found).await?;
        Ok(found)
    }
}

#[derive(Clone, Copy)]
enum Entity {
    Song,
    Album,
}

/// At most `BUDGET` calls in any `BUDGET_WINDOW`.
#[derive(Debug, Default)]
pub struct Budget {
    sent: VecDeque<Instant>,
}

impl Budget {
    /// Spends one call if the budget allows it.
    pub fn take(&mut self, now: Instant) -> bool {
        self.spend(now, BUDGET)
    }

    /// Spends one call only if songs would still have `SONGS_RESERVE` calls left after it.
    pub fn take_spare(&mut self, now: Instant) -> bool {
        self.spend(now, BUDGET - SONGS_RESERVE)
    }

    fn spend(&mut self, now: Instant, up_to: usize) -> bool {
        while self.sent.front().is_some_and(|t| *t + BUDGET_WINDOW <= now) {
            self.sent.pop_front();
        }
        let ok = self.sent.len() < up_to;
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
#[serde(rename_all = "camelCase")]
struct RawAlbum {
    collection_name: Option<String>,
    artist_name: Option<String>,
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

/// An `entity=album` answer as rows. Apple orders by relevance and gives no counts, so the
/// order stands in for popularity, on the scale Deezer's albums use (below songs': an album
/// leads only with the top result's lean toward full-length albums).
pub fn albums(body: Value) -> Result<Found> {
    #[derive(Deserialize)]
    struct Page {
        results: Vec<RawAlbum>,
    }
    let page: Page =
        serde_json::from_value(body).map_err(|e| Error::upstream(SERVICE, format!("unexpected answer: {e}")))?;
    let n = page.results.len().max(1) as f64;
    let albums = page
        .results
        .into_iter()
        .enumerate()
        .filter_map(|(i, r)| {
            let (title, kind) = release(r.collection_name.as_deref()?);
            let artist = r.artist_name?;
            let kind = if artist == "Various Artists" {
                "compilation"
            } else {
                kind
            };
            Some(AlbumHit {
                title,
                kind: Some(kind.to_owned()),
                date: r.release_date.as_deref().and_then(|d| d.get(..10)).map(str::to_owned),
                mbid: None,
                popularity: 0.6 * (1.0 - i as f64 / n),
                art: r.artwork_url100.as_deref().and_then(sized),
                artist,
            })
        })
        .filter(|a| !a.title.is_empty())
        .collect();
    Ok(Found {
        albums,
        ..Found::default()
    })
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
