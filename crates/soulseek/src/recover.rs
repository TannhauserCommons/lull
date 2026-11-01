//! Recovering Soulseek sessions (docs/context/soulseek.md#stale-sessions).
//! - **Stale:** the server can keep a session that reports "logged in" while none of its
//!   searches reach peers; a fresh login cures it. When several searches in a row get no answer
//!   at all, lull asks slskd to log in again, at most every `EVERY`, and never while a song is
//!   transferring (a new login drops peer connections).
//! - **Stuck:** slskd can stop halfway through a transition, logged out, with its own watchdog
//!   off (seen live 2026-11-01: "Disconnecting" for 40 minutes after an exception). After
//!   `STUCK_AFTER` lull forces fresh logins, backing off, then asks slskd to restart itself, and
//!   reports Soulseek down meanwhile.

use std::time::{Duration, Instant};

use lull_core::status::State;
use serde_json::{Value, json};

use lull_core::config::SlskdWebLogin;

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
/// Fresh logins that didn't help before lull escalates to restarting slskd.
const RESTART_AFTER: u32 = 2;
/// At most one restart this often: an slskd that wedges again soon needs a person.
const RESTART_EVERY: Duration = Duration::from_secs(30 * 60);

/// How long slskd has been stuck, and what lull did about it. The last restart outlives the
/// episode (slskd goes away while it restarts), so a repeat wedge isn't restarted in a loop.
#[derive(Debug, Default)]
pub struct Stuck {
    since: Option<(Instant, i64)>,
    tries: u32,
    relogins: u32,
    last_try: Option<Instant>,
    /// Why the last fresh login failed, if it did.
    login_error: Option<String>,
    restarted: Option<Restarted>,
}

#[derive(Debug, Clone)]
struct Restarted {
    at: Instant,
    unix: i64,
    error: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Connected, or plainly logged out (slskd's watchdog, or another client's login).
    Fine,
    /// Mid-transition, briefly: slskd usually finishes by itself.
    Settling,
    /// Stuck: act now (a fresh login, or a restart).
    Act,
    /// Stuck, and lull acted recently: wait.
    Waiting,
}

