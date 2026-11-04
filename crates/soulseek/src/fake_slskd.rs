//! A stand-in for slskd's download API in tests, writing files as slskd 0.26 does: a download
//! starts from the length of its incomplete file (`retry.partial: resume`, the default) or from 0
//! (`overwrite`), appends the peer's bytes at a set pace, then moves the file to the downloads
//! folder and reports `DownloadFileComplete`, in slskd's order: the transfer reads `Succeeded`
//! before the file has moved and before the event says where (slskd 0.26.2's
//! `DownloadService`). This lets lull's fetches be tested in-process against it.

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

pub struct Fake {
    pub slskd: Slskd,
    pub layout: Layout,
    /// The transfers folder: slskd's `incomplete` and `complete` are inside it.
    pub root: PathBuf,
}

struct Shared {
    /// The peers' files, by remote path: every peer shares these, unless `user|path` names one
    /// peer's own bytes for that path.
    files: HashMap<String, Vec<u8>>,
    layout: Layout,
    root: PathBuf,
    peer: Peer,
    transfers: Mutex<HashMap<String, Transfer>>,
    events: Mutex<Vec<Value>>,
    next: Mutex<u32>,
}

#[derive(Clone)]
struct Transfer {
    filename: String,
    state: &'static str,
    bytes: u64,
    task: Option<Arc<tokio::task::AbortHandle>>,
}

/// How the peer and slskd behave.
#[derive(Clone, Copy)]
pub struct Peer {
    /// Bytes sent every `TICK`.
    pub per_tick: usize,
    /// The speed slskd reports, bytes a second.
    pub reported: f64,
    /// slskd set to `retry.partial: overwrite`: every download starts over from byte 0.
    pub overwrite: bool,
    /// How long after a transfer reads `Succeeded` slskd moves the file and says where.
    pub settles: Duration,
}

impl Fake {
    /// slskd serving `files` (remote path to bytes) from one peer.
    pub async fn start(files: HashMap<String, Vec<u8>>, peer: Peer) -> std::io::Result<Fake> {
        let root = std::env::temp_dir().join(format!("lull-fake-slskd-{}-{}", std::process::id(), rand_id()));
        std::fs::create_dir_all(root.join("incomplete"))?;
        std::fs::create_dir_all(root.join("complete"))?;
        let layout = Layout::new(&root, None);
        let shared = Arc::new(Shared {
            files,
            layout: layout.clone(),
            root: root.clone(),
            peer,
            transfers: Mutex::default(),
            events: Mutex::default(),
            next: Mutex::new(0),
        });
        let app = Router::new()
            .route("/api/v0/transfers/downloads/batches", post(enqueue))
            .route("/api/v0/transfers/downloads/{user}", get(list))
            .route(
                "/api/v0/transfers/downloads/{user}/{id}",
                get(one).merge(delete(cancel)),
            )
            .route("/api/v0/events", get(events))
            .with_state(shared);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}", listener.local_addr()?);
        tokio::spawn(async move { axum::serve(listener, app).await });
        let slskd = Slskd::new(
            lull_core::http::Http::new().map_err(std::io::Error::other)?,
            &url,
            lull_core::config::Secret::new("key"),
        )
        .map_err(std::io::Error::other)?;
        Ok(Fake { slskd, layout, root })
    }
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
    let task = tokio::spawn(download(s.clone(), id.clone(), path, filename.clone(), bytes, at)).abort_handle();
    let t = Transfer {
        filename: filename.clone(),
        state: "InProgress",
        bytes: at,
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
async fn download(s: Arc<Shared>, id: String, path: PathBuf, filename: String, bytes: Vec<u8>, at: u64) {
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
        let end = (pos + s.peer.per_tick).min(bytes.len());
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

fn view(id: &str, t: &Transfer, speed: f64) -> Value {
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
