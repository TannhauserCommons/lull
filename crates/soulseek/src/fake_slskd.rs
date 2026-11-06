//! A stand-in for slskd's download API in tests, writing files as slskd 0.26 does: a download
//! starts from the length of its incomplete file (`retry.partial: resume`, the default) or from 0
//! (`overwrite`), appends the peer's bytes at a set pace, then moves the file to the downloads
//! folder and reports `DownloadFileComplete`, in slskd's order: the transfer reads `Succeeded`
//! before the file has moved and before the event says where (slskd 0.26.2's
//! `DownloadService`; pinned by the contract test, tests/slskd-contract). This lets lull's fetches
//! and their legs be tested in-process against it. Its searches find every file it serves
//! (`peer` shares them), after `SEARCH_TAKES`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::{Path as UrlPath, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post};
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;

use crate::layout::Layout;
use crate::slskd::Slskd;

/// The pace the peer sends at: this many bytes every `TICK`.
const TICK: Duration = Duration::from_millis(20);
/// How long a search runs before slskd calls it complete.
pub const SEARCH_TAKES: Duration = Duration::from_secs(1);
/// The length the search reports for every file, seconds: the tests' song's.
const LENGTH: u32 = 200;

pub struct Fake {
    pub slskd: Slskd,
    pub layout: Layout,
    /// The transfers folder: slskd's `incomplete` and `complete` are inside it.
    pub root: PathBuf,
    shared: Arc<Shared>,
}

struct Shared {
    /// The peers' files, by remote path: every peer shares these, unless `user|path` names one
    /// peer's own bytes for that path.
    files: HashMap<String, Vec<u8>>,
    layout: Layout,
    root: PathBuf,
    peer: Peer,
    /// Peers whose pace differs from `peer`'s.
    paces: HashMap<String, usize>,
    transfers: Mutex<HashMap<String, Transfer>>,
    /// Every download started, with the offset the peer was asked for.
    starts: Mutex<Vec<(String, u64)>>,
    events: Mutex<Vec<Value>>,
    next: Mutex<u32>,
    /// Every search started: its text, and when.
    searches: Mutex<Vec<(String, std::time::Instant)>>,
}

#[derive(Clone)]
struct Transfer {
    filename: String,
    state: &'static str,
    bytes: u64,
    /// Where the peer started sending from, and when: slskd's averageSpeed is what came since.
    from: (u64, std::time::Instant),
    task: Option<Arc<tokio::task::AbortHandle>>,
}

/// How the peer and slskd behave.
#[derive(Clone, Copy)]
pub struct Peer {
    /// Bytes sent every `TICK`.
    pub per_tick: usize,
    /// slskd set to `retry.partial: overwrite`: every download starts over from byte 0.
    pub overwrite: bool,
    /// The speed slskd reports, bytes a second, when not the rate the bytes came at.
    pub reported: Option<f64>,
    /// How long after a transfer reads `Succeeded` slskd moves the file and says where.
    pub settles: Duration,
}

impl Fake {
    /// slskd serving `files` (remote path to bytes) from one peer.
    pub async fn start(files: HashMap<String, Vec<u8>>, peer: Peer) -> std::io::Result<Fake> {
        Fake::start_with(files, peer, HashMap::new()).await
    }

    /// The same, with `paces` (user to bytes a tick) for peers that send at another pace.
    pub async fn start_with(
        files: HashMap<String, Vec<u8>>,
        peer: Peer,
        paces: HashMap<String, usize>,
    ) -> std::io::Result<Fake> {
        let root = std::env::temp_dir().join(format!("lull-fake-slskd-{}-{}", std::process::id(), rand_id()));
        std::fs::create_dir_all(root.join("incomplete"))?;
        std::fs::create_dir_all(root.join("complete"))?;
        let layout = Layout::new(&root, None);
        let shared = Arc::new(Shared {
            files,
            layout: layout.clone(),
            paces,
            root: root.clone(),
            peer,
            transfers: Mutex::default(),
            starts: Mutex::default(),
            events: Mutex::default(),
            next: Mutex::new(0),
            searches: Mutex::default(),
        });
        let app = Router::new()
            .route("/api/v0/transfers/downloads/batches", post(enqueue))
            .route("/api/v0/transfers/downloads/{user}", get(list))
            .route(
                "/api/v0/transfers/downloads/{user}/{id}",
                get(one).merge(delete(cancel)),
            )
            .route("/api/v0/events", get(events))
            .route("/api/v0/searches", post(start_search))
            .route(
                "/api/v0/searches/{id}",
                get(search)
                    .put(|| async { StatusCode::NO_CONTENT })
                    .delete(|| async { StatusCode::NO_CONTENT }),
            )
            .route("/api/v0/searches/{id}/responses", get(responses))
            .with_state(shared.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}", listener.local_addr()?);
        tokio::spawn(async move { axum::serve(listener, app).await });
        let slskd = Slskd::new(
            lull_core::http::Http::new().map_err(std::io::Error::other)?,
            &url,
            lull_core::config::Secret::new("key"),
        )
        .map_err(std::io::Error::other)?;
        Ok(Fake {
            slskd,
            layout,
            root,
            shared,
        })
    }

