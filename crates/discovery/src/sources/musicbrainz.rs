//! MusicBrainz (https://musicbrainz.org/doc/MusicBrainz_API): finding artists and release
//! groups by name, discographies, tracklists, tags, and recent releases by tag. The limit is
//! one request per second per IP on average; going over gets every request a 503.

use std::sync::Arc;

use lull_core::http::{self, Http};
use lull_core::ids::{normalize, normalize_artist};
use lull_core::{Error, Result};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use super::cache::{Cache, DAY, HOUR};
use super::{ArtistInfo, Breaker, CALL_TIMEOUT, Release, Song, Switch, top_tags, year};
use crate::search::merge::{AlbumHit, ArtistHit, Found, SongHit};

const SERVICE: &str = "MusicBrainz";
const WS: &str = "https://musicbrainz.org/ws/2";
/// Search scores below this are a different artist or release.
const MIN_SCORE: u8 = 90;

pub struct MusicBrainz {
    pub switch: Arc<Switch>,
    http: Http,
    cache: Cache,
    breaker: Breaker,
}

#[derive(Deserialize)]
struct Tag {
    name: String,
    #[serde(default)]
    count: i64,
}

#[derive(Deserialize)]
struct Credit {
    name: String,
    #[serde(default)]
    joinphrase: String,
    artist: CreditArtist,
}

