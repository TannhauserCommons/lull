//! The shared outbound HTTP client: timeouts on every call, and bounded retries with backoff
//! (honouring `Retry-After`) for failures that are worth retrying. Every crate calls the
//! outside world through this, so no call can hang forever or hammer a service.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;

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
/// Last.fm counts calls per IP (at most 5 a second, averaged over 5 minutes, and "several per
/// second" risks suspension), whichever crate makes them: scrobbling (lastfm), search and pages
/// (discovery) and history imports (collection) share one `Http::pace` under this key, at
/// discovery's careful 0.5 s.
pub const LASTFM: &str = "lastfm";
pub const LASTFM_PACE: Duration = Duration::from_millis(500);
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

/// How many requests lull sends at once to one host it fetches pictures from (a CDN, the Cover
/// Art Archive): every such host speaks HTTP/2, so they share one connection, and a long page
/// of covers never opens dozens of fetches against one host at once.
pub const PER_HOST: usize = 4;

#[derive(Clone)]
pub struct Http {
    client: reqwest::Client,
    /// Per paced service, when lull last sent it a request, shared by every clone (every crate).
    turns: Gates<Option<Instant>>,
    /// Per windowed service, the requests sent lately, shared by every clone (every crate).
    windows: Gates<Window>,
    /// Per windowed service, until when its quota is spent (`close_window`).
    closed: Arc<Mutex<HashMap<&'static str, Instant>>>,
    /// Per picture host, its places (`PER_HOST`).
    hosts: Arc<Mutex<HashMap<String, Arc<Semaphore>>>>,
}

/// One async gate per service. Callers queue at it in order, and one that gives up (a request
/// whose page moved on) simply leaves the queue: a turn is taken when the request goes out,
/// never booked ahead, so nothing that is no longer wanted holds up what is.
type Gates<T> = Arc<Mutex<HashMap<&'static str, Arc<tokio::sync::Mutex<T>>>>>;

fn gate<T: Default>(gates: &Gates<T>, service: &'static str) -> Arc<tokio::sync::Mutex<T>> {
    gates
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .entry(service)
        .or_default()
        .clone()
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
            closed: Arc::default(),
            hosts: Arc::default(),
        })
    }

    /// Waits for `service`'s next turn, so lull as a whole calls it at most once per `every`.
    /// Call it right before each request to a service with a published rate limit (MusicBrainz).
    /// The turn is taken when it returns, in the order callers came; a caller that stops waiting
    /// takes none.
    pub async fn pace(&self, service: &'static str, every: Duration) {
        let gate = gate(&self.turns, service);
        let mut last = gate.lock().await;
        if let Some(at) = *last {
            tokio::time::sleep_until(at + every).await;
        }
        *last = Some(Instant::now());
    }

    /// Waits for a place in `limit`'s window, so lull as a whole sends at most `per` requests in
    /// any `every` (bursts allowed). Call it right before each request to such a service (Deezer).
    /// Like `pace`, the place is taken when it returns.
    pub async fn window(&self, limit: Limit) {
        let gate = gate(&self.windows, limit.service);
        let mut window = gate.lock().await;
        loop {
            let closed = self
                .closed
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(limit.service)
                .copied();
            let now = Instant::now();
            let at = window.next(now, limit).max(closed.unwrap_or(now));
            if at <= now {
                window.sent(now);
                return;
            }
            // A quota error from a request already out may close the window meanwhile: look again.
            tokio::time::sleep_until(at).await;
        }
    }

    /// The service said its quota is reached: no request goes for a whole window.
    pub fn close_window(&self, limit: Limit) {
        self.closed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(limit.service, Instant::now() + limit.every);
    }

    /// A place among the `PER_HOST` requests that may be out to `url`'s host at once, held until
    /// the permit is dropped. For picture fetches, which have no published limit. (`None` never
    /// happens: the places are never closed.)
    pub async fn host_place(&self, url: &str) -> Option<OwnedSemaphorePermit> {
        let host = reqwest::Url::parse(url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_owned))
            .unwrap_or_default();
        let places = self
            .hosts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(host)
            .or_insert_with(|| Arc::new(Semaphore::new(PER_HOST)))
            .clone();
        places.acquire_owned().await.ok()
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

/// The requests a windowed service was sent lately, to tell when the next may go.
#[derive(Debug, Default)]
pub struct Window {
    /// Send times, oldest first.
    sent: VecDeque<Instant>,
}

impl Window {
    /// The earliest the next request may go: at most `per` in any `every`.
    pub fn next(&mut self, now: Instant, limit: Limit) -> Instant {
        while self.sent.front().is_some_and(|t| *t + limit.every <= now) {
            self.sent.pop_front();
        }
        if self.sent.len() >= limit.per {
            self.sent[self.sent.len() - limit.per] + limit.every
        } else {
            now
        }
    }

    pub fn sent(&mut self, at: Instant) {
        self.sent.push_back(at);
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
    fn a_window_keeps_to_its_limit() {
        let mut w = Window::default();
        let t0 = Instant::now();
        for _ in 0..50 {
            assert_eq!(w.next(t0, DEEZER), t0);
            w.sent(t0);
        }
        assert_eq!(
            w.next(t0, DEEZER),
            t0 + Duration::from_secs(5),
            "the 51st waits for the first to age out"
        );
        let later = t0 + Duration::from_secs(20);
        assert_eq!(w.next(later, DEEZER), later);
    }

    #[tokio::test(start_paused = true)]
    async fn a_paced_turn_is_taken_when_the_request_goes_not_booked_ahead() -> Result<()> {
        // Live, covers scrolled past kept their booked MusicBrainz turns, so the screen the listener
        // stopped on waited behind them. A caller that gives up must leave no turn behind.
        let http = Http::new()?;
        let every = Duration::from_secs(1);
        let t0 = Instant::now();
        http.pace("svc", every).await;
        assert_eq!(Instant::now(), t0, "the first goes at once");
        for _ in 0..30 {
            // Waiters that gave up before their turn (rows scrolled away).
            let _ = tokio::time::timeout(Duration::from_millis(10), http.pace("svc", every)).await;
        }
        http.pace("svc", every).await;
        assert_eq!(Instant::now(), t0 + every, "the next one still waits only one turn");
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn a_closed_window_holds_every_request_for_a_whole_window() -> Result<()> {
        let http = Http::new()?;
        let t0 = Instant::now();
        http.window(DEEZER).await;
        http.close_window(DEEZER);
        http.window(DEEZER).await;
        assert_eq!(Instant::now(), t0 + DEEZER.every, "after a quota error, a whole window");
        Ok(())
    }
}
