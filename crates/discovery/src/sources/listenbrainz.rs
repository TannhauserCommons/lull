//! ListenBrainz (https://listenbrainz.readthedocs.io): fresh releases, collaborative-filtering
//! recommendations, tag and artist radio, similar artists (labs API) and batch recording
//! metadata. Clients must stay under one call per second (429 beyond that).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use lull_core::config::Secret;
use lull_core::http::{self, Http};
use lull_core::{Error, Result};
use reqwest::StatusCode;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::cache::{Cache, DAY, HOUR};
use super::{Breaker, CALL_TIMEOUT, Release, Song, Switch, top_tags};

const SERVICE: &str = "ListenBrainz";
const API: &str = "https://api.listenbrainz.org/1";
const LABS: &str = "https://labs.api.listenbrainz.org";
/// The similarity model ListenBrainz's own explore pages use.
const SIMILARITY: &str =
    "session_based_days_7500_session_300_contribution_5_threshold_10_limit_100_filter_True_skip_30";
/// Recording metadata is asked for in batches of this size.
const METADATA_BATCH: usize = 50;
/// "No more than one call per second."
const PACE: Duration = Duration::from_secs(1);
/// Fresh releases are one 3-4 MB answer (a month of releases, about 8,500), fetched in the
/// background and cached for hours: CALL_TIMEOUT, meant for calls a listener waits on, cut it off
/// mid-body ("error decoding response body").
const FEED_TIMEOUT: Duration = Duration::from_secs(30);

pub struct ListenBrainz {
    pub switch: Arc<Switch>,
    http: Http,
    cache: Cache,
    token: Option<Secret>,
    breaker: Breaker,
}

/// How far LB Radio strays from the seed: ListenBrainz's own easy / medium / hard.
#[derive(Debug, Clone, Copy)]
pub enum Mode {
    Easy,
    Medium,
    Hard,
}

impl Mode {
    fn as_str(self) -> &'static str {
        match self {
            Mode::Easy => "easy",
            Mode::Medium => "medium",
            Mode::Hard => "hard",
        }
    }
}

/// A popularity window in percent (0 = obscure, 100 = the most played).
pub type Popularity = (u8, u8);

/// A recording picked by artist radio, with the (similar) artist it was picked for.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RadioPick {
    pub recording_mbid: String,
    pub artist_mbid: String,
    pub artist: String,
    /// All-time listens on ListenBrainz.
    pub listens: u64,
}

impl ListenBrainz {
    pub fn new(http: Http, cache: Cache, token: Option<Secret>) -> Self {
        ListenBrainz {
            switch: Switch::new(SERVICE),
            http,
            cache,
            token,
            breaker: Breaker::new(SERVICE),
        }
    }

    /// Releases of the last 30 days, every genre and size (about ten thousand), with tags.
    pub async fn fresh_releases(&self) -> Result<Vec<Release>> {
        self.switch.check()?;
        #[derive(Deserialize)]
        struct Raw {
            artist_credit_name: String,
            #[serde(default)]
            artist_mbids: Vec<String>,
            release_group_mbid: Option<String>,
            release_name: String,
            release_date: Option<String>,
            release_group_primary_type: Option<String>,
            #[serde(default)]
            release_tags: Vec<String>,
        }
        self.cache
            .get("lb:fresh:30".into(), 6 * HOUR, || async {
                let body: Value = self
                    .get_within(
                        format!("{API}/explore/fresh-releases/"),
                        &[("days", "30"), ("future", "false")],
                        FEED_TIMEOUT,
                    )
                    .await?
                    .unwrap_or_default();
                let raw: Vec<Raw> = serde_json::from_value(body["payload"]["releases"].clone()).map_err(bad)?;
                Ok(raw
                    .into_iter()
                    .map(|r| Release {
                        mbid: r.release_group_mbid,
                        title: r.release_name,
                        artist: r.artist_credit_name,
                        artist_mbid: r.artist_mbids.into_iter().next(),
                        kind: r.release_group_primary_type,
                        date: r.release_date,
                        tags: r.release_tags.iter().map(|t| t.to_lowercase()).collect(),
                    })
                    .collect())
            })
            .await
    }

