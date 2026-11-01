//! The listener's theme packages (ADR-0004): stored under `<data_dir>/themes/`, served to every
//! device, imported and exported as `.lulltheme`, reloaded live when a file changes. The five
//! built-in designs ship in the web UI in the same format (docs/themes.md); this crate only reads
//! their manifests, for `GET /api/themes`.

mod archive;
mod package;
mod store;
mod watch;

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use lull_core::db::Migrations;
use lull_core::{Ctx, Error, Result};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::package::{BUILT_IN, Manifest};
use crate::store::Store;

pub const MIGRATIONS: Migrations = Migrations {
    owner: "themes",
    steps: &[],
};

/// The built-in designs' manifests, compiled in from the web UI they ship with.
const BUILT_IN_MANIFESTS: [&str; 5] = [
    include_str!("../../../web/designs/lull/theme.json"),
    include_str!("../../../web/designs/skydeck/theme.json"),
    include_str!("../../../web/designs/starlight/theme.json"),
    include_str!("../../../web/designs/petal/theme.json"),
    include_str!("../../../web/designs/glasshouse/theme.json"),
];

pub struct Service {
    store: Store,
    /// One writer at a time: an import and a Theme Studio save of the same theme must not interleave.
    writing: tokio::sync::Mutex<()>,
}

pub async fn start(ctx: &Ctx) -> Result<Arc<Service>> {
    let dir = ctx.config.data_dir.join("themes");
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| Error::internal(format!("create {}: {e}", dir.display())))?;
    watch::spawn(ctx, dir.clone());
    Ok(Arc::new(Service {
        store: Store { dir },
        writing: tokio::sync::Mutex::new(()),
    }))
}

pub fn router(svc: Arc<Service>) -> Router {
    Router::new()
        .route("/api/themes", get(list).post(import))
        .route("/api/themes/{id}", get(export).put(replace).delete(remove))
        .route("/api/themes/{id}/{*path}", get(file).put(put_file))
        .layer(DefaultBodyLimit::max(package::MAX_PACKAGE_BYTES + 64 * 1024))
        .with_state(svc)
}

/// The five designs, then the listener's themes, each manifest plus `builtIn`.
async fn list(State(svc): State<Arc<Service>>) -> Result<Json<Value>> {
    let mut items = Vec::new();
    for raw in BUILT_IN_MANIFESTS {
        let m: Manifest =
            serde_json::from_str(raw).map_err(|e| Error::internal(format!("built-in theme.json: {e}")))?;
        items.push(item(&m, true)?);
    }
    let stored = blocking(move || svc.store.list()).await?;
    for m in stored {
        items.push(item(&m, false)?);
    }
    Ok(Json(json!({ "items": items })))
}

fn item(m: &Manifest, built_in: bool) -> Result<Value> {
    let mut v = serde_json::to_value(m).map_err(|e| Error::internal(e.to_string()))?;
    v["builtIn"] = json!(built_in);
    Ok(v)
}

#[derive(Deserialize)]
struct ImportQuery {
    #[serde(default)]
    replace: bool,
}

/// `POST /api/themes` (a `.lulltheme`): stores it under the id its manifest names. 409 when a
/// theme has that id already, unless `?replace=true`.
async fn import(State(svc): State<Arc<Service>>, Query(q): Query<ImportQuery>, body: Bytes) -> Result<Json<Value>> {
    let files = archive::read(&body)?;
    let _w = svc.writing.lock().await;
    let svc = svc.clone();
    let manifest = blocking(move || {
        let m = package::validate(&files, package::Origin::Listener)?;
        if svc.store.exists(&m.id) && !q.replace {
            return Err(Error::Conflict(format!(
                "a theme called {} ({}) is already here",
                m.name, m.id
            )));
        }
        svc.store.put(files)
    })
    .await?;
    Ok(Json(item(&manifest, false)?))
}

/// `PUT /api/themes/{id}` (a `.lulltheme`): creates or replaces that theme.
async fn replace(State(svc): State<Arc<Service>>, Path(id): Path<String>, body: Bytes) -> Result<Json<Value>> {
    let id = own_id(&id)?;
    let files = archive::read(&body)?;
    let _w = svc.writing.lock().await;
    let svc = svc.clone();
    let manifest = blocking(move || {
        let m = package::validate(&files, package::Origin::Listener)?;
        if m.id != id {
            return Err(Error::BadRequest(format!(
                "the package's theme.json says id {:?}, not {id:?}",
                m.id
            )));
        }
        svc.store.put(files)
    })
    .await?;
    Ok(Json(item(&manifest, false)?))
}

