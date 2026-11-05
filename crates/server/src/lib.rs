//! The lull hub: start-up, wiring and in-process restarts. Domain logic lives in the domain
//! crates; this crate builds them in dependency order and merges their routes
//! (docs/context/workspace.md, docs/context/hub.md). The `lull` binary and the desktop app both
//! start the hub through `run` (ADR-0002).

mod compress;
mod conditional;
mod config;
mod devices;
mod events;
mod gate;
mod hub;
mod pairing;
#[cfg(test)]
mod tests;
mod web;

use std::collections::BTreeMap;
use std::future::{Future, IntoFuture};
use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use lull_core::db::{self, Db};
use lull_core::{Ctx, Error};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

use crate::hub::Hub;

pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Runs the hub until `shutdown` resolves. `env` holds the `LULL_*` settings from the
/// environment (`lull_core::config::env_vars`) or from the desktop app; they win over the
/// settings stored in the app.
pub async fn run(
    env: BTreeMap<String, String>,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), BoxError> {
    // The data directory comes from `env` alone: it holds the database the stored settings live in.
    let db = Db::open(env.get("LULL_DATA_DIR").map_or("/data", String::as_str).as_ref())?;
    db.migrate(&db::CORE)?;
    let config = hub::configure(&env, &db).await?;
    let listener = tokio::net::TcpListener::bind(config.listen).await?;
    tracing::info!(listen = %config.listen, version = env!("CARGO_PKG_VERSION"), "lull is up");
    serve(listener, Hub::open(env, db, config).await?, shutdown).await
}

/// Serves the hub until `shutdown`, restarting its crates in place whenever one publishes
/// `hub.restart`. The listener stays bound throughout.
async fn serve(
    listener: tokio::net::TcpListener,
    hub: Arc<Hub>,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), BoxError> {
    let closed = CancellationToken::new();
    let server = axum::serve(listener, router(hub.clone())).with_graceful_shutdown(closed.clone().cancelled_owned());
    let restarts = async {
        let outcome = restart_until(&hub, shutdown).await;
        // Stopping the crates also ends the SSE streams, which graceful shutdown waits for.
        hub.stop().await;
        closed.cancel();
        outcome
    };
    let (served, restarted) = tokio::join!(server.into_future(), restarts);
    restarted?;
    Ok(served?)
}

async fn restart_until(hub: &Hub, shutdown: impl Future<Output = ()>) -> Result<(), Error> {
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            () = &mut shutdown => return Ok(()),
            Some(reason) = hub.restart_requested() => hub.restart(&reason).await?,
        }
    }
}

/// The hub's own routes, then the crates' (swapped on restart), all behind the pairing gate.
fn router(hub: Arc<Hub>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/api/ping", get(|| async { StatusCode::NO_CONTENT }))
        .merge(events::router())
        .merge(pairing::router())
        .merge(devices::router())
        .merge(config::router())
        .fallback(crates)
        .layer(axum::middleware::from_fn_with_state(hub.clone(), gate::require))
        .layer(axum::middleware::from_fn(conditional::not_modified))
        .layer(compress::layer())
        .with_state(hub)
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
        &lull_youtube::MIGRATIONS,
    ] {
        ctx.db.migrate(m)?;
    }

    let youtube = lull_youtube::start(ctx).await?;
    let soulseek = lull_soulseek::start(ctx, youtube.clone()).await?;
    let library = lull_library::start(ctx, soulseek.clone()).await?;
    let discovery = lull_discovery::start(ctx, library.clone(), soulseek.clone(), youtube.clone()).await?;
    // YouTube checks a copy's sound against the catalog's previews (ADR-0016).
    youtube.use_previews(discovery.clone());
    let lastfm = lull_lastfm::start(ctx).await?;
    let lyrics = lull_lyrics::start(ctx).await?;
    let collection =
        lull_collection::start(ctx, library.clone(), lastfm.clone(), discovery.clone(), youtube.clone()).await?;
    // Library mirrors likes and playlists into Jellyfin; collection starts after it (ADR-0003).
    lull_library::attach_collection(&library, collection.clone());
    let subsonic = lull_subsonic::start(ctx, library.clone(), collection.clone(), lyrics.clone()).await?;
    let skins = lull_skins::start(ctx).await?;
    let themes = lull_themes::start(ctx).await?;

    Ok(Router::new()
        .merge(lull_soulseek::router(soulseek, library.clone()))
        .merge(lull_youtube::router(youtube))
        .merge(lull_library::router(library))
        .merge(lull_discovery::router(discovery))
        .merge(lull_lastfm::router(lastfm))
        .merge(lull_lyrics::router(lyrics))
        .merge(lull_collection::router(collection))
        .merge(lull_skins::router(skins))
        .merge(lull_themes::router(themes))
        .merge(lull_core::pictures::router(ctx.pictures.clone()))
        // Subsonic apps sign in with an app password or API key; the gate leaves /rest to it.
        .merge(lull_subsonic::router(subsonic))
        .fallback(web::serve))
}

/// The running crates' routes and the web UI. Waits while the crates restart.
async fn crates(State(hub): State<Arc<Hub>>, req: Request) -> Response {
    hub.router().await.oneshot(req).await.into_response()
}

/// Always 200 while the process serves: a dependency being down is reported, not fatal.
async fn health(State(hub): State<Arc<Hub>>) -> Json<serde_json::Value> {
    let ctx = hub.ctx().await;
    let flags: Vec<&str> = ctx.config.flags.names().collect();
    Json(serde_json::json!({ "version": env!("CARGO_PKG_VERSION"), "services": ctx.status.snapshot(), "flags": flags }))
}
