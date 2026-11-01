//! The listener's theme packages (ADR-0004): stored under `<data_dir>/themes/`, served to every
//! device, imported and exported as `.lulltheme`, reloaded live when a file changes. The five
//! built-in designs ship in the web UI in the same format (docs/themes.md).
//! Built by the Themes session.

use std::sync::Arc;

use axum::Router;
use lull_core::db::Migrations;
use lull_core::{Ctx, Result};

pub const MIGRATIONS: Migrations = Migrations {
    owner: "themes",
    steps: &[],
};

pub struct Service;

pub async fn start(_ctx: &Ctx) -> Result<Arc<Service>> {
    Ok(Arc::new(Service))
}

pub fn router(svc: Arc<Service>) -> Router {
    Router::new().with_state(svc)
}