    /// Recordings ListenBrainz recommends for `user` from listeners like them (best first).
    pub async fn recommended(&self, user: &str) -> Result<Vec<String>> {
        self.switch.check()?;
        self.cache
            .get(format!("lb:cf:{user}"), DAY, || async {
                let body: Option<Value> = self
                    .get(
                        format!("{API}/cf/recommendation/user/{user}/recording"),
                        &[("count", "100")],
                    )
                    .await?;
                // 204: nothing generated for this user yet.
                let Some(body) = body else { return Ok(Vec::new()) };
                Ok(body["payload"]["mbids"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|m| m["recording_mbid"].as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default())
            })
            .await
    }

    /// Recordings tagged `tag` (on the recording, release or artist) within a popularity window.
    pub async fn tag_radio(&self, tag: &str, pop: Popularity, count: u32) -> Result<Vec<String>> {
        self.switch.check()?;
        #[derive(Deserialize)]
        struct Raw {
            recording_mbid: String,
        }
        let key = format!("lb:tag:{tag}:{}-{}:{count}", pop.0, pop.1);
        self.cache
            .get(key, DAY, || async {
                let (b, e, n) = (pop.0.to_string(), pop.1.to_string(), count.to_string());
                let raw: Vec<Raw> = self
                    .get(
                        format!("{API}/lb-radio/tags"),
                        &[("tag", tag), ("pop_begin", &b), ("pop_end", &e), ("count", &n)],
                    )
                    .await?
                    .unwrap_or_default();
                Ok(raw.into_iter().map(|r| r.recording_mbid).collect())
            })
            .await
    }

    /// Recordings by the artist and by artists similar to them (`similar` = 0: the artist only).
    pub async fn artist_radio(
        &self,
        artist_mbid: &str,
        mode: Mode,
        similar: u32,
        per_artist: u32,
        pop: Popularity,
    ) -> Result<Vec<RadioPick>> {
        self.switch.check()?;
        #[derive(Deserialize)]
        struct Raw {
            recording_mbid: String,
            similar_artist_mbid: String,
            similar_artist_name: String,
            #[serde(default)]
            total_listen_count: u64,
        }
        let key = format!(
            "lb:artist:{artist_mbid}:{}:{similar}:{per_artist}:{}-{}",
            mode.as_str(),
            pop.0,
            pop.1
        );
        self.cache
            .get(key, DAY, || async {
                let (s, n, b, e) = (
                    similar.to_string(),
                    per_artist.to_string(),
                    pop.0.to_string(),
                    pop.1.to_string(),
                );
                let raw: HashMap<String, Vec<Raw>> = self
                    .get(
                        format!("{API}/lb-radio/artist/{artist_mbid}"),
                        &[
                            ("mode", mode.as_str()),
                            ("max_similar_artists", &s),
                            ("max_recordings_per_artist", &n),
                            ("pop_begin", &b),
                            ("pop_end", &e),
                        ],
                    )
                    .await?
                    .unwrap_or_default();
                Ok(raw
                    .into_values()
                    .flatten()
                    .map(|r| RadioPick {
                        recording_mbid: r.recording_mbid,
                        artist_mbid: r.similar_artist_mbid,
                        artist: r.similar_artist_name,
                        listens: r.total_listen_count,
                    })
                    .collect())
            })
            .await
    }

    /// Artists whose listeners overlap with this one's, most similar first.
    pub async fn similar_artists(&self, artist_mbid: &str) -> Result<Vec<super::ArtistInfo>> {
        self.switch.check()?;
        #[derive(Deserialize)]
        struct Raw {
            artist_mbid: String,
            name: String,
        }
        self.cache
            .get(format!("lb:similar:{artist_mbid}"), 7 * DAY, || async {
                let raw: Vec<Raw> = self
                    .get(
                        format!("{LABS}/similar-artists/json"),
                        &[("artist_mbids", artist_mbid), ("algorithm", SIMILARITY)],
                    )
                    .await?
                    .unwrap_or_default();
                Ok(raw
                    .into_iter()
                    .filter(|r| r.artist_mbid != artist_mbid)
                    .map(|r| super::ArtistInfo {
                        mbid: Some(r.artist_mbid),
                        name: r.name,
                    })
                    .collect())
            })
            .await
    }

    /// Title, artist, release, length and tags for recordings, in the order asked; recordings
    /// ListenBrainz doesn't know are left out.
    pub async fn songs(&self, mbids: &[String]) -> Result<Vec<Song>> {
        self.switch.check()?;
        let mut found: HashMap<String, Song> = HashMap::new();
        let mut missing = Vec::new();
        for mbid in mbids {
            match self.cache.fresh::<Song>(song_key(mbid), 30 * DAY).await? {
                Some(song) => {
                    found.insert(mbid.clone(), song);
                }
                None => missing.push(mbid.clone()),
            }
        }
        for chunk in missing.chunks(METADATA_BATCH) {
            let ids = chunk.join(",");
            let body: HashMap<String, Value> = self
                .get(
                    format!("{API}/metadata/recording/"),
                    &[("recording_mbids", &ids), ("inc", "artist release tag")],
                )
                .await?
                .unwrap_or_default();
            for (mbid, meta) in body {
                if let Some(song) = song(&mbid, &meta) {
                    self.cache.put(song_key(&mbid), &song).await?;
                    found.insert(mbid, song);
                }
            }
        }
        Ok(mbids.iter().filter_map(|m| found.remove(m)).collect())
    }

    /// One paced GET; `None` for 204 (ListenBrainz's "nothing computed yet").
    async fn get<T: DeserializeOwned>(&self, url: String, query: &[(&str, &str)]) -> Result<Option<T>> {
        self.get_within(url, query, CALL_TIMEOUT).await
    }

    async fn get_within<T: DeserializeOwned>(
        &self,
        url: String,
        query: &[(&str, &str)],
        timeout: Duration,
    ) -> Result<Option<T>> {
        let call = self.breaker.call()?;
        let answer = async {
            self.http.pace(SERVICE, PACE).await;
            let resp = self
                .http
                .send(SERVICE, |c| {
                    let req = c.get(&url).query(query).timeout(timeout);
                    match &self.token {
                        Some(t) => req.header("Authorization", format!("Token {}", t.expose())),
                        None => req,
                    }
                })
                .await?;
            if resp.status() == StatusCode::NO_CONTENT {
                return Ok(None);
            }
            http::ok(SERVICE, resp)?.json().await.map(Some).map_err(bad)
        }
        .await;
        call.end(answer)
    }
}

fn song_key(mbid: &str) -> String {
    format!("lb:rec:{mbid}")
}

/// One entry of `/metadata/recording`.
fn song(mbid: &str, meta: &Value) -> Option<Song> {
    let title = meta["recording"]["name"].as_str()?.to_owned();
    let artist = meta["artist"]["name"].as_str()?.to_owned();
    let release = &meta["release"];
    let counted = ["recording", "release_group", "artist"]
        .iter()
        .flat_map(|k| meta["tag"][k].as_array().cloned().unwrap_or_default())
        .filter_map(|t| Some((t["tag"].as_str()?.to_owned(), t["count"].as_i64().unwrap_or(0))))
        .collect();
    Some(Song {
        mbid: Some(mbid.to_owned()),
        title,
        artist,
        artist_mbid: meta["artist"]["artists"][0]["artist_mbid"].as_str().map(str::to_owned),
        album: release["name"].as_str().unwrap_or_default().to_owned(),
        album_artist: release["album_artist_name"].as_str().map(str::to_owned),
        album_mbid: release["release_group_mbid"].as_str().map(str::to_owned),
        year: release["year"].as_u64().and_then(|y| u16::try_from(y).ok()),
        no: None,
        disc: None,
        duration_s: meta["recording"]["length"].as_u64().map(|ms| (ms / 1000) as u32),
        tags: top_tags(counted, 6),
        artist_source: None,
        isrc: None,
    })
}

fn bad(e: impl std::fmt::Display) -> Error {
    Error::upstream(SERVICE, format!("unexpected answer: {e}"))
}
