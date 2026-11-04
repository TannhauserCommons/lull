//! YouTube Music's search, a gap-filler behind the catalog (discovery.md#search): one call per
//! query to the endpoint its web player reads (`youtubei/v1/search`; YouTube publishes no API
//! for it, and yt-dlp reads the same one). It finds what Apple's store search lacks ("nevermind":
//! Nirvana's album; "radiohead creep": the song, 2.3B plays) in about 0.5 s, so it answers in
//! the first answer. Only its songs and albums are read, and only those the catalog doesn't
//! list are kept (`merge::gaps`).
//!
//! The answer's shape is YouTube's to change: anything this module can't read is no rows and
//! a `problem`, never a failed search, and `the_live_answer_still_reads` (run by the
//! `youtube-music-contract` workflow) says when the shape moved.

use std::time::Duration;

use lull_core::Result;
use lull_core::http::{Http, Limit};
use serde_json::{Value, json};

use super::merge::{AlbumHit, Found, SongHit, popularity};
use crate::sources::cache::Cache;

const SERVICE: &str = "YouTube Music";
const API: &str = "https://music.youtube.com/youtubei/v1/search?prettyPrint=false";
/// The web player's client, as YouTube Music's own pages send it.
const CLIENT: &str = "WEB_REMIX";
const CLIENT_VERSION: &str = "1.20241106.01.00";
/// YouTube publishes no limit; lull keeps to about one search a second, bursts of five.
const LIMIT: Limit = Limit {
    service: "youtube-music",
    per: 5,
    every: Duration::from_secs(5),
};
/// Past this it answers after the first answer anyway (it takes about 0.5 s).
const TIMEOUT: Duration = Duration::from_secs(4);
/// Answers are kept this long (seconds), as iTunes's are.
const KEEP_S: i64 = 600;
/// Plays at which a song scores 1 on `popularity`'s scale.
const FULL_PLAYS: f64 = 1e9;

pub struct YoutubeMusic {
    http: Http,
    cache: Cache,
}

impl YoutubeMusic {
    pub fn new(http: Http, cache: Cache) -> Self {
        YoutubeMusic { http, cache }
    }

    /// Songs and albums YouTube Music finds for `q`, its order and play counts kept.
    pub async fn search(&self, q: &str) -> Result<Found> {
        let key = format!("ytm:search:{q}");
        if let Some(found) = self.cache.fresh(key.clone(), KEEP_S).await? {
            return Ok(found);
        }
        self.http.window(LIMIT).await;
        let body = json!({
            "context": { "client": { "clientName": CLIENT, "clientVersion": CLIENT_VERSION, "hl": "en", "gl": "US" } },
            "query": q,
        });
        let answer: Value = self
            .http
            .json(SERVICE, |c| {
                c.post(API)
                    .header("Origin", "https://music.youtube.com")
                    .json(&body)
                    .timeout(TIMEOUT)
            })
            .await?;
        let found = answer_of(&answer);
        self.cache.put(key, &found).await?;
        Ok(found)
    }
}

/// The rows of a search answer: its top card when it is a song or an album, and as `fillers`
/// its listed songs, albums, EPs and singles. Videos, artists, playlists, episodes and profiles are left
/// out (Deezer answers artists; a video is someone's upload). A song has its plays for
/// popularity; an album as the top card counts in full, as YouTube Music's best answer
/// ("nevermind": Nirvana's); listed albums by their order, on the album scale the catalogs use.
pub fn answer_of(answer: &Value) -> Found {
    let sections = &answer["contents"]["tabbedSearchResultsRenderer"]["tabs"][0]["tabRenderer"]["content"]["sectionListRenderer"]
        ["contents"];
    let Some(sections) = sections.as_array() else {
        return Found {
            problem: Some("its answer had no results lull can read".into()),
            ..Found::default()
        };
    };
    let mut found = Found::default();
    let items: Vec<Item> = sections
        .iter()
        .flat_map(|s| {
            let card = s.get("musicCardShelfRenderer").and_then(Item::card);
            let listed = s["itemSectionRenderer"]["contents"]
                .as_array()
                .into_iter()
                .flatten()
                .chain(s["musicShelfRenderer"]["contents"].as_array().into_iter().flatten())
                .filter_map(|i| i.get("musicResponsiveListItemRenderer"))
                .filter_map(Item::listed);
            card.into_iter().chain(listed)
        })
        .collect();
    let albums = items.iter().filter(|i| i.album_kind().is_some()).count().max(1) as f64;
    let mut album_place = 0.0;
    let mut listed = Found::default();
    for item in items {
        let found = if item.card { &mut found } else { &mut listed };
        match (item.kind.as_str(), item.album_kind()) {
            ("Song", _) => found.songs.push(SongHit {
                title: item.title,
                artist: item.artist,
                popularity: item.plays.map_or(0.0, |n| popularity(n, FULL_PLAYS)),
                ..SongHit::default()
            }),
            (_, Some(kind)) => {
                found.albums.push(AlbumHit {
                    title: item.title,
                    artist: item.artist,
                    kind: Some(kind.into()),
                    date: item.year.map(|y| y.to_string()),
                    popularity: if item.card {
                        1.0
                    } else {
                        0.6 * (1.0 - album_place / albums)
                    },
                    ..AlbumHit::default()
                });
                album_place += 1.0;
            }
            _ => {}
        }
    }
    found.fillers = Some(Box::new(listed));
    found
}

