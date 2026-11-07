//! Shared contracts for every lull crate. Domain crates depend on this and nothing else of
//! lull's; `server` wires them together. Changes here are additive only (AGENTS.md).

pub mod art;
pub mod build;
pub mod catalog;
pub mod config;
pub mod db;
pub mod devices;
pub mod error;
pub mod etag;
pub mod events;
pub mod flags;
pub mod http;
pub mod identity;
pub mod ids;
pub mod m4a;
pub mod model;
pub mod pictures;
pub mod private;
pub mod process;
pub mod quality;
pub mod seams;
pub mod secrets;
pub mod settings;
pub mod status;
pub mod warm;
pub mod youtube;

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
    /// Outside pictures (covers, artist photos), ordered by what the listener's screens show
    /// (ADR-0033). Built with `Ctx`'s own `http`, `bus` and `shutdown`.
    pub pictures: pictures::Pictures,
    /// Cancelled when the hub stops this set of crates: on exit, and before an in-process restart
    /// rewires them with new settings (ADR-0002). Background tasks end on it; see `Ctx::spawn`.
    pub shutdown: tokio_util::sync::CancellationToken,
}

impl Ctx {
    /// The context a set of crates starts with: its status and pictures report on `bus`, and
    /// everything it starts ends with its `shutdown`.
    pub fn new(config: config::Config, db: db::Db, http: http::Http, bus: events::Bus) -> Ctx {
        let shutdown = tokio_util::sync::CancellationToken::new();
        let pictures = pictures::Pictures::new(
            config.data_dir.join("catalog-art"),
            http.clone(),
            bus.clone(),
            shutdown.clone(),
        );
        Ctx {
            config: Arc::new(config),
            db,
            http,
            status: status::Status::new(bus.clone()),
            bus,
            pictures,
            shutdown,
        }
    }

    /// Runs a background task until it finishes or the hub stops this set of crates, so a
    /// restart never leaves the old crates' loops running beside the new ones.
    pub fn spawn<F>(&self, task: F)
    where
        F: std::future::Future + Send + 'static,
        F::Output: Send,
    {
        let shutdown = self.shutdown.clone();
        tokio::spawn(async move {
            shutdown.run_until_cancelled(task).await;
        });
    }
}

/// Unix seconds now.
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
