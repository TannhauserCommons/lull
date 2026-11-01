//! Last.fm reads (https://www.last.fm/api), only when an API key is configured: artist bios and
//! listener counts, similar artists, tag descriptions, tag charts and similar tags. Last.fm
//! publishes no number, but warns against several calls per second; lull keeps to two.

use std::sync::Arc;
use std::time::Duration;

use lull_core::config::Secret;
use lull_core::http::Http;
use lull_core::ids::{normalize, normalize_artist};
use lull_core::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::cache::{Cache, DAY};
use super::{ArtistInfo, Breaker, CALL_TIMEOUT, Song, Switch, top_tags};
use crate::search::merge::{ArtistHit, Found, SongHit, popularity};

const SERVICE: &str = "Last.fm";
const ROOT: &str = "https://ws.audioscrobbler.com/2.0/";
/// "The artist you supplied could not be found" and the like: an answer, not a failure.
const NOT_FOUND: u64 = 6;
const PACE: Duration = Duration::from_millis(500);
/// Listeners that score 1 on `popularity`'s scale, for a track and for an artist.
const FULL_TRACK: f64 = 5e6;
const FULL_ARTIST: f64 = 1e7;

pub struct LastFm {
    pub switch: Arc<Switch>,
    http: Http,
    cache: Cache,
    api_key: Secret,
    breaker: Breaker,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Profile {
    /// The artist's MBID as Last.fm has it: radio's way to ListenBrainz when MusicBrainz can't
    /// answer.
    #[serde(default)]
    pub mbid: Option<String>,
    pub listeners: Option<u64>,
    pub bio: Option<String>,
    pub tags: Vec<String>,
}

impl LastFm {
    pub fn new(http: Http, cache: Cache, api_key: Secret) -> Self {
        LastFm {
            switch: Switch::new(SERVICE),
            http,
            cache,
            api_key,
            breaker: Breaker::new(SERVICE),
        }
    }

    pub async fn artist(&self, name: &str) -> Result<Profile> {
        self.cache
            .get(format!("lfm:artist:{}", normalize_artist(name)), 7 * DAY, || async {
                let body = self
                    .call("artist.getInfo", &[("artist", name), ("autocorrect", "1")])
                    .await?;
                let a = &body["artist"];
                let counted = list(&a["tags"]["tag"])
                    .iter()
                    .filter_map(|t| Some((t["name"].as_str()?.to_owned(), 0)))
                    .collect();
                Ok(Profile {
                    mbid: a["mbid"].as_str().filter(|m| !m.is_empty()).map(str::to_owned),
                    listeners: a["stats"]["listeners"].as_str().and_then(|n| n.parse().ok()),
                    bio: a["bio"]["summary"].as_str().and_then(plain_text),
                    // Last.fm lists them most used first already; the stable sort keeps that.
                    tags: top_tags(counted, 8),
                })
            })
            .await
    }

    pub async fn similar_artists(&self, name: &str) -> Result<Vec<ArtistInfo>> {
        self.cache
            .get(format!("lfm:similar:{}", normalize_artist(name)), 7 * DAY, || async {
                let body = self
                    .call(
                        "artist.getSimilar",
                        &[("artist", name), ("autocorrect", "1"), ("limit", "30")],
                    )
                    .await?;
                Ok(artists(&body["similarartists"]["artist"]))
            })
            .await
    }

    /// Catalog search (ADR-0008): `track.search` and `artist.search`, for independent and
    /// scrobble-known music and listener counts. Never a source of art.
    pub async fn search(&self, q: &str) -> Result<Found> {
        self.cache
            .get(format!("lfm:search:{}", normalize(q)), DAY, || async {
                let tracks = self.call("track.search", &[("track", q), ("limit", "20")]).await?;
                let artists = self.call("artist.search", &[("artist", q), ("limit", "10")]).await?;
                Ok(found(&tracks, &artists))
            })
            .await
    }

    /// Radio without ListenBrainz: tracks listened to alongside this one.
    pub async fn similar_tracks(&self, artist: &str, title: &str) -> Result<Vec<Song>> {
        let key = format!("lfm:similar-tracks:{}:{}", normalize_artist(artist), normalize(title));
        self.cache
            .get(key, 7 * DAY, || async {
                let body = self
                    .call(
                        "track.getSimilar",
                        &[
                            ("artist", artist),
                            ("track", title),
                            ("autocorrect", "1"),
                            ("limit", "50"),
                        ],
                    )
                    .await?;
                Ok(songs(&body["similartracks"]["track"]))
            })
            .await
    }

    /// Radio without ListenBrainz: an artist's most played tracks.
    pub async fn artist_top_tracks(&self, name: &str) -> Result<Vec<Song>> {
        self.cache
            .get(
                format!("lfm:top-tracks:{}", normalize_artist(name)),
                7 * DAY,
                || async {
                    let body = self
                        .call(
                            "artist.getTopTracks",
                            &[("artist", name), ("autocorrect", "1"), ("limit", "10")],
                        )
                        .await?;
                    Ok(songs(&body["toptracks"]["track"]))
                },
            )
            .await
    }

    /// Radio without ListenBrainz: a tag's most played tracks.
    pub async fn tag_top_tracks(&self, tag: &str) -> Result<Vec<Song>> {
        self.cache
            .get(format!("lfm:tag-tracks:{tag}"), DAY, || async {
                let body = self.call("tag.getTopTracks", &[("tag", tag), ("limit", "50")]).await?;
                Ok(songs(&body["tracks"]["track"]))
            })
            .await
    }

