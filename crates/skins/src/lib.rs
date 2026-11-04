//! Classic mode skins: Winamp Skin Museum search (proxied), `.wsz` download, validation and
//! cache, favourites, and the listener's own skins folder (docs/context/skins.md).

mod folder;
mod museum;
mod wsz;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
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
            root      TEXT NOT NULL,       -- LULL_SKINS_DIR when read; rows of another are dropped
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
    /// The listener's own skins (`LULL_SKINS_DIR`), when set.
    folder: Option<Arc<Folder>>,
}

pub async fn start(ctx: &Ctx) -> Result<Arc<Service>> {
    let dir = ctx.config.data_dir.join("skins");
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| Error::internal(format!("create {}: {e}", dir.display())))?;
    let folder = ctx.config.skins_dir.clone().map(|root| Folder::start(ctx, root));
    Ok(Arc::new(Service {
        ctx: ctx.clone(),
        museum: Museum::new(ctx.http.clone()),
        dir,
        folder,
    }))
}

/// `/api/skins/...`.
pub fn router(svc: Arc<Service>) -> Router {
    Router::new()
        .route("/api/skins", get(search))
        .route("/api/skins/favourites", get(favourites))
        .route("/api/skins/mine", get(mine))
        .route("/api/skins/mine/rescan", post(rescan))
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

/// `GET /api/skins/mine`: the skins folder's skins, once it has been read.
async fn mine(State(svc): State<Arc<Service>>) -> Json<Value> {
    let Some(folder) = &svc.folder else {
        return Json(json!({ "folder": null, "watching": false, "problem": null, "items": [], "skipped": [] }));
    };
    let listing = folder.listing().await;
    Json(mine_answer(folder, &listing))
}

/// `POST /api/skins/mine/rescan`: reads the folder now (for a folder whose changes the hub can't see).
async fn rescan(State(svc): State<Arc<Service>>) -> Result<Json<Value>> {
    let folder = svc
        .folder
        .as_ref()
        .ok_or_else(|| Error::NotFound("No skins folder is set: LULL_SKINS_DIR.".into()))?;
    let listing = folder.rescan().await?;
    Ok(Json(mine_answer(folder, &listing)))
}

fn mine_answer(folder: &Folder, listing: &folder::Listing) -> Value {
    json!({
        "folder": folder.root().display().to_string(),
        "watching": folder.watching(),
        "problem": listing.problem,
        "items": listing.items,
        "skipped": listing.skipped,
    })
}

/// `GET /api/skins/{md5}.wsz`: from the skins folder, else the cache, else downloaded from the
/// museum, validated and cached. The museum's files send no CORS headers, so Webamp loads them
/// from here, same origin.
async fn download(State(svc): State<Arc<Service>>, Path(file): Path<String>) -> Result<Response> {
    let md5 = file
        .strip_suffix(".wsz")
        .ok_or_else(|| Error::NotFound(format!("No skin file {file}.")))
        .and_then(valid_md5)?;
    let mine = match &svc.folder {
        Some(folder) => folder.file(&md5).await,
        None => None,
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
    use lull_core::status::Status;
    use tower::ServiceExt;

    use super::*;
    use crate::wsz::zip;

    async fn hub(skins: &std::path::Path, data: &std::path::Path) -> Result<Arc<Service>> {
        let vars: BTreeMap<String, String> = [
            ("LULL_SLSKD_URL", "http://slskd"),
            ("LULL_SLSKD_API_KEY", "k"),
            ("LULL_DATA_DIR", &data.to_string_lossy()),
            ("LULL_SKINS_DIR", &skins.to_string_lossy()),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
        let config = Config::from_vars(vars).map_err(|e| Error::internal(e.to_string()))?;
        let db = Db::memory()?;
        db.migrate(&db::CORE)?;
        db.migrate(&MIGRATIONS)?;
        let bus = Bus::new();
        start(&Ctx {
            config: Arc::new(config),
            db,
            http: Http::new()?,
            status: Status::new(bus.clone()),
            bus,
            shutdown: Default::default(),
        })
        .await
    }

    async fn call(app: &Router, method: &str, uri: &str) -> (StatusCode, Vec<u8>) {
        let req = Request::builder().method(method).uri(uri).body(Body::empty());
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
        let app = router(hub(&skins, &data).await?);

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
}
