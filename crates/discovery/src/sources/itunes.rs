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
use lull_core::ids::normalize_artist;
use lull_core::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::time::Instant;

use super::cache::{Cache, DAY};
use super::{AlbumSource, Song, year};
use crate::artist::discography::{Entry, Origin};
use crate::search::merge::{AlbumHit, Found, SongHit};

const SERVICE: &str = "iTunes";
const API: &str = "https://itunes.apple.com/search";
const LOOKUP: &str = "https://itunes.apple.com/lookup";
/// Tracklists are kept this long (seconds).
const KEEP_TRACKLIST_S: i64 = 7 * DAY;
const BUDGET: usize = 18;
/// Calls of the budget album searches leave to songs.
const SONGS_RESERVE: usize = 6;
const BUDGET_WINDOW: Duration = Duration::from_secs(60);
/// Songs and albums asked for per query.
const SONGS: &str = "25";
const ALBUMS: &str = "25";
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

impl Itunes {
    /// The songs of an album (`collectionId`) in order: one `lookup` call, kept a week. Fails
    /// while the budget is spent, so the album page asks MusicBrainz instead.
    pub async fn tracklist(&self, collection: u64) -> Result<Vec<Song>> {
        let key = format!("it:tracklist:{}:{collection}", self.country);
        self.cache
            .get(key, KEEP_TRACKLIST_S, || async {
                if !self
                    .budget
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take(Instant::now())
                {
                    return Err(Error::unavailable(SERVICE, "lull's calls for this minute are spent"));
                }
                let id = collection.to_string();
                let body: Value = self
                    .http
                    .json(SERVICE, |c| {
                        c.get(LOOKUP).query(&[
                            ("id", id.as_str()),
                            ("entity", "song"),
                            ("limit", "200"),
                            ("country", self.country.as_str()),
                        ])
                    })
                    .await?;
                tracklist(body)
            })
            .await
    }
}

impl Itunes {
    /// The store's songs (`Entity::Song`) or albums for `term`, with their ids and covers, for
    /// share links (`share.rs` caches the outcome). Fails while the budget is spent.
    pub async fn listings(&self, term: &str, entity: Entity) -> Result<Vec<Listing>> {
        if !self
            .budget
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take(Instant::now())
        {
            return Err(Error::unavailable(SERVICE, "lull's calls for this minute are spent"));
        }
        let name = match entity {
            Entity::Song => "song",
            Entity::Album => "album",
        };
        let body: Value = self
            .http
            .json(SERVICE, |c| {
                c.get(API).query(&[
                    ("term", term),
                    ("entity", name),
                    ("limit", "10"),
                    ("country", self.country.as_str()),
                ])
            })
            .await?;
        let page: Listings =
            serde_json::from_value(body).map_err(|e| Error::upstream(SERVICE, format!("unexpected answer: {e}")))?;
        Ok(page.results)
    }
}