impl Stuck {
    pub fn observe(&mut self, server: &Server, now: Instant) -> Verdict {
        if server.is_logged_in || !server.is_transitioning {
            self.settle();
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
        Verdict::Act
    }

    /// The episode is over (or slskd went away to restart): only the last restart is kept.
    pub fn settle(&mut self) {
        *self = Stuck {
            restarted: self.restarted.take(),
            ..Stuck::default()
        };
    }

    /// A restart is the next step: fresh logins didn't help and none was restarted lately.
    fn restart_due(&self, now: Instant) -> bool {
        self.relogins >= RESTART_AFTER
            && self
                .restarted
                .as_ref()
                .is_none_or(|r| now.duration_since(r.at) >= RESTART_EVERY)
    }
}

fn retry_after(tries: u32) -> Duration {
    STUCK_RETRY
        .saturating_mul(2u32.saturating_pow(tries.saturating_sub(1)))
        .min(STUCK_RETRY_MAX)
}

/// One look at a slskd caught mid-transition: forces a fresh login, or once those didn't help
/// and lull has slskd's web login, asks slskd to restart itself; and says what the `soulseek`
/// status should read. `None` while it isn't stuck (yet).
pub async fn unstick(
    slskd: &Slskd,
    login: Option<&SlskdWebLogin>,
    server: &Server,
    stuck: &mut Stuck,
    now: Instant,
) -> Option<(State, String, Value)> {
    match stuck.observe(server, now) {
        Verdict::Fine | Verdict::Settling => return None,
        Verdict::Waiting => {}
        Verdict::Act => match login.filter(|_| stuck.restart_due(now)) {
            Some(login) => {
                tracing::warn!("slskd is still stuck after fresh logins; asking it to restart itself");
                let error = slskd.restart(login).await.err().map(|e| e.to_string());
                stuck.restarted = Some(Restarted {
                    at: now,
                    unix: lull_core::now(),
                    error,
                });
            }
            None => {
                stuck.relogins += 1;
                stuck.login_error = force_login(slskd).await.err().map(|e| e.to_string());
            }
        },
    }
    Some(report(server, stuck, login.is_some(), now))
}

/// The `soulseek` status for a stuck slskd: down, how long, what lull did, and whether a person
/// (or the deploy) has to restart slskd (`extra.restartSlskd`).
fn report(server: &Server, stuck: &Stuck, can_restart: bool, now: Instant) -> (State, String, Value) {
    let what = match server.state.trim() {
        "" => "between connecting and disconnecting".to_owned(),
        s => format!("at \"{s}\""),
    };
    let (since, since_unix) = stuck.since.unwrap_or((now, lull_core::now()));
    let minutes = now.duration_since(since).as_secs() / 60;
    let mut detail = format!(
        "slskd has been stuck {what}, logged out, since {} ({minutes} min); lull asked it to log in again {} time{}",
        clock(since_unix),
        stuck.relogins,
        if stuck.relogins == 1 { "" } else { "s" },
    );
    if let Some(e) = &stuck.login_error {
        detail.push_str(&format!(" (the last ask failed: {e})"));
    }
    let recent = stuck
        .restarted
        .as_ref()
        .filter(|r| now.duration_since(r.at) < RESTART_EVERY);
    let by_hand = match recent {
        Some(Restarted { error: Some(e), .. }) => {
            detail.push_str(&format!(", and restarting it failed ({e}): restart slskd"));
            true
        }
        Some(r) if r.at < since => {
            detail.push_str(&format!(
                ", and it wedged again after lull restarted it at {}: restart slskd",
                clock(r.unix)
            ));
            true
        }
        Some(r) => {
            detail.push_str(&format!(", then asked it to restart itself at {}", clock(r.unix)));
            false
        }
        None if stuck.relogins >= RESTART_AFTER && !can_restart => {
            detail.push_str(
                ", which didn't help: restart slskd (lull restarts it itself with LULL_SLSKD_WEB_USERNAME and LULL_SLSKD_WEB_PASSWORD)",
            );
            true
        }
        None => false,
    };
    let extra = json!({
        "stuck": server.state,
        "stuckSince": since_unix,
        "relogins": stuck.relogins,
        "restartedAt": stuck.restarted.as_ref().map(|r| r.unix),
        "restartSlskd": by_hand,
    });
    (State::Down, detail, extra)
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
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::{get, post, put};

    const MIN: Duration = Duration::from_secs(60);

    struct Fake {
        slskd: Slskd,
        deletes: Arc<AtomicU32>,
        puts: Arc<AtomicU32>,
        restarts: Arc<AtomicU32>,
    }

    impl Fake {
        fn calls(&self) -> (u32, u32, u32) {
            let n = |c: &Arc<AtomicU32>| c.load(Ordering::SeqCst);
            (n(&self.deletes), n(&self.puts), n(&self.restarts))
        }
    }

    /// slskd wedged as seen live: logged out, "Disconnecting" forever, refusing the disconnect.
    /// Its web login is lull/secret; a restart needs that session's token, as in slskd 0.26.
    async fn wedged_slskd() -> Result<Fake, Box<dyn std::error::Error>> {
        let count = || Arc::new(AtomicU32::new(0));
        let (deletes, puts, restarts) = (count(), count(), count());
        let (d, p, r) = (deletes.clone(), puts.clone(), restarts.clone());
        let app = Router::new()
            .route(
                "/api/v0/application",
                get(|| async {
                    axum::Json(json!({
                        "server": { "state": "Disconnecting", "isLoggedIn": false, "isTransitioning": true },
                        "user": { "username": "lull-test" }
                    }))
                })
                .put(move |headers: HeaderMap| {
                    let ok = headers.get("authorization").is_some_and(|v| v == "Bearer jwt");
                    if ok {
                        r.fetch_add(1, Ordering::SeqCst);
                    }
                    async move {
                        if ok {
                            StatusCode::NO_CONTENT
                        } else {
                            StatusCode::UNAUTHORIZED
                        }
                    }
                }),
            )
            .route(
                "/api/v0/session",
                post(|axum::Json(login): axum::Json<Value>| async move {
                    match login == json!({ "username": "lull", "password": "secret" }) {
                        true => Ok(axum::Json(json!({ "token": "jwt", "tokenType": "Bearer" }))),
                        false => Err(StatusCode::UNAUTHORIZED),
                    }
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
        Ok(Fake {
            slskd,
            deletes,
            puts,
            restarts,
        })
    }

    fn login(password: &str) -> SlskdWebLogin {
        SlskdWebLogin {
            username: "lull".into(),
            password: lull_core::config::Secret::new(password),
        }
    }

    fn connected() -> Server {
        Server {
            is_logged_in: true,
            is_transitioning: false,
            state: "Connected, LoggedIn".into(),
            username: None,
        }
    }

    #[tokio::test]
    async fn a_slskd_stuck_disconnecting_gets_fresh_logins_with_backoff_and_reads_down()
    -> Result<(), Box<dyn std::error::Error>> {
        let fake = wedged_slskd().await?;
        let server = fake.slskd.server().await?;
        assert_eq!(server.state, "Disconnecting");
        let mut stuck = Stuck::default();
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        let mut look = async |s: u64| unstick(&fake.slskd, None, &server, &mut stuck, at(s)).await;

        assert!(look(0).await.is_none(), "a transition takes a moment");
        assert!(look(59).await.is_none());
        assert_eq!(fake.calls(), (0, 0, 0));

        let (state, detail, extra) = look(61).await.ok_or("no verdict")?;
        assert_eq!(fake.calls(), (1, 1, 0), "disconnect refused, connect asked anyway");
        assert_eq!(state, State::Down, "stuck is down, not 'reconnecting'");
        assert!(detail.contains("\"Disconnecting\""), "{detail}");
        assert_eq!(extra["restartSlskd"], false);

        assert!(look(61 + 119).await.is_some());
        assert_eq!(fake.calls(), (1, 1, 0), "the second try waits 2 minutes");
        let (_, detail, extra) = look(61 + 120).await.ok_or("no verdict")?;
        assert_eq!(fake.calls(), (2, 2, 0));
        assert_eq!(extra["restartSlskd"], true, "no web login: a person restarts slskd");
        assert!(detail.contains("LULL_SLSKD_WEB_USERNAME"), "{detail}");
        look(61 + 120 + 240).await;
        assert_eq!(fake.calls(), (3, 3, 0), "then 4 minutes, still fresh logins");

        assert!(
            unstick(&fake.slskd, None, &connected(), &mut stuck, at(1000))
                .await
                .is_none()
        );
        assert_eq!(stuck.tries, 0, "logged in again: the count starts over");
        Ok(())
    }

    #[tokio::test]
    async fn after_two_fresh_logins_a_stuck_slskd_is_asked_to_restart_itself_once()
    -> Result<(), Box<dyn std::error::Error>> {
        let fake = wedged_slskd().await?;
        let server = fake.slskd.server().await?;
        let login = login("secret");
        let mut stuck = Stuck::default();
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);

        unstick(&fake.slskd, Some(&login), &server, &mut stuck, at(0)).await;
        unstick(&fake.slskd, Some(&login), &server, &mut stuck, at(61)).await;
        let (_, _, extra) = unstick(&fake.slskd, Some(&login), &server, &mut stuck, at(181))
            .await
            .ok_or("no verdict")?;
        assert_eq!(fake.calls(), (2, 2, 0));
        assert_eq!(extra["restartSlskd"], false, "lull can restart it itself");
        let (_, detail, extra) = unstick(&fake.slskd, Some(&login), &server, &mut stuck, at(421))
            .await
            .ok_or("no verdict")?;
        assert_eq!(
            fake.calls(),
            (2, 2, 1),
            "the third step is a restart, with the web session"
        );
        assert!(detail.contains("asked it to restart itself"), "{detail}");
        assert_eq!(extra["restartSlskd"], false);

        // slskd goes away while it restarts, then comes back wedged again within 30 minutes.
        stuck.settle();
        unstick(&fake.slskd, Some(&login), &server, &mut stuck, at(600)).await;
        unstick(&fake.slskd, Some(&login), &server, &mut stuck, at(661)).await;
        unstick(&fake.slskd, Some(&login), &server, &mut stuck, at(781)).await;
        let (_, detail, extra) = unstick(&fake.slskd, Some(&login), &server, &mut stuck, at(1021))
            .await
            .ok_or("no verdict")?;
        assert_eq!(
            fake.calls(),
            (5, 5, 1),
            "no second restart this soon: fresh logins only"
        );
        assert_eq!(extra["restartSlskd"], true, "{detail}");
        assert!(detail.contains("wedged again"), "{detail}");
        Ok(())
    }

    #[tokio::test]
    async fn a_refused_web_login_says_to_restart_slskd_by_hand() -> Result<(), Box<dyn std::error::Error>> {
        let fake = wedged_slskd().await?;
        let server = fake.slskd.server().await?;
        let wrong = login("wrong");
        let mut stuck = Stuck::default();
        let t0 = Instant::now();
        for s in [0, 61, 181, 421] {
            unstick(
                &fake.slskd,
                Some(&wrong),
                &server,
                &mut stuck,
                t0 + Duration::from_secs(s),
            )
            .await;
        }
        let (_, detail, extra) = unstick(
            &fake.slskd,
            Some(&wrong),
            &server,
            &mut stuck,
            t0 + Duration::from_secs(425),
        )
        .await
        .ok_or("no verdict")?;
        assert_eq!(fake.calls().2, 0);
        assert_eq!(extra["restartSlskd"], true);
        assert!(detail.contains("restarting it failed"), "{detail}");
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
