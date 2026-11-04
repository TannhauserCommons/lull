//! Classic mode skins: Winamp Skin Museum search (proxied), `.wsz` download, validation and
//! cache, favourites, and the listener's own skins: the hub's folder, which skins added from any
//! device go to, and `LULL_SKINS_DIR`, read alongside it (docs/context/skins.md).

mod folder;
mod museum;
mod wsz;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::rejection::QueryRejection;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use lull_core::db::Migrations;
use lull_core::{Ctx, Error, Result};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::folder::Folder;
use crate::museum::{Museum, SERVICE};

pub const MIGRATIONS: Migrations = Migrations {
    owner: "skins",
    steps: &[
        r#"
        -- What the drawer shows for a favourite, so the list works while the museum is away.
        CREATE TABLE skins_favourites (
            md5             TEXT PRIMARY KEY,
            filename        TEXT NOT NULL,
            screenshot_url  TEXT NOT NULL,
            museum_url      TEXT NOT NULL,
            nsfw            INTEGER NOT NULL,
            added_at        INTEGER NOT NULL
        );
    "#,
        r#"
        -- The skins folder (LULL_SKINS_DIR) as last read: a rescan reads again only the files
        -- whose size or time changed.
        CREATE TABLE skins_folder (
            root      TEXT NOT NULL,       -- the folder read; rows of one no longer used are dropped
            path      TEXT NOT NULL,       -- under it, '/'-separated
            size      INTEGER NOT NULL,
            modified  INTEGER NOT NULL,    -- ns since 1970; -1: unknown, so always read again
            md5       TEXT,                -- NULL when the file isn't a skin,
            problem   TEXT,                -- and this says why
            PRIMARY KEY (root, path)
        );
    "#,
    ],
};

/// Classic skins are a few hundred KB; anything far bigger is not one.
const MAX_SKIN_BYTES: usize = 16 * 1024 * 1024;

pub struct Service {
    ctx: Ctx,
    museum: Museum,
    /// `<data dir>/skins/<md5>.wsz`: validated downloads, kept for good (a skin never changes).
    dir: PathBuf,
    /// `<data dir>/skins/mine`: the listener's own skins, added from any device and removable.
    own: Arc<Folder>,
    /// `LULL_SKINS_DIR`, when set: a folder the listener keeps themselves, read but never changed.
    folder: Option<Arc<Folder>>,
    /// One add or removal at a time, so a skin added twice at once is kept once.
    changing: tokio::sync::Mutex<()>,
}

pub async fn start(ctx: &Ctx) -> Result<Arc<Service>> {
    let dir = ctx.config.data_dir.join("skins");
    let own = dir.join("mine");
    tokio::fs::create_dir_all(&own)
        .await
        .map_err(|e| Error::internal(format!("create {}: {e}", own.display())))?;
    let roots: Vec<PathBuf> = std::iter::once(own.clone())
        .chain(ctx.config.skins_dir.clone())
        .collect();
    ctx.db
        .call(move |conn| folder::forget_others(conn, &roots.iter().map(PathBuf::as_path).collect::<Vec<_>>()))
        .await?;
    Ok(Arc::new(Service {
        ctx: ctx.clone(),
        museum: Museum::new(ctx.http.clone()),
        dir,
        own: Folder::start(ctx, own),
        folder: ctx.config.skins_dir.clone().map(|root| Folder::start(ctx, root)),
        changing: tokio::sync::Mutex::new(()),
    }))
}

/// `/api/skins/...`.
pub fn router(svc: Arc<Service>) -> Router {
    Router::new()
        .route("/api/skins", get(search))
        .route("/api/skins/favourites", get(favourites))
        .route(
            "/api/skins/mine",
            get(mine).post(add).layer(DefaultBodyLimit::max(MAX_SKIN_BYTES)),
        )
        .route("/api/skins/mine/rescan", post(rescan))
        .route("/api/skins/mine/{md5}", delete(remove))
        .route(
            "/api/skins/favourites/{md5}",
            put(add_favourite).delete(remove_favourite),
        )
        // `{md5}.wsz`: the router has no suffix patterns, so `download` checks the name.
        .route("/api/skins/{file}", get(download))
        .with_state(svc)
}

