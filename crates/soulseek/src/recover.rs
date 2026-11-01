//! Recovering Soulseek sessions (docs/context/soulseek.md#stale-sessions).
//! - **Stale:** the server can keep a session that reports "logged in" while none of its
//!   searches reach peers; a fresh login cures it. When several searches in a row get no answer
//!   at all, lull asks slskd to log in again, at most every `EVERY`, and never while a song is
//!   transferring (a new login drops peer connections).
//! - **Stuck:** slskd can stop halfway through a transition, logged out, with its own watchdog
//!   off (seen live 2026-11-01: "Disconnecting" for 40 minutes after an exception). After
//!   `STUCK_AFTER` lull forces a fresh login, backing off, and reports Soulseek down.

use std::time::{Duration, Instant};

use lull_core::status::State;
use serde_json::{Value, json};

use crate::slskd::{Server, Slskd};

/// Searches in a row that no peer answered before lull acts. One silent search can be a rare
/// query; three popular-enough ones in a row are the session.
pub const UNANSWERED: usize = 3;
/// At most one fresh login this often, so a quiet network never turns into a reconnect loop.
pub const EVERY: Duration = Duration::from_secs(15 * 60);

/// Whether to log in again now: the session looks stale, the last attempt is long enough ago,
/// and nothing would be cut off.
pub fn reconnect_due(unanswered: usize, since_last: Option<Duration>, transferring: bool) -> bool {
    unanswered >= UNANSWERED && since_last.is_none_or(|d| d >= EVERY) && !transferring
}

/// Logged out and mid-transition for this long is stuck, not reconnecting.
pub const STUCK_AFTER: Duration = Duration::from_secs(60);
/// Fresh logins for a stuck slskd: the second this long after the first, then doubling...
const STUCK_RETRY: Duration = Duration::from_secs(2 * 60);
/// ...up to this.
const STUCK_RETRY_MAX: Duration = Duration::from_secs(15 * 60);
/// After this many fresh logins that didn't help, the status says to restart slskd.
const RESTART_AFTER: u32 = 2;

/// How long slskd has been stuck, and the fresh logins lull asked for meanwhile.
#[derive(Debug, Default)]
pub struct Stuck {
    since: Option<(Instant, i64)>,
    tries: u32,
    last_try: Option<Instant>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Connected, or plainly logged out (slskd's watchdog, or another client's login).
    Fine,
    /// Mid-transition, briefly: slskd usually finishes by itself.
    Settling,
    /// Stuck: force a fresh login now.
    Relogin,
    /// Stuck, and the last fresh login was recent: wait for the next.
    Waiting,
}

impl Stuck {
    pub fn observe(&mut self, server: &Server, now: Instant) -> Verdict {
        if server.is_logged_in || !server.is_transitioning {
            *self = Stuck::default();
            return Verdict::Fine;
        }
        let (since, _) = *self.since.get_or_insert((now, lull_core::now()));
        if now.duration_since(since) < STUCK_AFTER {
            return Verdict::Settling;
        }
        if let Some(last) = self.last_try
            && now.duration_since(last) < retry_after(self.tries)
        {
            return Verdict::Waiting;
        }
        self.tries += 1;
        self.last_try = Some(now);
        Verdict::Relogin
    }
}

fn retry_after(tries: u32) -> Duration {
    STUCK_RETRY
        .saturating_mul(2u32.saturating_pow(tries.saturating_sub(1)))
        .min(STUCK_RETRY_MAX)
}

/// One look at a slskd caught mid-transition: forces a fresh login when one is due and says
/// what the `soulseek` status should read. `None` while it isn't stuck (yet).
pub async fn unstick(
    slskd: &Slskd,
    server: &Server,
    stuck: &mut Stuck,
    now: Instant,
) -> Option<(State, String, Value)> {
    let tried = match stuck.observe(server, now) {
        Verdict::Fine | Verdict::Settling => return None,
        Verdict::Waiting => None,
        Verdict::Relogin => Some(force_login(slskd).await),
    };
    let what = match server.state.trim() {
        "" => "between connecting and disconnecting".to_owned(),
        s => format!("at \"{s}\""),
    };
    let (_, since) = stuck.since?;
    let minutes = (lull_core::now() - since).max(0) / 60;
    let restart = stuck.tries >= RESTART_AFTER;
    let mut detail = format!(
        "slskd has been stuck {what}, logged out, since {} ({minutes} min); lull asked it to log in again {} time{}",
        clock(since),
        stuck.tries,
        if stuck.tries == 1 { "" } else { "s" },
    );
    if let Some(Err(e)) = &tried {
        detail.push_str(&format!(" (the last ask failed: {e})"));
    }
    if restart {
        detail.push_str(", which didn't help: restart slskd");
    }
    let extra = json!({ "stuck": server.state, "stuckSince": since, "relogins": stuck.tries, "restartSlskd": restart });
    Some((State::Down, detail, extra))
}

/// A wedged slskd may refuse the disconnect; the connect is what matters, so it runs anyway.
async fn force_login(slskd: &Slskd) -> lull_core::Result<()> {
    tracing::warn!("slskd is stuck mid-transition; asking it to log in again");
    if let Err(e) = slskd
        .disconnect("lull: slskd was stuck mid-transition; logging in again")
        .await
    {
        tracing::info!(error = %e, "slskd refused the disconnect; connecting anyway");
    }
    slskd.connect().await
}

