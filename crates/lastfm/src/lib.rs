//! Last.fm: web auth, now playing and scrobbles with an offline-safe retry queue, driven by
//! `play` events (docs/context/scrobbling.md). Also shares the listener's Last.fm history with
//! discovery as a `lastfm.history` event, so the crates stay apart.

mod api;
mod queue;
mod routes;
mod rules;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use lull_core::db::Migrations;
use lull_core::events::Event;
use lull_core::status::State;
use lull_core::{Ctx, Result};
use rusqlite::{Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::{Notify, broadcast};

use api::{Client, Failure, Play};

pub const MIGRATIONS: Migrations = Migrations {
    owner: "lastfm",
    steps: &[r#"
        CREATE TABLE lastfm_session (
            id            INTEGER PRIMARY KEY CHECK (id = 1),  -- one listener, one account
            user          TEXT NOT NULL,
            session_key   TEXT NOT NULL,
            scrobbling    INTEGER NOT NULL DEFAULT 1,
            use_history   INTEGER NOT NULL DEFAULT 1,
            connected_at  INTEGER NOT NULL
        );
        CREATE TABLE lastfm_queue (
            play_id     INTEGER PRIMARY KEY,  -- plays.id
            artist      TEXT NOT NULL,
            title       TEXT NOT NULL,
            album       TEXT NOT NULL,
            duration_s  INTEGER NOT NULL,
            started_at  INTEGER NOT NULL,
            attempts    INTEGER NOT NULL DEFAULT 0,
            next_at     INTEGER NOT NULL,
            last_error  TEXT
        );
        CREATE INDEX lastfm_queue_next ON lastfm_queue(next_at);
    "#],
};

/// The history signal is refreshed this often while connected.
const HISTORY_EVERY: Duration = Duration::from_secs(24 * 3600);
/// The queue is looked at at least this often, even with nothing due.
const IDLE_WAKE: Duration = Duration::from_secs(300);

pub struct Service {
    ctx: Ctx,
    /// `None` when the API key and secret aren't configured: everything Last.fm is off.
    client: Option<Client>,
    /// Wakes the scrobbler when a play is queued or "retry now" is pressed.
    wake: Notify,
    /// The CSRF nonce of the sign-in in progress (see routes::connect).
    pending_auth: Mutex<Option<routes::PendingAuth>>,
    /// Set when Last.fm revoked the session; cleared by connecting again.
    signed_out: Mutex<bool>,
    /// Why the last sign-in failed, shown in Settings until the next attempt.
    auth_error: Mutex<Option<String>>,
}

/// The stored connection: who, and the listener's two switches.
#[derive(Debug, Clone)]
struct Account {
    user: String,
    session_key: String,
    scrobbling: bool,
    use_history: bool,
}

pub async fn start(ctx: &Ctx) -> Result<Arc<Service>> {
    let svc = Arc::new(Service {
        ctx: ctx.clone(),
        client: ctx.config.lastfm.clone().map(|c| Client::new(ctx.http.clone(), c)),
        wake: Notify::new(),
        pending_auth: Mutex::new(None),
        signed_out: Mutex::new(false),
        auth_error: Mutex::new(None),
    });
    svc.report().await;
    if svc.client.is_some() {
        tokio::spawn(listen(svc.clone(), ctx.bus.subscribe()));
        tokio::spawn(scrobbler(svc.clone()));
        tokio::spawn(history(svc.clone()));
    }
    Ok(svc)
}

/// `/api/lastfm/...`.
pub fn router(svc: Arc<Service>) -> Router {
    routes::router(svc)
}

/// `play.started` / `play.ended` as collection publishes them (`at` is the play's start).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlayEvent {
    play_id: i64,
    at: i64,
    #[serde(default)]
    played_s: u32,
    duration: Option<u32>,
    artist: String,
    title: String,
    album: String,
}

impl PlayEvent {
    fn play(&self) -> Play {
        Play {
            artist: self.artist.clone(),
            title: self.title.clone(),
            album: self.album.clone(),
            duration_s: self.duration.unwrap_or(0),
            started_at: self.at,
        }
    }
}

/// Turns `play.started` into now playing and an eligible `play.ended` into a queued scrobble.
async fn listen(svc: Arc<Service>, mut rx: broadcast::Receiver<Event>) {
    loop {
        let event = match rx.recv().await {
            Ok(e) => e,
            Err(broadcast::error::RecvError::Lagged(n)) => {
                tracing::warn!(missed = n, "lastfm fell behind the event bus; some plays were not seen");
                continue;
            }
            Err(broadcast::error::RecvError::Closed) => return,
        };
        let started = match event.topic.as_str() {
            "play.started" => true,
            "play.ended" => false,
            _ => continue,
        };
        let outcome = match serde_json::from_value::<PlayEvent>(event.data) {
            Ok(p) if started => svc.now_playing(&p.play()).await,
            Ok(p) => svc.ended(p.play_id, p.play(), p.played_s).await,
            Err(e) => Err(lull_core::Error::internal(format!("{} payload: {e}", event.topic))),
        };
        if let Err(e) = outcome {
            tracing::warn!(error = %e, topic = %event.topic, "lastfm could not handle a play event");
        }
    }
}

/// Sends due scrobbles in batches; sleeps until the next one is due or something wakes it.
async fn scrobbler(svc: Arc<Service>) {
    loop {
        if let Err(e) = svc.flush().await {
            tracing::warn!(error = %e, "scrobble queue");
        }
        svc.report().await;
        let wait = match svc.ctx.db.call(|c| queue::summary(c)).await {
            Ok((n, Some(next))) if n > 0 => Duration::from_secs((next - lull_core::now()).clamp(1, 300) as u64),
            _ => IDLE_WAKE,
        };
        tokio::select! {
            _ = svc.wake.notified() => {}
            _ = tokio::time::sleep(wait) => {}
        }
    }
}

/// Publishes the listener's Last.fm history for discovery, now and then daily.
async fn history(svc: Arc<Service>) {
    loop {
        svc.share_history().await;
        tokio::time::sleep(HISTORY_EVERY).await;
    }
}

impl Service {
    fn client(&self) -> Result<&Client> {
        self.client.as_ref().ok_or_else(|| {
            lull_core::Error::unavailable(api::SERVICE, "set LULL_LASTFM_API_KEY and LULL_LASTFM_API_SECRET")
        })
    }

    async fn account(&self) -> Result<Option<Account>> {
        self.ctx.db.call(|c| account(c)).await
    }

    async fn now_playing(&self, play: &Play) -> Result<()> {
        let Some(acc) = self.account().await?.filter(|a| a.scrobbling) else {
            return Ok(());
        };
        self.client()?
            .now_playing(&acc.session_key, play)
            .await
            .or_else(|f| self.on_failure(f))
    }

    async fn ended(&self, play_id: i64, play: Play, played_s: u32) -> Result<()> {
        let Some(acc) = self.account().await? else {
            return Ok(());
        };
        let eligible = rules::scrobbles(play.duration_s, played_s);
        let status = match (eligible, acc.scrobbling) {
            (false, _) => "short",
            (true, false) => return Ok(()),
            (true, true) => {
                self.ctx
                    .db
                    .call(move |c| queue::push(c, play_id, &play, lull_core::now()))
                    .await?;
                self.wake.notify_one();
                "pending"
            }
        };
        self.ctx
            .bus
            .publish("scrobble", json!({ "playId": play_id, "status": status }));
        Ok(())
    }

    /// Sends every due batch until the queue is empty, waiting or Last.fm fails.
    async fn flush(&self) -> Result<()> {
        let client = self.client()?;
        let Some(acc) = self.account().await? else {
            return Ok(());
        };
        loop {
            let batch = self
                .ctx
                .db
                .call(|c| queue::due(c, lull_core::now(), api::MAX_BATCH))
                .await?;
            if batch.is_empty() {
                return Ok(());
            }
            let plays: Vec<Play> = batch.iter().map(|q| q.play.clone()).collect();
            let ids: Vec<i64> = batch.iter().map(|q| q.play_id).collect();
            match client.scrobble(&acc.session_key, &plays).await {
                Ok(()) => {
                    self.ctx.db.call(move |c| queue::remove(c, &ids)).await?;
                    for id in batch.iter().map(|q| q.play_id) {
                        self.ctx
                            .bus
                            .publish("scrobble", json!({ "playId": id, "status": "done" }));
                    }
                }
                Err(f) => {
                    let message = f.to_string();
                    self.ctx
                        .db
                        .call(move |c| queue::defer(c, &batch, lull_core::now(), &message))
                        .await?;
                    return self.on_failure(f);
                }
            }
        }
    }

    /// A revoked session is remembered (status, `/api/lastfm`); other failures just bubble up.
    fn on_failure(&self, f: Failure) -> Result<()> {
        if matches!(f, Failure::Session)
            && let Ok(mut out) = self.signed_out.lock()
        {
            *out = true;
        }
        Err(f.into())
    }

    fn is_signed_out(&self) -> bool {
        self.signed_out.lock().map(|s| *s).unwrap_or(false)
    }

    async fn share_history(&self) {
        let user = match self.account().await {
            Ok(Some(acc)) if acc.use_history => acc.user,
            Ok(_) => {
                self.ctx
                    .bus
                    .publish("lastfm.history", json!({ "user": null, "artists": [] }));
                return;
            }
            Err(e) => return tracing::warn!(error = %e, "lastfm account"),
        };
        let Ok(client) = self.client() else { return };
        match client.top_artists(&user).await {
            Ok(artists) => {
                let artists: Vec<_> = artists
                    .into_iter()
                    .map(|(name, plays)| json!({ "name": name, "plays": plays }))
                    .collect();
                self.ctx
                    .bus
                    .publish("lastfm.history", json!({ "user": user, "artists": artists }));
            }
            Err(f) => {
                if let Err(e) = self.on_failure(f) {
                    tracing::warn!(error = %e, "Last.fm history");
                }
            }
        }
    }

    /// Sets the `lastfm` service state from what is stored (drives /health and the status tray).
    async fn report(&self) {
        let (state, detail, queued) = match self.describe().await {
            Ok(s) => s,
            Err(e) => (State::Warn, format!("Last.fm state is unreadable: {e}"), 0),
        };
        self.ctx
            .status
            .set("lastfm", state, detail, json!({ "queued": queued }));
    }

    async fn describe(&self) -> Result<(State, String, u64)> {
        if self.client.is_none() {
            return Ok((State::Off, "Last.fm is not set up on the server".into(), 0));
        }
        let (acc, (queued, next), error) = self
            .ctx
            .db
            .call(|c| Ok((account(c)?, queue::summary(c)?, queue::last_error(c)?)))
            .await?;
        let Some(acc) = acc else {
            return Ok((State::Off, "Not connected".into(), queued));
        };
        let waiting = |s: &str| format!("{queued} scrobble{} waiting{s}", if queued == 1 { "" } else { "s" });
        Ok(if self.is_signed_out() {
            (
                State::Warn,
                format!("Last.fm signed lull out; connect again. {}", waiting("")),
                queued,
            )
        } else if queued > 0 && error.is_some() {
            let secs = next.map(|n| (n - lull_core::now()).max(0)).unwrap_or(0);
            (State::Warn, waiting(&format!(", retrying in {secs} s")), queued)
        } else if !acc.scrobbling {
            (
                State::Ok,
                format!("Connected as {}, scrobbling paused", acc.user),
                queued,
            )
        } else {
            (State::Ok, format!("Connected as {}", acc.user), queued)
        })
    }
}

fn account(conn: &Connection) -> Result<Option<Account>> {
    Ok(conn
        .query_row(
            "SELECT user, session_key, scrobbling, use_history FROM lastfm_session WHERE id = 1",
            [],
            |r| {
                Ok(Account {
                    user: r.get(0)?,
                    session_key: r.get(1)?,
                    scrobbling: r.get(2)?,
                    use_history: r.get(3)?,
                })
            },
        )
        .optional()?)
}