    /// The text of every search started, in order.
    pub fn searches(&self) -> Vec<String> {
        self.shared
            .searches
            .lock()
            .map(|s| s.iter().map(|(text, _)| text.clone()).collect())
            .unwrap_or_default()
    }

    /// The offsets downloads were asked from, in order.
    pub fn starts(&self) -> Vec<u64> {
        self.shared
            .starts
            .lock()
            .map(|s| s.iter().map(|(_, at)| *at).collect())
            .unwrap_or_default()
    }

    /// The offsets one peer's downloads were asked from, in order.
    pub fn starts_of(&self, user: &str) -> Vec<u64> {
        self.shared
            .starts
            .lock()
            .map(|s| s.iter().filter(|(u, _)| u == user).map(|(_, at)| *at).collect())
            .unwrap_or_default()
    }
}

async fn start_search(State(s): State<Arc<Shared>>, axum::Json(body): axum::Json<Value>) -> axum::Json<Value> {
    let text = body["searchText"].as_str().unwrap_or_default().to_owned();
    let n = s.searches.lock().map_or(0, |mut all| {
        all.push((text, std::time::Instant::now()));
        all.len()
    });
    axum::Json(json!({ "id": format!("s{n}"), "state": "InProgress", "responseCount": 0, "fileCount": 0 }))
}

/// The files `peer` shares: every one not named for one peer.
fn shared_files(s: &Shared) -> Vec<(&String, &Vec<u8>)> {
    s.files.iter().filter(|(path, _)| !path.contains('|')).collect()
}

async fn search(State(s): State<Arc<Shared>>, UrlPath(id): UrlPath<String>) -> Result<axum::Json<Value>, StatusCode> {
    let n: usize = id.trim_start_matches('s').parse().map_err(|_| StatusCode::NOT_FOUND)?;
    let started = s
        .searches
        .lock()
        .ok()
        .and_then(|all| all.get(n.wrapping_sub(1)).map(|(_, at)| *at))
        .ok_or(StatusCode::NOT_FOUND)?;
    let files = shared_files(&s).len();
    Ok(axum::Json(if started.elapsed() >= SEARCH_TAKES {
        json!({ "id": id, "state": "Completed, Succeeded", "responseCount": u32::from(files > 0), "fileCount": files,
                "endedAt": "2026-11-06T00:00:00Z" })
    } else {
        json!({ "id": id, "state": "InProgress", "responseCount": 0, "fileCount": 0 })
    }))
}

async fn responses(State(s): State<Arc<Shared>>) -> axum::Json<Value> {
    let files: Vec<Value> = shared_files(&s)
        .into_iter()
        .map(|(path, bytes)| {
            json!({ "filename": path, "size": bytes.len(), "bitRate": 320, "bitDepth": null, "sampleRate": 44100,
                    "length": LENGTH, "isVariableBitRate": false })
        })
        .collect();
    axum::Json(match files.is_empty() {
        true => json!([]),
        false => json!([{ "username": "peer", "hasFreeUploadSlot": true, "queueLength": 0, "uploadSpeed": 2_000_000,
                          "files": files }]),
    })
}

fn rand_id() -> u64 {
    use std::hash::{BuildHasher, RandomState};
    RandomState::new().hash_one(0u8)
}

