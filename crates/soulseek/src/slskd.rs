//! The slskd client: the only code that talks to slskd (REST API v0, `X-API-Key`). Shapes are
//! slskd 0.26's (docs/context/soulseek.md#slskd); only the fields lull reads are declared, so
//! anything else slskd adds is ignored.

use std::time::Duration;

use lull_core::config::{Secret, SlskdWebLogin};
use lull_core::http::{self, Http};
use lull_core::{Error, Result};
use reqwest::{Method, StatusCode, Url};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::json;

const SERVICE: &str = "soulseek";

#[derive(Clone)]
pub struct Slskd {
    http: Http,
    base: Url,
    key: Secret,
}

/// One of slskd's recent log lines.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogRecord {
    pub timestamp: String,
    pub level: String,
    pub message: String,
}

impl LogRecord {
    pub fn is_problem(&self) -> bool {
        let l = self.level.to_ascii_lowercase();
        l.starts_with("warn") || l.starts_with("err") || l.starts_with("fatal")
    }
}

#[derive(Debug, Deserialize)]
struct Application {
    server: Server,
    user: Option<User>,
}

#[derive(Debug, Deserialize)]
struct User {
    username: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Server {
    #[serde(default)]
    pub username: Option<String>,
    pub is_logged_in: bool,
    /// Connecting, logging in or disconnecting right now.
    pub is_transitioning: bool,
    /// slskd's state flags, e.g. `Connected, LoggedIn` or `Disconnecting`.
    #[serde(default)]
    pub state: String,
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
    /// Set while slskd waits to retry a failed attempt itself (the state still reads `Completed`).
    #[serde(default)]
    pub next_attempt_at: Option<String>,
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
        } else if s.contains("Completed") && self.next_attempt_at.is_some() {
            Phase::Queued
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
    #[serde(default)]
    pub soulseek: SoulseekOptions,
}

/// The Soulseek account slskd's options hold (the password comes back redacted).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SoulseekOptions {
    pub username: Option<String>,
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
        Ok(Server {
            username: app.user.and_then(|u| u.username),
            ..app.server
        })
    }

    /// slskd's own view, for diagnostics: the server connection, the distributed network (how
    /// searches reach peers) and the reconnect watchdog, as slskd reports them.
    pub async fn state(&self) -> Result<serde_json::Value> {
        let app: serde_json::Value = self.get(&["application"]).await?;
        let pick = [
            "version",
            "server",
            "distributedNetwork",
            "connectionWatchdog",
            "pendingReconnect",
        ];
        Ok(pick
            .iter()
            .map(|k| (k.to_string(), app.get(*k).cloned().unwrap_or_default()))
            .collect())
    }

    /// slskd's most recent log lines, oldest first. Needs the administrator role; empty without it.
    pub async fn recent_log(&self) -> Result<Vec<LogRecord>> {
        match self.get(&["logs", "live"]).await {
            Ok(lines) => Ok(lines),
            Err(Error::Upstream { reason, .. }) if reason.contains("403") => Ok(Vec::new()),
            Err(e) => Err(e),
        }
    }

    /// A fresh login: an intentional disconnect, then slskd's watchdog connects again (and keeps
    /// retrying until it's in). Needs a readwrite or administrator key.
    pub async fn relogin(&self, reason: &str) -> Result<()> {
        self.disconnect(reason).await?;
        self.connect().await
    }

    /// Asks slskd to restart itself (`PUT /application`). slskd 0.26 allows that to a web session
    /// only, never to an API key, so lull logs in with slskd's web login first. A headless slskd
    /// (the desktop app's) refuses web logins.
    pub async fn restart(&self, login: &SlskdWebLogin) -> Result<()> {
        #[derive(Deserialize)]
        struct Session {
            token: String,
        }
        let url = self.url(&["session"])?;
        let body = json!({ "username": login.username, "password": login.password.expose() });
        let resp = self.http.send(SERVICE, |c| c.post(url.clone()).json(&body)).await?;
        let session: Session = match resp.status() {
            StatusCode::UNAUTHORIZED => {
                return Err(Error::unavailable(
                    SERVICE,
                    "slskd refused its web login; check LULL_SLSKD_WEB_USERNAME and LULL_SLSKD_WEB_PASSWORD",
                ));
            }
            StatusCode::FORBIDDEN => {
                return Err(Error::unavailable(
                    SERVICE,
                    "slskd runs headless, so it takes no web login",
                ));
            }
            _ => decode(http::ok(SERVICE, resp)?).await?,
        };
        let url = self.url(&["application"])?;
        let resp = self
            .http
            .send(SERVICE, |c| c.put(url.clone()).bearer_auth(&session.token))
            .await?;
        http::ok(SERVICE, resp).map(drop)
    }

    /// An intentional disconnect. Needs a readwrite or administrator key.
    pub async fn disconnect(&self, reason: &str) -> Result<()> {
        let resp = self.send(Method::DELETE, &["server"], Some(json!(reason))).await?;
        http::ok(SERVICE, resp).map(drop)
    }

    /// Connects and logs in with the account slskd has now. Needs a readwrite or administrator key.
    pub async fn connect(&self) -> Result<()> {
        let resp = self.send(Method::PUT, &["server"], None).await?;
        http::ok(SERVICE, resp).map(drop)
    }

    /// slskd reloaded a changed Soulseek account while logged in, and waits for a reconnect to
    /// use it.
    pub async fn pending_reconnect(&self) -> Result<bool> {
        let app: serde_json::Value = self.get(&["application"]).await?;
        Ok(app
            .get("pendingReconnect")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false))
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
