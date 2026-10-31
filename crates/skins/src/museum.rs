//! The Winamp Skin Museum's public GraphQL API (docs/context/skins.md). Pages are cached for an
//! hour, a failure pauses calls for a while (serving the last copy when there is one), and the
//! shared HTTP client adds timeouts and retries with backoff.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use lull_core::http::Http;
use lull_core::{Error, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const URL: &str = "https://api.webamp.org/graphql";
pub(crate) const SERVICE: &str = "Winamp Skin Museum";
pub(crate) const PAGE: usize = 30;
const FRESH_FOR: Duration = Duration::from_secs(3600);
const PAUSE_AFTER_FAILURE: Duration = Duration::from_secs(30);
const MAX_PAGES: usize = 200;

/// One skin as the museum describes it.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Skin {
    pub md5: String,
    pub filename: String,
    pub screenshot_url: String,
    pub museum_url: String,
    pub nsfw: bool,
}

/// `Skin` plus where to download it, for a single lookup.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Download {
    #[serde(flatten)]
    pub skin: Skin,
    pub download_url: String,
}

#[derive(Debug, Clone)]
pub(crate) struct Page {
    pub skins: Vec<Skin>,
    pub more: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct PageKey {
    query: String,
    offset: usize,
    nsfw: bool,
}

pub(crate) struct Museum {
    http: Http,
    pages: Mutex<HashMap<PageKey, (Instant, Page)>>,
    paused_until: Mutex<Option<Instant>>,
}

const FIELDS: &str = "md5 filename screenshot_url museum_url nsfw";

impl Museum {
    pub(crate) fn new(http: Http) -> Self {
        Museum {
            http,
            pages: Mutex::default(),
            paused_until: Mutex::default(),
        }
    }

    /// A page of search results (`query` non-empty) or of the museum's own order. NSFW skins are
    /// left out unless `nsfw`.
    pub(crate) async fn page(&self, query: &str, offset: usize, nsfw: bool) -> Result<Page> {
        let key = PageKey {
            query: query.to_lowercase(),
            offset,
            nsfw,
        };
        let cached = self.cached(&key);
        if let Some((at, page)) = &cached
            && at.elapsed() < FRESH_FOR
        {
            return Ok(page.clone());
        }
        match self.fetch_page(&key).await {
            Ok(page) => {
                self.remember(key, page.clone());
                Ok(page)
            }
            // A stale page beats an error while the museum is away.
            Err(e) => cached.map(|(_, page)| page).ok_or(e),
        }
    }

    /// One skin with its download URL, or `None` when the museum has no such skin.
    pub(crate) async fn skin(&self, md5: &str) -> Result<Option<Download>> {
        #[derive(Deserialize)]
        struct Data {
            fetch_skin_by_md5: Option<Download>,
        }
        let query = format!("query($md5: String!) {{ fetch_skin_by_md5(md5: $md5) {{ {FIELDS} download_url }} }}");
        let data: Data = self.graphql(&query, json!({ "md5": md5 })).await?;
        Ok(data.fetch_skin_by_md5)
    }

    async fn fetch_page(&self, key: &PageKey) -> Result<Page> {
        let (skins, more) = if key.query.is_empty() {
            // The museum refuses `filter` together with `sort: MUSEUM`; NSFW is filtered below.
            #[derive(Deserialize)]
            struct Data {
                skins: Connection,
            }
            #[derive(Deserialize)]
            struct Connection {
                count: usize,
                nodes: Vec<Skin>,
            }
            let query = format!(
                "query($first: Int!, $offset: Int!) {{ skins(first: $first, offset: $offset, sort: MUSEUM) {{ count nodes {{ {FIELDS} }} }} }}"
            );
            let data: Data = self
                .graphql(&query, json!({ "first": PAGE, "offset": key.offset }))
                .await?;
            let more = key.offset + data.skins.nodes.len() < data.skins.count;
            (data.skins.nodes, more)
        } else {
            #[derive(Deserialize)]
            struct Data {
                search_classic_skins: Vec<Skin>,
            }
            let query = format!(
                "query($q: String!, $first: Int!, $offset: Int!, $nsfw: Boolean) {{ search_classic_skins(query: $q, first: $first, offset: $offset, nsfw: $nsfw) {{ {FIELDS} }} }}"
            );
            // One extra row says whether there is a next page.
            let vars = json!({ "q": key.query, "first": PAGE + 1, "offset": key.offset, "nsfw": key.nsfw });
            let mut skins = self.graphql::<Data>(&query, vars).await?.search_classic_skins;
            let more = skins.len() > PAGE;
            skins.truncate(PAGE);
            (skins, more)
        };
        let skins = skins.into_iter().filter(|s| key.nsfw || !s.nsfw).collect();
        Ok(Page { skins, more })
    }

    /// Posts a query; a failure pauses further calls so a busy UI can't hammer the museum.
    async fn graphql<T: DeserializeOwned>(&self, query: &str, variables: Value) -> Result<T> {
        if let Some(until) = self.paused_until.lock().ok().and_then(|p| *p)
            && Instant::now() < until
        {
            let wait = until.saturating_duration_since(Instant::now()).as_secs() + 1;
            return Err(Error::unavailable(
                SERVICE,
                format!("it failed just now, trying again in {wait} s"),
            ));
        }
        let outcome = self.post(query, variables).await;
        if outcome.is_err()
            && let Ok(mut paused) = self.paused_until.lock()
        {
            *paused = Some(Instant::now() + PAUSE_AFTER_FAILURE);
        }
        outcome
    }

    async fn post<T: DeserializeOwned>(&self, query: &str, variables: Value) -> Result<T> {
        #[derive(Serialize)]
        struct Request<'a> {
            query: &'a str,
            variables: Value,
        }
        #[derive(Deserialize)]
        struct Response<T> {
            data: Option<T>,
            #[serde(default)]
            errors: Vec<GraphqlError>,
        }
        #[derive(Deserialize)]
        struct GraphqlError {
            message: String,
        }
        let body = Request { query, variables };
        let resp: Response<T> = self.http.json(SERVICE, |c| c.post(URL).json(&body)).await?;
        if let Some(e) = resp.errors.first() {
            return Err(Error::upstream(SERVICE, e.message.clone()));
        }
        resp.data
            .ok_or_else(|| Error::upstream(SERVICE, "answered without data"))
    }

    fn cached(&self, key: &PageKey) -> Option<(Instant, Page)> {
        self.pages.lock().ok()?.get(key).cloned()
    }

    /// Keeps at most `MAX_PAGES`, dropping the oldest first.
    fn remember(&self, key: PageKey, page: Page) {
        let Ok(mut pages) = self.pages.lock() else { return };
        if pages.len() >= MAX_PAGES
            && !pages.contains_key(&key)
            && let Some(oldest) = pages.iter().min_by_key(|(_, (at, _))| *at).map(|(k, _)| k.clone())
        {
            pages.remove(&oldest);
        }
        pages.insert(key, (Instant::now(), page));
    }
}