#[derive(Deserialize)]
struct CreditArtist {
    id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
struct RawGroup {
    id: String,
    title: String,
    primary_type: Option<String>,
    first_release_date: Option<String>,
    #[serde(default)]
    artist_credit: Vec<Credit>,
    #[serde(default)]
    tags: Vec<Tag>,
    #[serde(default)]
    genres: Vec<Tag>,
    score: Option<u8>,
}

impl RawGroup {
    fn into_release(self, fallback_artist: Option<&ArtistInfo>) -> Release {
        let (artist, artist_mbid) = match credited(&self.artist_credit) {
            Some(c) => c,
            None => fallback_artist
                .map(|a| (a.name.clone(), a.mbid.clone()))
                .unwrap_or_default(),
        };
        Release {
            mbid: Some(self.id),
            title: self.title,
            artist,
            artist_mbid,
            kind: self.primary_type,
            date: self.first_release_date.filter(|d| !d.is_empty()),
            tags: tags(self.tags, self.genres),
        }
    }
}

/// "A & B" and the first artist's MBID, from an artist credit.
fn credited(credit: &[Credit]) -> Option<(String, Option<String>)> {
    let first = credit.first()?;
    let name = credit.iter().map(|c| format!("{}{}", c.name, c.joinphrase)).collect();
    Some((name, Some(first.artist.id.clone())))
}

fn tags(tags: Vec<Tag>, genres: Vec<Tag>) -> Vec<String> {
    top_tags(genres.into_iter().chain(tags).map(|t| (t.name, t.count)).collect(), 8)
}

/// A Lucene phrase, quotes and backslashes escaped.
fn phrase(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

impl MusicBrainz {
    pub fn new(http: Http, cache: Cache) -> Self {
        MusicBrainz {
            switch: Switch::new(SERVICE),
            http,
            cache,
            breaker: Breaker::new(SERVICE),
        }
    }

    /// The artist called `name`, when MusicBrainz is sure enough.
    pub async fn find_artist(&self, name: &str) -> Result<Option<ArtistInfo>> {
        self.switch.check()?;
        #[derive(Deserialize)]
        struct Found {
            artists: Vec<Raw>,
        }
        #[derive(Deserialize)]
        struct Raw {
            id: String,
            name: String,
            score: Option<u8>,
        }
        let key = format!("mb:find-artist:{}", normalize_artist(name));
        self.cache
            .get(key, 30 * DAY, || async {
                let found: Found = self
                    .get(
                        "artist",
                        &[("query", &format!("artist:{}", phrase(name))), ("limit", "5")],
                    )
                    .await?;
                Ok(found
                    .artists
                    .into_iter()
                    .filter(|a| a.score.unwrap_or(0) >= MIN_SCORE)
                    .find(|a| normalize_artist(&a.name) == normalize_artist(name))
                    .map(|a| ArtistInfo {
                        mbid: Some(a.id),
                        name: a.name,
                    }))
            })
            .await
    }

    /// The artist's tags and genres, most used first.
    pub async fn artist_tags(&self, mbid: &str) -> Result<Vec<String>> {
        self.switch.check()?;
        #[derive(Deserialize)]
        struct Raw {
            #[serde(default)]
            tags: Vec<Tag>,
            #[serde(default)]
            genres: Vec<Tag>,
        }
        self.cache
            .get(format!("mb:artist-tags:{mbid}"), 30 * DAY, || async {
                let raw: Raw = self.get(&format!("artist/{mbid}"), &[("inc", "tags genres")]).await?;
                Ok(tags(raw.tags, raw.genres))
            })
            .await
    }

    /// Albums, EPs and singles credited to the artist, newest first.
    pub async fn discography(&self, artist: &ArtistInfo) -> Result<Vec<Release>> {
        self.switch.check()?;
        #[derive(Deserialize)]
        struct Found {
            #[serde(rename = "release-groups")]
            groups: Vec<RawGroup>,
        }
        let Some(mbid) = &artist.mbid else {
            return Ok(Vec::new());
        };
        let mut releases: Vec<Release> = self
            .cache
            .get(format!("mb:discography:{mbid}"), 7 * DAY, || async {
                let found: Found = self
                    .get(
                        "release-group",
                        &[("artist", mbid), ("type", "album|ep|single"), ("limit", "100")],
                    )
                    .await?;
                Ok(found.groups.into_iter().map(|g| g.into_release(Some(artist))).collect())
            })
            .await?;
        releases.sort_by(|a, b| b.date.cmp(&a.date));
        Ok(releases)
    }

    /// The release group `title` by `artist`, when MusicBrainz is sure enough.
    pub async fn find_release(&self, artist: &str, title: &str) -> Result<Option<Release>> {
        self.switch.check()?;
        #[derive(Deserialize)]
        struct Found {
            #[serde(rename = "release-groups")]
            groups: Vec<RawGroup>,
        }
        let key = format!("mb:find-release:{}:{}", normalize_artist(artist), normalize(title));
        self.cache
            .get(key, 30 * DAY, || async {
                let query = format!("releasegroup:{} AND artist:{}", phrase(title), phrase(artist));
                let found: Found = self.get("release-group", &[("query", &query), ("limit", "5")]).await?;
                Ok(found
                    .groups
                    .into_iter()
                    .filter(|g| g.score.unwrap_or(0) >= MIN_SCORE && normalize(&g.title) == normalize(title))
                    .map(|g| g.into_release(None))
                    .next())
            })
            .await
    }

    /// One release group, with its tags.
    pub async fn release(&self, mbid: &str) -> Result<Release> {
        self.switch.check()?;
        self.cache
            .get(format!("mb:release:{mbid}"), 30 * DAY, || async {
                let raw: RawGroup = self
                    .get(
                        &format!("release-group/{mbid}"),
                        &[("inc", "artist-credits tags genres")],
                    )
                    .await?;
                Ok(raw.into_release(None))
            })
            .await
    }

    /// The tracks of a release group's earliest official release.
    pub async fn tracklist(&self, group: &Release) -> Result<Vec<Song>> {
        self.switch.check()?;
        #[derive(Deserialize)]
        struct Found {
            releases: Vec<RawRelease>,
        }
        #[derive(Deserialize)]
        struct RawRelease {
            date: Option<String>,
            #[serde(default)]
            media: Vec<Medium>,
        }
        #[derive(Deserialize)]
        struct Medium {
            position: Option<u32>,
            #[serde(default)]
            tracks: Vec<RawTrack>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "kebab-case")]
        struct RawTrack {
            position: Option<u32>,
            title: String,
            length: Option<u64>,
            recording: Recording,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "kebab-case")]
        struct Recording {
            id: String,
            #[serde(default)]
            artist_credit: Vec<Credit>,
        }
        let Some(mbid) = &group.mbid else { return Ok(Vec::new()) };
        self.cache
            .get(format!("mb:tracklist:{mbid}"), 30 * DAY, || async {
                let found: Found = self
                    .get(
                        "release",
                        &[
                            ("release-group", mbid),
                            ("status", "official"),
                            ("inc", "recordings artist-credits media"),
                            ("limit", "25"),
                        ],
                    )
                    .await?;
                // Undated releases sort last.
                let first = found
                    .releases
                    .into_iter()
                    .filter(|r| !r.media.is_empty())
                    .min_by_key(|r| {
                        r.date
                            .clone()
                            .filter(|d| !d.is_empty())
                            .unwrap_or_else(|| "9999".into())
                    });
                let Some(release) = first else { return Ok(Vec::new()) };
                let discs = release.media.len();
                Ok(release
                    .media
                    .into_iter()
                    .flat_map(|m| {
                        let disc = m.position.filter(|_| discs > 1);
                        m.tracks.into_iter().map(move |t| (disc, t))
                    })
                    .map(|(disc, t)| {
                        let (artist, artist_mbid) = credited(&t.recording.artist_credit)
                            .unwrap_or_else(|| (group.artist.clone(), group.artist_mbid.clone()));
                        Song {
                            mbid: Some(t.recording.id),
                            title: t.title,
                            artist,
                            artist_mbid,
                            album: group.title.clone(),
                            album_artist: Some(group.artist.clone()),
                            album_mbid: group.mbid.clone(),
                            year: group.date.as_deref().and_then(year),
                            no: t.position,
                            disc,
                            duration_s: t.length.map(|ms| (ms / 1000) as u32),
                            tags: group.tags.clone(),
                        }
                    })
                    .collect())
            })
            .await
    }