/// A skin as the drawer shows it (`MOCK.skins`, with museum links instead of palettes).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SkinItem {
    md5: String,
    name: String,
    file: String,
    screenshot_url: String,
    museum_url: String,
    nsfw: bool,
    favourite: bool,
    downloaded: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchQuery {
    #[serde(default)]
    q: String,
    #[serde(default)]
    offset: usize,
    /// The listener's "Show NSFW skins" setting (per device, so the UI passes it).
    #[serde(default)]
    nsfw: bool,
}

/// `{ items, more }`: a search page, or the museum's own order when `q` is empty.
async fn search(State(svc): State<Arc<Service>>, q: Result<Query<SearchQuery>, QueryRejection>) -> Result<Json<Value>> {
    let Query(q) = q.map_err(|e| Error::BadRequest(e.body_text()))?;
    let query = q.q.trim();
    if query.chars().count() > 100 {
        return Err(Error::BadRequest("A skin search is at most 100 characters.".into()));
    }
    if q.offset > 100_000 {
        return Err(Error::BadRequest("offset is past the end of the museum.".into()));
    }
    let page = svc.museum.page(query, q.offset, q.nsfw).await?;
    let favourites = svc.ctx.db.call(|conn| favourite_md5s(conn)).await?;
    let items: Vec<SkinItem> = page
        .skins
        .into_iter()
        .map(|s| {
            let favourite = favourites.contains(&s.md5);
            svc.item(s.md5, s.filename, s.screenshot_url, s.museum_url, s.nsfw, favourite)
        })
        .collect();
    Ok(Json(json!({ "items": items, "more": page.more })))
}

async fn favourites(State(svc): State<Arc<Service>>) -> Result<Json<Value>> {
    let rows = svc
        .ctx
        .db
        .call(|conn| {
            Ok(conn
                .prepare(
                    "SELECT md5, filename, screenshot_url, museum_url, nsfw FROM skins_favourites
                     ORDER BY added_at DESC, rowid DESC",
                )?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?
                .collect::<rusqlite::Result<Vec<(String, String, String, String, bool)>>>()?)
        })
        .await?;
    let items: Vec<SkinItem> = rows
        .into_iter()
        .map(|(md5, file, shot, museum, nsfw)| svc.item(md5, file, shot, museum, nsfw, true))
        .collect();
    Ok(Json(json!({ "items": items })))
}

