//! Shared contracts for every lull crate. Domain crates depend on this and nothing else of
//! lull's; `server` wires them together. Changes here are additive only (AGENTS.md).

pub mod catalog;
pub mod config;
pub mod db;
pub mod devices;
pub mod error;
pub mod events;
pub mod http;
pub mod ids;
pub mod model;
pub mod seams;
pub mod settings;
pub mod status;

use std::sync::Arc;

pub use error::{Error, Result};

/// What every crate gets at start-up.
#[derive(Clone)]
pub struct Ctx {
    pub config: Arc<config::Config>,
    pub db: db::Db,
    pub http: http::Http,
    pub bus: events::Bus,
    pub status: status::Status,
}

/// Unix seconds now.
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
