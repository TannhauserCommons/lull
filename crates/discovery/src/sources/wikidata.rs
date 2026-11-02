//! Wikidata's query service (https://query.wikidata.org): an artist's photo on Wikimedia Commons
//! (P18) and its iTunes (P2850) and Deezer (P2722) ids, found by MusicBrainz artist id (P434).
//! Every artist of a page goes in one query. The service limits by processing time per minute;
//! lull sends at most one query a second, and keeps each answer a month.

use std::collections::HashMap;
use std::time::Duration;

use lull_core::http::Http;
use lull_core::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::cache::{Cache, DAY};
use super::{Breaker, CALL_TIMEOUT};

const SERVICE: &str = "Wikidata";
const SPARQL: &str = "https://query.wikidata.org/sparql";
const PACE: Duration = Duration::from_secs(1);
const KEEP_S: i64 = 30 * DAY;

pub struct Wikidata {
    http: Http,
    cache: Cache,
    breaker: Breaker,
}

/// What Wikidata knows of an artist; all `None` when it has no item for the MBID.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WikiArtist {
    /// A Commons photo at any width (`{size}`).
    pub image: Option<String>,
    pub itunes: Option<u64>,
    pub deezer: Option<u64>,
}

impl Wikidata {
    pub fn new(http: Http, cache: Cache) -> Self {
        Wikidata {
            http,
            cache,
            breaker: Breaker::new(SERVICE),
        }
    }

    /// The artists behind `mbids`: cached ones at once, the rest in one query.
    pub async fn artists(&self, mbids: &[String]) -> Result<HashMap<String, WikiArtist>> {
        let mut known = HashMap::new();
        let mut missing = Vec::new();
        for mbid in mbids.iter().filter(|m| is_mbid(m)) {
            match self.cache.fresh::<WikiArtist>(key(mbid), KEEP_S).await? {
                Some(found) => {
                    known.insert(mbid.clone(), found);
                }
                None => missing.push(mbid.clone()),
            }
        }
        if missing.is_empty() {
            return Ok(known);
        }
        let found = self.query(&missing).await?;
        for mbid in missing {
            let artist = found.get(&mbid).cloned().unwrap_or_default();
            self.cache.put(key(&mbid), &artist).await?;
            known.insert(mbid, artist);
        }
        Ok(known)
    }

    async fn query(&self, mbids: &[String]) -> Result<HashMap<String, WikiArtist>> {
        let call = self.breaker.call()?;
        let answer = async {
            self.http.pace(SERVICE, PACE).await;
            let q = sparql(mbids);
            let body: Value = self
                .http
                .json(SERVICE, |c| {
                    c.get(SPARQL)
                        .query(&[("query", q.as_str()), ("format", "json")])
                        .header("Accept", "application/sparql-results+json")
                        .timeout(CALL_TIMEOUT)
                })
                .await?;
            bindings(&body)
        }
        .await;
        call.end(answer)
    }
}

fn key(mbid: &str) -> String {
    format!("wd:artist:{mbid}")
}

/// MBIDs are UUIDs; anything else never reaches the query.
fn is_mbid(s: &str) -> bool {
    s.len() == 36 && s.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
}

fn sparql(mbids: &[String]) -> String {
    let values: Vec<String> = mbids.iter().map(|m| format!("\"{m}\"")).collect();
    format!(
        "SELECT ?mbid ?img ?apple ?deezer WHERE {{ VALUES ?mbid {{ {} }} ?item wdt:P434 ?mbid . \
         OPTIONAL {{ ?item wdt:P18 ?img }} OPTIONAL {{ ?item wdt:P2850 ?apple }} OPTIONAL {{ ?item wdt:P2722 ?deezer }} }}",
        values.join(" ")
    )
}

/// The query's rows per MBID; an artist with several photos keeps the first.
pub fn bindings(body: &Value) -> Result<HashMap<String, WikiArtist>> {
    let rows = body["results"]["bindings"]
        .as_array()
        .ok_or_else(|| Error::upstream(SERVICE, "its answer has no results"))?;
    let mut out: HashMap<String, WikiArtist> = HashMap::new();
    for row in rows {
        let Some(mbid) = row["mbid"]["value"].as_str() else {
            continue;
        };
        let value = |k: &str| row[k]["value"].as_str();
        let a = out.entry(mbid.to_owned()).or_default();
        a.image = a.image.take().or_else(|| value("img").and_then(commons));
        a.itunes = a.itunes.or_else(|| value("apple").and_then(|v| v.parse().ok()));
        a.deezer = a.deezer.or_else(|| value("deezer").and_then(|v| v.parse().ok()));
    }
    Ok(out)
}

/// `http://commons.wikimedia.org/wiki/Special:FilePath/<file>` at any width: Commons answers
/// with a redirect to a thumbnail.
fn commons(url: &str) -> Option<String> {
    let file = url.split("/Special:FilePath/").nth(1).filter(|f| !f.is_empty())?;
    Some(format!(
        "https://commons.wikimedia.org/wiki/Special:FilePath/{file}?width={{size}}"
    ))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::bindings;

    #[test]
    fn a_photo_and_ids_come_from_the_query_rows() {
        // Live, 2026-11-02: Radiohead and Şivan Perwer have photos; BIJI has no Wikidata item.
        let body = json!({ "results": { "bindings": [
            { "mbid": { "value": "a74b1b7f-71a5-4011-9441-d0b5e4122711" },
              "img": { "value": "http://commons.wikimedia.org/wiki/Special:FilePath/RadioheadO2211125%20composite.jpg" },
              "apple": { "value": "657515" }, "deezer": { "value": "399" } },
            { "mbid": { "value": "9c21c0c1-b242-49a3-b676-cc9a2642e39e" },
              "img": { "value": "http://commons.wikimedia.org/wiki/Special:FilePath/%C5%9Eivan%20Perwer.jpg" } }
        ] } });
        let found = bindings(&body).unwrap_or_default();
        let radiohead = &found["a74b1b7f-71a5-4011-9441-d0b5e4122711"];
        assert_eq!(
            radiohead.image.as_deref(),
            Some("https://commons.wikimedia.org/wiki/Special:FilePath/RadioheadO2211125%20composite.jpg?width={size}")
        );
        assert_eq!((radiohead.itunes, radiohead.deezer), (Some(657_515), Some(399)));
        assert_eq!(found["9c21c0c1-b242-49a3-b676-cc9a2642e39e"].deezer, None);
        assert!(!found.contains_key("712b5d5f-78ec-41c8-acfe-d4c1210e14c5"));
    }
}
