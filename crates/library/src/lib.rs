//! The saved library, from a music folder or Jellyfin (ADR-0003): one index keyed by track id,
//! streaming and covers for saved tracks, switching between the sources, connecting and
//! mirroring Jellyfin, and the save pipeline (complete transfer, MusicBrainz match, Cover Art
//! Archive, tags, write, pick-up), and auto-upgrade of songs saved below the saving minimum
//! (docs/context/library.md, docs/context/save-flow.md).

mod art;
mod connect;
mod files;
mod folder;
mod index;
mod items;
mod jellyfin;
mod jobs;
mod matching;
mod mirror;
mod musicbrainz;
mod naming;
mod routes;
mod save;
mod switch;
mod sync;
mod tags;
mod upgrade;

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use async_trait::async_trait;
use axum::Router;
use axum::http::HeaderValue;
use axum::response::Response;
use lull_core::config::LibraryMode;
use lull_core::db::Migrations;
use lull_core::ids::{AlbumId, ArtistId, TrackId};
use lull_core::model::{Album, Artist, Track};
use lull_core::seams::{self, Collection, Transfers};
use lull_core::status::State;
use lull_core::{Ctx, Error, Result};
use tokio::sync::Notify;

use crate::folder::Folder;
use crate::index::Index;
use crate::jellyfin::Jellyfin;
use crate::jobs::Jobs;
use crate::naming::Template;

