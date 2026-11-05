//! Apple's iTunes Search (https://performance-partners.apple.com/search-api): the fast source
//! for tracks and albums where Deezer's catalog is closed (ADR-0008 update). Per query, in the
//! store of `LULL_CATALOG_COUNTRY`: an `entity=song` call, whose songs also bring the albums
//! they are on, and an `entity=album` call beside it, so a soundtrack or compilation is found
//! by its own name. Apple asks for about 20 calls a minute, so lull keeps a budget below that,
//! shared by everyone using the hub: album searches and pages use only what songs leave spare.
//! With it spent a call waits its turn (`Budget::take_turn`), the newest search first, rather
//! than a search answering without iTunes: that answer would be kept for ten minutes and shown
//! to everyone searching the same words.

use std::cmp::Reverse;
use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use lull_core::http::Http;
use lull_core::ids::{loose_artist, loose_title, normalize, normalize_artist};
use lull_core::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Notify;
use tokio::time::Instant;

use super::cache::{Cache, DAY};
use super::{AlbumSource, ArtistSource, Song, year};
use crate::artist::discography::{Entry, Origin};
use crate::search::merge::{self, AlbumHit, ArtistHit, Found, SongHit};

const SERVICE: &str = "iTunes";
const API: &str = "https://itunes.apple.com/search";
const LOOKUP: &str = "https://itunes.apple.com/lookup";
/// Tracklists are kept this long (seconds).
const KEEP_TRACKLIST_S: i64 = 7 * DAY;
const BUDGET: usize = 18;
/// Calls of the budget that work nobody is waiting on (artist pages' catalogs, share links,
/// covers, link imports) leaves to the listener's searches.
const SONGS_RESERVE: usize = 6;
const BUDGET_WINDOW: Duration = Duration::from_secs(60);
/// The longest a search's album call waits for its turn when searches have spent the budget:
/// the first answer waits for the songs, which arrive with it.
const ALBUM_WAIT: Duration = Duration::from_secs(5);
/// Songs and albums asked for per query.
const SONGS: &str = "25";
const ALBUMS: &str = "25";
/// Answers are kept this long (seconds).
const KEEP_S: i64 = 600;
/// Songs looked up to match another service's song are kept this long (seconds).
const KEEP_MATCH_S: i64 = 7 * DAY;

pub struct Itunes {
    http: Http,
    cache: Cache,
    country: String,
    budget: Mutex<Budget>,
    /// Woken whenever a waiting call takes its turn or gives up, so the next in line looks.
    turn: Notify,
}

impl Itunes {
    pub fn new(http: Http, cache: Cache, country: String) -> Self {
        Itunes {
            http,
            cache,
            country,
            budget: Mutex::default(),
            turn: Notify::new(),
        }
    }

    pub fn country(&self) -> &str {
        &self.country
    }

    /// Songs for finding a song another service names (a playlist link): kept a week, and only
    /// with the calls left spare by everything a listener is looking at.
    pub async fn match_songs(&self, q: &str) -> Result<Vec<SongHit>> {
        let key = format!("it:match:{}:{q}", self.country);
        if let Some(found) = self.cache.fresh(key.clone(), KEEP_MATCH_S).await? {
            return Ok(found);
        }
        self.wait_turn(Need::Background).await?;
        let body: Value = self
            .http
            .json(SERVICE, |c| {
                c.get(API).query(&[
                    ("term", q),
                    ("entity", "song"),
                    ("limit", "10"),
                    ("country", self.country.as_str()),
                ])
            })
            .await?;
        let found = songs(body)?.songs;
        self.cache.put(key, &found).await?;
        Ok(found)
    }

    /// Songs for `q` with the albums they are on, and albums by their own name, each call waiting
    /// its turn of the budget. Album search failing, or not getting a turn within `ALBUM_WAIT`,
    /// leaves the songs, and is named in `problem`.
    pub async fn search(&self, q: &str) -> Result<Found> {
        let albums = async {
            tokio::time::timeout(ALBUM_WAIT, self.find(q, Entity::Album))
                .await
                .unwrap_or_else(|_| Err(Error::unavailable(SERVICE, "lull's calls for this minute are spent")))
        };
        let (songs, albums) = tokio::join!(self.find(q, Entity::Song), albums);
        Ok(answer(q, songs?, albums))
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
        self.wait_turn(match entity {
            Entity::Song => Need::Whole,
            Entity::Album => Need::Albums,
        })
        .await?;
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
        self.tracklist_for(collection, None).await
    }

