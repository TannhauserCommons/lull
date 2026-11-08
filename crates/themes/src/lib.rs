//! The listener's theme packages (ADR-0004): stored under `<data_dir>/themes/`, served to every
//! device, imported and exported as `.lulltheme`, reloaded live when a file changes. The five
//! built-in designs ship in the web UI in the same format (docs/themes.md); this crate only reads
//! their manifests, for `GET /api/themes`. Sound and easter-egg packs (ADR-0081) live inside a
//! theme or on their own under `<data_dir>/packs/` (`.lullpack`), handled the same way.

mod archive;
mod live;
mod ogg;
mod pack;
mod package;
mod packs;
mod pictures;
mod store;
mod watch;

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, put};
use axum::{Json, Router};
use lull_core::db::Migrations;
use lull_core::{Ctx, Error, Result};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::pack::Home;
use crate::package::{BUILT_IN, Manifest};
use crate::packs::PackStore;
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
        store::read_folder(path)?
    } else {
        archive::read(
            &std::fs::read(path).map_err(|e| Error::BadRequest(format!("{}: {e}", path.display())))?,
            "theme.json",
        )?
    };
    package::validate(&files, package::Origin::Listener).map(|m| m.id)
}

pub struct Service {
    store: Store,
    /// The pictures lull made of each theme (ADR-0080), beside the packages
    pictures: Pictures,
    /// The listener's standalone packs (ADR-0081)
    packs: PackStore,
    /// One writer at a time: an import and a Theme Studio save of the same theme must not interleave.
    writing: tokio::sync::Mutex<()>,
}

pub async fn start(ctx: &Ctx) -> Result<Arc<Service>> {
    let dir = ctx.config.data_dir.join("themes");
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| Error::internal(format!("create {}: {e}", dir.display())))?;
    watch::spawn(ctx, dir.clone(), "theme.changed");
    let pictures = ctx.config.data_dir.join("theme-pictures");
    tokio::fs::create_dir_all(&pictures)
        .await
        .map_err(|e| Error::internal(format!("create {}: {e}", pictures.display())))?;
    let packs = ctx.config.data_dir.join("packs");
    tokio::fs::create_dir_all(&packs)
        .await
        .map_err(|e| Error::internal(format!("create {}: {e}", packs.display())))?;
    watch::spawn(ctx, packs.clone(), "pack.changed");
    Ok(Arc::new(Service {
        store: Store { dir },
        pictures: Pictures { dir: pictures },
        packs: PackStore { dir: packs },
        writing: tokio::sync::Mutex::new(()),
    }))
}

