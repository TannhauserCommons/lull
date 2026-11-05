//! The listener's theme packages (ADR-0004): stored under `<data_dir>/themes/`, served to every
//! device, imported and exported as `.lulltheme`, reloaded live when a file changes. The five
//! built-in designs ship in the web UI in the same format (docs/themes.md); this crate only reads
//! their manifests, for `GET /api/themes`.

mod archive;
mod package;
mod pictures;
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
use crate::pictures::Pictures;
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

/// Checks a package as the hub's import does, from a `.lulltheme` or a package folder, and returns its id: a package
/// lull's own tooling makes passes here before anyone shares it (`cargo run -p lull-themes --bin lull-theme-check`).
pub fn check(path: &std::path::Path) -> Result<String> {
    let files = if path.is_dir() {
        store::folder(path)?
    } else {
        archive::read(&std::fs::read(path).map_err(|e| Error::BadRequest(format!("{}: {e}", path.display())))?)?
    };
    package::validate(&files, package::Origin::Listener).map(|m| m.id)
}

pub struct Service {
    store: Store,
    /// The pictures lull made of each theme (ADR-0080), beside the packages
    pictures: Pictures,
    /// One writer at a time: an import and a Theme Studio save of the same theme must not interleave.
    writing: tokio::sync::Mutex<()>,
}

pub async fn start(ctx: &Ctx) -> Result<Arc<Service>> {
    let dir = ctx.config.data_dir.join("themes");
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| Error::internal(format!("create {}: {e}", dir.display())))?;
    watch::spawn(ctx, dir.clone());
    let pictures = ctx.config.data_dir.join("theme-pictures");
    tokio::fs::create_dir_all(&pictures)
        .await
        .map_err(|e| Error::internal(format!("create {}: {e}", pictures.display())))?;
    Ok(Arc::new(Service {
        store: Store { dir },
        pictures: Pictures { dir: pictures },
        writing: tokio::sync::Mutex::new(()),
    }))
}

pub fn router(svc: Arc<Service>) -> Router {
    Router::new()
        .route("/api/themes", get(list).post(import))
        .route("/api/themes/{id}", get(export).put(replace).delete(remove))
        .route("/api/themes/{id}/{*path}", get(file).put(put_file))
        .route("/api/theme-pictures/{id}/{name}", get(picture).put(put_picture))
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
    let stored = blocking(move || {
        svc.store
            .list()?
            .iter()
            .map(|m| svc.listed(m))
            .collect::<Result<Vec<_>>>()
    })
    .await?;
    items.extend(stored);
    Ok(Json(json!({ "items": items })))
}

fn item(m: &Manifest, built_in: bool) -> Result<Value> {
    let mut v = serde_json::to_value(m).map_err(|e| Error::internal(e.to_string()))?;
    v["builtIn"] = json!(built_in);
    Ok(v)
}

impl Service {
    /// A stored theme as the hub answers with it: its manifest, the `rev` of its files now and the
    /// modes lull has a picture of for that rev (ADR-0080).
    fn listed(&self, m: &Manifest) -> Result<Value> {
        let rev = pictures::rev(&self.store.dir.join(&m.id));
        let mut v = item(m, false)?;
        v["pictures"] = json!(self.pictures.made(&m.id, &rev));
        v["rev"] = json!(rev);
        Ok(v)
    }
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
        let m = svc.store.put(files)?;
        svc.listed(&m)
    })
    .await?;
    Ok(Json(manifest))
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
        let m = svc.store.put(files)?;
        svc.listed(&m)
    })
    .await?;
    Ok(Json(manifest))
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
        let m = svc.store.put(files)?;
        svc.listed(&m)
    })
    .await?;
    Ok(Json(manifest))
}

async fn remove(State(svc): State<Arc<Service>>, Path(id): Path<String>) -> Result<StatusCode> {
    let id = own_id(&id)?;
    let _w = svc.writing.lock().await;
    let svc = svc.clone();
    blocking(move || {
        svc.store.delete(&id)?;
        svc.pictures.delete(&id)
    })
    .await?;
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

/// `GET /api/theme-pictures/{id}/{rev}-{mode}.webp`: a picture lull made of the theme. Its name
/// changes with the theme, so it never needs asking again.
async fn picture(State(svc): State<Arc<Service>>, Path((id, name)): Path<(String, String)>) -> Result<Response> {
    let id = own_id(&id)?;
    let (rev, mode) = pictures::parse_name(&name)
        .map(|(rev, mode)| (rev.to_owned(), mode))
        .ok_or_else(|| Error::NotFound(format!("theme {id} has no picture {name}")))?;
    let data = blocking(move || svc.pictures.file(&id, &rev, mode)).await?;
    Ok((
        [
            (header::CONTENT_TYPE, "image/webp"),
            (header::CACHE_CONTROL, "private, max-age=31536000, immutable"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        data,
    )
        .into_response())
}

/// `PUT /api/theme-pictures/{id}/{rev}-{mode}.webp` (a 320×200 WebP a page drew): kept while `rev`
/// is still the theme's; 409 once its files changed, so a picture of an older theme is never kept.
async fn put_picture(
    State(svc): State<Arc<Service>>,
    Path((id, name)): Path<(String, String)>,
    body: Bytes,
) -> Result<StatusCode> {
    let id = own_id(&id)?;
    let (rev, mode) = pictures::parse_name(&name)
        .map(|(rev, mode)| (rev.to_owned(), mode))
        .ok_or_else(|| Error::BadRequest(format!("{name:?} is not <rev>-<day|night>.webp")))?;
    let _w = svc.writing.lock().await;
    let svc = svc.clone();
    blocking(move || {
        let m = svc.store.manifest(&id)?;
        if !m.modes.contains(&mode) {
            return Err(Error::BadRequest(format!("{} has no {} mode", m.name, mode.word())));
        }
        if pictures::rev(&svc.store.dir.join(&id)) != rev {
            return Err(Error::Conflict(format!(
                "{} changed while its picture was drawn; draw it again",
                m.name
            )));
        }
        svc.pictures.put(&id, &rev, mode, &body)
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
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
