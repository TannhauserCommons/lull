//! lull: start-up and wiring. Domain logic lives in the domain crates; this binary builds them
//! in dependency order and merges their routes (docs/context/workspace.md).

mod events;
mod gate;
mod web;

use std::process::ExitCode;
use std::sync::Arc;

use axum::routing::get;
use axum::{Json, Router};
use lull_core::config::Config;
use lull_core::db::{self, Db};
use lull_core::events::Bus;
use lull_core::http::Http;
use lull_core::status::Status;
use lull_core::{Ctx, Error};

#[tokio::main]
async fn main() -> ExitCode {
    let outcome = match std::env::args().nth(1).as_deref() {
        None | Some("serve") => serve().await,
        Some("healthcheck") => healthcheck().await,
        Some(other) => Err(format!("unknown command {other:?}; use serve or healthcheck").into()),
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("lull: {e}");
            ExitCode::FAILURE
        }
    }
}

type BoxError = Box<dyn std::error::Error>;

async fn serve() -> Result<(), BoxError> {
    let config = Config::from_env()?;
    init_logs(&config.log)?;
    let ctx = context(config)?;
    let app = wire(&ctx).await?;
    let listener = tokio::net::TcpListener::bind(ctx.config.listen).await?;
    tracing::info!(listen = %ctx.config.listen, version = env!("CARGO_PKG_VERSION"), "lull is up");
    axum::serve(listener, app).with_graceful_shutdown(shutdown()).await?;
    Ok(())
}

fn context(config: Config) -> Result<Ctx, Error> {
    let bus = Bus::new();
    Ok(Ctx {
        db: Db::open(&config.data_dir)?,
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

    let api = Router::new()
        .merge(lull_soulseek::router(soulseek, library.clone()))
        .merge(lull_library::router(library))
        .merge(lull_discovery::router(discovery))
        .merge(lull_lastfm::router(lastfm))
        .merge(lull_lyrics::router(lyrics))
        .merge(lull_collection::router(collection))
        .merge(lull_skins::router(skins))
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

fn init_logs(filter: &str) -> Result<(), BoxError> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(tracing_subscriber::EnvFilter::try_new(filter)?)
        .init();
    Ok(())
}

async fn shutdown() {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let mut term = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(s) => s,
            Err(_) => return ctrl_c.await.unwrap_or(()),
        };
        tokio::select! { _ = ctrl_c => {}, _ = term.recv() => {} }
    }
    #[cfg(not(unix))]
    ctrl_c.await.unwrap_or(());
}

/// For the container healthcheck (the runtime image has no curl): exit 0 when `/health` answers.
async fn healthcheck() -> Result<(), BoxError> {
    let listen = std::env::var("LULL_LISTEN").unwrap_or_else(|_| "0.0.0.0:8080".into());
    let port = listen.rsplit(':').next().unwrap_or("8080");
    let resp = reqwest::Client::new()
        .get(format!("http://127.0.0.1:{port}/health"))
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await?;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("health answered {}", resp.status()).into())
    }
}
