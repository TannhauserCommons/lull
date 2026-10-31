//! The shared outbound HTTP client: timeouts on every call, and bounded retries with backoff
//! (honouring `Retry-After`) for failures that are worth retrying. Every crate calls the
//! outside world through this, so no call can hang forever or hammer a service.

use std::collections::HashMap;
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

#[derive(Clone)]
pub struct Http {
    client: reqwest::Client,
    /// The next free slot per paced service, shared by every clone (every crate).
    turns: Arc<Mutex<HashMap<&'static str, Instant>>>,
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