    /// `tracklist`, for work nobody is looking at (identifying artists): it waits for what
    /// searches leave spare of the budget, served last, rather than failing when it is spent.
    pub async fn tracklist_in_turn(&self, collection: u64) -> Result<Vec<Song>> {
        self.tracklist_for(collection, Some(Need::Background)).await
    }

    async fn tracklist_for(&self, collection: u64, need: Option<Need>) -> Result<Vec<Song>> {
        // "-ids": answers kept before songs carried their artist's id are asked again.
        let key = format!("it:tracklist-ids:{}:{collection}", self.country);
        self.cache
            .get(key, KEEP_TRACKLIST_S, || async {
                match need {
                    Some(need) => self.wait_turn(need).await?,
                    None if !self.budget().take(Instant::now()) => {
                        return Err(Error::unavailable(SERVICE, "lull's calls for this minute are spent"));
                    }
                    None => {}
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
    /// share links (`share.rs` caches the outcome). Over the budget it waits its turn, after
    /// searches' songs.
    pub async fn listings(&self, term: &str, entity: Entity) -> Result<Vec<Listing>> {
        self.wait_turn(Need::Spare).await?;
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
    /// An album's cover by name, as a picture source (`{size}x{size}bb`), from the calls searches
    /// leave spare (it waits behind everything a listener is looking at); `None` when the store
    /// has no album of that artist and title. Kept a month.
    pub async fn album_cover(&self, artist: &str, title: &str) -> Result<Option<String>> {
        let key = format!(
            "it:album-cover:{}:{}:{}",
            self.country,
            normalize_artist(artist),
            normalize(title)
        );
        if let Some(found) = self.cache.fresh(key.clone(), 30 * DAY).await? {
            return Ok(found);
        }
        self.wait_turn(Need::Background).await?;
        let term = format!("{artist} {title}");
        let body: Value = self
            .http
            .json(SERVICE, |c| {
                c.get(API).query(&[
                    ("term", term.as_str()),
                    ("entity", "album"),
                    ("limit", "5"),
                    ("country", self.country.as_str()),
                ])
            })
            .await?;
        let found = cover_of(albums(body)?, artist, title);
        self.cache.put(key, &found).await?;
        Ok(found)
    }
}

/// The cover of the album that is this artist's and this title, among search's albums.
fn cover_of(found: Found, artist: &str, title: &str) -> Option<String> {
    found
        .albums
        .into_iter()
        .find(|a| loose_artist(&a.artist) == loose_artist(artist) && loose_title(&a.title) == loose_title(title))
        .and_then(|a| a.art)
}

impl Itunes {
    /// What the store id `id` is (a song with its album, an album, or an artist), for a link
    /// someone pasted: in this hub's store, else the US one, where Odesli's links mostly point.
    /// Kept a month; it waits its turn, as the listener is waiting.
    pub async fn lookup(&self, id: u64) -> Result<Found> {
        let mut countries = vec![self.country.as_str()];
        if !self.country.eq_ignore_ascii_case("US") {
            countries.push("US");
        }
        for country in countries {
            let key = format!("it:lookup:{country}:{id}");
            let found = self
                .cache
                .get(key, 30 * DAY, || async {
                    self.wait_turn(Need::Whole).await?;
                    let id = id.to_string();
                    let body: Value = self
                        .http
                        .json(SERVICE, |c| {
                            c.get(LOOKUP).query(&[("id", id.as_str()), ("country", country)])
                        })
                        .await?;
                    looked_up(body)
                })
                .await?;
            if !(found.songs.is_empty() && found.albums.is_empty() && found.artists.is_empty()) {
                return Ok(found);
            }
        }
        Ok(Found::default())
    }

    /// What the store has of an artist (`artistId`): its releases and the ones it appears on,
    /// and its songs, most played first as far as Apple's order says. Two `lookup` calls, kept a
    /// day; they wait for what searches leave spare of the budget (the page shows them late).
    pub async fn artist_catalog(&self, id: u64) -> Result<ArtistCatalog> {
        let key = format!("it:artist:{}:{id}", self.country);
        self.cache
            .get(key, DAY, || async {
                let lookup = |entity: &'static str| {
                    let id = id.to_string();
                    async move {
                        self.wait_turn(Need::Spare).await?;
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

    /// Waits for a call of the budget: served by need, then newest first, so the words a
    /// listener ended on go before the ones typed on the way. Fails after `BUDGET_WINDOW`.
    async fn wait_turn(&self, need: Need) -> Result<()> {
        let until = Instant::now() + BUDGET_WINDOW;
        let ticket = Waiting::join(self, need);
        loop {
            // Made before the check, so a turn taken in between still wakes it.
            let turn = self.turn.notified();
            let now = Instant::now();
            let next = match self.budget().take_turn(ticket.0, now) {
                Ok(()) => return Ok(()),
                Err(next) => next,
            };
            if now >= until {
                return Err(Error::unavailable(SERVICE, "lull's calls for this minute are spent"));
            }
            tokio::select! {
                () = tokio::time::sleep_until(next.min(until)) => {}
                () = turn => {}
            }
        }
    }

    fn budget(&self) -> MutexGuard<'_, Budget> {
        self.budget.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A call in line for the budget; leaving it (served, or given up) lets the next one look.
struct Waiting<'a>(u64, &'a Itunes);

impl<'a> Waiting<'a> {
    fn join(itunes: &'a Itunes, need: Need) -> Self {
        Waiting(itunes.budget().join(need), itunes)
    }
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        self.1.budget().leave(self.0);
        self.1.turn.notify_waiters();
    }
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

/// A `lookup` answer as what it is: a song with its album, an album, or an artist.
pub fn looked_up(body: Value) -> Result<Found> {
    let wrapper = body
        .pointer("/results/0/wrapperType")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    Ok(match wrapper.as_str() {
        "track" => songs(body)?,
        "collection" => albums(body)?,
        "artist" => Found {
            artists: body
                .pointer("/results/0/artistName")
                .and_then(Value::as_str)
                .map(|name| ArtistHit {
                    name: name.to_owned(),
                    ..ArtistHit::default()
                })
                .into_iter()
                .collect(),
            ..Found::default()
        },
        _ => Found::default(),
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

/// Who a call is for, in the order calls are served when they are scarce.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Need {
    /// A search's songs, an artist page's store lookup: may use the whole budget.
    Whole,
    /// A search's albums: the whole budget too, after the songs. The listener is waiting on
    /// them: the songs arrive with them, and an answer without them fixes a search's top
    /// result without its albums (`search::first_answer`).
    Albums,
    /// An artist page's catalog, a share link: what leaves `SONGS_RESERVE` for searches.
    Spare,
    /// Work nobody is looking at (matching a playlist link's songs): spare calls, served last.
    Background,
}

impl Need {
    fn up_to(self) -> usize {
        match self {
            Need::Whole | Need::Albums => BUDGET,
            Need::Spare | Need::Background => BUDGET - SONGS_RESERVE,
        }
    }
}

/// At most `BUDGET` calls in any `BUDGET_WINDOW`, and the calls waiting for room.
#[derive(Debug, Default)]
pub struct Budget {
    sent: VecDeque<Instant>,
    /// Calls in line by need and ticket (a later ticket is newer).
    waiting: Vec<(Need, u64)>,
    tickets: u64,
}

impl Budget {
    /// Spends one call now if the whole budget allows it: for calls with a fallback of their own
    /// (an album page asks MusicBrainz), which don't wait.
    pub fn take(&mut self, now: Instant) -> bool {
        self.expire(now);
        let ok = self.sent.len() < BUDGET;
        if ok {
            self.sent.push_back(now);
        }
        ok
    }

    /// Puts a call in line; its ticket.
    pub fn join(&mut self, need: Need) -> u64 {
        self.tickets += 1;
        self.waiting.push((need, self.tickets));
        self.tickets
    }

    pub fn leave(&mut self, ticket: u64) {
        self.waiting.retain(|(_, t)| *t != ticket);
    }

    /// Spends a call for `ticket` when it is first in line (by need, then newest) and its need
    /// has room, and takes it out of line. Otherwise, when room for it may free up.
    pub fn take_turn(&mut self, ticket: u64, now: Instant) -> std::result::Result<(), Instant> {
        self.expire(now);
        let first = self
            .waiting
            .iter()
            .min_by_key(|&&(need, t)| (need, Reverse(t)))
            .map(|w| w.1);
        let Some(&(need, _)) = self.waiting.iter().find(|(_, t)| *t == ticket) else {
            return Err(now + BUDGET_WINDOW);
        };
        let up_to = need.up_to();
        if first == Some(ticket) && self.sent.len() < up_to {
            self.sent.push_back(now);
            self.leave(ticket);
            return Ok(());
        }
        // When the call that makes room ages out; one ahead in line wakes it by taking its turn.
        Err(match self.sent.len().checked_sub(up_to) {
            Some(i) => self.sent[i] + BUDGET_WINDOW,
            None => now + BUDGET_WINDOW,
        })
    }

    fn expire(&mut self, now: Instant) {
        while self.sent.front().is_some_and(|t| *t + BUDGET_WINDOW <= now) {
            self.sent.pop_front();
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Raw {
    wrapper_type: Option<String>,
    track_name: Option<String>,
    artist_id: Option<u64>,
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
            artist_source: r.artist_id.map(ArtistSource::Itunes),
            ..SongHit::default()
        });
    }
    Ok(found)
}

/// A search's answer from its song and album calls: the albums its songs are on, then the ones
/// found by name; songs found for their album's name lend it their popularity
/// (`merge::credit_albums`).
pub fn answer(q: &str, songs: Found, albums: Result<Found>) -> Found {
    let mut found = songs;
    match albums {
        Ok(albums) => found.albums.extend(albums.albums),
        Err(e) => found.problem = Some(format!("its album search failed: {e}")),
    }
    merge::credit_albums(&merge::Query::new(q), &mut found);
    found
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
                artist_source: r.artist_id.map(ArtistSource::Itunes),
                isrc: None,
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

#[cfg(test)]
mod budget_tests {
    use super::*;

    /// One call that joins the line alone and takes its turn at once, or doesn't.
    fn turn(b: &mut Budget, need: Need, now: Instant) -> bool {
        let t = b.join(need);
        let ok = b.take_turn(t, now).is_ok();
        b.leave(t);
        ok
    }

    #[test]
    fn spare_calls_leave_the_reserve_to_searches() {
        let (mut b, now) = (Budget::default(), Instant::now());
        assert_eq!((0..25).filter(|_| turn(&mut b, Need::Spare, now)).count(), 12);
        assert!(!turn(&mut b, Need::Background, now));
        assert!(turn(&mut b, Need::Albums, now), "a search's albums use the reserve");
        assert_eq!((0..25).filter(|_| turn(&mut b, Need::Whole, now)).count(), 5);
        assert!(!b.take(now + Duration::from_secs(59)));
        assert!(turn(&mut b, Need::Spare, now + BUDGET_WINDOW));
    }

    #[test]
    fn with_the_budget_spent_the_newest_search_goes_first_and_imports_last() {
        let (mut b, now) = (Budget::default(), Instant::now());
        for _ in 0..BUDGET {
            assert!(b.take(now));
        }
        let import = b.join(Need::Background);
        let typed_on_the_way = b.join(Need::Whole);
        let albums = b.join(Need::Albums);
        let ended_on = b.join(Need::Whole);
        assert_eq!(b.take_turn(ended_on, now), Err(now + BUDGET_WINDOW));
        let later = now + BUDGET_WINDOW;
        assert!(
            b.take_turn(typed_on_the_way, later).is_err(),
            "a newer search is in line"
        );
        assert!(b.take_turn(import, later).is_err());
        assert!(b.take_turn(ended_on, later).is_ok());
        assert!(b.take_turn(typed_on_the_way, later).is_ok());
        assert!(b.take_turn(import, later).is_err(), "the album search goes first");
        assert!(b.take_turn(albums, later).is_ok());
        assert!(b.take_turn(import, later).is_ok());
    }
}
