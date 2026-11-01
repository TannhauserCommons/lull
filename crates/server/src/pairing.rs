//! Pairing a device (ADR-0002): the setup code that pairs the first one, and the single-use pair
//! codes a paired device hands to the next (QR `<hub url>/#/pair/<code>`).

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::{HeaderMap, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use lull_core::config::{Config, Secret};
use lull_core::devices::{self, Device, Kind};
use lull_core::status::State as Health;
use lull_core::{Error, Result};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::gate;
use crate::hub::Hub;

/// No 0/O or 1/I, so a code read off a screen is typed right. 32 symbols: a byte maps evenly.
const ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const PAIR_CODE_LEN: usize = 6;
/// Longer than a pair code: it stays valid until the first device claims it.
const SETUP_CODE_LEN: usize = 8;
const PAIR_CODE_TTL_S: i64 = 10 * 60;
/// Wrong codes allowed per minute before every code is refused for the rest of that minute,
/// so guessing a 6-character code within its 10 minutes stays hopeless.
const MISSES_PER_MINUTE: usize = 10;

pub struct Pairing {
    /// `LULL_SETUP_CODE`, used instead of a random setup code.
    fixed: Option<Secret>,
    /// The code that pairs the first device; `None` while a device is paired.
    setup: Mutex<Option<String>>,
    /// Pair code -> when it expires (unix seconds).
    codes: Mutex<HashMap<String, i64>>,
    /// When each recent wrong code came in.
    misses: Mutex<VecDeque<i64>>,
    /// Serialises claims, so two devices can't both become the first.
    claiming: tokio::sync::Mutex<()>,
}

impl Pairing {
    pub fn new(fixed: Option<Secret>) -> Pairing {
        Pairing {
            fixed,
            setup: Mutex::default(),
            codes: Mutex::default(),
            misses: Mutex::default(),
            claiming: tokio::sync::Mutex::new(()),
        }
    }

    /// No device is paired: accept the setup code, and print it unless the listener chose it.
    pub fn arm_setup(&self) {
        let code = match &self.fixed {
            Some(fixed) => {
                tracing::warn!("no device is paired yet: open lull and enter the setup code from LULL_SETUP_CODE");
                normalize(fixed.expose())
            }
            None => match random_code(SETUP_CODE_LEN) {
                Ok(code) => {
                    tracing::warn!(setup_code = %code, "no device is paired yet: open lull and enter this setup code");
                    code
                }
                Err(e) => {
                    tracing::error!(error = %e, "no setup code: restart lull to try again");
                    return;
                }
            },
        };
        *lock(&self.setup) = Some(code);
    }

    /// A new single-use pair code and when it expires.
    pub fn issue(&self, now: i64) -> Result<(String, i64)> {
        let code = random_code(PAIR_CODE_LEN)?;
        let expires = now + PAIR_CODE_TTL_S;
        let mut codes = lock(&self.codes);
        codes.retain(|_, at| *at > now);
        codes.insert(code.clone(), expires);
        Ok((code, expires))
    }

    /// Uses up a pair code: true once, while it is valid.
    pub fn redeem(&self, code: &str, now: i64) -> bool {
        let code = normalize(code);
        self.attempt(now, || lock(&self.codes).remove(&code).is_some_and(|at| at > now))
    }

    fn setup_matches(&self, code: &str, now: i64) -> bool {
        let code = normalize(code);
        self.attempt(now, || {
            lock(&self.setup)
                .as_deref()
                .is_some_and(|setup| constant_eq(setup.as_bytes(), code.as_bytes()))
        })
    }

    /// Runs `check` unless too many codes were wrong in the last minute, and counts a miss.
    fn attempt(&self, now: i64, check: impl FnOnce() -> bool) -> bool {
        let mut misses = lock(&self.misses);
        while misses.front().is_some_and(|at| now - at >= 60) {
            misses.pop_front();
        }
        if misses.len() >= MISSES_PER_MINUTE {
            return false;
        }
        let ok = check();
        if !ok {
            misses.push_back(now);
        }
        ok
    }
}

pub fn router() -> Router<Arc<Hub>> {
    Router::new()
        .route("/api/setup", get(setup))
        .route("/api/setup/claim", post(claim))
        .route("/api/pair/codes", post(issue))
        .route("/api/pair", post(pair))
}

/// `GET /api/setup` (open): whether the hub has its first device, what first run still needs,
/// and whether the Soulseek account is set in lull (an slskd the app runs) or where slskd runs.
async fn setup(State(hub): State<Arc<Hub>>) -> Result<Json<Value>> {
    let ctx = hub.ctx().await;
    let services = ctx.status.snapshot();
    let ready = |name: &str| {
        services
            .get(name)
            .is_some_and(|s| matches!(s.state, Health::Ok | Health::Warn))
    };
    Ok(Json(json!({
        "claimed": hub.paired().await?,
        "soulseekManaged": ctx.config.soulseek.config_file.is_some(),
        "steps": { "library": ready("library"), "soulseek": ready("soulseek"), "lastfm": ready("lastfm") },
    })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PairBody {
    code: String,
    name: String,
    #[serde(default)]
    kind: NewKind,
}

/// What can pair with a code; Subsonic clients get an app password instead.
#[derive(Deserialize, Default, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum NewKind {
    #[default]
    Browser,
    App,
}

/// `POST /api/setup/claim` (open): the first device, with the setup code.
async fn claim(
    State(hub): State<Arc<Hub>>,
    headers: HeaderMap,
    body: Result<Json<PairBody>, JsonRejection>,
) -> Result<Response> {
    let Json(body) = body.map_err(bad_json)?;
    let name = device_name(&body.name)?;
    let _first = hub.claiming().await;
    if hub.paired().await? || !hub.pairing.setup_matches(&body.code, lull_core::now()) {
        return Err(Error::Unauthorized);
    }
    let (device, token) = devices::create(&hub.db, &name, kind(body.kind)).await?;
    *lock(&hub.pairing.setup) = None;
    tracing::info!(device = %device.id, "the first device is paired");
    welcome(&hub, &headers, device, token).await
}

/// `POST /api/pair/codes`: a code for the next device, from a paired one.
async fn issue(State(hub): State<Arc<Hub>>, headers: HeaderMap) -> Result<Json<Value>> {
    let (code, expires_at) = hub.pairing.issue(lull_core::now())?;
    let url = format!("{}/#/pair/{code}", origin(&headers, &hub.ctx().await.config));
    Ok(Json(json!({ "code": code, "url": url, "expiresAt": expires_at })))
}

/// `POST /api/pair` (open): a new device, with a pair code.
async fn pair(
    State(hub): State<Arc<Hub>>,
    headers: HeaderMap,
    body: Result<Json<PairBody>, JsonRejection>,
) -> Result<Response> {
    let Json(body) = body.map_err(bad_json)?;
    let name = device_name(&body.name)?;
    if !hub.pairing.redeem(&body.code, lull_core::now()) {
        return Err(Error::Unauthorized);
    }
    let (device, token) = devices::create(&hub.db, &name, kind(body.kind)).await?;
    tracing::info!(device = %device.id, "a device is paired");
    welcome(&hub, &headers, device, token).await
}

/// Browsers get their token as the cookie (they never see it); apps get it in the body.
async fn welcome(hub: &Hub, headers: &HeaderMap, device: Device, token: Secret) -> Result<Response> {
    if device.kind == Kind::App {
        return Ok(Json(json!({ "device": device, "token": token.expose() })).into_response());
    }
    let secure = gate::https(headers, &hub.ctx().await.config);
    let cookie =
        gate::cookie(token.expose(), secure).ok_or_else(|| Error::internal("device token is not a cookie value"))?;
    Ok(([(header::SET_COOKIE, cookie)], Json(json!({ "device": device }))).into_response())
}

impl Hub {
    async fn claiming(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.pairing.claiming.lock().await
    }
}

/// The URL the hub is reached at: the public URL, else what the asking device used.
fn origin(headers: &HeaderMap, config: &Config) -> String {
    if let Some(url) = &config.public_url {
        return url.clone();
    }
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost");
    let scheme = if gate::https(headers, config) { "https" } else { "http" };
    format!("{scheme}://{host}")
}

fn kind(kind: NewKind) -> Kind {
    match kind {
        NewKind::Browser => Kind::Browser,
        NewKind::App => Kind::App,
    }
}

pub fn device_name(raw: &str) -> Result<String> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(Error::BadRequest("Give the device a name.".into()));
    }
    Ok(name.to_owned())
}

pub fn bad_json(e: JsonRejection) -> Error {
    Error::BadRequest(e.body_text())
}

/// Codes are typed by people: case, spaces and dashes don't matter.
fn normalize(code: &str) -> String {
    code.chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

fn random_code(len: usize) -> Result<String> {
    let mut bytes = vec![0u8; len];
    getrandom::fill(&mut bytes).map_err(|e| Error::internal(format!("random: {e}")))?;
    Ok(bytes
        .iter()
        .map(|b| char::from(ALPHABET[usize::from(b % 32)]))
        .collect())
}

fn constant_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pair_code_works_once_and_only_for_ten_minutes() -> Result<()> {
        let pairing = Pairing::new(None);
        let (code, expires) = pairing.issue(1_000)?;
        assert_eq!(expires, 1_000 + PAIR_CODE_TTL_S);
        assert!(pairing.redeem(&code.to_lowercase(), 1_001), "codes ignore case");
        assert!(!pairing.redeem(&code, 1_002), "a code is single use");
        let (late, _) = pairing.issue(2_000)?;
        assert!(
            !pairing.redeem(&late, 2_000 + PAIR_CODE_TTL_S),
            "an expired code is refused"
        );
        Ok(())
    }

    #[test]
    fn too_many_wrong_codes_lock_every_code_out_for_a_minute() -> Result<()> {
        let pairing = Pairing::new(None);
        let (code, _) = pairing.issue(0)?;
        for _ in 0..MISSES_PER_MINUTE {
            assert!(!pairing.redeem("WRONG1", 10));
        }
        assert!(!pairing.redeem(&code, 20), "refused while locked out, even when right");
        assert!(pairing.redeem(&code, 70), "a minute later the right code works");
        Ok(())
    }

    #[test]
    fn the_setup_code_from_the_environment_is_accepted_as_typed() {
        let pairing = Pairing::new(Some(Secret::new("lull-setup")));
        assert!(
            !pairing.setup_matches("LULLSETUP", 0),
            "not armed while a device is paired"
        );
        pairing.arm_setup();
        assert!(pairing.setup_matches(" LULL SETUP ", 0));
    }
}
