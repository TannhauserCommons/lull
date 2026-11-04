//! Deezer (https://developers.deezer.com/api): catalog search with covers, artist photos and
//! popularity, no key needed. The limit is 50 requests per 5 s per IP, kept in lull's one Deezer
//! window (`http::DEEZER`, shared with collection's playlist links); going over is answered with
//! HTTP 200 and `error.code` 4, which closes that window for 5 s and is tried once more.
//! Where Deezer isn't open (in some countries) track and album searches come back empty with a
//! `total` above zero: that is reported, not mistaken for "nothing found".

use lull_core::http::{DEEZER, Http};
use lull_core::ids::{normalize, normalize_artist};
use lull_core::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::cache::{Cache, DAY};
use super::{AlbumSource, ArtistSource, Song};
use crate::artist::discography::{Entry, Origin};
use crate::search::merge::{AlbumHit, ArtistHit, Found, PlaylistHit, SongHit, popularity};

const SERVICE: &str = "Deezer";
const API: &str = "https://api.deezer.com";
const CDN: &str = "https://cdn-images.dzcdn.net/images";
/// `error.code` for "Quota limit exceeded".
const QUOTA: u64 = 4;
/// `error.code` for "no data": an empty answer, not a failure.
const NO_DATA: u64 = 800;
/// Search answers are kept this long (seconds), and served stale when Deezer fails.
const KEEP_S: i64 = 600;
/// Playlists whose fans are looked up, of a playlist search's answer, and how long they're kept.
const WITH_FANS: usize = 4;
const FANS_FOR: i64 = 7 * DAY;
/// Tracklists are kept this long (seconds).
const KEEP_TRACKLIST_S: i64 = 7 * DAY;
/// Rank, fans: the counts that score 1 on `popularity`'s scale.
const FULL_RANK: f64 = 1e6;
const FULL_FANS: f64 = 1e7;

pub struct Deezer {
    http: Http,
    cache: Cache,
}

impl Deezer {
    pub fn new(http: Http, cache: Cache) -> Self {
        Deezer { http, cache }
    }