/// `PUT /api/themes/{id}/{path}` (Theme Studio): one file. A new theme starts with its theme.json
/// (and gets an empty theme.css); every write is checked as the whole package it makes.
async fn put_file(
    State(svc): State<Arc<Service>>,
    Path((id, path)): Path<(String, String)>,
    body: Bytes,
) -> Result<Json<Value>> {
    let id = own_id(&id)?;
    let _w = svc.writing.lock().await;
    let svc = svc.clone();
    let manifest = blocking(move || {
        let mut files = if svc.store.exists(&id) {
            svc.store.files(&id)?
        } else if path == "theme.json" {
            package::Files::from([("theme.css".to_owned(), Vec::new())])
        } else {
            return Err(Error::NotFound(format!("no theme {id}; save its theme.json first")));
        };
        files.insert(path, body.to_vec());
        let m = package::validate(&files, package::Origin::Listener)?;
        if m.id != id {
            return Err(Error::BadRequest(format!("theme.json says id {:?}, not {id:?}", m.id)));
        }
        svc.store.put(files)
    })
    .await?;
    Ok(Json(item(&manifest, false)?))
}

async fn remove(State(svc): State<Arc<Service>>, Path(id): Path<String>) -> Result<StatusCode> {
    let id = own_id(&id)?;
    let _w = svc.writing.lock().await;
    let svc = svc.clone();
    blocking(move || svc.store.delete(&id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/themes/{id}.lulltheme`: the stored package as a zip.
async fn export(State(svc): State<Arc<Service>>, Path(name): Path<String>) -> Result<Response> {
    let id = name
        .strip_suffix(".lulltheme")
        .ok_or_else(|| Error::NotFound(format!("no such endpoint: /api/themes/{name}")))?;
    let id = own_id(id)?;
    let file = format!("{id}.lulltheme");
    let zip = blocking(move || archive::write(&svc.store.files(&id)?)).await?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/zip".to_owned()),
            (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{file}\"")),
        ],
        zip,
    )
        .into_response())
}

/// `GET /api/themes/{id}/{path}`: a file of a stored theme, revalidated by ETag.
async fn file(
    State(svc): State<Arc<Service>>,
    Path((id, path)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    let id = own_id(&id)?;
    if !path.split('/').all(|s| !s.is_empty() && !s.starts_with('.')) {
        return Err(Error::NotFound(format!("theme {id} has no {path}")));
    }
    let mime = mime(&path);
    let data = blocking(move || svc.store.file(&id, &path)).await?;
    let etag = format!("\"{:016x}\"", fingerprint(&data));
    if headers.get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok()) == Some(etag.as_str()) {
        return Ok(StatusCode::NOT_MODIFIED.into_response());
    }
    Ok((
        [
            (header::CONTENT_TYPE, mime.to_owned()),
            (header::ETAG, etag),
            (header::CACHE_CONTROL, "no-cache".to_owned()),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_owned()),
            // opened on its own (not as a stylesheet or image of the app), a theme file can't run anything
            (
                header::CONTENT_SECURITY_POLICY,
                "sandbox; default-src 'none'; img-src 'self' data:; style-src 'unsafe-inline'".to_owned(),
            ),
        ],
        data,
    )
        .into_response())
}

/// A listener's theme id: well formed, and not one of the built-ins (they are static files of the UI).
fn own_id(id: &str) -> Result<String> {
    if !package::valid_id(id) || BUILT_IN.contains(&id) {
        return Err(Error::NotFound(format!("no stored theme {id}")));
    }
    Ok(id.to_owned())
}

fn mime(path: &str) -> &'static str {
    match path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).as_deref() {
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("html") => "text/html; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("avif") => "image/avif",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("ttf") => "font/ttf",
        Some("otf") => "font/otf",
        _ => "application/octet-stream",
    }
}

fn fingerprint(data: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    data.hash(&mut h);
    h.finish()
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| Error::internal(format!("themes task: {e}")))?
}
