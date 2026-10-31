//! The home feed (new releases from follows, picks with reasons, radio starters), radio and the
//! recency-weighted taste model (docs/context/discovery.md, docs/context/taste.md).

use std::sync::Arc;

use axum::Router;
use lull_core::db::Migrations;
use lull_core::seams::Library;
use lull_core::{Ctx, Result};

pub const MIGRATIONS: Migrations = Migrations {
    owner: "discovery",
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

/// `/api/feed`, `/api/radio`, `/api/tags/{id}`, `/api/feedback`, `/api/taste/reset`.
pub fn router(_svc: Arc<Service>) -> Router {
    Router::new() // SKELETON
}