    /// Tracks, albums and artists for `q`. A part that fails leaves the others, and is named in
    /// `problem`; all three failing is an error.
    pub async fn search(&self, q: &str) -> Result<Found> {
        self.cache
            .get(format!("dz:search:{q}"), KEEP_S, || async {
                let (track_q, album_q, artist_q) = (
                    [("q", q), ("limit", "25")],
                    [("q", q), ("limit", "15")],
                    [("q", q), ("limit", "10")],
                );
                let (tracks, albums, artists) = tokio::join!(
                    self.get("search/track", &track_q),
                    self.get("search/album", &album_q),
                    self.get("search/artist", &artist_q),
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
                    playlists: Vec::new(),
                })
            })
            .await
    }

    /// Artists only: where Deezer's catalog is closed, its artist search still answers.
    pub async fn artists(&self, q: &str) -> Result<Found> {
        self.cache
            .get(format!("dz:artists:{q}"), KEEP_S, || async {
                Ok(Found {
                    artists: artists_of(self.get("search/artist", &[("q", q), ("limit", "10")]).await?)?,
                    ..Found::default()
                })
            })
            .await
    }

    /// Playlists for `q` (15, Deezer's order), kept a day. Answers in every country. Fans come
    /// only with a playlist's whole page (seconds for a long one), so the answer carries the fans
    /// lull already knows, and the first few unknown ones are fetched after it, for next time.
    pub async fn playlists(&self, q: &str) -> Result<Vec<PlaylistHit>> {
        let mut found: Vec<PlaylistHit> = self
            .cache
            .get(format!("dz:playlists:{q}"), DAY, || async {
                playlists_of(self.get("search/playlist", &[("q", q), ("limit", "15")]).await?)
            })
            .await?;
        let mut unknown = Vec::new();
        for hit in found.iter_mut().take(WITH_FANS) {
            match self.cache.fresh::<Option<u64>>(fans_key(&hit.id), FANS_FOR).await? {
                Some(fans) => hit.fans = fans,
                None => unknown.push(hit.id.clone()),
            }
        }
        if !unknown.is_empty() {
            tokio::spawn(learn_fans(self.http.clone(), self.cache.clone(), unknown));
        }
        Ok(found)
    }

    /// Songs only, for finding a song another service names (a playlist link): kept a day.
    pub async fn tracks(&self, q: &str) -> Result<Vec<SongHit>> {
        self.cache
            .get(format!("dz:tracks:{q}"), DAY, || async {
                songs(self.get("search/track", &[("q", q), ("limit", "10")]).await?)
            })
            .await
    }

    /// The song with this ISRC (12 letters and digits), kept a month. A lookup by ISRC answers
    /// even where Deezer's catalog is closed.
    pub async fn by_isrc(&self, isrc: &str) -> Result<Option<SongHit>> {
        if isrc.len() != 12 || !isrc.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Ok(None);
        }
        self.cache
            .get(format!("dz:isrc:{isrc}"), 30 * DAY, || async {
                let body = self.get(&format!("track/isrc:{isrc}"), &[]).await?;
                if body.get("data").is_some() {
                    return Ok(None); // "no data": Deezer has no song with it
                }
                let track: RawTrack = serde_json::from_value(body)
                    .map_err(|e| Error::upstream(SERVICE, format!("unexpected track answer: {e}")))?;
                Ok(Some(hit(track)))
            })
            .await
    }

    /// One song by id, with its album, kept a month (answers where the catalog is closed too).
    pub async fn track(&self, id: u64) -> Result<Option<SongHit>> {
        self.cache
            .get(format!("dz:track:{id}"), 30 * DAY, || async {
                let body = self.get(&format!("track/{id}"), &[]).await?;
                if body.get("data").is_some() {
                    return Ok(None);
                }
                let track: RawTrack = serde_json::from_value(body)
                    .map_err(|e| Error::upstream(SERVICE, format!("unexpected track answer: {e}")))?;
                Ok(Some(hit(track)))
            })
            .await
    }

    /// One album by id, kept a month (answers where the catalog is closed too).
    pub async fn album(&self, id: u64) -> Result<Option<AlbumHit>> {
        self.cache
            .get(format!("dz:album:{id}"), 30 * DAY, || async {
                let body = self.get(&format!("album/{id}"), &[]).await?;
                if body.get("data").is_some() {
                    return Ok(None);
                }
                let album: RawAlbum = serde_json::from_value(body)
                    .map_err(|e| Error::upstream(SERVICE, format!("unexpected album answer: {e}")))?;
                Ok(Some(album_hit(album, 0.0)))
            })
            .await
    }

    /// Whether Deezer's catalog is open where lull runs (`/infos`).
    pub async fn is_open(&self) -> Result<bool> {
        self.wait_turn().await;
        open(&self.http.json(SERVICE, |c| c.get(format!("{API}/infos"))).await?)
    }

    /// The artists called `name`: the exact name (ignoring case), else the same without a
    /// leading "the" ("The Prodigy" is not the rapper Prodigy). Several can share a name; the
    /// caller tells them apart.
    pub async fn artists_named(&self, name: &str) -> Result<Vec<DeezerArtist>> {
        self.cache
            .get(format!("dz:artists-named:{}", normalize(name)), 7 * DAY, || async {
                let found: Vec<DeezerArtist> = Page::<RawArtist>::of(
                    self.get("search/artist", &[("q", name), ("limit", "10")]).await?,
                    "artists",
                )?
                .into_iter()
                .filter_map(RawArtist::artist)
                .collect();
                let exact: Vec<DeezerArtist> = found
                    .iter()
                    .filter(|a| normalize(&a.name) == normalize(name))
                    .cloned()
                    .collect();
                if !exact.is_empty() {
                    return Ok(exact);
                }
                Ok(found
                    .into_iter()
                    .filter(|a| normalize_artist(&a.name) == normalize_artist(name))
                    .collect())
            })
            .await
    }

    /// One artist by id (answers where the catalog is closed too).
    pub async fn artist(&self, id: u64) -> Result<Option<DeezerArtist>> {
        self.cache
            .get(format!("dz:artist:{id}"), 7 * DAY, || async {
                let raw: RawArtist = serde_json::from_value(self.get(&format!("artist/{id}"), &[]).await?)
                    .map_err(|e| Error::upstream(SERVICE, format!("unexpected artist answer: {e}")))?;
                Ok(raw.artist())
            })
            .await
    }

    /// The artist's albums, EPs and singles, where the catalog is open.
    pub async fn artist_releases(&self, id: u64) -> Result<Vec<Entry>> {
        self.cache
            .get(format!("dz:artist-albums:{id}"), DAY, || async {
                artist_releases(self.get(&format!("artist/{id}/albums"), &[("limit", "100")]).await?)
            })
            .await
    }

    /// The artist's most played songs, by Deezer's rank, where the catalog is open.
    pub async fn artist_top(&self, id: u64) -> Result<Vec<SongHit>> {
        self.cache
            .get(format!("dz:artist-top:{id}"), DAY, || async {
                songs(self.get(&format!("artist/{id}/top"), &[("limit", "20")]).await?)
            })
            .await
    }

    /// Covers of the artist's own songs on its Deezer radio, which answers where the catalog is
    /// closed: what an artist "photo" is checked against, as Deezer shows a cover when the
    /// artist has no photo. Empty for an artist too small to have a radio.
    pub async fn artist_covers(&self, id: u64) -> Result<Vec<String>> {
        #[derive(Deserialize)]
        struct Raw {
            md5_image: Option<String>,
            artist: RawId,
        }
        #[derive(Deserialize)]
        struct RawId {
            id: u64,
        }
        self.cache
            .get(format!("dz:artist-covers:{id}"), 7 * DAY, || async {
                let tracks = Page::<Raw>::of(
                    self.get(&format!("artist/{id}/radio"), &[("limit", "50")]).await?,
                    "radio",
                )?;
                let mut covers: Vec<String> = Vec::new();
                for t in tracks.into_iter().filter(|t| t.artist.id == id) {
                    if let Some(cover) = t.md5_image.as_deref().and_then(|m| picture("cover", m))
                        && !covers.contains(&cover)
                    {
                        covers.push(cover);
                    }
                }
                Ok(covers)
            })
            .await
    }

    /// The songs of an album, in order: one call, kept a week. A lookup by id answers even
    /// where Deezer's catalog is closed.
    pub async fn tracklist(&self, album: u64) -> Result<Vec<Song>> {
        self.cache
            // "-ids": answers kept before songs carried their artist's id and ISRC are asked again.
            .get(format!("dz:tracklist-ids:{album}"), KEEP_TRACKLIST_S, || async {
                tracklist(self.get(&format!("album/{album}/tracks"), &[("limit", "200")]).await?)
            })
            .await
    }

    /// Deezer's window is lull's (`http::DEEZER`): collection's playlist links share it.
    async fn wait_turn(&self) {
        self.http.window(DEEZER).await;
    }

    async fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<Value> {
        for attempt in 0..2 {
            self.wait_turn().await;
            let body: Value = self
                .http
                .json(SERVICE, |c| c.get(format!("{API}/{path}")).query(query))
                .await?;
            match answer(body) {
                Err(Refusal::Quota) if attempt == 0 => {
                    tracing::info!("Deezer quota reached; waiting for the next window");
                    self.http.close_window(DEEZER);
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

/// An artist as a track names it: its id and name.
#[derive(Deserialize)]
struct RawNamed {
    id: u64,
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
    id: Option<u64>,
    title: String,
    md5_image: Option<String>,
}

#[derive(Deserialize)]
struct RawAlbum {
    id: Option<u64>,
    title: String,
    record_type: Option<String>,
    md5_image: Option<String>,
    artist: Named,
}

#[derive(Deserialize)]
struct RawArtist {
    id: Option<u64>,
    name: String,
    picture_xl: Option<String>,
    nb_fan: Option<u64>,
}

impl RawArtist {
    fn image(&self) -> Option<String> {
        self.picture_xl
            .as_deref()
            .and_then(artist_md5)
            .and_then(|m| picture("artist", m))
    }

    fn artist(self) -> Option<DeezerArtist> {
        Some(DeezerArtist {
            picture: self.image(),
            id: self.id?,
            name: self.name,
            fans: self.nb_fan,
        })
    }
}

/// An artist on Deezer. `picture` may be a cover: Deezer shows one for an artist without a photo.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeezerArtist {
    pub id: u64,
    pub name: String,
    pub picture: Option<String>,
    pub fans: Option<u64>,
}

/// An `artist/{id}/albums` answer as the artist's releases. Deezer names no artist on them: the
/// artist is the page's.
pub fn artist_releases(body: Value) -> Result<Vec<Entry>> {
    #[derive(Deserialize)]
    struct Raw {
        id: Option<u64>,
        title: String,
        record_type: Option<String>,
        release_date: Option<String>,
        md5_image: Option<String>,
        explicit_lyrics: Option<bool>,
    }
    Ok(Page::<Raw>::of(body, "albums")?
        .into_iter()
        .map(|a| Entry {
            own: Some(true),
            explicit: a.explicit_lyrics.unwrap_or(false),
            origin: Origin::Deezer,
            album: AlbumHit {
                title: a.title,
                kind: a.record_type.as_deref().map(kind),
                date: a.release_date.filter(|d| !d.is_empty() && !d.starts_with("0000")),
                art: a.md5_image.as_deref().and_then(|m| picture("cover", m)),
                source: a.id.map(AlbumSource::Deezer),
                ..AlbumHit::default()
            },
        })
        .collect())
}

fn fans_key(id: &str) -> String {
    format!("dz:playlist-fans:{id}")
}

/// Fetches playlists' fans into the cache, one at a time; a failure is tried again next search.
async fn learn_fans(http: Http, cache: Cache, ids: Vec<String>) {
    for id in ids {
        http.window(DEEZER).await;
        let url = format!("{API}/playlist/{id}");
        let fans = match http.json::<Value>(SERVICE, |c| c.get(&url)).await.map(answer) {
            Ok(Ok(body)) => body["fans"].as_u64(),
            Ok(Err(Refusal::Quota)) => {
                http.close_window(DEEZER);
                continue;
            }
            Ok(Err(Refusal::Other(why))) => {
                tracing::info!(why, playlist = id, "a Deezer playlist's fans weren't fetched");
                continue;
            }
            Err(e) => {
                tracing::info!(error = %e, playlist = id, "a Deezer playlist's fans weren't fetched");
                continue;
            }
        };
        if let Err(e) = cache.put(fans_key(&id), &fans).await {
            tracing::warn!(error = %e, "a Deezer playlist's fans weren't kept");
        }
    }
}

/// A `search/playlist` answer as playlist rows, in Deezer's order.
pub fn playlists_of(body: Value) -> Result<Vec<PlaylistHit>> {
    #[derive(Deserialize)]
    struct Raw {
        id: u64,
        title: String,
        nb_tracks: Option<u32>,
        picture_xl: Option<String>,
        user: Option<Named>,
    }
    Ok(Page::<Raw>::of(body, "playlists")?
        .into_iter()
        .enumerate()
        .map(|(i, p)| PlaylistHit {
            site: "deezer".into(),
            id: p.id.to_string(),
            title: p.title,
            owner: p.user.map(|u| u.name).unwrap_or_default(),
            cover: p.picture_xl,
            tracks: p.nb_tracks,
            fans: None,
            rank: i as u32,
        })
        .collect())
}

pub fn songs(body: Value) -> Result<Vec<SongHit>> {
    Ok(Page::<RawTrack>::of(body, "tracks")?.into_iter().map(hit).collect())
}

fn hit(t: RawTrack) -> SongHit {
    SongHit {
        title: t.title,
        artist: t.artist.name,
        album: t.album.title,
        duration: t.duration.filter(|d| *d > 0),
        isrcs: t.isrc.into_iter().collect(),
        popularity: t.rank.map_or(0.0, |r| popularity(r, FULL_RANK)),
        art: t.album.md5_image.as_deref().and_then(|m| picture("cover", m)),
        album_source: t.album.id.map(AlbumSource::Deezer),
        ..SongHit::default()
    }
}

/// An `album/{id}/tracks` answer as the album's songs, in order. The album's own name and
/// artist are the page's to set.
pub fn tracklist(body: Value) -> Result<Vec<Song>> {
    #[derive(Deserialize)]
    struct Raw {
        title: String,
        duration: Option<u32>,
        track_position: Option<u32>,
        disk_number: Option<u32>,
        isrc: Option<String>,
        artist: RawNamed,
    }
    let raw = Page::<Raw>::of(body, "album tracks")?;
    let discs = raw.iter().any(|t| t.disk_number.is_some_and(|d| d > 1));
    let mut songs: Vec<Song> = raw
        .into_iter()
        .map(|t| Song {
            mbid: None,
            title: t.title,
            artist: t.artist.name,
            artist_mbid: None,
            album: String::new(),
            album_artist: None,
            album_mbid: None,
            year: None,
            no: t.track_position,
            disc: t.disk_number.filter(|_| discs),
            duration_s: t.duration.filter(|d| *d > 0),
            tags: Vec::new(),
            artist_source: Some(ArtistSource::Deezer(t.artist.id)),
            isrc: t.isrc.filter(|i| !i.is_empty()),
        })
        .collect();
    songs.sort_by_key(|s| (s.disc.unwrap_or(1), s.no.unwrap_or(u32::MAX)));
    Ok(songs)
}

pub fn albums_of(body: Value) -> Result<Vec<AlbumHit>> {
    let raw = Page::<RawAlbum>::of(body, "albums")?;
    let n = raw.len().max(1) as f64;
    // Album answers carry no count, so Deezer's own order (relevance, then popularity) stands in.
    Ok(raw
        .into_iter()
        .enumerate()
        .map(|(i, a)| album_hit(a, 0.6 * (1.0 - i as f64 / n)))
        .collect())
}

fn album_hit(a: RawAlbum, popularity: f64) -> AlbumHit {
    AlbumHit {
        title: a.title,
        artist: a.artist.name,
        kind: a.record_type.as_deref().map(kind),
        popularity,
        art: a.md5_image.as_deref().and_then(|m| picture("cover", m)),
        source: a.id.map(AlbumSource::Deezer),
        ..AlbumHit::default()
    }
}

pub fn artists_of(body: Value) -> Result<Vec<ArtistHit>> {
    Ok(Page::<RawArtist>::of(body, "artists")?
        .into_iter()
        .map(|a| ArtistHit {
            image: a.image(),
            popularity: a.nb_fan.map_or(0.0, |f| popularity(f, FULL_FANS)),
            audience: a.nb_fan,
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
