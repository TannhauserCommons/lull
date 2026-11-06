//! The hub's admin socket (ADR-0090): `lull setup`, `lull status`, `lull config` and `lull pair`,
//! run on the hub's own computer, talk to the running hub through `<data dir>/admin.sock`, with the
//! same routes and checks the web page uses. Only the hub's own user and root can open the socket
//! (mode 0600), and they can read its database anyway, so on the few routes those commands need, a
//! request that came in over it stands for a paired device. Everything else still needs a token.

use std::sync::Arc;

use axum::extract::State;
use axum::http::Method;
use axum::routing::get;
use axum::{Extension, Json, Router};
use lull_core::{Error, Result};
use serde_json::{Value, json};

use crate::hub::Hub;

/// Marks a request that came in over the admin socket. Only the socket's listener sets it.
#[derive(Debug, Clone, Copy)]
pub struct Admin;

/// The routes the commands use: setup's state, the settings, the library source, a pair code
/// and `/api/host`.
pub fn may(method: &Method, path: &str) -> bool {
    matches!(
        (method, path),
        (
            &Method::GET,
            "/api/setup" | "/api/config" | "/api/library/source" | "/api/host"
        ) | (&Method::PATCH, "/api/config")
            | (&Method::PUT, "/api/library/source")
            | (&Method::POST, "/api/pair/codes")
    )
}

pub fn router() -> Router<Arc<Hub>> {
    Router::new().route("/api/host", get(host))
}

/// `GET /api/host`, over the socket only: what the commands print that no setting holds.
async fn host(State(hub): State<Arc<Hub>>, admin: Option<Extension<Admin>>) -> Result<Json<Value>> {
    if admin.is_none() {
        return Err(Error::NotFound("/api/host answers lull's commands only".into()));
    }
    let ctx = hub.ctx().await;
    Ok(Json(json!({
        "version": lull_core::build::version(),
        "port": ctx.config.listen.port(),
        "dataDir": ctx.config.data_dir,
        "lan": crate::managed::lan_ip(),
        "publicUrl": ctx.config.public_url,
        "paired": hub.paired().await?,
    })))
}

/// Serves the hub's routes on `<data dir>/admin.sock` until the task is aborted. A socket left by
/// a hub that stopped is replaced.
#[cfg(unix)]
pub async fn listen(hub: Arc<Hub>) -> std::result::Result<tokio::task::JoinHandle<()>, crate::BoxError> {
    use std::os::unix::fs::PermissionsExt;

    let path = hub.ctx().await.config.data_dir.join(SOCKET);
    match std::fs::remove_file(&path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
        _ => {}
    }
    let listener = tokio::net::UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    let routes = crate::router(hub).layer(Extension(Admin));
    Ok(tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, routes).await {
            tracing::warn!("lull's commands can't reach the hub: {e}");
        }
    }))
}

/// The socket's name in the data folder; the commands look for it there.
#[cfg(unix)]
pub const SOCKET: &str = "admin.sock";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_socket_opens_only_what_the_commands_need() {
        assert!(may(&Method::PATCH, "/api/config"));
        assert!(may(&Method::POST, "/api/pair/codes"));
        assert!(!may(&Method::DELETE, "/api/devices/x"));
        assert!(!may(&Method::GET, "/api/devices"));
        assert!(!may(&Method::POST, "/api/library/scan"));
    }
}
