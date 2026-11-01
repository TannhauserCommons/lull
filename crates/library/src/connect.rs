//! Connecting Jellyfin from the app (ADR-0003): finding servers on the local network (UDP
//! 7359), Quick Connect, and a one-time sign-in whose password is never stored. Each ends with
//! an access token, which switch.rs stores as `LULL_JELLYFIN_API_KEY`.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use lull_core::http::{self, Http};
use lull_core::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::net::UdpSocket;

use crate::{Service, switch};

const SERVICE: &str = "jellyfin";
const DISCOVERY_PORT: u16 = 7359;
/// How long servers get to answer the broadcast.
const DISCOVERY_WAIT: Duration = Duration::from_millis(1500);
/// Jellyfin forgets a Quick Connect secret after a few minutes; lull forgets it a little later.
const PENDING_FOR: Duration = Duration::from_secs(15 * 60);
/// Jellyfin keys a token to the device that asked for it: one hub, one device.
const CLIENT: &str = concat!(
    r#"MediaBrowser Client="lull", Device="lull hub", DeviceId="lull-hub", Version=""#,
    env!("CARGO_PKG_VERSION"),
    r#"""#
);

/// What a sign-in gives: the token, and the user it reads the library as.
pub struct Token {
    pub access_token: String,
    pub user: String,
}

/// A Jellyfin server's address as the listener typed it, checked.
pub fn server_url(raw: &str) -> Result<String> {
    let url = raw.trim().trim_end_matches('/');
    if url.starts_with("http://") || url.starts_with("https://") {
        Ok(url.to_owned())
    } else {
        Err(Error::BadRequest(format!(
            "{url:?} isn't a Jellyfin address: use http://host:8096 or https://…"
        )))
    }
}

#[derive(Debug, Serialize)]
pub struct Found {
    name: String,
    url: String,
    version: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Reply {
    address: String,
    id: String,
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct PublicInfo {
    version: Option<String>,
}

/// Jellyfin servers answering a broadcast on the local network. A hub in a container on a bridge
/// network sees none: its broadcast stays inside the container network.
pub async fn discover(http: &Http) -> Result<Vec<Found>> {
    let socket = UdpSocket::bind(("0.0.0.0", 0))
        .await
        .map_err(|e| Error::internal(format!("discovery socket: {e}")))?;
    socket
        .set_broadcast(true)
        .map_err(|e| Error::internal(format!("discovery socket: {e}")))?;
    socket
        .send_to(b"Who is JellyfinServer?", ("255.255.255.255", DISCOVERY_PORT))
        .await
        .map_err(|e| Error::unavailable(SERVICE, format!("the discovery broadcast failed: {e}")))?;
    let mut replies: Vec<Reply> = Vec::new();
    let deadline = tokio::time::Instant::now() + DISCOVERY_WAIT;
    let mut buf = [0u8; 4096];
    while let Ok(received) = tokio::time::timeout_at(deadline, socket.recv_from(&mut buf)).await {
        let Ok((n, from)) = received else { break };
        match serde_json::from_slice::<Reply>(&buf[..n]) {
            Ok(r) if !replies.iter().any(|seen| seen.id == r.id) => replies.push(r),
            Ok(_) => {}
            Err(e) => tracing::debug!(%from, error = %e, "not a Jellyfin discovery reply"),
        }
    }
    let mut found = Vec::new();
    for r in replies {
        let url = r.address.trim_end_matches('/').to_owned();
        let info: Option<PublicInfo> = http
            .json(SERVICE, |c| c.get(format!("{url}/System/Info/Public")))
            .await
            .ok();
        found.push(Found {
            name: r.name,
            version: info.and_then(|i| i.version),
            url,
        });
    }
    Ok(found)
}

/// Quick Connect attempts waiting for the listener's approval: secret → server address.
#[derive(Default)]
pub struct Pending(Mutex<HashMap<String, (String, Instant)>>);

impl Pending {
    pub fn add(&self, secret: &str, url: String) {
        let mut map = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        map.retain(|_, (_, at)| at.elapsed() < PENDING_FOR);
        map.insert(secret.to_owned(), (url, Instant::now()));
    }

    fn url(&self, secret: &str) -> Option<String> {
        let map = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        map.get(secret).map(|(url, _)| url.clone())
    }

    fn remove(&self, secret: &str) {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).remove(secret);
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Started {
    pub secret: String,
    pub code: String,
}

/// Starts Quick Connect: the listener approves `code` in Jellyfin.
pub async fn initiate(http: &Http, url: &str) -> Result<Started> {
    let resp = http
        .send(SERVICE, |c| {
            c.post(format!("{url}/QuickConnect/Initiate"))
                .header(reqwest::header::AUTHORIZATION, CLIENT)
        })
        .await?;
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err(Error::Conflict(
            "Quick Connect is off on this Jellyfin: sign in with a username and password instead.".into(),
        ));
    }
    http::ok(SERVICE, resp)?
        .json()
        .await
        .map_err(|e| Error::upstream(SERVICE, format!("bad Quick Connect answer: {e}")))
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ConnectState {
    authenticated: bool,
}

/// `waiting`, `expired`, or `approved`: then the token is stored and the hub switches to Jellyfin.
pub async fn poll(svc: &Service, secret: &str) -> Result<&'static str> {
    let url = svc
        .connects
        .url(secret)
        .ok_or_else(|| Error::NotFound("No Quick Connect is waiting under that secret: start again.".into()))?;
    let resp = svc
        .ctx
        .http
        .send(SERVICE, |c| {
            c.get(format!("{url}/QuickConnect/Connect"))
                .query(&[("secret", secret)])
                .header(reqwest::header::AUTHORIZATION, CLIENT)
        })
        .await?;
    // Jellyfin answers 404 for a secret it has forgotten (expired or already used).
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        svc.connects.remove(secret);
        return Ok("expired");
    }
    let state: ConnectState = http::ok(SERVICE, resp)?
        .json()
        .await
        .map_err(|e| Error::upstream(SERVICE, format!("bad Quick Connect state: {e}")))?;
    if !state.authenticated {
        return Ok("waiting");
    }
    let token = authenticate(
        &svc.ctx.http,
        &url,
        "/Users/AuthenticateWithQuickConnect",
        json!({ "Secret": secret }),
    )
    .await?;
    svc.connects.remove(secret);
    switch::to_jellyfin(svc, url, token)?;
    Ok("approved")
}

/// The fallback when Quick Connect is off. The password goes to Jellyfin once and is dropped.
pub async fn sign_in(http: &Http, url: &str, username: &str, password: &str) -> Result<Token> {
    authenticate(
        http,
        url,
        "/Users/AuthenticateByName",
        json!({ "Username": username, "Pw": password }),
    )
    .await
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct AuthResult {
    access_token: String,
    user: AuthUser,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct AuthUser {
    name: String,
}

async fn authenticate(http: &Http, url: &str, path: &str, body: serde_json::Value) -> Result<Token> {
    let resp = http
        .send(SERVICE, |c| {
            c.post(format!("{url}{path}"))
                .header(reqwest::header::AUTHORIZATION, CLIENT)
                .json(&body)
        })
        .await?;
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err(Error::BadRequest("Jellyfin refused that sign-in.".into()));
    }
    let auth: AuthResult = http::ok(SERVICE, resp)?
        .json()
        .await
        .map_err(|e| Error::upstream(SERVICE, format!("bad sign-in answer: {e}")))?;
    Ok(Token {
        access_token: auth.access_token,
        user: auth.user.name,
    })
}
