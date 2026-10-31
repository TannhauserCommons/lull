//! Classic mode skins: Winamp Skin Museum search (proxied), `.wsz` download, validation and
//! cache, and favourites (docs/context/skins.md).

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
use axum::routing::{get, put};
use axum::{Json, Router};
use lull_core::db::Migrations;
use lull_core::{Ctx, Error, Result};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::museum::{Museum, SERVICE};

pub const MIGRATIONS: Migrations = Migrations {
    owner: "skins",
    steps: &[r#"
        -- What the drawer shows for a favourite, so the list works while the museum is away.
        CREATE TABLE skins_favourites (
            md5             TEXT PRIMARY KEY,
            filename        TEXT NOT NULL,
            screenshot_url  TEXT NOT NULL,
            museum_url      TEXT NOT NULL,
            nsfw            INTEGER NOT NULL,
            added_at        INTEGER NOT NULL
        );
    "#],
};

/// Classic skins are a few hundred KB; anything far bigger is not one.
const MAX_SKIN_BYTES: usize = 16 * 1024 * 1024;

pub struct Service {
    ctx: Ctx,
    museum: Museum,
    /// `<data dir>/skins/<md5>.wsz`: validated downloads, kept for good (a skin never changes).
    dir: PathBuf,
}

pub async fn start(ctx: &Ctx) -> Result<Arc<Service>> {
    let dir = ctx.config.data_dir.join("skins");
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| Error::internal(format!("create {}: {e}", dir.display())))?;
    Ok(Arc::new(Service {
        ctx: ctx.clone(),
        museum: Museum::new(ctx.http.clone()),
        dir,
    }))
}

/// `/api/skins/...`.
pub fn router(svc: Arc<Service>) -> Router {
    Router::new()
        .route("/api/skins", get(search))
        .route("/api/skins/favourites", get(favourites))
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

/// `GET /api/skins/{md5}.wsz`: from the cache, else downloaded from the museum, validated and cached.
/// The museum's files send no CORS headers, so Webamp loads them from here, same origin.
async fn download(State(svc): State<Arc<Service>>, Path(file): Path<String>) -> Result<Response> {
    let md5 = file
        .strip_suffix(".wsz")
        .ok_or_else(|| Error::NotFound(format!("No skin file {file}.")))
        .and_then(valid_md5)?;
    let path = svc.path(&md5);
    let bytes = match tokio::fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => svc.fetch(&md5).await?,
        Err(e) => return Err(Error::internal(format!("read {}: {e}", path.display()))),
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
