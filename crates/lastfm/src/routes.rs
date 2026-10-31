//! `/api/lastfm`: connection state and switches, the web auth flow, disconnect and retry.
//!
//! Sign-in: `/connect` redirects the browser to Last.fm with a callback carrying a one-time
//! `state` nonce; Last.fm sends the browser back to `/callback?state=…&token=…`, the token is
//! traded for a session key, and the browser lands on Settings > Last.fm. The nonce ties the
//! callback to a sign-in this lull started, so no other page can attach its own account.

use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::Redirect;
use axum::routing::{get, post};
use axum::{Json, Router};
use lull_core::{Error, Result};
use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::{Service, account, api, queue};

/// How long a sign-in may take on Last.fm's page.
const AUTH_WINDOW: Duration = Duration::from_secs(15 * 60);
/// Where the browser lands after the callback (the web UI's Last.fm settings).
const SETTINGS: &str = "/#/settings/lastfm";

pub struct PendingAuth {
    nonce: String,
    until: Instant,
}

pub fn router(svc: Arc<Service>) -> Router {
    Router::new()
        .route("/api/lastfm", get(state).patch(switches).delete(disconnect))
        .route("/api/lastfm/connect", get(connect))
        .route("/api/lastfm/callback", get(callback))
        .route("/api/lastfm/retry", post(retry))
        .with_state(svc)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Connection {
    /// The server has API credentials; without them Last.fm is off.
    configured: bool,
    connected: bool,
    user: Option<String>,
    scrobbling: bool,
    use_history: bool,
    queued: u64,
    /// The latest problem in a sentence (sign-in failed, signed out, scrobbles failing).
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

async fn state(State(svc): State<Arc<Service>>) -> Result<Json<Connection>> {
    let (acc, (queued, _), queue_error) = svc
        .ctx
        .db
        .call(|c| Ok((account(c)?, queue::summary(c)?, queue::last_error(c)?)))
        .await?;
    let error = svc
        .is_signed_out()
        .then(|| api::Failure::Session.to_string())
        .or_else(|| svc.auth_error.lock().ok().and_then(|e| e.clone()))
        .or(queue_error);
    Ok(Json(Connection {
        configured: svc.client.is_some(),
        connected: acc.is_some(),
        user: acc.as_ref().map(|a| a.user.clone()),
        scrobbling: acc.as_ref().is_some_and(|a| a.scrobbling),
        use_history: acc.as_ref().is_some_and(|a| a.use_history),
        queued,
        error,
    }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Switches {
    scrobbling: Option<bool>,
    use_history: Option<bool>,
}

/// `PATCH /api/lastfm { scrobbling?, useHistory? }`: the listener's two Last.fm switches.
async fn switches(State(svc): State<Arc<Service>>, Json(body): Json<Switches>) -> Result<Json<Connection>> {
    let changed = svc
        .ctx
        .db
        .call(move |c| {
            Ok(c.execute(
                "UPDATE lastfm_session SET scrobbling = COALESCE(?1, scrobbling),
                 use_history = COALESCE(?2, use_history) WHERE id = 1",
                params![body.scrobbling, body.use_history],
            )?)
        })
        .await?;
    if changed == 0 {
        return Err(Error::Conflict("Last.fm is not connected".into()));
    }
    if body.use_history.is_some() {
        let svc = svc.clone();
        tokio::spawn(async move { svc.share_history().await });
    }
    svc.report().await;
    state(State(svc)).await
}

async fn connect(State(svc): State<Arc<Service>>) -> Result<Redirect> {
    let client = svc.client()?;
    let public =
        svc.ctx.config.public_url.as_deref().ok_or_else(|| {
            Error::unavailable(api::SERVICE, "set LULL_PUBLIC_URL so Last.fm can send you back to lull")
        })?;
    let nonce = nonce();
    let cb = format!("{public}/api/lastfm/callback?state={nonce}");
    let url = reqwest::Url::parse_with_params(api::AUTH_PAGE, &[("api_key", client.api_key()), ("cb", &cb)])
        .map_err(|e| Error::internal(format!("Last.fm auth URL: {e}")))?;
    if let Ok(mut pending) = svc.pending_auth.lock() {
        *pending = Some(PendingAuth {
            nonce,
            until: Instant::now() + AUTH_WINDOW,
        });
    }
    Ok(Redirect::to(url.as_str()))
}

#[derive(Deserialize)]
struct Callback {
    state: String,
    token: String,
}

async fn callback(State(svc): State<Arc<Service>>, Query(q): Query<Callback>) -> Result<Redirect> {
    let outcome = sign_in(&svc, q).await;
    if let Ok(mut e) = svc.auth_error.lock() {
        *e = outcome.as_ref().err().map(ToString::to_string);
    }
    if let Err(e) = &outcome {
        tracing::warn!(error = %e, "Last.fm sign-in failed");
    }
    svc.report().await;
    Ok(Redirect::to(SETTINGS))
}

async fn sign_in(svc: &Arc<Service>, q: Callback) -> Result<()> {
    let expected = svc.pending_auth.lock().ok().and_then(|mut p| p.take());
    let valid = expected.is_some_and(|p| p.nonce == q.state && Instant::now() < p.until);
    if !valid {
        return Err(Error::BadRequest(
            "This Last.fm sign-in was not started here or took too long; connect again".into(),
        ));
    }
    let session = svc.client()?.session(&q.token).await?;
    svc.ctx
        .db
        .call(move |c| {
            c.execute(
                "INSERT INTO lastfm_session (id, user, session_key, connected_at) VALUES (1, ?1, ?2, ?3)
                 ON CONFLICT(id) DO UPDATE SET user = excluded.user, session_key = excluded.session_key,
                 connected_at = excluded.connected_at",
                params![session.name, session.key, lull_core::now()],
            )?;
            Ok(())
        })
        .await?;
    if let Ok(mut out) = svc.signed_out.lock() {
        *out = false;
    }
    svc.wake.notify_one();
    let svc = svc.clone();
    tokio::spawn(async move { svc.share_history().await });
    Ok(())
}

/// Forgets the account and drops the queued scrobbles (the UI warns before calling this).
async fn disconnect(State(svc): State<Arc<Service>>) -> Result<StatusCode> {
    svc.ctx
        .db
        .call(|c| {
            c.execute("DELETE FROM lastfm_session", [])?;
            queue::clear(c)
        })
        .await?;
    if let Ok(mut out) = svc.signed_out.lock() {
        *out = false;
    }
    svc.share_history().await;
    svc.report().await;
    Ok(StatusCode::NO_CONTENT)
}

/// "Retry now": every waiting scrobble is sent at once.
async fn retry(State(svc): State<Arc<Service>>) -> Result<StatusCode> {
    svc.client()?;
    svc.ctx.db.call(|c| queue::wake_all(c)).await?;
    svc.wake.notify_one();
    Ok(StatusCode::NO_CONTENT)
}

/// 128 unguessable bits: `RandomState` is keyed from the OS's random source.
fn nonce() -> String {
    let a = RandomState::new().hash_one(Instant::now());
    let b = RandomState::new().hash_one(a);
    format!("{a:016x}{b:016x}")
}
