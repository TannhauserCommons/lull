//! Classic mode skins: Winamp Skin Museum search (proxied), `.wsz` download, validation and
//! cache, and favourites.

use std::sync::Arc;

use axum::Router;
use lull_core::db::Migrations;
use lull_core::{Ctx, Result};

pub const MIGRATIONS: Migrations = Migrations {
    owner: "skins",
    steps: &[],
};

pub struct Service {
    #[allow(dead_code)] // SKELETON: used once the crate is built out
    ctx: Ctx,
}

pub async fn start(ctx: &Ctx) -> Result<Arc<Service>> {
    Ok(Arc::new(Service { ctx: ctx.clone() }))
}

/// `/api/skins/...`.
pub fn router(_svc: Arc<Service>) -> Router {
    Router::new() // SKELETON
}
