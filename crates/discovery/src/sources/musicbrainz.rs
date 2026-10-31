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
use super::{ArtistInfo, Release, Song, Switch, top_tags, year};

const SERVICE: &str = "MusicBrainz";
const WS: &str = "https://musicbrainz.org/ws/2";
/// Search scores below this are a different artist or release.
const MIN_SCORE: u8 = 90;

pub struct MusicBrainz {
    pub switch: Arc<Switch>,
    http: Http,
    cache: Cache,
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

    async fn get<T: DeserializeOwned>(&self, path: &str, query: &[(&str, &str)]) -> Result<T> {
        self.http.pace(http::MUSICBRAINZ, http::MUSICBRAINZ_PACE).await;
        self.http
            .json(SERVICE, |c| {
                c.get(format!("{WS}/{path}")).query(query).query(&[("fmt", "json")])
            })
            .await
            .map_err(|e| match e {
                Error::NotFound(_) => Error::NotFound("MusicBrainz doesn't know it".into()),
                e => e,
            })
    }
}