pub fn router(svc: Arc<Service>) -> Router {
    Router::new()
        .route("/api/themes", get(list).post(import))
        .route("/api/themes/{id}", get(export).put(replace).delete(remove))
        .route("/api/themes/{id}/{*path}", get(file).put(put_file))
        .route("/api/themes/{id}/pack", put(put_theme_pack).delete(remove_theme_pack))
        .route("/api/theme-pictures/{id}/{name}", get(picture).put(put_picture))
        .route("/api/packs", get(list_packs).post(import_pack))
        .route(
            "/api/packs/{id}",
            get(export_pack).put(replace_pack).delete(remove_pack),
        )
        .route("/api/packs/{id}/{*path}", get(pack_file).put(put_pack_file))
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

/// A theme as listed. A built-in design's sounds are its pack in `web/packs/`, not in its package,
/// so only a listener's theme says `pack: true`.
fn item(m: &Manifest, built_in: bool) -> Result<Value> {
    let mut v = serde_json::to_value(m).map_err(|e| Error::internal(e.to_string()))?;
    v["builtIn"] = json!(built_in);
    v["pack"] = json!(false);
    Ok(v)
}

impl Service {
    /// A stored theme as the hub answers with it: its manifest, the `rev` of its files now and the
    /// modes lull has a picture of for that rev (ADR-0080).
    fn listed(&self, m: &Manifest) -> Result<Value> {
        let dir = self.store.dir.join(&m.id);
        let rev = pictures::rev(&dir);
        let mut v = item(m, false)?;
        v["pictures"] = json!(self.pictures.made(&m.id, &rev));
        v["rev"] = json!(rev);
        v["pack"] = json!(dir.join("pack.json").is_file());
        Ok(v)
    }

    /// A stored standalone pack as `GET /api/packs` lists it.
    fn pack_listed(&self, id: &str) -> Result<Value> {
        let p = self.packs.pack(id)?;
        let dir = self.packs.dir.join(id);
        let mut v = packs::listed(&p, &dir, &format!("/api/packs/{id}"));
        v["id"] = json!(id);
        v["name"] = json!(p.name);
        v["author"] = json!(p.author);
        v["source"] = json!("pack");
        v["theme"] = Value::Null;
        v["rev"] = json!(pictures::rev(&dir));
        Ok(v)
    }

    /// The pack a stored theme carries, named and signed as the theme.
    fn theme_pack_listed(&self, m: &Manifest) -> Result<Value> {
        let dir = self.store.dir.join(&m.id);
        let p = pack::read(&self.store.file(&m.id, "pack.json")?, Home::Theme)?;
        let mut v = packs::listed(&p, &dir, &format!("/api/themes/{}", m.id));
        if p.description.is_empty() {
            v["description"] = json!(m.description);
        }
        v["id"] = json!(m.id);
        v["name"] = json!(m.name);
        v["author"] = json!(m.author);
        v["source"] = json!("theme");
        v["theme"] = json!(m.id);
        v["rev"] = json!(pictures::rev(&dir));
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
    let files = archive::read(&body, "theme.json")?;
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
    let files = archive::read(&body, "theme.json")?;
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
    Ok(zip_download(&file, zip))
}

fn zip_download(file: &str, zip: Vec<u8>) -> Response {
    (
        [
            (header::CONTENT_TYPE, "application/zip".to_owned()),
            (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{file}\"")),
        ],
        zip,
    )
        .into_response()
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
    Ok(served(&headers, mime, data))
}

/// A package file, ETag'd and revalidated each time (its name doesn't change with it).
fn served(headers: &HeaderMap, mime: &str, data: Vec<u8>) -> Response {
    let etag = format!("\"{:016x}\"", fingerprint(&data));
    if headers.get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok()) == Some(etag.as_str()) {
        return StatusCode::NOT_MODIFIED.into_response();
    }
    (
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
        .into_response()
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

/// `PUT /api/themes/{id}/pack` (a `.lullpack`): that pack becomes the theme's, in place of its
/// `pack.json`, `pack.css`, `sounds/` and egg slots. Its id, name and author go: the theme's apply.
async fn put_theme_pack(State(svc): State<Arc<Service>>, Path(id): Path<String>, body: Bytes) -> Result<Json<Value>> {
    let id = own_id(&id)?;
    let mut incoming = archive::read(&body, "pack.json")?;
    if let Some(path) = incoming.keys().find(|p| !pack::is_pack_file(p)) {
        return Err(Error::BadRequest(format!(
            "{path} has no place in a pack (pack.json, pack.css, sounds/*.ogg, slots/egg.<name>.html)"
        )));
    }
    let raw = incoming
        .get("pack.json")
        .ok_or_else(|| Error::BadRequest("the pack has no pack.json".into()))?;
    let mut manifest: serde_json::Map<String, Value> =
        serde_json::from_slice(raw).map_err(|e| Error::BadRequest(format!("pack.json: {e}")))?;
    for key in ["id", "name", "author"] {
        manifest.remove(key);
    }
    let mut raw = serde_json::to_vec_pretty(&manifest).map_err(|e| Error::internal(format!("pack.json: {e}")))?;
    raw.push(b'\n');
    incoming.insert("pack.json".into(), raw);
    let _w = svc.writing.lock().await;
    let svc = svc.clone();
    let manifest = blocking(move || {
        let mut files = svc.store.files(&id)?;
        files.retain(|path, _| !pack::is_pack_file(path));
        files.extend(incoming);
        let m = svc.store.put(files)?;
        svc.listed(&m)
    })
    .await?;
    Ok(Json(manifest))
}

/// `DELETE /api/themes/{id}/pack`: the theme without its sounds and eggs.
async fn remove_theme_pack(State(svc): State<Arc<Service>>, Path(id): Path<String>) -> Result<StatusCode> {
    let id = own_id(&id)?;
    let _w = svc.writing.lock().await;
    let svc = svc.clone();
    blocking(move || {
        let mut files = svc.store.files(&id)?;
        let before = files.len();
        files.retain(|path, _| !pack::is_pack_file(path));
        if files.len() == before {
            return Err(Error::NotFound(format!("theme {id} has no pack")));
        }
        svc.store.put(files).map(|_| ())
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Every pack the listener added, then each theme's that carries one. The built-in packs are static
/// files of the UI (`/packs/<id>/`), which the page reads itself.
async fn list_packs(State(svc): State<Arc<Service>>) -> Result<Json<Value>> {
    let items = blocking(move || {
        let mut items = Vec::new();
        for id in svc.packs.ids()? {
            match svc.pack_listed(&id) {
                Ok(v) => items.push(v),
                Err(e) => tracing::warn!(pack = %id, error = %e, "stored pack skipped"),
            }
        }
        for m in svc.store.list()? {
            if svc.store.dir.join(&m.id).join("pack.json").is_file() {
                match svc.theme_pack_listed(&m) {
                    Ok(v) => items.push(v),
                    Err(e) => tracing::warn!(theme = %m.id, error = %e, "theme's pack skipped"),
                }
            }
        }
        Ok(items)
    })
    .await?;
    Ok(Json(json!({ "packs": items })))
}

/// `POST /api/packs` (a `.lullpack`): stores it under the id its `pack.json` names. 409 when a pack
/// has that id already, unless `?replace=true`.
async fn import_pack(
    State(svc): State<Arc<Service>>,
    Query(q): Query<ImportQuery>,
    body: Bytes,
) -> Result<Json<Value>> {
    let files = archive::read(&body, "pack.json")?;
    let _w = svc.writing.lock().await;
    let svc = svc.clone();
    let listed = blocking(move || {
        let p = pack::validate(&files, Home::Standalone)?
            .ok_or_else(|| Error::BadRequest("the pack has no pack.json".into()))?;
        let id = p.id.unwrap_or_default();
        if svc.packs.exists(&id) && !q.replace {
            return Err(Error::Conflict(format!(
                "a pack called {} ({id}) is already here",
                p.name.unwrap_or_default()
            )));
        }
        svc.packs.put(&files)?;
        svc.pack_listed(&id)
    })
    .await?;
    Ok(Json(listed))
}

/// `PUT /api/packs/{id}` (a `.lullpack`): creates or replaces that pack.
async fn replace_pack(State(svc): State<Arc<Service>>, Path(id): Path<String>, body: Bytes) -> Result<Json<Value>> {
    let id = own_pack_id(&id)?;
    let files = archive::read(&body, "pack.json")?;
    let _w = svc.writing.lock().await;
    let svc = svc.clone();
    let listed = blocking(move || {
        check_pack_id(&files, &id)?;
        svc.packs.put(&files)?;
        svc.pack_listed(&id)
    })
    .await?;
    Ok(Json(listed))
}

/// `PUT /api/packs/{id}/{path}`: one file (a replaced sound, the listener's own pack). A new pack
/// starts with its `pack.json`; every write is checked as the whole pack it makes.
async fn put_pack_file(
    State(svc): State<Arc<Service>>,
    Path((id, path)): Path<(String, String)>,
    body: Bytes,
) -> Result<Json<Value>> {
    let id = own_pack_id(&id)?;
    let _w = svc.writing.lock().await;
    let svc = svc.clone();
    let listed = blocking(move || {
        let mut files = if svc.packs.exists(&id) {
            svc.packs.files(&id)?
        } else if path == "pack.json" {
            package::Files::new()
        } else {
            return Err(Error::NotFound(format!("no pack {id}; save its pack.json first")));
        };
        files.insert(path, body.to_vec());
        check_pack_id(&files, &id)?;
        svc.packs.put(&files)?;
        svc.pack_listed(&id)
    })
    .await?;
    Ok(Json(listed))
}

/// The pack in `files` is valid and its `pack.json` says `id`.
fn check_pack_id(files: &package::Files, id: &str) -> Result<()> {
    let p = pack::validate(files, Home::Standalone)?
        .ok_or_else(|| Error::BadRequest("the pack has no pack.json".into()))?;
    match p.id.as_deref() {
        Some(said) if said == id => Ok(()),
        said => Err(Error::BadRequest(format!(
            "the pack's pack.json says id {said:?}, not {id:?}"
        ))),
    }
}

async fn remove_pack(State(svc): State<Arc<Service>>, Path(id): Path<String>) -> Result<StatusCode> {
    let id = own_pack_id(&id)?;
    let _w = svc.writing.lock().await;
    let svc = svc.clone();
    blocking(move || svc.packs.delete(&id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/packs/{id}.lullpack`: the stored pack as a zip.
async fn export_pack(State(svc): State<Arc<Service>>, Path(name): Path<String>) -> Result<Response> {
    let id = name
        .strip_suffix(".lullpack")
        .ok_or_else(|| Error::NotFound(format!("no such endpoint: /api/packs/{name}")))?;
    let id = own_pack_id(id)?;
    let file = format!("{id}.lullpack");
    let zip = blocking(move || archive::write(&svc.packs.files(&id)?)).await?;
    Ok(zip_download(&file, zip))
}

/// `GET /api/packs/{id}/{path}`: a file of a stored pack, revalidated by ETag.
async fn pack_file(
    State(svc): State<Arc<Service>>,
    Path((id, path)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    let id = own_pack_id(&id)?;
    if !package::plain_path(&path) {
        return Err(Error::NotFound(format!("pack {id} has no {path}")));
    }
    let mime = mime(&path);
    let data = blocking(move || svc.packs.file(&id, &path)).await?;
    Ok(served(&headers, mime, data))
}

/// A listener's pack id: well formed, and not one of the built-in packs (static files of the UI).
fn own_pack_id(id: &str) -> Result<String> {
    if !package::valid_id(id) || pack::BUILT_IN.contains(&id) {
        return Err(Error::NotFound(format!("no stored pack {id}")));
    }
    Ok(id.to_owned())
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
        Some("ogg") => "audio/ogg",
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

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    use super::*;
    use crate::ogg::fixture;

    fn service(name: &str) -> (Router, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!("lull-packs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for dir in ["themes", "theme-pictures", "packs"] {
            let _ = std::fs::create_dir_all(root.join(dir));
        }
        let svc = Arc::new(Service {
            store: Store {
                dir: root.join("themes"),
            },
            pictures: Pictures {
                dir: root.join("theme-pictures"),
            },
            packs: PackStore {
                dir: root.join("packs"),
            },
            writing: tokio::sync::Mutex::new(()),
        });
        (router(svc), root)
    }

    /// Status, content type and body.
    async fn send(app: &Router, method: &str, uri: &str, body: Vec<u8>) -> (StatusCode, String, Vec<u8>) {
        let Ok(req) = Request::builder().method(method).uri(uri).body(Body::from(body)) else {
            return (StatusCode::IM_A_TEAPOT, String::new(), Vec::new());
        };
        let Ok(res) = app.clone().oneshot(req).await;
        let status = res.status();
        let mime = res
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let body = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap_or_default();
        (status, mime, body.to_vec())
    }

    async fn json_at(app: &Router, uri: &str) -> Value {
        serde_json::from_slice(&send(app, "GET", uri, Vec::new()).await.2).unwrap_or(Value::Null)
    }

    fn zip(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let files: package::Files = files.iter().map(|(p, d)| ((*p).to_owned(), d.clone())).collect();
        archive::write(&files).unwrap_or_default()
    }

    const PACK: &str = r#"{ "id": "rain", "name": "Rain", "author": "you", "api": 1,
        "sounds": { "play": "sounds/tap.ogg", "buffering": { "file": "sounds/drizzle.ogg", "loop": true } },
        "eggs": [{ "name": "rainy-song", "when": [{ "words": { "in": "title", "any": "rain" } }], "sound": "sounds/tap.ogg" }] }"#;

    fn lullpack() -> Vec<(&'static str, Vec<u8>)> {
        vec![
            ("pack.json", PACK.as_bytes().to_vec()),
            ("sounds/tap.ogg", fixture::opus(0.2)),
            ("sounds/drizzle.ogg", fixture::opus(6.0)),
            ("slots/egg.rainy-song.html", b"<i>{{word}}</i>".to_vec()),
        ]
    }

    #[tokio::test]
    async fn a_theme_that_carries_a_pack_imports_lists_it_and_can_have_it_replaced_or_taken_out() {
        let (app, root) = service("theme");
        let theme = br##"{ "id": "mine", "name": "Mine", "author": "me", "api": 1, "shell": "rail",
            "swatches": ["#ffffff", "#000000", "#123456", "#abcdef"], "modes": ["day"] }"##;
        let package = zip(&[
            ("theme.json", theme.to_vec()),
            ("theme.css", Vec::new()),
            (
                "pack.json",
                br#"{ "api": 1, "sounds": { "boot": "sounds/start.ogg" } }"#.to_vec(),
            ),
            ("pack.css", b":root[data-egg~=\"x\"] { color: red }".to_vec()),
            ("sounds/start.ogg", fixture::opus(5.0)),
        ]);
        assert_eq!(send(&app, "POST", "/api/themes", package).await.0, StatusCode::OK);
        let themes = json_at(&app, "/api/themes").await;
        let mine = themes["items"]
            .as_array()
            .and_then(|t| t.iter().find(|t| t["id"] == "mine"))
            .cloned();
        assert_eq!(mine.map(|t| t["pack"].clone()), Some(json!(true)));
        let packs = json_at(&app, "/api/packs").await;
        let listed = &packs["packs"][0];
        assert_eq!(
            (&listed["source"], &listed["theme"], &listed["name"], &listed["author"]),
            (&json!("theme"), &json!("mine"), &json!("Mine"), &json!("me"))
        );
        assert_eq!(listed["sounds"]["boot"], "/api/themes/mine/sounds/start.ogg");
        assert_eq!(listed["css"], "/api/themes/mine/pack.css");
        let (status, mime, _) = send(&app, "GET", "/api/themes/mine/sounds/start.ogg", Vec::new()).await;
        assert_eq!((status, mime.as_str()), (StatusCode::OK, "audio/ogg"));

        // a .lullpack put into the theme: its own name goes, the theme's stays
        assert_eq!(
            send(&app, "PUT", "/api/themes/mine/pack", zip(&lullpack())).await.0,
            StatusCode::OK
        );
        let listed = json_at(&app, "/api/packs").await["packs"][0].clone();
        assert_eq!((&listed["name"], &listed["css"]), (&json!("Mine"), &Value::Null));
        assert_eq!(
            listed["sounds"]["buffering"],
            json!({ "url": "/api/themes/mine/sounds/drizzle.ogg", "loop": true })
        );
        assert_eq!(listed["eggs"][0]["sound"], "/api/themes/mine/sounds/tap.ogg");
        assert_eq!(listed["slots"], json!(["rainy-song"]));

        assert_eq!(
            send(&app, "DELETE", "/api/themes/mine/pack", Vec::new()).await.0,
            StatusCode::NO_CONTENT
        );
        assert_eq!(json_at(&app, "/api/packs").await, json!({ "packs": [] }));
        assert!(!root.join("themes/mine/sounds").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn a_lullpack_imports_lists_exports_the_same_and_deletes() {
        let (app, root) = service("round-trip");
        let file = zip(&lullpack());
        assert_eq!(send(&app, "POST", "/api/packs", file.clone()).await.0, StatusCode::OK);
        assert_eq!(send(&app, "POST", "/api/packs", file).await.0, StatusCode::CONFLICT);
        let listed = json_at(&app, "/api/packs").await["packs"][0].clone();
        assert_eq!(
            (&listed["id"], &listed["source"], &listed["theme"]),
            (&json!("rain"), &json!("pack"), &Value::Null)
        );
        assert_eq!(listed["sounds"]["play"], "/api/packs/rain/sounds/tap.ogg");
        assert_eq!(
            listed["eggs"][0]["when"],
            json!([{ "words": { "in": "title", "any": "rain" } }])
        );
        let (status, mime, _) = send(&app, "GET", "/api/packs/rain/sounds/drizzle.ogg", Vec::new()).await;
        assert_eq!((status, mime.as_str()), (StatusCode::OK, "audio/ogg"));

        let (status, _, exported) = send(&app, "GET", "/api/packs/rain.lullpack", Vec::new()).await;
        let want: package::Files = lullpack().into_iter().map(|(p, d)| (p.to_owned(), d)).collect();
        assert_eq!(
            (status, archive::read(&exported, "pack.json").ok()),
            (StatusCode::OK, Some(want))
        );

        assert_eq!(
            send(&app, "DELETE", "/api/packs/rain", Vec::new()).await.0,
            StatusCode::NO_CONTENT
        );
        assert_eq!(json_at(&app, "/api/packs").await, json!({ "packs": [] }));
        assert_eq!(
            send(&app, "GET", "/api/packs/default/pack.json", Vec::new()).await.0,
            StatusCode::NOT_FOUND
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