    /// The tag's wiki summary, as plain text.
    pub async fn tag_description(&self, tag: &str) -> Result<Option<String>> {
        self.cache
            .get(format!("lfm:tag:{tag}"), 30 * DAY, || async {
                let body = self.call("tag.getInfo", &[("tag", tag)]).await?;
                Ok(body["tag"]["wiki"]["summary"].as_str().and_then(plain_text))
            })
            .await
    }

    pub async fn tag_artists(&self, tag: &str) -> Result<Vec<ArtistInfo>> {
        self.cache
            .get(format!("lfm:tag-artists:{tag}"), 7 * DAY, || async {
                let body = self.call("tag.getTopArtists", &[("tag", tag), ("limit", "24")]).await?;
                Ok(artists(&body["topartists"]["artist"]))
            })
            .await
    }

    pub async fn similar_tags(&self, tag: &str) -> Result<Vec<String>> {
        self.cache
            .get(format!("lfm:tag-similar:{tag}"), 30 * DAY, || async {
                let body = self.call("tag.getSimilar", &[("tag", tag)]).await?;
                Ok(list(&body["similartags"]["tag"])
                    .iter()
                    .filter_map(|t| t["name"].as_str().map(str::to_lowercase))
                    .collect())
            })
            .await
    }

    /// A paced read. "Not found" comes back as an empty object, so callers see no data.
    async fn call(&self, method: &str, params: &[(&str, &str)]) -> Result<Value> {
        let call = self.breaker.call()?;
        let answer = async {
            self.http.pace(SERVICE, PACE).await;
            let resp = self
                .http
                .send(SERVICE, |c| {
                    c.get(ROOT)
                        .query(&[
                            ("method", method),
                            ("api_key", self.api_key.expose()),
                            ("format", "json"),
                        ])
                        .query(params)
                        .timeout(CALL_TIMEOUT)
                })
                .await?;
            let status = resp.status();
            let body: Value = resp.json().await.unwrap_or(Value::Null);
            match body["error"].as_u64() {
                Some(NOT_FOUND) => Ok(Value::Object(Default::default())),
                Some(code) => Err(Error::upstream(
                    SERVICE,
                    format!("error {code}: {}", body["message"].as_str().unwrap_or("no message")),
                )),
                None if status.is_success() => Ok(body),
                None => Err(Error::upstream(SERVICE, format!("answered {status}"))),
            }
        }
        .await;
        call.end(answer)
    }
}

/// Last.fm gives a single item as an object and several as an array.
fn list(v: &Value) -> Vec<Value> {
    match v {
        Value::Array(a) => a.clone(),
        Value::Null => Vec::new(),
        other => vec![other.clone()],
    }
}

fn artists(v: &Value) -> Vec<ArtistInfo> {
    list(v)
        .iter()
        .filter_map(|a| {
            Some(ArtistInfo {
                name: a["name"].as_str()?.to_owned(),
                mbid: a["mbid"].as_str().filter(|m| !m.is_empty()).map(str::to_owned),
            })
        })
        .collect()
}

/// Track lists (similar, top) as songs. Last.fm names no release, so `album` is empty.
pub fn songs(v: &Value) -> Vec<Song> {
    list(v)
        .iter()
        .filter_map(|t| {
            let artist = t["artist"]["name"].as_str().or(t["artist"].as_str())?.to_owned();
            let seconds = t["duration"]
                .as_u64()
                .or_else(|| t["duration"].as_str().and_then(|d| d.parse().ok()))
                .filter(|d| *d > 0);
            Some(Song {
                mbid: t["mbid"].as_str().filter(|m| !m.is_empty()).map(str::to_owned),
                title: t["name"].as_str()?.to_owned(),
                artist_mbid: t["artist"]["mbid"]
                    .as_str()
                    .filter(|m| !m.is_empty())
                    .map(str::to_owned),
                artist,
                album: String::new(),
                album_artist: None,
                album_mbid: None,
                year: None,
                no: None,
                disc: None,
                duration_s: seconds.map(|d| d as u32),
                tags: Vec::new(),
            })
        })
        .collect()
}

/// `track.search` and `artist.search` answers as catalog rows.
pub fn found(tracks: &Value, artists: &Value) -> Found {
    let count = |v: &Value| v["listeners"].as_str().and_then(|n| n.parse::<u64>().ok());
    let mbid = |v: &Value| v["mbid"].as_str().filter(|m| !m.is_empty()).map(str::to_owned);
    Found {
        songs: list(&tracks["results"]["trackmatches"]["track"])
            .iter()
            .filter_map(|t| {
                Some(SongHit {
                    title: t["name"].as_str()?.to_owned(),
                    artist: t["artist"].as_str()?.to_owned(),
                    mbid: mbid(t),
                    popularity: count(t).map_or(0.0, |n| popularity(n, FULL_TRACK)),
                    lastfm_only: true,
                    ..SongHit::default()
                })
            })
            .collect(),
        artists: list(&artists["results"]["artistmatches"]["artist"])
            .iter()
            .filter_map(|a| {
                Some(ArtistHit {
                    name: a["name"].as_str()?.to_owned(),
                    mbid: mbid(a),
                    popularity: count(a).map_or(0.0, |n| popularity(n, FULL_ARTIST)),
                    image: None,
                    audience: count(a),
                    lastfm_only: true,
                })
            })
            .collect(),
        albums: Vec::new(),
        problem: None,
    }
}

/// Last.fm summaries are HTML ending in a "Read more on Last.fm" link: keep the prose.
fn plain_text(html: &str) -> Option<String> {
    let mut text = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => text.push(c),
            _ => {}
        }
    }
    let text = text.trim_end().trim_end_matches("Read more on Last.fm").trim();
    let text = text.trim_end_matches('.').trim_end().to_owned();
    (!text.is_empty()).then(|| format!("{text}."))
}
