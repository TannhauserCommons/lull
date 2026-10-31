//! Jellyfin's HTTP API, authenticated with an API key: only the calls lull needs
//! (docs/context/library.md).

use std::collections::BTreeMap;
use std::time::Duration;

use axum::http::HeaderValue;
use lull_core::config;
use lull_core::http::{self, Http};
use lull_core::{Error, Result};
use serde::{Deserialize, Deserializer};
use tokio::sync::OnceCell;

const SERVICE: &str = "jellyfin";
/// Items per page when reading the whole library.
const PAGE: usize = 2000;
/// What lull reads of an audio item beyond Jellyfin's defaults.
const FIELDS: &str = "Path,MediaSources,Genres,ProviderIds,DateCreated,PremiereDate";
/// A streamed body lasts as long as the listener keeps reading it.
const STREAM_TIMEOUT: Duration = Duration::from_secs(4 * 3600);

pub struct Jellyfin {
    http: Http,
    url: String,
    auth: String,
    user_name: Option<String>,
    user_id: OnceCell<String>,
}

/// An audio item, as `/Items` returns it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Item {
    pub id: String,
    pub name: Option<String>,
    pub album: Option<String>,
    pub album_id: Option<String>,
    pub album_artist: Option<String>,
    #[serde(default, deserialize_with = "or_default")]
    pub artists: Vec<String>,
    pub index_number: Option<u32>,
    pub parent_index_number: Option<u32>,
    pub production_year: Option<u16>,
    pub premiere_date: Option<String>,
    /// 100 ns units.
    pub run_time_ticks: Option<u64>,
    #[serde(default, deserialize_with = "or_default")]
    pub genres: Vec<String>,
    pub path: Option<String>,
    pub container: Option<String>,
    #[serde(default, deserialize_with = "or_default")]
    pub media_sources: Vec<MediaSource>,
    #[serde(default, deserialize_with = "or_default")]
    pub provider_ids: BTreeMap<String, String>,
    pub date_created: Option<String>,
    pub album_primary_image_tag: Option<String>,
    #[serde(default, deserialize_with = "or_default")]
    pub image_tags: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MediaSource {
    pub container: Option<String>,
    pub size: Option<u64>,
    pub bitrate: Option<u64>,
    #[serde(default, deserialize_with = "or_default")]
    pub media_streams: Vec<MediaStream>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MediaStream {
    #[serde(rename = "Type")]
    pub kind: Option<String>,
    pub codec: Option<String>,
    pub sample_rate: Option<u32>,
    pub bit_depth: Option<u8>,
    pub bit_rate: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Page {
    #[serde(default, deserialize_with = "or_default")]
    items: Vec<Item>,
    total_record_count: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct User {
    id: String,
    name: String,
    policy: Option<Policy>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Policy {
    is_administrator: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct VirtualFolder {
    item_id: Option<String>,
    collection_type: Option<String>,
    #[serde(default, deserialize_with = "or_default")]
    locations: Vec<String>,
}

/// A music library: the item to refresh, and the folders it covers.
#[derive(Debug, Clone)]
pub struct MusicLibrary {
    pub item_id: String,
    pub locations: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Task {
    key: String,
    state: String,
    current_progress_percentage: Option<f64>,
}

/// Jellyfin's library scan, as its scheduled task reports it.
pub enum Scan {
    Idle,
    Running { pct: u8 },
}

impl Jellyfin {
    pub fn new(http: Http, cfg: &config::Jellyfin) -> Self {
        Jellyfin {
            http,
            url: cfg.url.clone(),
            auth: format!("MediaBrowser Token=\"{}\"", cfg.api_key.expose()),
            user_name: cfg.user.clone(),
            user_id: OnceCell::new(),
        }
    }

    /// The user whose library lull reads: `LULL_JELLYFIN_USER`, else the first administrator.
    async fn user_id(&self) -> Result<&str> {
        self.user_id
            .get_or_try_init(|| async {
                let users: Vec<User> = self.get_json("/Users", &[]).await?;
                pick_user(users, self.user_name.as_deref())
            })
            .await
            .map(String::as_str)
    }

    /// Cheap authenticated call: proves Jellyfin is up and the key works.
    pub async fn ping(&self) -> Result<()> {
        let _: serde_json::Value = self.get_json("/System/Info", &[]).await?;
        self.user_id().await.map(|_| ())
    }

    /// Every audio item the user can see, page by page.
    pub async fn all_audio(&self) -> Result<Vec<Item>> {
        let mut items = Vec::new();
        loop {
            let start = items.len().to_string();
            let page: Page = self
                .items(&[
                    ("SortBy", "DateCreated,SortName"),
                    ("StartIndex", &start),
                    ("Limit", &PAGE.to_string()),
                ])
                .await?;
            let got = page.items.len();
            items.extend(page.items);
            if got == 0 || items.len() >= page.total_record_count {
                return Ok(items);
            }
        }
    }

    /// The most recently added audio items: what a save's rescan brings in.
    pub async fn latest_audio(&self, limit: usize) -> Result<Vec<Item>> {
        let page: Page = self
            .items(&[
                ("SortBy", "DateCreated"),
                ("SortOrder", "Descending"),
                ("Limit", &limit.to_string()),
            ])
            .await?;
        Ok(page.items)
    }

    async fn items(&self, extra: &[(&str, &str)]) -> Result<Page> {
        let user = self.user_id().await?.to_owned();
        let mut query = vec![
            ("userId", user.as_str()),
            ("IncludeItemTypes", "Audio"),
            ("Recursive", "true"),
            ("Fields", FIELDS),
        ];
        query.extend_from_slice(extra);
        self.get_json("/Items", &query).await
    }

    /// Jellyfin's music libraries and their folders, as Jellyfin sees them.
    pub async fn music_libraries(&self) -> Result<Vec<MusicLibrary>> {
        let folders: Vec<VirtualFolder> = self.get_json("/Library/VirtualFolders", &[]).await?;
        Ok(folders
            .into_iter()
            .filter(|f| f.collection_type.as_deref() == Some("music"))
            .filter_map(|f| {
                Some(MusicLibrary {
                    item_id: f.item_id?,
                    locations: f.locations,
                })
            })
            .collect())
    }

    /// An item's primary image scaled to `size`, or `None` when it has none.
    pub async fn image(&self, item: &str, size: u32) -> Result<Option<Vec<u8>>> {
        let size = size.to_string();
        let resp = self
            .http
            .send(SERVICE, |c| {
                self.authed(c.get(format!("{}/Items/{item}/Images/Primary", self.url)))
                    .query(&[
                        ("maxWidth", size.as_str()),
                        ("maxHeight", size.as_str()),
                        ("quality", "90"),
                    ])
            })
            .await?;
        match http::ok(SERVICE, resp) {
            Ok(resp) => resp
                .bytes()
                .await
                .map(|b| Some(b.to_vec()))
                .map_err(|e| Error::upstream(SERVICE, format!("image body: {e}"))),
            Err(Error::NotFound(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// The original file, with the listener's Range header passed through.
    pub async fn stream(&self, item: &str, range: Option<HeaderValue>) -> Result<reqwest::Response> {
        self.http
            .send(SERVICE, |c| {
                let req = self
                    .authed(c.get(format!("{}/Audio/{item}/stream", self.url)))
                    .query(&[("static", "true")])
                    .timeout(STREAM_TIMEOUT);
                match &range {
                    Some(r) => req.header(reqwest::header::RANGE, r.clone()),
                    None => req,
                }
            })
            .await
    }

    /// Scans one library (a music library after a save) for new and changed files, as the
    /// library's own "Scan library" does. Works whether or not realtime monitoring is on.
    pub async fn refresh_library(&self, library_item: &str) -> Result<()> {
        let url = format!("{}/Items/{library_item}/Refresh", self.url);
        let resp = self
            .http
            .send(SERVICE, |c| {
                self.authed(c.post(&url)).query(&[
                    ("Recursive", "true"),
                    ("MetadataRefreshMode", "Default"),
                    ("ImageRefreshMode", "Default"),
                    ("ReplaceAllMetadata", "false"),
                    ("ReplaceAllImages", "false"),
                ])
            })
            .await?;
        http::ok(SERVICE, resp).map(|_| ())
    }

    /// Starts Jellyfin's "Scan media library" task (all libraries, as its own button does).
    pub async fn scan_library(&self) -> Result<()> {
        self.post(&format!("{}/Library/Refresh", self.url)).await
    }

    pub async fn scan_state(&self) -> Result<Scan> {
        let tasks: Vec<Task> = self.get_json("/ScheduledTasks", &[("isHidden", "false")]).await?;
        let task = tasks
            .into_iter()
            .find(|t| t.key == "RefreshLibrary")
            .ok_or_else(|| Error::upstream(SERVICE, "has no library scan task"))?;
        Ok(match task.state.as_str() {
            "Idle" => Scan::Idle,
            _ => Scan::Running {
                pct: task.current_progress_percentage.unwrap_or(0.0).clamp(0.0, 100.0) as u8,
            },
        })
    }

    async fn post(&self, url: &str) -> Result<()> {
        let resp = self.http.send(SERVICE, |c| self.authed(c.post(url))).await?;
        http::ok(SERVICE, resp).map(|_| ())
    }

    async fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str, query: &[(&str, &str)]) -> Result<T> {
        let resp = self
            .http
            .send(SERVICE, |c| {
                self.authed(c.get(format!("{}{path}", self.url))).query(query)
            })
            .await?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(Error::unavailable(SERVICE, "it refused the API key"));
        }
        http::ok(SERVICE, resp)?
            .json()
            .await
            .map_err(|e| Error::upstream(SERVICE, format!("bad JSON from {path}: {e}")))
    }

    fn authed(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        req.header(reqwest::header::AUTHORIZATION, &self.auth)
    }
}

fn pick_user(users: Vec<User>, wanted: Option<&str>) -> Result<String> {
    let found = match wanted {
        Some(name) => users.into_iter().find(|u| u.name.eq_ignore_ascii_case(name)),
        None => users
            .into_iter()
            .find(|u| u.policy.as_ref().is_some_and(|p| p.is_administrator)),
    };
    found.map(|u| u.id).ok_or_else(|| match wanted {
        Some(name) => Error::unavailable(SERVICE, format!("it has no user named {name:?} (LULL_JELLYFIN_USER)")),
        None => Error::unavailable(SERVICE, "it has no administrator user"),
    })
}

/// Jellyfin sends `null` for some empty lists and maps.
fn or_default<'de, D, T>(d: D) -> std::result::Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}
