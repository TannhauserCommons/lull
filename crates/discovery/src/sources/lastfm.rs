//! Last.fm reads (https://www.last.fm/api), only when an API key is configured: artist bios and
//! listener counts, similar artists, tag descriptions, tag charts and similar tags. Last.fm
//! publishes no number, but warns against several calls per second; lull keeps to two.

use std::sync::Arc;
use std::time::Duration;

use lull_core::config::Secret;
use lull_core::http::Http;
use lull_core::ids::normalize_artist;
use lull_core::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::cache::{Cache, DAY};
use super::{ArtistInfo, Switch, top_tags};

const SERVICE: &str = "Last.fm";
const ROOT: &str = "https://ws.audioscrobbler.com/2.0/";
/// "The artist you supplied could not be found" and the like: an answer, not a failure.
const NOT_FOUND: u64 = 6;
const PACE: Duration = Duration::from_millis(500);

pub struct LastFm {
    pub switch: Arc<Switch>,
    http: Http,
    cache: Cache,
    api_key: Secret,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Profile {
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