async fn enqueue(
    State(s): State<Arc<Shared>>,
    axum::Json(body): axum::Json<Value>,
) -> Result<axum::Json<Value>, StatusCode> {
    let user = body["username"].as_str().ok_or(StatusCode::BAD_REQUEST)?.to_owned();
    let filename = body["files"][0]["filename"]
        .as_str()
        .ok_or(StatusCode::BAD_REQUEST)?
        .to_owned();
    let bytes = s
        .files
        .get(&format!("{user}|{filename}"))
        .or_else(|| s.files.get(&filename))
        .ok_or(StatusCode::NOT_FOUND)?
        .clone();
    let id = {
        let mut n = s.next.lock().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        *n += 1;
        format!("t{n}")
    };
    let path = s.layout.incomplete(&user, &filename).remove(0);
    let at = match std::fs::metadata(&path) {
        Ok(m) if m.len() > 0 && !s.peer.overwrite => m.len(),
        _ => 0,
    };
    if let Ok(mut starts) = s.starts.lock() {
        starts.push((user.clone(), at));
    }
    let pace = s.paces.get(&user).copied().unwrap_or(s.peer.per_tick);
    let task = tokio::spawn(download(s.clone(), id.clone(), path, filename.clone(), bytes, at, pace)).abort_handle();
    let t = Transfer {
        filename: filename.clone(),
        state: "InProgress",
        bytes: at,
        from: (at, std::time::Instant::now()),
        task: Some(Arc::new(task)),
    };
    if let Ok(mut all) = s.transfers.lock() {
        all.insert(id.clone(), t.clone());
    }
    Ok(axum::Json(
        json!({ "batch": { "transfers": [view(&id, &t, s.peer.reported)] } }),
    ))
}

/// The peer sends from `at`: appended (resume) or written from a fresh file (overwrite).
async fn download(s: Arc<Shared>, id: String, path: PathBuf, filename: String, bytes: Vec<u8>, at: u64, pace: usize) {
    let open = async {
        if let Some(dir) = path.parent() {
            tokio::fs::create_dir_all(dir).await?;
        }
        let mut o = tokio::fs::OpenOptions::new();
        if at == 0 {
            o.write(true).create(true).truncate(true)
        } else {
            o.append(true)
        }
        .open(&path)
        .await
    };
    let Ok(mut file) = open.await else { return };
    let mut pos = at as usize;
    while pos < bytes.len() {
        let end = (pos + pace).min(bytes.len());
        if file.write_all(&bytes[pos..end]).await.is_err() || file.flush().await.is_err() {
            return;
        }
        pos = end;
        if let Ok(mut all) = s.transfers.lock()
            && let Some(t) = all.get_mut(&id)
        {
            t.bytes = pos as u64;
        }
        tokio::time::sleep(TICK).await;
    }
    drop(file);
    // As slskd does: done first, then the move, then the event that says where.
    if let Ok(mut all) = s.transfers.lock()
        && let Some(t) = all.get_mut(&id)
    {
        t.state = "Completed, Succeeded";
    }
    tokio::time::sleep(s.peer.settles).await;
    let name = filename.rsplit(['\\', '/']).next().unwrap_or("song").to_owned();
    let done = s.root.join("complete").join(&name);
    if tokio::fs::rename(&path, &done).await.is_err() {
        return;
    }
    if let Ok(mut events) = s.events.lock() {
        let data = json!({ "localFilename": format!("/transfers/complete/{name}"), "transfer": { "id": id } });
        events.insert(0, json!({ "type": "DownloadFileComplete", "data": data.to_string() }));
    }
}

fn view(id: &str, t: &Transfer, reported: Option<f64>) -> Value {
    let speed = reported.unwrap_or_else(|| (t.bytes - t.from.0) as f64 / t.from.1.elapsed().as_secs_f64().max(0.001));
    json!({
        "id": id, "filename": t.filename, "state": t.state, "bytesTransferred": t.bytes,
        "averageSpeed": speed, "placeInQueue": null, "exception": null,
    })
}

async fn list(State(s): State<Arc<Shared>>, UrlPath(_user): UrlPath<String>) -> axum::Json<Value> {
    let files: Vec<Value> = s
        .transfers
        .lock()
        .map(|all| all.iter().map(|(id, t)| view(id, t, s.peer.reported)).collect())
        .unwrap_or_default();
    axum::Json(json!({ "directories": [{ "files": files }] }))
}

async fn one(
    State(s): State<Arc<Shared>>,
    UrlPath((_user, id)): UrlPath<(String, String)>,
) -> Result<axum::Json<Value>, StatusCode> {
    let all = s.transfers.lock().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    all.get(&id)
        .map(|t| axum::Json(view(&id, t, s.peer.reported)))
        .ok_or(StatusCode::NOT_FOUND)
}

async fn cancel(State(s): State<Arc<Shared>>, UrlPath((_user, id)): UrlPath<(String, String)>) -> StatusCode {
    let gone = s.transfers.lock().ok().and_then(|mut all| all.remove(&id));
    if let Some(task) = gone.and_then(|t| t.task) {
        task.abort();
    }
    StatusCode::NO_CONTENT
}

async fn events(State(s): State<Arc<Shared>>) -> axum::Json<Value> {
    axum::Json(json!(s.events.lock().map(|e| e.clone()).unwrap_or_default()))
}