impl Itunes {
    /// The store's artists called `name` (case and a leading "the" aside), best known first. Kept
    /// a month: an artist's id doesn't change.
    pub async fn artists(&self, name: &str) -> Result<Vec<StoreArtist>> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Raw {
            artist_id: Option<u64>,
            artist_name: Option<String>,
        }
        #[derive(Deserialize)]
        struct Page {
            results: Vec<Raw>,
        }
        let key = format!("it:artists:{}:{}", self.country, normalize_artist(name));
        self.cache
            .get(key, 30 * DAY, || async {
                self.spend(Instant::now())?;
                let body: Value = self
                    .http
                    .json(SERVICE, |c| {
                        c.get(API).query(&[
                            ("term", name),
                            ("entity", "musicArtist"),
                            ("limit", "10"),
                            ("country", self.country.as_str()),
                        ])
                    })
                    .await?;
                let page: Page = serde_json::from_value(body)
                    .map_err(|e| Error::upstream(SERVICE, format!("unexpected answer: {e}")))?;
                Ok(page
                    .results
                    .into_iter()
                    .filter_map(|r| {
                        Some(StoreArtist {
                            id: r.artist_id?,
                            name: r.artist_name?,
                        })
                    })
                    .filter(|a| normalize_artist(&a.name) == normalize_artist(name))
                    .collect())
            })
            .await
    }

    /// What the store has of an artist (`artistId`): its releases and the ones it appears on,
    /// and its songs, most played first as far as Apple's order says. Two `lookup` calls, kept a
    /// day; they use only what search leaves spare of the budget.
    pub async fn artist_catalog(&self, id: u64) -> Result<ArtistCatalog> {
        let key = format!("it:artist:{}:{id}", self.country);
        self.cache
            .get(key, DAY, || async {
                let lookup = |entity: &'static str| {
                    let id = id.to_string();
                    async move {
                        self.spend_spare(Instant::now())?;
                        self.http
                            .json::<Value>(SERVICE, |c| {
                                c.get(LOOKUP).query(&[
                                    ("id", id.as_str()),
                                    ("entity", entity),
                                    ("limit", "200"),
                                    ("country", self.country.as_str()),
                                ])
                            })
                            .await
                    }
                };
                let (albums, songs) = tokio::join!(lookup("album"), lookup("song"));
                artist_catalog(albums?, songs?, id)
            })
            .await
    }

    fn spend(&self, now: Instant) -> Result<()> {
        if self.budget.lock().unwrap_or_else(PoisonError::into_inner).take(now) {
            Ok(())
        } else {
            Err(Error::unavailable(SERVICE, "lull's calls for this minute are spent"))
        }
    }

    fn spend_spare(&self, now: Instant) -> Result<()> {
        if self
            .budget
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take_spare(now)
        {
            Ok(())
        } else {
            Err(Error::unavailable(SERVICE, "lull's calls for this minute are spent"))
        }
    }
}

/// An artist in the store.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoreArtist {
    pub id: u64,
    pub name: String,
}

/// `artist_catalog`'s answer.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ArtistCatalog {
    pub releases: Vec<Entry>,
    /// Songs credited to the artist (alone or with others), Apple's order.
    pub songs: Vec<SongHit>,
}

/// The two `lookup` answers (`entity=album`, `entity=song`) of the artist `id` as its
/// releases: the albums the store lists for it (its own when their `artistId` is its), then the
/// albums its songs are on that those miss (soundtracks, compilations, other artists' EPs).
/// DJ mixes are left out: they are someone's set, not a release to play.
pub fn artist_catalog(albums: Value, songs_body: Value, id: u64) -> Result<ArtistCatalog> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct RawCollection {
        wrapper_type: Option<String>,
        collection_id: Option<u64>,
        collection_name: Option<String>,
        artist_id: Option<u64>,
        artist_name: Option<String>,
        release_date: Option<String>,
        artwork_url100: Option<String>,
        collection_explicitness: Option<String>,
    }
    #[derive(Deserialize)]
    struct Page {
        results: Vec<RawCollection>,
    }
    let page: Page =
        serde_json::from_value(albums).map_err(|e| Error::upstream(SERVICE, format!("unexpected answer: {e}")))?;
    let mut releases: Vec<Entry> = page
        .results
        .into_iter()
        .filter(|r| r.wrapper_type.as_deref() == Some("collection"))
        .filter_map(|r| {
            let (title, kind) = release(r.collection_name.as_deref()?);
            let artist = r.artist_name?;
            let kind = if artist == "Various Artists" {
                "compilation"
            } else {
                kind
            };
            Some(Entry {
                own: Some(r.artist_id == Some(id)),
                explicit: r.collection_explicitness.as_deref() == Some("explicit"),
                origin: Origin::Itunes,
                album: AlbumHit {
                    title,
                    kind: Some(kind.to_owned()),
                    date: r.release_date.as_deref().and_then(|d| d.get(..10)).map(str::to_owned),
                    art: r.artwork_url100.as_deref().and_then(sized),
                    source: r.collection_id.map(AlbumSource::Itunes),
                    artist,
                    ..AlbumHit::default()
                },
            })
        })
        .collect();
    let found = songs(songs_body)?;
    for album in found.albums {
        let listed = releases.iter().any(|e| e.album.source == album.source);
        if listed || is_dj_mix(&album.title) {
            continue;
        }
        releases.push(Entry {
            own: None,
            explicit: false,
            origin: Origin::Itunes,
            album: AlbumHit {
                popularity: 0.0,
                ..album
            },
        });
    }
    Ok(ArtistCatalog {
        releases,
        songs: found.songs.into_iter().filter(|s| !is_dj_mix(&s.album)).collect(),
    })
}

