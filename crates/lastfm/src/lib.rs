//! Last.fm: web auth, now playing and scrobbles with an offline-safe retry queue, driven by
//! `play` events (docs/context/scrobbling.md). Also shares the listener's Last.fm history with
//! discovery as a `lastfm.history` event, so the crates stay apart.

mod api;
mod queue;
mod routes;
mod rules;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use axum::Router;
use lull_core::db::Migrations;
use lull_core::events::Event;
use lull_core::model::LatePlay;
use lull_core::seams::Scrobbles;
use lull_core::status::State;
use lull_core::{Ctx, Error, Result};
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
/// Last.fm takes no scrobble timestamped more than this long ago.
const MAX_AGE_S: i64 = 14 * 24 * 3600;
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

/// The name the session key is kept under in the credentials' vault (ADR-0091).
const SESSION: &str = "lastfm.session";

/// The stored connection: who, and the listener's two switches.
#[derive(Debug, Clone)]
struct Account {
    user: String,
    /// `None` while the vault holding it can't be read (a locked keyring): nothing is sent, and
    /// scrobbles wait in the queue.
    session_key: Option<String>,
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
    move_session(ctx).await;
    svc.report().await;
    if svc.client.is_some() {
        ctx.spawn(listen(svc.clone(), ctx.bus.subscribe()));
        ctx.spawn(scrobbler(svc.clone()));
        ctx.spawn(history(svc.clone()));
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
    /// `plays.id` as a string.
    play_id: String,
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
            Ok(p) => match p.play_id.parse() {
                Ok(id) => svc.ended(id, p.play(), p.played_s).await,
                Err(_) => Err(lull_core::Error::internal(format!(
                    "play.ended names play {:?}",
                    p.play_id
                ))),
            },
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

/// History's "Send to Last.fm" (api.md): plays that ended while nothing was listening, queued
/// like any scrobble, each at most once (the queue's key is the play).
#[async_trait]
impl Scrobbles for Service {
    fn refusal(&self, play: &LatePlay, now: i64) -> Option<String> {
        if now - play.started_at > MAX_AGE_S {
            return Some("it is older than 14 days, which Last.fm no longer accepts.".into());
        }
        (!rules::scrobbles(play.duration_s, play.played_s)).then(|| {
            if play.duration_s != 0 && play.duration_s <= rules::MIN_LENGTH_S {
                "Last.fm doesn't count tracks of 30 seconds or less.".into()
            } else {
                "it wasn't played for half its length or 4 minutes, so Last.fm wouldn't count it.".into()
            }
        })
    }

    async fn submit(&self, plays: Vec<LatePlay>) -> Result<()> {
        self.client()?;
        if !self.account().await?.is_some_and(|a| a.scrobbling) {
            return Err(Error::Conflict(
                "Connect Last.fm and switch on \"Scrobble what I play\" in Settings first.".into(),
            ));
        }
        let now = lull_core::now();
        if let Some((play, why)) = plays.iter().find_map(|p| self.refusal(p, now).map(|why| (p, why))) {
            return Err(Error::BadRequest(format!("\u{201c}{}\u{201d}: {why}", play.title)));
        }
        let ids: Vec<i64> = plays.iter().map(|p| p.play_id).collect();
        self.ctx
            .db
            .call(move |conn| {
                // One transaction: a play already queued refuses the lot, and nothing is kept.
                let tx = conn.transaction()?;
                for p in &plays {
                    if !queue::push(&tx, p.play_id, &Play::from(p), now)? {
                        return Err(Error::Conflict(format!(
                            "\u{201c}{}\u{201d} is already waiting to be sent.",
                            p.title
                        )));
                    }
                }
                tx.commit()?;
                Ok(())
            })
            .await?;
        self.wake.notify_one();
        for id in ids {
            self.publish_scrobble(id, "pending");
        }
        Ok(())
    }
}

impl Service {
    fn client(&self) -> Result<&Client> {
        self.client.as_ref().ok_or_else(|| {
            lull_core::Error::unavailable(api::SERVICE, "set LULL_LASTFM_API_KEY and LULL_LASTFM_API_SECRET")
        })
    }

    async fn account(&self) -> Result<Option<Account>> {
        let db = &self.ctx.db;
        let acc = db.call(|c| account(c)).await?;
        Ok(acc.map(|acc| Account {
            session_key: acc
                .session_key
                .and_then(|k| lull_core::secrets::resolve(db, SESSION, k)),
            ..acc
        }))
    }

    async fn now_playing(&self, play: &Play) -> Result<()> {
        let Some(key) = self
            .account()
            .await?
            .filter(|a| a.scrobbling)
            .and_then(|a| a.session_key)
        else {
            return Ok(());
        };
        self.client()?
            .now_playing(&key, play)
            .await
            .or_else(|f| self.on_failure(f))
    }

    /// Queues an eligible play and reports the verdict; with scrobbling off, says nothing.
    async fn ended(&self, play_id: i64, play: Play, played_s: u32) -> Result<()> {
        if !self.account().await?.is_some_and(|a| a.scrobbling) {
            return Ok(());
        }
        let status = if rules::scrobbles(play.duration_s, played_s) {
            self.ctx
                .db
                .call(move |c| queue::push(c, play_id, &play, lull_core::now()).map(drop))
                .await?;
            self.wake.notify_one();
            "pending"
        } else {
            "short"
        };
        self.publish_scrobble(play_id, status);
        Ok(())
    }

    /// `scrobble { playId, status }` for History (collection keeps the latest per play).
    fn publish_scrobble(&self, play_id: i64, status: &str) {
        self.ctx
            .bus
            .publish("scrobble", json!({ "playId": play_id.to_string(), "status": status }));
    }

    /// Sends every due batch until the queue is empty, waiting or Last.fm fails.
    async fn flush(&self) -> Result<()> {
        let client = self.client()?;
        let Some(key) = self.account().await?.and_then(|a| a.session_key) else {
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
            match client.scrobble(&key, &plays).await {
                Ok(()) => {
                    self.ctx.db.call(move |c| queue::remove(c, &ids)).await?;
                    for q in &batch {
                        self.publish_scrobble(q.play_id, "done");
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
                    session_key: Some(r.get(1)?),
                    scrobbling: r.get(2)?,
                    use_history: r.get(3)?,
                })
            },
        )
        .optional()?)
}

/// Takes the session key out of a copy of the database (a backup taken before an update, once the update
/// has proved itself), leaving a placeholder. The caller has `secure_delete` on. True when it changed.
pub fn scrub(tx: &rusqlite::Transaction<'_>) -> Result<bool> {
    let has_table = tx
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'lastfm_session'",
            [],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    let column: Option<String> = if has_table {
        tx.query_row("SELECT session_key FROM lastfm_session WHERE id = 1", [], |r| r.get(0))
            .optional()?
    } else {
        None
    };
    match column {
        Some(key) if !lull_core::secrets::is_placeholder(&key) => {
            tx.execute(
                "UPDATE lastfm_session SET session_key = ?1 WHERE id = 1",
                [lull_core::secrets::placeholder()?],
            )?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// With the `secrets` switch on, a session key still in `lastfm_session` (from before the switch, or
/// stored by an older lull) moves to the vault, and a placeholder overwrites it on disk once the vault
/// has read it back. One that can't move stays, and is tried again at the next start.
async fn move_session(ctx: &Ctx) {
    let db = &ctx.db;
    let column = match db.call(|c| account(c)).await {
        Ok(Some(Account {
            session_key: Some(k), ..
        })) => k,
        Ok(_) => return,
        Err(e) => return tracing::warn!("reading the Last.fm session to move it: {e}"),
    };
    let moved = match lull_core::secrets::adopt(db, SESSION, &column).await {
        Ok(Some(placeholder)) => {
            lull_core::secrets::erase(db, move |tx| {
                tx.execute("UPDATE lastfm_session SET session_key = ?1 WHERE id = 1", [placeholder])?;
                Ok(())
            })
            .await
        }
        Ok(None) => return,
        Err(e) => Err(e),
    };
    if let Err(e) = moved {
        tracing::warn!("the Last.fm session stays in the database for now: {e}");
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use lull_core::config::Config;
    use lull_core::db::Db;
    use lull_core::events::Bus;
    use lull_core::http::Http;
    use rusqlite::params;

    use super::*;

    fn service() -> Result<Arc<Service>> {
        build(false)
    }

    fn build(with_key: bool) -> Result<Arc<Service>> {
        let vars: BTreeMap<String, String> = [("LULL_SLSKD_URL", "http://slskd"), ("LULL_SLSKD_API_KEY", "k")]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect();
        let config = Config::from_vars(vars).map_err(|e| lull_core::Error::internal(e.to_string()))?;
        let db = Db::memory()?;
        db.migrate(&MIGRATIONS)?;
        let bus = Bus::new();
        let http = Http::new()?;
        let creds = lull_core::config::LastFm {
            api_key: lull_core::config::Secret::new("key"),
            api_secret: lull_core::config::Secret::new("secret"),
        };
        Ok(Arc::new(Service {
            ctx: Ctx::new(config, db, http.clone(), bus),
            client: with_key.then(|| Client::new(http, creds)),
            wake: Notify::new(),
            pending_auth: Mutex::new(None),
            signed_out: Mutex::new(false),
            auth_error: Mutex::new(None),
        }))
    }

    async fn connect(svc: &Service, scrobbling: bool) -> Result<()> {
        svc.ctx
            .db
            .call(move |c| {
                c.execute(
                    "INSERT OR REPLACE INTO lastfm_session (id, user, session_key, scrobbling, connected_at)
                     VALUES (1, 'listener', 'sk', ?1, 0)",
                    params![scrobbling],
                )?;
                Ok(())
            })
            .await
    }

    /// Ends play `id` from the `play.ended` payload collection publishes (api.md#events).
    async fn end(svc: &Service, id: i64, played_s: u32) -> Result<()> {
        let p: PlayEvent = serde_json::from_value(json!({
            "playId": id.to_string(), "trackId": "deezer:1", "at": 1_000, "playedS": played_s,
            "completed": played_s == 254, "skipped": false, "duration": 254,
            "artist": "Piernikowski", "title": "Daj Mi Jeszcze Jedna Szanse", "album": "",
        }))
        .map_err(|e| lull_core::Error::internal(e.to_string()))?;
        svc.ended(id, p.play(), p.played_s).await
    }

    fn verdicts(rx: &mut broadcast::Receiver<Event>) -> Vec<String> {
        std::iter::from_fn(|| rx.try_recv().ok())
            .filter(|e| e.topic == "scrobble")
            .map(|e| {
                format!(
                    "{} {}",
                    e.data["playId"].as_str().unwrap_or("?"),
                    e.data["status"].as_str().unwrap_or("?")
                )
            })
            .collect()
    }

    /// History's `scrobble: null` on a full play means Last.fm wasn't listening when it ended:
    /// such a play is dropped, not kept for a later connect.
    #[tokio::test]
    async fn only_plays_ended_while_connected_and_scrobbling_get_a_verdict() -> Result<()> {
        let svc = service()?;
        let mut rx = svc.ctx.bus.subscribe();

        end(&svc, 1, 254).await?; // not connected yet
        connect(&svc, false).await?;
        end(&svc, 2, 254).await?; // scrobbling switched off
        assert!(verdicts(&mut rx).is_empty());

        connect(&svc, true).await?;
        end(&svc, 3, 254).await?;
        end(&svc, 4, 20).await?;
        assert_eq!(verdicts(&mut rx), ["3 pending", "4 short"]);
        let queued = svc.ctx.db.call(|c| queue::summary(c)).await?.0;
        assert_eq!(queued, 1, "only play 3 waits to be sent; plays 1 and 2 are gone");
        Ok(())
    }
    fn late(play_id: i64, ago: i64, played_s: u32) -> LatePlay {
        LatePlay {
            play_id,
            artist: "Grouper".into(),
            title: "Kelso".into(),
            album: "Shade".into(),
            duration_s: 200,
            played_s,
            started_at: lull_core::now() - ago,
        }
    }

    #[tokio::test]
    async fn a_late_play_must_pass_last_fms_rule_and_age() -> Result<()> {
        let svc = service()?;
        let now = lull_core::now();
        let why = |p: LatePlay| svc.refusal(&p, now);
        assert_eq!(why(late(1, 3600, 100)), None, "half of 200 s");
        assert!(why(late(2, 3600, 99)).is_some_and(|w| w.contains("half")));
        assert!(why(late(3, 14 * 86_400 + 5, 200)).is_some_and(|w| w.contains("14 days")));
        assert_eq!(why(late(4, 14 * 86_400 - 60, 200)), None, "just inside the limit");
        let short = LatePlay {
            duration_s: 30,
            played_s: 30,
            ..late(5, 60, 30)
        };
        assert!(why(short).is_some_and(|w| w.contains("30 seconds")));
        Ok(())
    }

    #[tokio::test]
    async fn a_late_play_is_queued_once_and_all_or_nothing() -> Result<()> {
        let svc = build(true)?;
        let mut rx = svc.ctx.bus.subscribe();
        let waiting = |svc: &Arc<Service>| {
            let svc = svc.clone();
            async move { svc.ctx.db.call(|c| queue::summary(c)).await.map(|s| s.0) }
        };

        let early = svc.submit(vec![late(1, 3600, 150)]).await;
        assert!(matches!(early, Err(Error::Conflict(_))), "not connected: {early:?}");
        connect(&svc, false).await?;
        let paused = svc.submit(vec![late(1, 3600, 150)]).await;
        assert!(matches!(paused, Err(Error::Conflict(_))), "scrobbling off: {paused:?}");
        assert_eq!(waiting(&svc).await?, 0);

        connect(&svc, true).await?;
        svc.submit(vec![late(1, 3600, 150)]).await?;
        assert_eq!(verdicts(&mut rx), ["1 pending"]);
        let stored = svc.ctx.db.call(|c| queue::due(c, i64::MAX, 10)).await?;
        assert_eq!(
            stored[0].play.started_at,
            late(1, 3600, 150).started_at,
            "its own start"
        );

        // Once: the same play again, or a play the live path already queued, is refused and
        // takes nothing else with it.
        end(&svc, 2, 254).await?;
        assert_eq!(verdicts(&mut rx), ["2 pending"]);
        for repeated in [late(1, 3600, 150), late(2, 3600, 200)] {
            let again = svc.submit(vec![late(3, 3600, 150), repeated]).await;
            assert!(matches!(again, Err(Error::Conflict(_))), "{again:?}");
        }
        let old = svc.submit(vec![late(4, 15 * 86_400, 200)]).await;
        assert!(matches!(old, Err(Error::BadRequest(_))), "{old:?}");
        assert_eq!(waiting(&svc).await?, 2, "only plays 1 and 2 wait");
        assert!(verdicts(&mut rx).is_empty());
        Ok(())
    }
}