    /// Release groups tagged `tag` first released on or after `since` (YYYY-MM-DD).
    pub async fn tagged_releases(&self, tag: &str, since: &str) -> Result<Vec<Release>> {
        self.switch.check()?;
        #[derive(Deserialize)]
        struct Found {
            #[serde(rename = "release-groups")]
            groups: Vec<RawGroup>,
        }
        self.cache
            .get(format!("mb:tagged:{tag}:{since}"), 12 * HOUR, || async {
                let query = format!("tag:{} AND firstreleasedate:[{since} TO *]", phrase(tag));
                let found: Found = self.get("release-group", &[("query", &query), ("limit", "50")]).await?;
                let mut releases: Vec<Release> = found.groups.into_iter().map(|g| g.into_release(None)).collect();
                releases.sort_by(|a, b| b.date.cmp(&a.date));
                Ok(releases)
            })
            .await
    }

    /// Catalog search (ADR-0008): one recording search per query, for its songs and the release
    /// groups and artists they name. Recordings where artist and title together hold the words
    /// score highest; a title or an artist that is the whole query also counts.
    pub async fn search(&self, q: &str) -> Result<Found> {
        self.switch.check()?;
        // Letters and digits only, so nothing in it is Lucene syntax.
        let words = normalize(q);
        if words.is_empty() {
            return Ok(Found::default());
        }
        self.cache
            .get(format!("mb:search:{words}"), DAY, || async {
                let query = format!(
                    "(recording:({words}) AND artist:({words})) OR recording:\"{words}\" OR artist:\"{words}\""
                );
                recordings(self.get("recording", &[("query", &query), ("limit", "25")]).await?)
            })
            .await
    }

    /// Catalog search for a submitted query: release groups by title, so EPs and albums only
    /// MusicBrainz knows (Bandcamp releases) are found by name. Paced after the recording search.
    pub async fn search_releases(&self, q: &str) -> Result<Found> {
        self.switch.check()?;
        let words = normalize(q);
        if words.is_empty() {
            return Ok(Found::default());
        }
        self.cache
            .get(format!("mb:search-releases:{words}"), DAY, || async {
                let query = format!("releasegroup:\"{words}\" OR releasegroup:({words})");
                release_groups(self.get("release-group", &[("query", &query), ("limit", "15")]).await?)
            })
            .await
    }

    async fn get<T: DeserializeOwned>(&self, path: &str, query: &[(&str, &str)]) -> Result<T> {
        let call = self.breaker.call()?;
        self.http.pace(http::MUSICBRAINZ, http::MUSICBRAINZ_PACE).await;
        let answer = self
            .http
            .json(SERVICE, |c| {
                c.get(format!("{WS}/{path}"))
                    .query(query)
                    .query(&[("fmt", "json")])
                    .timeout(CALL_TIMEOUT)
            })
            .await;
        call.end(answer).map_err(|e| match e {
            Error::NotFound(_) => Error::NotFound("MusicBrainz doesn't know it".into()),
            e => e,
        })
    }
}

/// "album", "EP", "single", "compilation" from a release group's types.
fn release_kind(primary: Option<&str>, secondary: &[String]) -> Option<String> {
    if secondary.iter().any(|t| t == "Compilation") {
        return Some("compilation".into());
    }
    primary.map(|t| match t {
        "EP" => "EP".to_owned(),
        other => other.to_lowercase(),
    })
}

