//! What the listener keeps: likes, follows, history (plays), playlists with M3U import and
//! export, stats and settings. Publishes `play` events that Last.fm and the taste model consume.

use std::sync::Arc;

use axum::Router;
use lull_core::db::Migrations;
use lull_core::seams::Library;
use lull_core::{Ctx, Result};

pub const MIGRATIONS: Migrations = Migrations {
    owner: "collection",
    steps: &[],
};

pub struct Service {
    #[allow(dead_code)] // SKELETON: used once the crate is built out
    ctx: Ctx,
    #[allow(dead_code)] // SKELETON
    library: Arc<dyn Library>,
}

pub async fn start(ctx: &Ctx, library: Arc<dyn Library>) -> Result<Arc<Service>> {
    Ok(Arc::new(Service {
        ctx: ctx.clone(),
        library,
    }))
}

/// `/api/likes`, `/api/follows`, `/api/plays`, `/api/history`, `/api/playlists/...`, `/api/stats`, `/api/settings`.
pub fn router(_svc: Arc<Service>) -> Router {
    Router::new() // SKELETON
}