/// "Halloween XII (DJ Mix)", "Gold (Mixed)": a DJ's set.
fn is_dj_mix(title: &str) -> bool {
    let t = title.to_lowercase();
    t.ends_with("(dj mix)") || t.contains("(mixed)") || t.contains("[mixed]")
}

/// One song or album in the store, as `listings` gives it.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    pub track_id: Option<u64>,
    pub track_name: Option<String>,
    pub collection_id: Option<u64>,
    pub collection_name: Option<String>,
    pub artist_name: Option<String>,
    pub artwork_url100: Option<String>,
}

#[derive(Deserialize)]
struct Listings {
    results: Vec<Listing>,
}

#[derive(Clone, Copy)]
pub enum Entity {
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
    wrapper_type: Option<String>,
    track_name: Option<String>,
    artist_name: Option<String>,
    collection_id: Option<u64>,
    collection_name: Option<String>,
    collection_artist_name: Option<String>,
    track_time_millis: Option<u64>,
    release_date: Option<String>,
    artwork_url100: Option<String>,
    track_number: Option<u32>,
    disc_number: Option<u32>,
    disc_count: Option<u32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawAlbum {
    collection_id: Option<u64>,
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
        let source = r.collection_id.map(AlbumSource::Itunes);
        if !album.is_empty() {
            found.albums.push(AlbumHit {
                title: album.clone(),
                artist: album_artist.clone().unwrap_or_else(|| artist.clone()),
                kind: Some(kind.clone()),
                date: date.clone(),
                mbid: None,
                popularity: popularity * 0.5,
                art: art.clone(),
                source,
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
            album_source: source,
            ..SongHit::default()
        });
    }
    Ok(found)
}

/// A `lookup?entity=song` answer as the album's songs, in order. The album's own name and artist
/// are the page's to set: these are Apple's.
pub fn tracklist(body: Value) -> Result<Vec<Song>> {
    let page: Page =
        serde_json::from_value(body).map_err(|e| Error::upstream(SERVICE, format!("unexpected answer: {e}")))?;
    let mut songs: Vec<Song> = page
        .results
        .into_iter()
        .filter(|r| r.wrapper_type.as_deref() == Some("track"))
        .filter_map(|r| {
            let (album, _) = release(r.collection_name.as_deref().unwrap_or_default());
            Some(Song {
                mbid: None,
                title: r.track_name?,
                album_artist: r.collection_artist_name,
                artist: r.artist_name?,
                artist_mbid: None,
                album,
                album_mbid: None,
                year: r.release_date.as_deref().and_then(year),
                no: r.track_number,
                disc: r.disc_number.filter(|_| r.disc_count.is_some_and(|n| n > 1)),
                duration_s: r.track_time_millis.map(|ms| (ms / 1000) as u32).filter(|s| *s > 0),
                tags: Vec::new(),
            })
        })
        .collect();
    songs.sort_by_key(|s| (s.disc.unwrap_or(1), s.no.unwrap_or(u32::MAX)));
    Ok(songs)
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
                source: r.collection_id.map(AlbumSource::Itunes),
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