pub const MIGRATIONS: Migrations = Migrations {
    owner: "library",
    steps: &[
        r#"
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
    "#,
        r#"
        -- The music folder's index (folder mode): an unchanged file isn't read again.
        CREATE TABLE library_files (
            path      TEXT PRIMARY KEY,         -- under LULL_MUSIC_DIR, '/'-separated
            modified  INTEGER NOT NULL,         -- unix seconds
            size      INTEGER NOT NULL,
            picture   INTEGER NOT NULL,         -- 1 = the file embeds a picture
            json      TEXT NOT NULL
        );
        -- The Jellyfin mirror (docs/context/library.md#jellyfin-mirror): what was imported once,
        -- and which Jellyfin playlist mirrors which lull playlist.
        CREATE TABLE library_mirror (
            key    TEXT PRIMARY KEY,
            value  TEXT NOT NULL
        );
    "#,
        r#"
        -- Songs saved below the saving minimum, searched again for an upgrade (upgrade.rs).
        CREATE TABLE library_upgrades (
            track_id  TEXT PRIMARY KEY,     -- the id the save was asked by
            ext       TEXT NOT NULL,        -- the saved file's extension, for the format list
            release   TEXT,                 -- the save's release (jobs::Release JSON)
            next_at   INTEGER NOT NULL      -- unix seconds of the next search
        );
    "#,
    ],
};

pub struct Service {
    ctx: Ctx,
    transfers: Arc<dyn Transfers>,
    /// Where the index comes from; switching restarts the hub's crates (switch.rs).
    mode: LibraryMode,
    /// Configured Jellyfin, in either mode: read in Jellyfin mode, and for switching to it.
    jellyfin: Option<Jellyfin>,
    /// `LULL_MUSIC_DIR`: indexed and watched in folder mode, read for switching in Jellyfin mode.
    folder: Folder,
    index: RwLock<Index>,
    /// The Jellyfin music library over `LULL_MUSIC_DIR`, once known (sync.rs).
    music: RwLock<Option<sync::MusicFolder>>,
    /// When saves last asked Jellyfin to scan (save.rs batches them).
    scans: Mutex<save::Scans>,
    jobs: Arc<Jobs>,
    naming: Template,
    art_dir: PathBuf,
    /// Cover lookups outside the library, running or recently failed (art.rs).
    lookups: Arc<art::Lookups>,
    /// Wakes the sync loop early (a save waits for Jellyfin, a rescan finished).
    wake: Notify,
    /// Quick Connect attempts waiting for approval (connect.rs).
    connects: connect::Pending,
    /// Set while a switch runs, so two can't overlap.
    switching: Arc<Mutex<bool>>,
    /// The listener's likes and playlists, for the Jellyfin mirror. Collection starts after
    /// library, so the hub hands it over later (`attach_collection`).
    collection: OnceLock<Arc<dyn Collection>>,
    /// The saving profile and the auto-upgrade pacer (upgrade.rs).
    upgrades: upgrade::Upgrades,
}

pub async fn start(ctx: &Ctx, transfers: Arc<dyn Transfers>) -> Result<Arc<Service>> {
    let library = &ctx.config.library;
    let naming = Template::parse(&library.naming).map_err(|e| Error::internal(format!("LULL_NAMING {e}")))?;
    let aliases = sync::load_aliases(&ctx.db).await?;
    let (folder, items) = match library.mode {
        LibraryMode::Folder => {
            let rows = folder::load(&ctx.db).await?;
            let items = rows.iter().map(|r| r.saved.clone()).collect();
            (Folder::with(library.music_dir.clone(), &rows), items)
        }
        LibraryMode::Jellyfin => (Folder::new(library.music_dir.clone()), sync::load_items(&ctx.db).await?),
    };
    let jobs = Jobs::load(ctx.db.clone(), ctx.bus.clone()).await?;
    jobs.follow(ctx);
    let svc = Arc::new(Service {
        ctx: ctx.clone(),
        transfers,
        mode: library.mode,
        jellyfin: ctx
            .config
            .jellyfin
            .as_ref()
            .map(|cfg| Jellyfin::new(ctx.http.clone(), cfg)),
        folder,
        index: RwLock::new(Index::new(items, aliases)),
        music: RwLock::new(None),
        scans: Mutex::default(),
        jobs,
        naming,
        art_dir: ctx.config.data_dir.join("art"),
        lookups: Arc::default(),
        wake: Notify::new(),
        connects: connect::Pending::default(),
        switching: Arc::default(),
        collection: OnceLock::new(),
        upgrades: upgrade::Upgrades::default(),
    });
    // Before collection starts, so its start-up `settings` arrives (ADR-0050).
    ctx.spawn(upgrade::follow_settings(&svc));
    ctx.spawn(upgrade::run(svc.clone()));
    ctx.spawn(routes::save_requests(svc.clone()));
    match svc.mode {
        LibraryMode::Folder => {
            svc.report(State::Warn, "reading the music folder");
            ctx.spawn(folder::run(svc.clone()));
        }
        LibraryMode::Jellyfin => {
            svc.report(State::Warn, "connecting");
            ctx.spawn(sync::run(svc.clone()));
        }
    }
    save::resume(&svc);
    Ok(svc)
}

/// `/api/tracks/{id}`, `/api/library/...`, `/api/jellyfin/...`, `/api/art/{id}`, `/api/saves/...`,
/// `/api/jobs`.
pub fn router(svc: Arc<Service>) -> Router {
    routes::router(svc)
}

/// Hands library the listener's collection once `collection` has started, which starts the
/// Jellyfin mirror in Jellyfin mode (docs/context/library.md#jellyfin-mirror).
pub fn attach_collection(svc: &Arc<Service>, collection: Arc<dyn Collection>) {
    if svc.collection.set(collection).is_ok() && svc.mode == LibraryMode::Jellyfin {
        svc.ctx.spawn(mirror::run(svc.clone()));
    }
}

impl Service {
    fn index(&self) -> RwLockReadGuard<'_, Index> {
        self.index.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn index_mut(&self) -> RwLockWriteGuard<'_, Index> {
        self.index.write().unwrap_or_else(PoisonError::into_inner)
    }

    /// The `library` status, one in both modes: the mode leads the detail ("Jellyfin: …",
    /// "Folder: …"), and `extra` carries `{ mode, songs }`.
    fn report(&self, state: State, detail: impl std::fmt::Display) {
        let mode = match self.mode {
            LibraryMode::Folder => "Folder",
            LibraryMode::Jellyfin => "Jellyfin",
        };
        let extra = serde_json::json!({ "mode": self.mode.as_str(), "songs": self.index().len() });
        self.ctx
            .status
            .set("library", state, format!("{mode}: {detail}"), extra);
    }

    /// The `library` status is `ok`: in Jellyfin mode, Jellyfin answered and was read.
    fn online(&self) -> bool {
        self.ctx
            .status
            .snapshot()
            .get("library")
            .is_some_and(|s| s.state == State::Ok)
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

    async fn artists(&self) -> Result<Vec<Artist>> {
        Ok(self.index().artists("name", "")?.into_iter().cloned().collect())
    }

    async fn albums(&self, artist: Option<&ArtistId>) -> Result<Vec<Album>> {
        let index = self.index();
        Ok(index
            .albums("artist", "")?
            .into_iter()
            .filter(|e| artist.is_none_or(|a| e.album.artist_id == *a))
            .map(|e| e.album.clone())
            .collect())
    }

    async fn album(&self, id: &AlbumId) -> Result<Option<(Album, Vec<Track>)>> {
        let index = self.index();
        Ok(index.album(id).map(|e| {
            let tracks = e.album.track_ids.iter().filter_map(|t| index.track(t)).collect();
            (e.album.clone(), tracks)
        }))
    }

    async fn stream(&self, id: &TrackId, range: Option<HeaderValue>) -> Result<Response> {
        routes::stream_saved(self, id, range).await
    }

    /// Any size (Subsonic's `getCoverArt`): the smallest served size that covers it, else the
    /// largest; 512 when none is asked for.
    async fn cover(&self, id: &AlbumId, size: Option<u32>) -> Result<Response> {
        let size = match size {
            Some(s) => lull_core::art::SIZES.into_iter().find(|&b| b >= s).unwrap_or(2048),
            None => 512,
        };
        routes::cover_response(self, id, size).await
    }
}
