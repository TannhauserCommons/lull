//! Last.fm: web auth, now playing and scrobbles with an offline-safe retry queue, driven by
//! `play` events (docs/context/scrobbling.md).

use std::sync::Arc;

use axum::Router;
use lull_core::db::Migrations;
use lull_core::{Ctx, Result};

pub const MIGRATIONS: Migrations = Migrations {
    owner: "lastfm",
    steps: &[],
};

pub struct Service {
    #[allow(dead_code)] // SKELETON: used once the crate is built out
    ctx: Ctx,
}

pub async fn start(ctx: &Ctx) -> Result<Arc<Service>> {
    // SKELETON: subscribe to play events, report status ("off" when not configured).
    Ok(Arc::new(Service { ctx: ctx.clone() }))
}

/// `/api/lastfm/...`.
pub fn router(_svc: Arc<Service>) -> Router {
    Router::new() // SKELETON
}