/// "04:05 UTC" for a unix time: when the last fresh login happened, in the status line.
pub fn clock(unix: i64) -> String {
    let day = unix.rem_euclid(86_400);
    format!("{:02}:{:02} UTC", day / 3600, day % 3600 / 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    use axum::Router;
    use axum::http::StatusCode;
    use axum::routing::{get, put};

    const MIN: Duration = Duration::from_secs(60);

    /// slskd wedged as seen live: logged out, "Disconnecting" forever, refusing the disconnect.
    async fn wedged_slskd() -> Result<(Slskd, Arc<AtomicU32>, Arc<AtomicU32>), Box<dyn std::error::Error>> {
        let (deletes, puts) = (Arc::new(AtomicU32::new(0)), Arc::new(AtomicU32::new(0)));
        let (d, p) = (deletes.clone(), puts.clone());
        let app = Router::new()
            .route(
                "/api/v0/application",
                get(|| async {
                    axum::Json(json!({
                        "server": { "state": "Disconnecting", "isLoggedIn": false, "isTransitioning": true },
                        "user": { "username": "lull-test" }
                    }))
                }),
            )
            .route(
                "/api/v0/server",
                put(move || {
                    p.fetch_add(1, Ordering::SeqCst);
                    async { StatusCode::OK }
                })
                .delete(move || {
                    d.fetch_add(1, Ordering::SeqCst);
                    async { StatusCode::CONFLICT }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}", listener.local_addr()?);
        tokio::spawn(async move { axum::serve(listener, app).await });
        let slskd = Slskd::new(
            lull_core::http::Http::new()?,
            &url,
            lull_core::config::Secret::new("key"),
        )?;
        Ok((slskd, deletes, puts))
    }

    #[tokio::test]
    async fn a_slskd_stuck_disconnecting_gets_fresh_logins_with_backoff_and_reads_down()
    -> Result<(), Box<dyn std::error::Error>> {
        let (slskd, deletes, puts) = wedged_slskd().await?;
        let server = slskd.server().await?;
        assert_eq!(server.state, "Disconnecting");
        let calls = || (deletes.load(Ordering::SeqCst), puts.load(Ordering::SeqCst));
        let mut stuck = Stuck::default();
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);

        assert!(
            unstick(&slskd, &server, &mut stuck, at(0)).await.is_none(),
            "a transition takes a moment"
        );
        assert!(unstick(&slskd, &server, &mut stuck, at(59)).await.is_none());
        assert_eq!(calls(), (0, 0));

        let (state, detail, extra) = unstick(&slskd, &server, &mut stuck, at(61)).await.ok_or("no verdict")?;
        assert_eq!(calls(), (1, 1), "disconnect refused, connect asked anyway");
        assert_eq!(state, State::Down, "stuck is down, not 'reconnecting'");
        assert!(detail.contains("\"Disconnecting\""), "{detail}");
        assert_eq!(extra["restartSlskd"], false);

        assert!(unstick(&slskd, &server, &mut stuck, at(61 + 119)).await.is_some());
        assert_eq!(calls(), (1, 1), "the second try waits 2 minutes");
        unstick(&slskd, &server, &mut stuck, at(61 + 120)).await;
        assert_eq!(calls(), (2, 2));
        unstick(&slskd, &server, &mut stuck, at(61 + 120 + 239)).await;
        assert_eq!(calls(), (2, 2), "then 4 minutes");
        let (_, detail, extra) = unstick(&slskd, &server, &mut stuck, at(61 + 120 + 240))
            .await
            .ok_or("no verdict")?;
        assert_eq!(calls(), (3, 3));
        assert_eq!(extra["restartSlskd"], true);
        assert!(detail.ends_with("restart slskd"), "{detail}");

        let back = Server {
            is_logged_in: true,
            is_transitioning: false,
            state: "Connected, LoggedIn".into(),
            username: None,
        };
        assert!(unstick(&slskd, &back, &mut stuck, at(1000)).await.is_none());
        assert_eq!(stuck.tries, 0, "logged in again: the count starts over");
        Ok(())
    }

    #[test]
    fn plainly_logged_out_is_left_to_slskd() {
        let out = Server {
            is_logged_in: false,
            is_transitioning: false,
            state: "Disconnected".into(),
            username: None,
        };
        let mut stuck = Stuck::default();
        let t0 = Instant::now();
        assert_eq!(stuck.observe(&out, t0), Verdict::Fine);
        assert_eq!(stuck.observe(&out, t0 + 10 * STUCK_AFTER), Verdict::Fine);
        assert_eq!(retry_after(1), STUCK_RETRY);
        assert_eq!(retry_after(30), STUCK_RETRY_MAX);
    }

    #[test]
    fn logs_in_again_only_after_repeated_silence_and_never_in_a_loop_or_mid_song() {
        // A rare query or two that nobody answered: leave the session alone.
        assert!(!reconnect_due(UNANSWERED - 1, None, false));
        // Silence that keeps happening: act, the first time and again after the pause.
        assert!(reconnect_due(UNANSWERED, None, false));
        assert!(reconnect_due(UNANSWERED + 4, Some(EVERY), false));
        // Too soon after the last fresh login: wait, however many searches went unanswered.
        assert!(!reconnect_due(UNANSWERED + 10, Some(EVERY - MIN), false));
        // A song is transferring: a new login would cut it off.
        assert!(!reconnect_due(UNANSWERED, None, true));
        assert_eq!(clock(1_793_505_900), "04:05 UTC");
    }
}
