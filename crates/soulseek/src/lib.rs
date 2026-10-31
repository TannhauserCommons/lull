//! Soulseek through slskd: search (streamed, merged, ranked), transfers, progressive streaming
//! from growing files, peer fallback and the size-capped stream cache. This crate is the only
//! code that talks to slskd (docs/context/soulseek.md).

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use axum::Router;
use lull_core::db::Migrations;
use lull_core::ids::TrackId;
use lull_core::model::SourceRef;
use lull_core::seams::{Library, Transfers};
use lull_core::status::State;
use lull_core::{Ctx, Error, Result};

pub const MIGRATIONS: Migrations = Migrations {
    owner: "soulseek",
    steps: &[],
};

pub struct Service {
    #[allow(dead_code)] // SKELETON: used once the crate is built out
    ctx: Ctx,
}

pub async fn start(ctx: &Ctx) -> Result<Arc<Service>> {
    // SKELETON: connect to slskd, keep status current, start cache eviction.
    ctx.status
        .set("soulseek", State::Down, "not built yet", serde_json::Value::Null);
    Ok(Arc::new(Service { ctx: ctx.clone() }))
}

/// `/api/search`, `/api/tracks/{id}/sources`, `/api/stream/{id}`, `/api/transfers/...`.
pub fn router(_svc: Arc<Service>, _library: Arc<dyn Library>) -> Router {
    Router::new() // SKELETON
}

#[async_trait]
impl Transfers for Service {
    async fn complete(&self, _track: &TrackId, _source: &SourceRef) -> Result<PathBuf> {
        Err(Error::unavailable("soulseek", "transfers are not built yet")) // SKELETON
    }
}
