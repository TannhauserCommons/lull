//! The saved library: Jellyfin reads and streaming for saved tracks, and the save pipeline
//! (complete transfer, MusicBrainz match, Cover Art Archive, tags, write, rescan)
//! (docs/context/library.md, docs/context/save-flow.md).

mod art;
mod index;
mod items;
mod jellyfin;
mod jobs;
mod matching;
mod musicbrainz;
mod naming;
mod routes;
mod save;
mod sync;
mod tags;

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use async_trait::async_trait;
use axum::Router;
use lull_core::db::Migrations;
use lull_core::ids::TrackId;
use lull_core::model::Track;
use lull_core::seams::{self, Transfers};
use lull_core::status::State;
use lull_core::{Ctx, Error, Result};
use tokio::sync::Notify;

use crate::index::Index;
use crate::jellyfin::Jellyfin;
use crate::jobs::Jobs;
use crate::naming::Template;

pub const MIGRATIONS: Migrations = Migrations {
    owner: "library",
    steps: &[r#"
        -- The last full read of Jellyfin, so the library lists while Jellyfin is down.
        CREATE TABLE library_items (
            item_id  TEXT PRIMARY KEY,
            json     TEXT NOT NULL
        );
        -- A save asked under one id whose tags gave another (docs/context/save-flow.md#ids).
        CREATE TABLE library_aliases (
            from_id  TEXT PRIMARY KEY,
            to_id    TEXT NOT NULL
        );
        CREATE TABLE library_jobs (
            id          TEXT PRIMARY KEY,
            kind        TEXT NOT NULL,          -- save | rescan
            json        TEXT NOT NULL,
            finished    INTEGER NOT NULL,       -- 1 = nothing left to do
            updated_at  INTEGER NOT NULL
        );
    "#],
};

pub struct Service {
    ctx: Ctx,
    transfers: Arc<dyn Transfers>,
    /// `None` when Jellyfin isn't configured: nothing is saved, nothing can be.
    jellyfin: Option<Jellyfin>,
    index: RwLock<Index>,
    /// The Jellyfin music library over `LULL_MUSIC_DIR`, once known (sync.rs).
    music: RwLock<Option<sync::MusicFolder>>,
    /// When saves last asked Jellyfin to scan (save.rs batches them).
    scans: Mutex<save::Scans>,
    jobs: Arc<Jobs>,
    naming: Template,
    art_dir: PathBuf,
    /// Wakes the sync loop early (a save waits for Jellyfin, a rescan finished).
    wake: Notify,
}

pub async fn start(ctx: &Ctx, transfers: Arc<dyn Transfers>) -> Result<Arc<Service>> {
    let naming =
        Template::parse(&ctx.config.library.naming).map_err(|e| Error::internal(format!("LULL_NAMING {e}")))?;
    let (items, aliases) = sync::load(&ctx.db).await?;
    let jobs = Jobs::load(ctx.db.clone(), ctx.bus.clone()).await?;
    let jellyfin = ctx
        .config
        .jellyfin
        .as_ref()
        .map(|cfg| Jellyfin::new(ctx.http.clone(), cfg));
    let svc = Arc::new(Service {
        ctx: ctx.clone(),
        transfers,
        jellyfin,
        index: RwLock::new(Index::new(items, aliases)),
        music: RwLock::new(None),
        scans: Mutex::default(),
        jobs,
        naming,
        art_dir: ctx.config.data_dir.join("art"),
        wake: Notify::new(),
    });
    match &svc.jellyfin {
        None => ctx.status.set(
            "jellyfin",
            State::Off,
            "Not set up: set LULL_JELLYFIN_URL and LULL_JELLYFIN_API_KEY to save songs",
            serde_json::Value::Null,
        ),
        Some(_) => {
            ctx.status.set(
                "jellyfin",
                State::Warn,
                "Connecting to Jellyfin",
                serde_json::Value::Null,
            );
            tokio::spawn(sync::run(svc.clone()));
            save::resume(&svc);
        }
    }
    Ok(svc)
}

/// `/api/tracks/{id}`, `/api/library/...`, `/api/art/{id}`, `/api/saves/...`, `/api/jobs`.
pub fn router(svc: Arc<Service>) -> Router {
    routes::router(svc)
}

impl Service {
    fn index(&self) -> RwLockReadGuard<'_, Index> {
        self.index.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn index_mut(&self) -> RwLockWriteGuard<'_, Index> {
        self.index.write().unwrap_or_else(PoisonError::into_inner)
    }

    fn music(&self) -> Option<sync::MusicFolder> {
        self.music.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    fn jellyfin(&self) -> Result<&Jellyfin> {
        self.jellyfin.as_ref().ok_or_else(|| {
            Error::unavailable(
                "jellyfin",
                "it isn't set up (LULL_JELLYFIN_URL and LULL_JELLYFIN_API_KEY)",
            )
        })
    }
}

#[async_trait]
impl seams::Library for Service {
    async fn search(&self, query: &str, limit: usize) -> Result<Vec<Track>> {
        Ok(self.index().search(query, limit))
    }

    async fn saved(&self, ids: &[TrackId]) -> Result<Vec<Track>> {
        let index = self.index();
        Ok(ids.iter().filter_map(|id| index.track(id)).collect())
    }
}