/// The museum is asked once, so a favourite always names a real skin and the list renders offline.
async fn add_favourite(State(svc): State<Arc<Service>>, Path(md5): Path<String>) -> Result<StatusCode> {
    let md5 = valid_md5(&md5)?;
    let skin = svc
        .museum
        .skin(&md5)
        .await?
        .ok_or_else(|| Error::NotFound(format!("The {SERVICE} has no skin {md5}.")))?
        .skin;
    svc.ctx
        .db
        .call(move |conn| {
            conn.execute(
                "INSERT INTO skins_favourites (md5, filename, screenshot_url, museum_url, nsfw, added_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT(md5) DO NOTHING",
                params![
                    skin.md5,
                    skin.filename,
                    skin.screenshot_url,
                    skin.museum_url,
                    skin.nsfw,
                    lull_core::now()
                ],
            )?;
            Ok(())
        })
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove_favourite(State(svc): State<Arc<Service>>, Path(md5): Path<String>) -> Result<StatusCode> {
    let md5 = valid_md5(&md5)?;
    svc.ctx
        .db
        .call(move |conn| {
            conn.execute("DELETE FROM skins_favourites WHERE md5 = ?1", [md5])?;
            Ok(())
        })
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/skins/mine`: the listener's own skins, once both folders have been read.
async fn mine(State(svc): State<Arc<Service>>) -> Json<Value> {
    Json(svc.mine_answer().await)
}

/// `POST /api/skins/mine/rescan`: reads both folders now (for a folder whose changes the hub can't
/// see).
async fn rescan(State(svc): State<Arc<Service>>) -> Result<Json<Value>> {
    svc.own.rescan().await?;
    if let Some(folder) = &svc.folder {
        folder.rescan().await?;
    }
    Ok(Json(svc.mine_answer().await))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AddQuery {
    /// The file's name on the device it came from.
    name: String,
}

/// `POST /api/skins/mine?name=` (body: the `.wsz`, at most 16 MiB): keeps the skin in the hub's
/// folder, checked as a download is. 201 `{ added: true, item }`; 200 `{ added: false, item }` when
/// the listener has it already (the same bytes, in either folder).
async fn add(
    State(svc): State<Arc<Service>>,
    q: Result<Query<AddQuery>, QueryRejection>,
    body: Bytes,
) -> Result<(StatusCode, Json<Value>)> {
    let Query(q) = q.map_err(|e| Error::BadRequest(e.body_text()))?;
    let name = file_name(&q.name);
    wsz::validate(&body).map_err(|why| Error::BadRequest(format!("{name} isn't a classic skin: {why}.")))?;
    let md5 = folder::md5_hex(&body);
    let _one = svc.changing.lock().await;
    if let Some(item) = svc.item_of(&md5).await {
        return Ok((StatusCode::OK, Json(json!({ "added": false, "item": item }))));
    }
    let root = svc.own.root().to_owned();
    tokio::task::spawn_blocking(move || keep(&root, &name, &body))
        .await
        .map_err(|e| Error::internal(format!("keeping a skin: {e}")))??;
    svc.own.rescan().await?;
    let item = svc
        .item_of(&md5)
        .await
        .ok_or_else(|| Error::internal("a skin just kept isn't in the hub's folder"))?;
    tracing::info!(md5, "a skin was added");
    Ok((StatusCode::CREATED, Json(json!({ "added": true, "item": item }))))
}

/// `DELETE /api/skins/mine/{md5}`: removes the skin from the hub's folder (every copy). 409 for a
/// skin only in `LULL_SKINS_DIR`, which lull never changes.
async fn remove(State(svc): State<Arc<Service>>, Path(md5): Path<String>) -> Result<StatusCode> {
    let md5 = valid_md5(&md5)?;
    let _one = svc.changing.lock().await;
    let paths = svc.own.paths(&md5);
    if paths.is_empty() {
        let item = svc.item_of(&md5).await;
        return Err(match (&svc.folder, item) {
            (Some(folder), Some(item)) => Error::Conflict(format!(
                "{} is in your skins folder {}, which lull only reads: remove it there.",
                item["file"].as_str().unwrap_or("That skin"),
                folder.root().display()
            )),
            _ => Error::NotFound(format!("None of your skins is {md5}.")),
        });
    }
    for path in paths {
        match tokio::fs::remove_file(&path).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(Error::internal(format!("remove {}: {e}", path.display()))),
        }
    }
    svc.own.rescan().await?;
    tracing::info!(md5, "a skin was removed");
    Ok(StatusCode::NO_CONTENT)
}

/// A name for the file on the hub: the device's, without folders or characters a file system
/// refuses, ending in `.wsz`.
fn file_name(given: &str) -> String {
    let base = given.rsplit(['/', '\\']).next().unwrap_or_default();
    let stem = match base.rsplit_once('.') {
        Some((stem, ext)) if ext.eq_ignore_ascii_case("wsz") => stem,
        _ => base,
    };
    let clean: String = stem
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
        .take(80)
        .collect();
    let clean = clean.trim().trim_matches('.').trim();
    // Names Windows keeps for devices, should the hub's data be copied there.
    let up = clean.to_ascii_uppercase();
    let reserved = matches!(up.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (up.len() == 4 && (up.starts_with("COM") || up.starts_with("LPT")) && up.as_bytes()[3].is_ascii_digit());
    match clean {
        "" => "Skin.wsz".to_owned(),
        _ if reserved => format!("{clean} skin.wsz"),
        _ => format!("{clean}.wsz"),
    }
}

/// Writes the skin into `root` under `name`, or `name (2)` and so on when that is taken, beside its
/// final name first so the watcher never reads half a skin.
fn keep(root: &std::path::Path, name: &str, bytes: &[u8]) -> Result<()> {
    let stem = name.strip_suffix(".wsz").unwrap_or(name);
    let target = (1..1000)
        .map(|n| match n {
            1 => root.join(name),
            n => root.join(format!("{stem} ({n}).wsz")),
        })
        .find(|p| !p.exists())
        .ok_or_else(|| Error::Conflict(format!("The hub already keeps a thousand skins named {name}.")))?;
    let part = root.join(format!(".{stem}.part"));
    let io = |e: std::io::Error| Error::internal(format!("keep {}: {e}", target.display()));
    std::fs::write(&part, bytes).map_err(io)?;
    std::fs::rename(&part, &target).map_err(io)
}

/// `GET /api/skins/{md5}.wsz`: from the skins folder, else the cache, else downloaded from the
/// museum, validated and cached. The museum's files send no CORS headers, so Webamp loads them
/// from here, same origin.
async fn download(State(svc): State<Arc<Service>>, Path(file): Path<String>) -> Result<Response> {
    let md5 = file
        .strip_suffix(".wsz")
        .ok_or_else(|| Error::NotFound(format!("No skin file {file}.")))
        .and_then(valid_md5)?;
    let mine = match (svc.own.file(&md5).await, &svc.folder) {
        (Some(bytes), _) => Some(bytes),
        (None, Some(folder)) => folder.file(&md5).await,
        (None, None) => None,
    };
    let path = svc.path(&md5);
    let bytes = match mine {
        Some(bytes) => bytes,
        None => match tokio::fs::read(&path).await {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => svc.fetch(&md5).await?,
            Err(e) => return Err(Error::internal(format!("read {}: {e}", path.display()))),
        },
    };
    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream"),
            // Named by its md5, so it never changes.
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        bytes,
    )
        .into_response())
}

impl Service {
    /// The listener's own skins: the hub's folder first, then `LULL_SKINS_DIR`'s skins it doesn't
    /// hold, each with its `source` (`hub`, removable here, or `folder`). `folder`, `watching` and
    /// `problem` describe `LULL_SKINS_DIR` (`problem` also says when the hub's own can't be read).
    async fn mine_answer(&self) -> Value {
        let own = self.own.listing().await;
        let extra = match &self.folder {
            Some(f) => Some((f, f.listing().await)),
            None => None,
        };
        let tagged = |v: Value, source: &str| match v {
            Value::Object(mut o) => {
                o.insert("source".into(), json!(source));
                Value::Object(o)
            }
            v => v,
        };
        let mut seen = std::collections::HashSet::new();
        let mut items = Vec::new();
        let mut skipped = Vec::new();
        let lists = std::iter::once(("hub", own.clone())).chain(extra.iter().map(|(_, l)| ("folder", l.clone())));
        for (source, listing) in lists {
            for item in &listing.items {
                if seen.insert(item.md5.clone()) {
                    items.push(tagged(json!(item), source));
                }
            }
            skipped.extend(listing.skipped.iter().map(|s| tagged(json!(s), source)));
        }
        let problem = extra
            .as_ref()
            .and_then(|(_, l)| l.problem.clone())
            .or(own.problem.clone());
        json!({
            "hub": self.own.root().display().to_string(),
            "folder": extra.as_ref().map(|(f, _)| f.root().display().to_string()),
            "watching": extra.as_ref().is_some_and(|(f, _)| f.watching()),
            "problem": problem,
            "items": items,
            "skipped": skipped,
        })
    }

    /// One of the listener's own skins, as `GET /api/skins/mine` lists it.
    async fn item_of(&self, md5: &str) -> Option<Value> {
        let answer = self.mine_answer().await;
        answer["items"].as_array()?.iter().find(|i| i["md5"] == md5).cloned()
    }

    fn path(&self, md5: &str) -> PathBuf {
        self.dir.join(format!("{md5}.wsz"))
    }

    fn item(&self, md5: String, file: String, shot: String, museum: String, nsfw: bool, favourite: bool) -> SkinItem {
        SkinItem {
            downloaded: self.path(&md5).is_file(),
            name: display_name(&file),
            md5,
            file,
            screenshot_url: shot,
            museum_url: museum,
            nsfw,
            favourite,
        }
    }

    async fn fetch(&self, md5: &str) -> Result<Vec<u8>> {
        let skin = self
            .museum
            .skin(md5)
            .await?
            .ok_or_else(|| Error::NotFound(format!("The {SERVICE} has no skin {md5}.")))?;
        let bytes = self.download_bytes(&skin.download_url).await?;
        wsz::validate(&bytes).map_err(|why| Error::upstream(SERVICE, format!("skin {md5}: {why}")))?;
        self.store(md5, &bytes).await?;
        tracing::info!(md5, file = skin.skin.filename, bytes = bytes.len(), "skin cached");
        Ok(bytes)
    }

    /// The body, refusing anything larger than a skin can be.
    async fn download_bytes(&self, url: &str) -> Result<Vec<u8>> {
        let resp = self
            .ctx
            .http
            .send(SERVICE, |c| c.get(url).timeout(Duration::from_secs(60)))
            .await?;
        let mut resp = lull_core::http::ok(SERVICE, resp)?;
        if resp.content_length().is_some_and(|n| n > MAX_SKIN_BYTES as u64) {
            return Err(Error::upstream(
                SERVICE,
                "the skin file is too large to be a classic skin",
            ));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = resp
            .chunk()
            .await
            .map_err(|e| Error::upstream(SERVICE, format!("download broke off: {}", e.without_url())))?
        {
            if bytes.len() + chunk.len() > MAX_SKIN_BYTES {
                return Err(Error::upstream(
                    SERVICE,
                    "the skin file is too large to be a classic skin",
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }

    /// Written beside its final name, then renamed, so a reader never sees half a skin.
    async fn store(&self, md5: &str, bytes: &[u8]) -> Result<()> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let part = self
            .dir
            .join(format!("{md5}.{}.part", NEXT.fetch_add(1, Ordering::Relaxed)));
        let io = |e: std::io::Error| Error::internal(format!("cache skin {md5}: {e}"));
        tokio::fs::write(&part, bytes).await.map_err(io)?;
        tokio::fs::rename(&part, self.path(md5)).await.map_err(io)
    }
}

fn favourite_md5s(conn: &rusqlite::Connection) -> Result<std::collections::HashSet<String>> {
    Ok(conn
        .prepare("SELECT md5 FROM skins_favourites")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?)
}

/// Skins are named by the md5 of their file: 32 lowercase hex digits.
fn valid_md5(md5: &str) -> Result<String> {
    if md5.len() == 32 && md5.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        Ok(md5.to_owned())
    } else {
        Err(Error::NotFound(format!("{md5:?} is not a skin id.")))
    }
}

/// "plum_cassette.wsz" -> "plum cassette": the museum has file names only.
fn display_name(file: &str) -> String {
    let stem = match file.rsplit_once('.') {
        Some((stem, ext)) if ext.eq_ignore_ascii_case("wsz") || ext.eq_ignore_ascii_case("zip") => stem,
        _ => file,
    };
    stem.replace('_', " ").trim().to_owned()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use axum::body::Body;
    use axum::http::Request;
    use lull_core::config::Config;
    use lull_core::db::{self, Db};
    use lull_core::events::Bus;
    use lull_core::http::Http;
    use tower::ServiceExt;

    use super::*;
    use crate::wsz::zip;

    /// A hub keeping its data in `data`, reading `skins` as `LULL_SKINS_DIR` when given.
    async fn hub(skins: Option<&std::path::Path>, data: &std::path::Path) -> Result<Arc<Service>> {
        let mut vars: BTreeMap<String, String> = [
            ("LULL_SLSKD_URL", "http://slskd"),
            ("LULL_SLSKD_API_KEY", "k"),
            ("LULL_DATA_DIR", &*data.to_string_lossy()),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
        if let Some(skins) = skins {
            vars.insert("LULL_SKINS_DIR".into(), skins.to_string_lossy().into_owned());
        }
        let config = Config::from_vars(vars).map_err(|e| Error::internal(e.to_string()))?;
        let db = Db::memory()?;
        db.migrate(&db::CORE)?;
        db.migrate(&MIGRATIONS)?;
        let bus = Bus::new();
        start(&Ctx::new(config, db, Http::new()?, bus)).await
    }

    async fn call(app: &Router, method: &str, uri: &str) -> (StatusCode, Vec<u8>) {
        send(app, method, uri, Vec::new()).await
    }

    async fn send(app: &Router, method: &str, uri: &str, body: Vec<u8>) -> (StatusCode, Vec<u8>) {
        let req = Request::builder().method(method).uri(uri).body(Body::from(body));
        let Ok(Ok(res)) = (match req {
            Ok(req) => app.clone().oneshot(req).await.map(Ok),
            Err(e) => Ok(Err(e)),
        }) else {
            return (StatusCode::IM_A_TEAPOT, Vec::new());
        };
        let status = res.status();
        let body = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap_or_default();
        (status, body.to_vec())
    }

    fn json_of(body: &[u8]) -> Value {
        serde_json::from_slice(body).unwrap_or(Value::Null)
    }

    #[tokio::test]
    async fn the_skins_folder_is_listed_served_and_read_again_when_asked() -> Result<()> {
        let root = std::env::temp_dir().join(format!("lull-skins-api-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (skins, data) = (root.join("skins"), root.join("data"));
        let _ = std::fs::create_dir_all(skins.join("Bento"));
        let bento = zip(&[("main.bmp", b"BM bento")]);
        let _ = std::fs::write(skins.join("Bento/Bento_Classified.wsz"), &bento);
        let _ = std::fs::write(skins.join("broken.wsz"), b"<html>404</html>");
        let app = router(hub(Some(&skins), &data).await?);

        let (status, body) = call(&app, "GET", "/api/skins/mine").await;
        let listed = json_of(&body);
        let md5 = listed["items"][0]["md5"].as_str().unwrap_or_default().to_owned();
        let (served, bytes) = call(&app, "GET", &format!("/api/skins/{md5}.wsz")).await;

        let plum = zip(&[("main.bmp", b"BM plum")]);
        let _ = std::fs::write(skins.join("plum.wsz"), &plum);
        let _ = std::fs::remove_file(skins.join("broken.wsz"));
        let (_, body) = call(&app, "POST", "/api/skins/mine/rescan").await;
        let again = json_of(&body);
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(status, StatusCode::OK);
        assert_eq!(listed["folder"], json!(skins.display().to_string()));
        assert_eq!(listed["items"].as_array().map(Vec::len), Some(1));
        assert_eq!(listed["items"][0]["name"], "Bento Classified");
        assert_eq!(listed["items"][0]["path"], "Bento/Bento_Classified.wsz");
        assert_eq!(listed["items"][0]["source"], "folder");
        assert_eq!(
            listed["hub"],
            json!(data.join("skins").join("mine").display().to_string())
        );
        assert_eq!(listed["skipped"][0]["path"], "broken.wsz");
        assert_eq!(listed["skipped"][0]["why"], "the file is not a zip archive");
        assert_eq!((served, bytes), (StatusCode::OK, bento));
        let names: Vec<&str> = again["items"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|i| i["file"].as_str())
            .collect();
        assert_eq!(names, vec!["Bento_Classified.wsz", "plum.wsz"]);
        assert_eq!(again["skipped"], json!([]));
        Ok(())
    }

    fn names(listed: &Value) -> Vec<(String, String)> {
        listed["items"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|i| {
                let (file, source) = (i["file"].as_str(), i["source"].as_str());
                (file.unwrap_or("").to_owned(), source.unwrap_or("").to_owned())
            })
            .collect()
    }

    #[tokio::test]
    async fn an_added_skin_is_checked_kept_once_and_served_to_every_device() -> Result<()> {
        let root = std::env::temp_dir().join(format!("lull-skins-add-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (skins, data) = (root.join("skins"), root.join("data"));
        let _ = std::fs::create_dir_all(&skins);
        let folders = zip(&[("main.bmp", b"BM folder")]);
        let _ = std::fs::write(skins.join("Folders.wsz"), &folders);
        let app = router(hub(Some(&skins), &data).await?);
        let plum = zip(&[("Plum/MAIN.BMP", b"BM plum")]);
        let other = zip(&[("main.bmp", b"BM another plum")]);

        let not_skin = send(&app, "POST", "/api/skins/mine?name=notes.wsz", b"<html>".to_vec()).await;
        let modern = zip(&[("skin.xml", b"<skin/>")]);
        let modern = send(&app, "POST", "/api/skins/mine?name=Modern.wsz", modern).await;
        let added = send(
            &app,
            "POST",
            "/api/skins/mine?name=C%3A%5Cskins%5CPlum.wsz",
            plum.clone(),
        )
        .await;
        let again = send(&app, "POST", "/api/skins/mine?name=Plum%20copy.wsz", plum.clone()).await;
        let in_folder = send(&app, "POST", "/api/skins/mine?name=Mine.wsz", folders.clone()).await;
        let clash = send(&app, "POST", "/api/skins/mine?name=Plum.wsz", other).await;
        let huge = send(
            &app,
            "POST",
            "/api/skins/mine?name=Huge.wsz",
            vec![0; MAX_SKIN_BYTES + 1],
        )
        .await;
        let (_, body) = call(&app, "GET", "/api/skins/mine").await;
        let listed = json_of(&body);
        let md5 = json_of(&added.1)["item"]["md5"].as_str().unwrap_or_default().to_owned();
        let served = call(&app, "GET", &format!("/api/skins/{md5}.wsz")).await;
        let on_disk = std::fs::read(data.join("skins/mine/Plum.wsz")).unwrap_or_default();
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(not_skin.0, StatusCode::BAD_REQUEST);
        assert_eq!(
            json_of(&not_skin.1)["message"],
            "notes.wsz isn't a classic skin: the file is not a zip archive."
        );
        assert_eq!(modern.0, StatusCode::BAD_REQUEST);
        assert_eq!(added.0, StatusCode::CREATED);
        assert_eq!(
            json_of(&added.1)["item"]["file"],
            "Plum.wsz",
            "the device's folders are dropped"
        );
        assert_eq!(json_of(&added.1)["item"]["source"], "hub");
        assert_eq!(
            (again.0, json_of(&again.1)["added"].clone()),
            (StatusCode::OK, json!(false))
        );
        assert_eq!(
            json_of(&again.1)["item"]["md5"],
            json!(md5),
            "the same bytes are one skin"
        );
        assert_eq!(
            json_of(&in_folder.1)["item"]["source"],
            "folder",
            "kept already, in the folder"
        );
        assert_eq!(clash.0, StatusCode::CREATED);
        assert_eq!(json_of(&clash.1)["item"]["file"], "Plum (2).wsz");
        assert_eq!(huge.0, StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(
            names(&listed),
            [("Plum (2).wsz", "hub"), ("Plum.wsz", "hub"), ("Folders.wsz", "folder")]
                .map(|(f, s)| (f.to_owned(), s.to_owned()))
        );
        assert_eq!((served.0, served.1), (StatusCode::OK, plum.clone()));
        assert_eq!(on_disk, plum);
        Ok(())
    }

    #[tokio::test]
    async fn a_skin_is_removed_from_the_hubs_folder_but_never_from_the_listeners_own() -> Result<()> {
        let root = std::env::temp_dir().join(format!("lull-skins-remove-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (skins, data) = (root.join("skins"), root.join("data"));
        let _ = std::fs::create_dir_all(&skins);
        let folders = zip(&[("main.bmp", b"BM folder")]);
        let _ = std::fs::write(skins.join("Folders.wsz"), &folders);
        let app = router(hub(Some(&skins), &data).await?);
        let plum = zip(&[("main.bmp", b"BM plum")]);
        let added = send(&app, "POST", "/api/skins/mine?name=Plum.wsz", plum).await;
        let md5 = json_of(&added.1)["item"]["md5"].as_str().unwrap_or_default().to_owned();
        let in_folder = folder::md5_hex(&folders);

        let removed = call(&app, "DELETE", &format!("/api/skins/mine/{md5}")).await;
        let (_, body) = call(&app, "GET", "/api/skins/mine").await;
        let kept = call(&app, "DELETE", &format!("/api/skins/mine/{in_folder}")).await;
        let unknown = call(&app, "DELETE", &format!("/api/skins/mine/{}", "0".repeat(32))).await;
        let file_left = data.join("skins/mine/Plum.wsz").exists();
        let folder_left = skins.join("Folders.wsz").exists();
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(removed.0, StatusCode::NO_CONTENT);
        assert!(!file_left);
        assert_eq!(
            names(&json_of(&body)),
            [("Folders.wsz".to_owned(), "folder".to_owned())]
        );
        assert_eq!(kept.0, StatusCode::CONFLICT);
        assert!(
            json_of(&kept.1)["message"].as_str().is_some_and(
                |m| m.starts_with("Folders.wsz is in your skins folder") && m.ends_with("remove it there.")
            )
        );
        assert!(folder_left);
        assert_eq!(unknown.0, StatusCode::NOT_FOUND);
        Ok(())
    }

    #[tokio::test]
    async fn without_a_skins_folder_the_hubs_own_is_there() -> Result<()> {
        let root = std::env::temp_dir().join(format!("lull-skins-own-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let app = router(hub(None, &root).await?);
        let (status, body) = call(&app, "GET", "/api/skins/mine").await;
        let made = root.join("skins/mine").is_dir();
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(status, StatusCode::OK);
        assert!(made);
        let listed = json_of(&body);
        assert_eq!(
            (listed["folder"].clone(), listed["items"].clone()),
            (Value::Null, json!([]))
        );
        assert_eq!(listed["problem"], Value::Null);
        Ok(())
    }

    #[test]
    fn a_kept_skins_name_is_safe_on_any_file_system() {
        assert_eq!(file_name("Plum.wsz"), "Plum.wsz");
        assert_eq!(file_name("../../etc/Plum.WSZ"), "Plum.wsz");
        assert_eq!(file_name("C:\\skins\\Bento: v2?.wsz"), "Bento v2.wsz");
        assert_eq!(file_name("no extension"), "no extension.wsz");
        assert_eq!(file_name("...wsz"), "Skin.wsz");
        assert_eq!(file_name("con.wsz"), "con skin.wsz");
        assert_eq!(file_name("a\u{20ac}b.wsz"), "a\u{20ac}b.wsz");
        assert_eq!(file_name(&"x".repeat(200)).len(), 84);
    }
}
