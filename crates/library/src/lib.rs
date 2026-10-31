//! The saved library: Jellyfin reads and streaming for saved tracks, and the save pipeline
//! (complete transfer, MusicBrainz match, Cover Art Archive, tags, write, rescan)
//! (docs/context/save-flow.md).

use std::sync::Arc;

use async_trait::async_trait;
use axum::Router;
use lull_core::db::Migrations;
use lull_core::ids::TrackId;
use lull_core::model::Track;
use lull_core::seams::{self, Transfers};
use lull_core::status::State;
use lull_core::{Ctx, Error, Result};

pub const MIGRATIONS: Migrations = Migrations {
    owner: "library",
    steps: &[],
};

pub struct Service {
    #[allow(dead_code)] // SKELETON: used once the crate is built out
    ctx: Ctx,
    #[allow(dead_code)] // SKELETON
    transfers: Arc<dyn Transfers>,
}

pub async fn start(ctx: &Ctx, transfers: Arc<dyn Transfers>) -> Result<Arc<Service>> {
    // SKELETON: check Jellyfin, keep status current.
    ctx.status
        .set("jellyfin", State::Down, "not built yet", serde_json::Value::Null);
    Ok(Arc::new(Service {
        ctx: ctx.clone(),
        transfers,
    }))
}

/// `/api/library/...`, `/api/tracks|albums|artists/{id}`, `/api/saves/...`, `/api/art/{id}`.
pub fn router(_svc: Arc<Service>) -> Router {
    Router::new() // SKELETON
}

#[async_trait]
impl seams::Library for Service {
    async fn search(&self, _query: &str, _limit: usize) -> Result<Vec<Track>> {
        Err(Error::unavailable("jellyfin", "the library is not built yet")) // SKELETON
    }
    async fn saved(&self, _ids: &[TrackId]) -> Result<Vec<Track>> {
        Err(Error::unavailable("jellyfin", "the library is not built yet")) // SKELETON
    }
}
