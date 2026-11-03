//! The shared outbound HTTP client: timeouts on every call, and bounded retries with backoff
//! (honouring `Retry-After`) for failures that are worth retrying. Every crate calls the
//! outside world through this, so no call can hang forever or hammer a service.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use reqwest::{RequestBuilder, Response, StatusCode};
use serde::de::DeserializeOwned;

use crate::error::{Error, Result};

const ATTEMPTS: u32 = 3;
const FIRST_BACKOFF: Duration = Duration::from_millis(400);
const MAX_WAIT: Duration = Duration::from_secs(10);

/// MusicBrainz and others require a User-Agent naming the app and a contact.
pub const USER_AGENT: &str = concat!(
    "lull/",
    env!("CARGO_PKG_VERSION"),
    " ( https://github.com/TannhauserCommons/lull )"
);

/// MusicBrainz allows one request per second per IP, whichever crate sends it (`Http::pace`).
pub const MUSICBRAINZ_PACE: Duration = Duration::from_millis(1100);
/// The `Http::pace` key for MusicBrainz: every crate must use this one, so they share the budget.
pub const MUSICBRAINZ: &str = "musicbrainz";
/// Deezer allows 50 requests per 5 s per IP, whichever crate sends them (`Http::window`): search
/// and pages (discovery) and playlist links (collection) share it.
pub const DEEZER: Limit = Limit {
    service: "deezer",
    per: 50,
    every: Duration::from_secs(5),
};

/// A service's published request limit: at most `per` requests in any `every`.
#[derive(Debug, Clone, Copy)]
pub struct Limit {
    pub service: &'static str,
    pub per: usize,
    pub every: Duration,
}

#[derive(Clone)]
pub struct Http {
    client: reqwest::Client,
    /// The next free slot per paced service, shared by every clone (every crate).
    turns: Arc<Mutex<HashMap<&'static str, Instant>>>,
    /// The requests booked per windowed service, shared by every clone (every crate).
    windows: Arc<Mutex<HashMap<&'static str, Window>>>,
}

impl Http {
    pub fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|e| Error::internal(format!("http client: {e}")))?;
        Ok(Http {
            client,
            turns: Arc::default(),
            windows: Arc::default(),
        })
    }

    /// Waits for `service`'s next turn, so lull as a whole calls it at most once per `every`.
    /// Call it before each request to a service with a published rate limit (MusicBrainz).
    pub async fn pace(&self, service: &'static str, every: Duration) {
        let wait = {
            let mut turns = self.turns.lock().unwrap_or_else(PoisonError::into_inner);
            let now = Instant::now();
            let slot = turns.get(service).copied().filter(|t| *t > now).unwrap_or(now);
            turns.insert(service, slot + every);
            slot - now
        };
        tokio::time::sleep(wait).await;
    }

    /// Waits for a place in `limit`'s window, so lull as a whole sends at most `per` requests in
    /// any `every` (bursts allowed). Call it before each request to such a service (Deezer).
    pub async fn window(&self, limit: Limit) {
        let at = self
            .windows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(limit.service)
            .or_default()
            .reserve(Instant::now(), limit);
        tokio::time::sleep(at.saturating_duration_since(Instant::now())).await;
    }

    /// The service said its quota is reached: no request goes for a whole window.
    pub fn close_window(&self, limit: Limit) {
        self.windows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(limit.service)
            .or_default()
            .close(Instant::now(), limit);
    }

    /// The raw client, for building requests. Override the 20 s timeout per request with
    /// `RequestBuilder::timeout` when a call is legitimately long (a streamed body).
    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }

    /// Sends the request built by `make`, retrying connection failures, 429 and 502-504.
    /// Any other status comes back to the caller as a response.
    pub async fn send(
        &self,
        service: &'static str,
        make: impl Fn(&reqwest::Client) -> RequestBuilder,
    ) -> Result<Response> {
        let mut backoff = FIRST_BACKOFF;
        for attempt in 1..=ATTEMPTS {
            let last = attempt == ATTEMPTS;
            match make(&self.client).send().await {
                Ok(resp) if retryable(resp.status()) && !last => {
                    tokio::time::sleep(retry_after(&resp).unwrap_or(backoff)).await;
                }
                Ok(resp) if retryable(resp.status()) => {
                    return Err(Error::upstream(service, format!("answered {}", resp.status())));
                }
                Ok(resp) => return Ok(resp),
                Err(e) if (e.is_connect() || e.is_timeout()) && !last => tokio::time::sleep(backoff).await,
                Err(e) => return Err(Error::unavailable(service, e.without_url().to_string())),
            }
            backoff *= 2;
        }
        Err(Error::unavailable(service, "no attempts left"))
    }

    /// `send`, then decode a JSON body. 404 becomes `NotFound`; any other non-2xx is `Upstream`.
    pub async fn json<T: DeserializeOwned>(
        &self,
        service: &'static str,
        make: impl Fn(&reqwest::Client) -> RequestBuilder,
    ) -> Result<T> {
        let resp = ok(service, self.send(service, make).await?)?;
        resp.json()
            .await
            .map_err(|e| Error::upstream(service, format!("bad JSON: {e}")))
    }
}

/// When a windowed service's requests may go: at most `per` in any `every`, and none for a
/// window after the service said its quota was reached.
#[derive(Debug, Default)]
pub struct Window {
    /// Send times handed out, oldest first.
    sent: VecDeque<Instant>,
    closed_until: Option<Instant>,
}

impl Window {
    /// The time the next request may be sent, booked.
    pub fn reserve(&mut self, now: Instant, limit: Limit) -> Instant {
        while self.sent.front().is_some_and(|t| *t + limit.every <= now) {
            self.sent.pop_front();
        }
        let mut at = self.closed_until.filter(|u| *u > now).unwrap_or(now);
        if self.sent.len() >= limit.per {
            at = at.max(self.sent[self.sent.len() - limit.per] + limit.every);
        }
        let i = self.sent.partition_point(|t| *t <= at);
        self.sent.insert(i, at);
        at
    }

    pub fn close(&mut self, now: Instant, limit: Limit) {
        self.closed_until = Some(now + limit.every);
    }
}

/// Turns a non-2xx response into the matching error.
pub fn ok(service: &'static str, resp: Response) -> Result<Response> {
    match resp.status() {
        s if s.is_success() => Ok(resp),
        StatusCode::NOT_FOUND => Err(Error::NotFound(format!("{service}: not found"))),
        s => Err(Error::upstream(service, format!("answered {s}"))),
    }
}

fn retryable(s: StatusCode) -> bool {
    matches!(
        s,
        StatusCode::TOO_MANY_REQUESTS
            | StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT
    )
}

fn retry_after(resp: &Response) -> Option<Duration> {
    let secs: u64 = resp
        .headers()
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .parse()
        .ok()?;
    Some(Duration::from_secs(secs).min(MAX_WAIT))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_keeps_to_its_limit_and_waits_out_a_quota() {
        let mut w = Window::default();
        let t0 = Instant::now();
        for _ in 0..50 {
            assert_eq!(w.reserve(t0, DEEZER), t0);
        }
        assert_eq!(
            w.reserve(t0, DEEZER),
            t0 + Duration::from_secs(5),
            "the 51st waits for the first to age out"
        );
        let later = t0 + Duration::from_secs(20);
        assert_eq!(w.reserve(later, DEEZER), later);
        w.close(later, DEEZER);
        assert_eq!(
            w.reserve(later, DEEZER),
            later + Duration::from_secs(5),
            "after a quota error, a whole window"
        );
    }
}
