//! The slskd client: the only code that talks to slskd (REST API v0, `X-API-Key`). Shapes are
//! slskd 0.26's (docs/context/soulseek.md#slskd); only the fields lull reads are declared, so
//! anything else slskd adds is ignored.

use std::time::Duration;

use lull_core::config::Secret;
use lull_core::http::{self, Http};
use lull_core::{Error, Result};
use reqwest::{Method, StatusCode, Url};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::json;

const SERVICE: &str = "soulseek";

#[derive(Clone)]
pub struct Slskd {
    http: Http,
    base: Url,
    key: Secret,
}

#[derive(Debug, Deserialize)]
struct Application {
    server: Server,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Server {
    #[serde(default)]
    pub username: Option<String>,
    pub is_logged_in: bool,
    /// Connecting, logging in or disconnecting right now.
    pub is_transitioning: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Search {
    pub id: String,
    /// Flags as text, e.g. `InProgress` or `Completed, TimedOut`.
    pub state: String,
    pub response_count: u32,
    pub file_count: u32,
}

impl Search {
    pub fn is_complete(&self) -> bool {
        self.state.contains("Completed")
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Response {
    pub username: String,
    pub has_free_upload_slot: bool,
    pub queue_length: u64,
    /// Bytes per second.
    pub upload_speed: u64,
    #[serde(default)]
    pub files: Vec<File>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct File {
    pub filename: String,
    pub size: u64,
    pub bit_rate: Option<u32>,
    pub bit_depth: Option<u32>,
    pub sample_rate: Option<u32>,
    /// Seconds.
    pub length: Option<u32>,
    pub is_variable_bit_rate: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transfer {
    pub id: String,
    pub filename: String,
    /// Flags as text: `Queued, Remotely`, `InProgress`, `Completed, Succeeded`, `Completed, Errored`...
    pub state: String,
    pub bytes_transferred: u64,
    /// Bytes per second.
    pub average_speed: f64,
    pub place_in_queue: Option<u32>,
    pub exception: Option<String>,
}

/// Where a transfer stands, from its state flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Requested or initialising: the peer hasn't answered yet.
    Starting,
    Queued,
    InProgress,
    Succeeded,
    /// Cancelled, errored, rejected, timed out.
    Failed,
}

impl Transfer {
    pub fn phase(&self) -> Phase {
        let s = self.state.as_str();
        if s.contains("Succeeded") {
            Phase::Succeeded
        } else if s.contains("Completed") {
            Phase::Failed
        } else if s.contains("InProgress") {
            Phase::InProgress
        } else if s.contains("Queued") {
            Phase::Queued
        } else {
            Phase::Starting
        }
    }
}

#[derive(Debug, Deserialize)]
struct Batch {
    batch: BatchTransfers,
}

#[derive(Debug, Deserialize)]
struct BatchTransfers {
    #[serde(default)]
    transfers: Vec<Transfer>,
}

#[derive(Debug, Deserialize)]
struct UserTransfers {
    #[serde(default)]
    directories: Vec<TransferDirectory>,
}

#[derive(Debug, Deserialize)]
struct TransferDirectory {
    #[serde(default)]
    files: Vec<Transfer>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Options {
    pub directories: Directories,
}

/// slskd's own paths, as slskd sees them.
#[derive(Debug, Clone, Deserialize)]
pub struct Directories {
    pub incomplete: String,
    pub downloads: String,
}

/// One shared directory, as `GET /shares` lists it per host.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Share {
    #[serde(default)]
    pub is_excluded: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EventRecord {
    #[serde(rename = "type")]
    kind: String,
    data: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DownloadFileComplete {
    local_filename: String,
    transfer: TransferId,
}

#[derive(Debug, Deserialize)]
struct TransferId {
    #[serde(alias = "Id")]
    id: String,
}

impl Slskd {
    pub fn new(http: Http, url: &str, key: Secret) -> Result<Self> {
        let base = Url::parse(&format!("{url}/api/v0/"))
            .map_err(|e| Error::internal(format!("LULL_SLSKD_URL is not a URL: {e}")))?;
        Ok(Slskd { http, base, key })
    }

    /// The Soulseek connection. Read from `/application`, which any key may read
    /// (`/server` needs the administrator role).
    pub async fn server(&self) -> Result<Server> {
        let app: Application = self.get(&["application"]).await?;
        Ok(app.server)
    }

    /// Needs an API key with the administrator role; `None` when the key isn't one.
    pub async fn options(&self) -> Result<Option<Options>> {
        match self.get(&["options"]).await {
            Ok(o) => Ok(Some(o)),
            Err(Error::Upstream { reason, .. }) if reason.contains("403") => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Every shared directory, across hosts.
    pub async fn shares(&self) -> Result<Vec<Share>> {
        let hosts: std::collections::BTreeMap<String, Vec<Share>> = self.get(&["shares"]).await?;
        Ok(hosts.into_values().flatten().collect())
    }

    /// Asks slskd to rescan its shares (a scan already running is fine).
    pub async fn rescan_shares(&self) -> Result<()> {
        let resp = self.send(Method::PUT, &["shares"], None).await?;
        match resp.status() {
            StatusCode::CONFLICT => Ok(()),
            _ => http::ok(SERVICE, resp).map(drop),
        }
    }

    /// Starts a network search. `inactivity` ends it once no peer has answered for that long.
    pub async fn start_search(&self, text: &str, inactivity: Duration, response_limit: u32) -> Result<Search> {
        let body = json!({
            "searchText": text,
            "searchTimeout": inactivity.as_millis() as u64,
            "responseLimit": response_limit,
            "fileLimit": 20_000,
            "filterResponses": true,
        });
        self.json(Method::POST, &["searches"], Some(body)).await
    }

    pub async fn search(&self, id: &str) -> Result<Search> {
        self.get(&["searches", id]).await
    }

    /// slskd keeps responses to itself until a search ends; then they're all here.
    pub async fn responses(&self, id: &str) -> Result<Vec<Response>> {
        self.get(&["searches", id, "responses"]).await
    }

    /// Ends a search early; what has arrived so far is kept as its responses.
    pub async fn stop_search(&self, id: &str) -> Result<()> {
        let resp = self.send(Method::PUT, &["searches", id], None).await?;
        http::ok(SERVICE, resp).map(drop)
    }

    pub async fn delete_search(&self, id: &str) -> Result<()> {
        let resp = self.send(Method::DELETE, &["searches", id], None).await?;
        match resp.status() {
            StatusCode::NOT_FOUND => Ok(()),
            _ => http::ok(SERVICE, resp).map(drop),
        }
    }

    /// Queues a download and returns its transfer. A file already downloading from that peer
    /// is not queued twice; its existing transfer comes back instead. A partial file left by an
    /// earlier attempt is resumed by slskd, not fetched again.
    pub async fn enqueue(&self, user: &str, filename: &str, size: u64) -> Result<Transfer> {
        let body = json!({ "username": user, "files": [{ "filename": filename, "size": size }] });
        let resp = self
            .send(Method::POST, &["transfers", "downloads", "batches"], Some(body))
            .await?;
        if resp.status() == StatusCode::NOT_FOUND {
            return Err(Error::upstream(SERVICE, format!("{user} is offline")));
        }
        let queued: Batch = decode(http::ok(SERVICE, resp)?).await?;
        if let Some(t) = queued.batch.transfers.into_iter().find(|t| t.filename == filename) {
            return Ok(t);
        }
        self.downloads(user)
            .await?
            .into_iter()
            .find(|t| t.filename == filename && !matches!(t.phase(), Phase::Succeeded | Phase::Failed))
            .ok_or_else(|| Error::upstream(SERVICE, format!("slskd would not queue {filename} from {user}")))
    }

    async fn downloads(&self, user: &str) -> Result<Vec<Transfer>> {
        let t: UserTransfers = self.get(&["transfers", "downloads", user]).await?;
        Ok(t.directories.into_iter().flat_map(|d| d.files).collect())
    }

    pub async fn download(&self, user: &str, id: &str) -> Result<Transfer> {
        self.get(&["transfers", "downloads", user, id]).await
    }

    /// Cancels a download and drops its record; slskd keeps the partial file for a resume.
    pub async fn cancel(&self, user: &str, id: &str) -> Result<()> {
        let mut url = self.url(&["transfers", "downloads", user, id])?;
        url.query_pairs_mut().append_pair("remove", "true");
        let resp = self.request(Method::DELETE, url, None).await?;
        match resp.status() {
            StatusCode::NOT_FOUND => Ok(()),
            _ => http::ok(SERVICE, resp).map(drop),
        }
    }

    /// Where slskd put a finished download (its path), from its `DownloadFileComplete` event.
    /// Events come newest first; a just-finished transfer is near the top.
    pub async fn finished_path(&self, transfer_id: &str) -> Result<Option<String>> {
        let mut url = self.url(&["events"])?;
        url.query_pairs_mut().append_pair("limit", "200");
        let resp = http::ok(SERVICE, self.request(Method::GET, url, None).await?)?;
        let events: Vec<EventRecord> = decode(resp).await?;
        Ok(events
            .into_iter()
            .filter(|e| e.kind == "DownloadFileComplete")
            .filter_map(|e| serde_json::from_str::<DownloadFileComplete>(&e.data).ok())
            .find(|e| e.transfer.id.eq_ignore_ascii_case(transfer_id))
            .map(|e| e.local_filename))
    }

    async fn get<T: DeserializeOwned>(&self, path: &[&str]) -> Result<T> {
        self.json(Method::GET, path, None).await
    }

    async fn json<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &[&str],
        body: Option<serde_json::Value>,
    ) -> Result<T> {
        let resp = http::ok(SERVICE, self.send(method, path, body).await?)?;
        decode(resp).await
    }

    async fn send(&self, method: Method, path: &[&str], body: Option<serde_json::Value>) -> Result<reqwest::Response> {
        let url = self.url(path)?;
        self.request(method, url, body).await
    }

    async fn request(&self, method: Method, url: Url, body: Option<serde_json::Value>) -> Result<reqwest::Response> {
        let resp = self
            .http
            .send(SERVICE, |c| {
                let req = c
                    .request(method.clone(), url.clone())
                    .header("X-API-Key", self.key.expose());
                match &body {
                    Some(b) => req.json(b),
                    None => req,
                }
            })
            .await?;
        match resp.status() {
            StatusCode::UNAUTHORIZED => Err(Error::unavailable(
                SERVICE,
                "slskd rejected the API key; check LULL_SLSKD_API_KEY",
            )),
            _ => Ok(resp),
        }
    }

    /// Path segments are percent-encoded one by one: user names and ids may hold anything.
    fn url(&self, path: &[&str]) -> Result<Url> {
        let mut url = self.base.clone();
        url.path_segments_mut()
            .map_err(|_| Error::internal("LULL_SLSKD_URL cannot be a base URL"))?
            .pop_if_empty()
            .extend(path);
        Ok(url)
    }
}

async fn decode<T: DeserializeOwned>(resp: reqwest::Response) -> Result<T> {
    resp.json()
        .await
        .map_err(|e| Error::upstream(SERVICE, format!("slskd sent unexpected JSON: {e}")))
}