/// One result: its kind ("Song", "Album", "Single", "Video"…), title, artist and what else the
/// line says.
#[derive(Debug, Default)]
struct Item {
    kind: String,
    title: String,
    artist: String,
    plays: Option<u64>,
    year: Option<u16>,
    /// The top card, not a listed row.
    card: bool,
}

impl Item {
    /// `title`, and a subtitle "Album • Nirvana • 1991".
    fn card(card: &Value) -> Option<Item> {
        let mut item = Item::of(text(&card["title"])?, &words(&card["subtitle"]), None)?;
        item.card = true;
        Some(item)
    }

    /// Columns: the title; "Song • Radiohead"; "2.3B plays".
    fn listed(row: &Value) -> Option<Item> {
        let column = |i: usize| &row["flexColumns"][i]["musicResponsiveListItemFlexColumnRenderer"]["text"];
        Item::of(text(column(0))?, &words(column(1)), text(column(2)))
    }

    fn of(title: String, line: &[String], extra: Option<String>) -> Option<Item> {
        let (kind, rest) = line.split_first()?;
        let artist = rest.first()?.clone();
        let mut item = Item {
            kind: kind.clone(),
            title,
            artist,
            ..Item::default()
        };
        for word in rest[1..].iter().chain(extra.as_ref()) {
            item.plays = item.plays.or_else(|| plays(word));
            item.year = item
                .year
                .or_else(|| word.parse().ok().filter(|y| (1900..=2100).contains(y)));
        }
        Some(item)
    }

    /// What kind of album the item is, in lull's words; `None` for anything else.
    fn album_kind(&self) -> Option<&'static str> {
        match self.kind.as_str() {
            "Album" => Some("album"),
            "EP" => Some("EP"),
            "Single" => Some("single"),
            _ => None,
        }
    }
}

/// A text's runs joined.
fn text(v: &Value) -> Option<String> {
    let s: String = v["runs"]
        .as_array()?
        .iter()
        .filter_map(|r| r["text"].as_str())
        .collect();
    (!s.trim().is_empty()).then_some(s)
}

/// A line's parts between " • ".
fn words(v: &Value) -> Vec<String> {
    text(v)
        .map(|s| {
            s.split(" • ")
                .map(|p| p.trim().to_owned())
                .filter(|p| !p.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// "2.3B plays", "632M plays", "48K plays".
fn plays(s: &str) -> Option<u64> {
    let n = s.strip_suffix(" plays")?;
    let (digits, scale) = match n.chars().last()? {
        'K' => (&n[..n.len() - 1], 1e3),
        'M' => (&n[..n.len() - 1], 1e6),
        'B' => (&n[..n.len() - 1], 1e9),
        _ => (n, 1.0),
    };
    let value: f64 = digits.replace(',', "").parse().ok()?;
    Some((value * scale) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// YouTube Music's live answer still has the shape lull reads. Run by the
    /// `youtube-music-contract` workflow; it needs the network.
    #[tokio::test]
    #[ignore = "network: YouTube Music's live search"]
    async fn the_live_answer_still_reads() -> std::result::Result<(), Box<dyn std::error::Error>> {
        let http = Http::new()?;
        let body = json!({
            "context": { "client": { "clientName": CLIENT, "clientVersion": CLIENT_VERSION, "hl": "en", "gl": "US" } },
            "query": "nirvana nevermind",
        });
        let answer: Value = http
            .json(SERVICE, |c| {
                c.post(API).header("Origin", "https://music.youtube.com").json(&body)
            })
            .await?;
        let found = answer_of(&answer);
        assert_eq!(found.problem, None);
        let listed = found.fillers.as_deref().cloned().unwrap_or_default();
        let albums: Vec<&AlbumHit> = found.albums.iter().chain(&listed.albums).collect();
        let songs: Vec<&SongHit> = found.songs.iter().chain(&listed.songs).collect();
        assert!(
            albums.iter().any(|a| a.title == "Nevermind" && a.artist == "Nirvana"),
            "{albums:?}"
        );
        assert!(
            songs.iter().any(|s| s.artist == "Nirvana" && s.popularity > 0.5),
            "{songs:?}"
        );
        Ok(())
    }
}
