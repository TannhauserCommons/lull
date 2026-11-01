//! The lull hub: start-up and wiring. Domain logic lives in the domain crates; this crate builds
//! them in dependency order and merges their routes (docs/context/workspace.md). The `lull`
//! binary and the desktop app both start the hub through `run` (ADR-0002).

mod events;
mod gate;
mod web;

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;

use axum::routing::get;
use axum::{Json, Router};
use lull_core::config::Config;
use lull_core::db::{self, Db};
use lull_core::events::Bus;
use lull_core::http::Http;
use lull_core::status::Status;
use lull_core::{Ctx, Error, settings};

pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Runs the hub until `shutdown` resolves. `env` holds the `LULL_*` settings from the
/// environment (`lull_core::config::env_vars`) or from the desktop app; they win over the
/// settings stored in the app.
pub async fn run(
    env: BTreeMap<String, String>,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), BoxError> {
    let (config, db) = resolve(env).await?;
    let ctx = context(config, db)?;
    let app = wire(&ctx).await?;
    let listener = tokio::net::TcpListener::bind(ctx.config.listen).await?;
    tracing::info!(listen = %ctx.config.listen, version = env!("CARGO_PKG_VERSION"), "lull is up");
    axum::serve(listener, app).with_graceful_shutdown(shutdown).await?;
    Ok(())
}

/// The environment merged over the stored settings. The data directory comes from `env` alone:
/// it holds the database the stored settings live in.
async fn resolve(env: BTreeMap<String, String>) -> Result<(Config, Db), BoxError> {
    let data_dir = env.get("LULL_DATA_DIR").map_or("/data", String::as_str);
    let db = Db::open(data_dir.as_ref())?;
    db.migrate(&db::CORE)?;
    let (vars, _) = settings::merge(&env, &settings::stored(&db).await?);
    Ok((Config::from_vars(vars)?, db))
}

fn context(config: Config, db: Db) -> Result<Ctx, Error> {
    let bus = Bus::new();
    Ok(Ctx {
        db,
        http: Http::new()?,
        status: Status::new(bus.clone()),
        bus,
        config: Arc::new(config),
    })
}

/// Builds every crate (seam providers first) and merges their routes. The only place crates meet.
async fn wire(ctx: &Ctx) -> Result<Router, Error> {
    for m in [
        &db::CORE,
        &lull_core::catalog::MIGRATIONS,
        &lull_soulseek::MIGRATIONS,
        &lull_library::MIGRATIONS,
        &lull_discovery::MIGRATIONS,
        &lull_lastfm::MIGRATIONS,
        &lull_lyrics::MIGRATIONS,
        &lull_collection::MIGRATIONS,
        &lull_skins::MIGRATIONS,
        &lull_subsonic::MIGRATIONS,
        &lull_themes::MIGRATIONS,
    ] {
        ctx.db.migrate(m)?;
    }

    let soulseek = lull_soulseek::start(ctx).await?;
    let library = lull_library::start(ctx, soulseek.clone()).await?;
    let discovery = lull_discovery::start(ctx, library.clone()).await?;
    let lastfm = lull_lastfm::start(ctx).await?;
    let lyrics = lull_lyrics::start(ctx).await?;
    let collection = lull_collection::start(ctx, library.clone()).await?;
    let skins = lull_skins::start(ctx).await?;
    let themes = lull_themes::start(ctx).await?;

    let api = Router::new()
        .merge(lull_soulseek::router(soulseek, library.clone()))
        .merge(lull_library::router(library))
        .merge(lull_discovery::router(discovery))
        .merge(lull_lastfm::router(lastfm))
        .merge(lull_lyrics::router(lyrics))
        .merge(lull_collection::router(collection))
        .merge(lull_skins::router(skins))
        .merge(lull_themes::router(themes))
        .merge(events::router(ctx.bus.clone()))
        .route("/api/ping", get(|| async { axum::http::StatusCode::NO_CONTENT }))
        .layer(axum::middleware::from_fn_with_state(
            gate::Gate::new(&ctx.config),
            gate::require,
        ));

    let status = ctx.status.clone();
    Ok(Router::new()
        .merge(api)
        .merge(gate::router(&ctx.config))
        .route("/health", get(move || async move { Json(health(&status)) }))
        .fallback(web::serve))
}

/// Always 200 while the process serves: a dependency being down is reported, not fatal.
fn health(status: &Status) -> serde_json::Value {
    serde_json::json!({ "version": env!("CARGO_PKG_VERSION"), "services": status.snapshot() })
}