/// A release-group search's answer as album rows, with Cover Art Archive covers.
pub fn release_groups(body: serde_json::Value) -> Result<Found> {
    #[derive(Deserialize)]
    struct Found_ {
        #[serde(rename = "release-groups")]
        groups: Vec<Group>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "kebab-case")]
    struct Group {
        id: String,
        title: String,
        primary_type: Option<String>,
        #[serde(default)]
        secondary_types: Vec<String>,
        first_release_date: Option<String>,
        #[serde(default)]
        artist_credit: Vec<Credit>,
    }
    let found: Found_ =
        serde_json::from_value(body).map_err(|e| Error::upstream(SERVICE, format!("unexpected search answer: {e}")))?;
    Ok(Found {
        albums: found
            .groups
            .into_iter()
            .filter_map(|g| {
                let (artist, _) = credited(&g.artist_credit)?;
                Some(AlbumHit {
                    kind: release_kind(g.primary_type.as_deref(), &g.secondary_types),
                    date: g.first_release_date.filter(|d| !d.is_empty()),
                    art: Some(crate::art::cover_archive(&g.id)),
                    mbid: Some(g.id),
                    title: g.title,
                    artist,
                    popularity: 0.0,
                })
            })
            .collect(),
        ..Found::default()
    })
}

/// A recording search's answer as catalog rows. Each song is placed on its first official album
/// (else EP or single, else any release); covers come from the Cover Art Archive.
pub fn recordings(body: serde_json::Value) -> Result<Found> {
    #[derive(Deserialize)]
    struct Found_ {
        recordings: Vec<Recording>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "kebab-case")]
    struct Recording {
        id: String,
        title: String,
        length: Option<u64>,
        #[serde(default)]
        artist_credit: Vec<Credit>,
        #[serde(default)]
        isrcs: Vec<String>,
        #[serde(default)]
        releases: Vec<RawRelease>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "kebab-case")]
    struct RawRelease {
        title: String,
        status: Option<String>,
        date: Option<String>,
        #[serde(default)]
        artist_credit: Vec<Credit>,
        release_group: Option<Group>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "kebab-case")]
    struct Group {
        id: String,
        primary_type: Option<String>,
        #[serde(default)]
        secondary_types: Vec<String>,
    }
    /// Lower is a better home for a song: an official studio album or soundtrack first (a
    /// soundtrack is where its songs come from), compilations and live releases last.
    fn preference(r: &RawRelease) -> (u8, String) {
        let official = r.status.as_deref() == Some("Official");
        let original = |g: &Group| {
            g.secondary_types.iter().all(|t| {
                t == "Soundtrack" || (t == "Compilation" && g.secondary_types.iter().any(|t| t == "Soundtrack"))
            })
        };
        let class = match &r.release_group {
            Some(g) if original(g) => match g.primary_type.as_deref() {
                Some("Album") => 0,
                Some("EP" | "Single") => 1,
                _ => 2,
            },
            _ => 3,
        };
        let date = r
            .date
            .clone()
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| "9999".into());
        (class + if official { 0 } else { 4 }, date)
    }
    fn kind(g: &Group) -> Option<String> {
        release_kind(g.primary_type.as_deref(), &g.secondary_types)
    }

    let found: Found_ =
        serde_json::from_value(body).map_err(|e| Error::upstream(SERVICE, format!("unexpected search answer: {e}")))?;
    let mut out = Found::default();
    for r in found.recordings {
        let Some((artist, artist_mbid)) = credited(&r.artist_credit) else {
            continue;
        };
        let home = r.releases.iter().min_by_key(|rel| preference(rel));
        let group = home.and_then(|h| h.release_group.as_ref());
        let art = group.map(|g| crate::art::cover_archive(&g.id));
        let album_artist = home
            .and_then(|h| credited(&h.artist_credit))
            .map(|(name, _)| name)
            .filter(|name| *name != artist);
        if let (Some(home), Some(group)) = (home, group) {
            out.albums.push(AlbumHit {
                title: home.title.clone(),
                artist: album_artist.clone().unwrap_or_else(|| artist.clone()),
                kind: kind(group),
                date: home.date.clone().filter(|d| !d.is_empty()),
                mbid: Some(group.id.clone()),
                popularity: 0.0,
                art: art.clone(),
            });
        }
        if !out.artists.iter().any(|a| a.mbid == artist_mbid) {
            out.artists.push(ArtistHit {
                name: artist.clone(),
                mbid: artist_mbid,
                popularity: 0.0,
                image: None,
                audience: None,
                lastfm_only: false,
            });
        }
        out.songs.push(SongHit {
            saved: None,
            title: r.title,
            album: home.map(|h| h.title.clone()).unwrap_or_default(),
            album_kind: group.and_then(kind),
            year: home.and_then(|h| h.date.as_deref()).and_then(year),
            duration: r.length.map(|ms| (ms / 1000) as u32).filter(|s| *s > 0),
            isrcs: r.isrcs,
            mbid: Some(r.id),
            popularity: 0.0,
            art,
            lastfm_only: false,
            artist,
            album_artist,
        });
    }
    Ok(out)
}
