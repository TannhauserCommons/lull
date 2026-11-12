//! Soulseek through slskd: search (merged, ranked), transfers with peer fallback, progressive
//! streaming from growing files and the size-capped stream cache. This crate is the only code
//! that talks to slskd (docs/context/soulseek.md).

mod account;
mod albums;
mod api;
mod budget;
mod cache;
#[cfg(test)]
mod fake_slskd;
mod fetch;
mod files;
mod layout;
mod matching;
mod peers;
mod policy;
mod prefetch;
mod prewarm;
mod range;
mod ranges;
mod rank;
mod recover;
mod rules;
mod search;
mod share;
mod slskd;
mod songs;
mod sources;
mod stream;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use axum::Router;
use lull_core::db::Migrations;
use lull_core::ids::TrackId;
use lull_core::model::{SourceRef, Track};
use lull_core::quality::{Pick, Quality, SaveCandidate};
use lull_core::seams::{Library, Transfers, Youtube};
use lull_core::status::State;
use lull_core::youtube::{self, Mode, Want as YoutubeWant};
use lull_core::{Ctx, Error, Result, catalog};
use serde_json::Value;

use crate::cache::{Cache, Pin};
use crate::fetch::Fetcher;
use crate::layout::Layout;
use crate::peers::Peers;
use crate::rules::{Current, Purpose};
use crate::search::Searches;
use crate::slskd::Slskd;
use crate::sources::{Ranked, Sources};

pub const MIGRATIONS: Migrations = Migrations {
    owner: "soulseek",
    steps: &[
        r#"
        -- Every file lull asked slskd for: the stream cache's index. Only these are ever evicted.
        CREATE TABLE soulseek_files (
            track_id    TEXT NOT NULL,
            user        TEXT NOT NULL,
            remote_path TEXT NOT NULL,
            size        INTEGER NOT NULL,
            local_path  TEXT NOT NULL,
            complete    INTEGER NOT NULL DEFAULT 0,
            used_at     INTEGER NOT NULL,          -- unix seconds, for least-recently-used eviction
            PRIMARY KEY (user, remote_path)
        );
        CREATE INDEX soulseek_files_track ON soulseek_files(track_id);
        -- Peers that stalled on us recently; ranking demotes them.
        CREATE TABLE soulseek_peers (
            user        TEXT PRIMARY KEY,
            stalls      INTEGER NOT NULL,
            stalled_at  INTEGER NOT NULL
        );
    "#,
        r#"
        -- Whether peers share an album (Transfers::album_available), by loose artist and title:
        -- found answers are used for 7 days, not found for 1.
        CREATE TABLE soulseek_albums (
            artist      TEXT NOT NULL,
            album       TEXT NOT NULL,
            available   INTEGER NOT NULL,
            checked_at  INTEGER NOT NULL,          -- unix seconds
            PRIMARY KEY (artist, album)
        );
    "#,
        r#"
        -- How fast each peer actually sent to this hub (ADR-0015): a running average of its
        -- transfers' rates in bytes a second, each weighted down by half every 7 days, and what
        -- the peer said its speed was when last measured (0: unknown), to calibrate peers not
        -- measured yet.
        CREATE TABLE soulseek_speeds (
            user        TEXT PRIMARY KEY,
            rate        REAL NOT NULL,
            weight      REAL NOT NULL,
            reported    REAL NOT NULL,
            measured_at INTEGER NOT NULL    -- unix seconds
        );
    "#,
        r#"
        -- The bytes of a copy that have arrived, as JSON [[start, end], ...] (ADR-0014): a copy
        -- arrives in legs that start where the listener seeks. NULL: a file written front to
        -- back, whose length is what arrived.
        ALTER TABLE soulseek_files ADD COLUMN ranges TEXT;
    "#,
        r#"
        -- Whether peers share a song (Transfers::song_available, songs.rs), for shelves'
        -- availability marks: found answers are used for 7 days, not found for 1.
        CREATE TABLE soulseek_songs (
            track_id    TEXT PRIMARY KEY,
            available   INTEGER NOT NULL,
            checked_at  INTEGER NOT NULL          -- unix seconds
        );
    "#,
    ],
};

/// How often the connection is checked while it's healthy, and while it isn't.
const CHECK_OK: Duration = Duration::from_secs(15);
const CHECK_DOWN: Duration = Duration::from_secs(5);
/// How often the cache is measured and idle finished songs are let go.
const HOUSEKEEPING: Duration = Duration::from_secs(60);

pub struct Service {
    ctx: Ctx,
    slskd: Slskd,
    online: AtomicBool,
    searches: Searches,
    sources: Sources,
    fetcher: Fetcher,
    cache: Cache,
    rules: Current,
    /// The last fresh login lull asked for because the server was silent: when (to space them out)
    /// and its unix time.
    relogged: Mutex<Option<(std::time::Instant, i64)>>,
    /// Songs being saved: pinned in the cache until their save ends.
    saving: Mutex<HashMap<TrackId, Pin>>,
    /// The songs devices asked to preload (`PUT /api/stream/prefetch`).
    prefetch: prefetch::Lists,
    /// Album availability is checked one album at a time.
    album_checks: tokio::sync::Mutex<()>,
    /// One song checked at a time (`songs::available`).
    song_checks: tokio::sync::Mutex<()>,
}

/// `youtube` is the second source: fetches turn to it by the listener's mode (docs/context/youtube.md).
pub async fn start(ctx: &Ctx, youtube: Arc<dyn Youtube>) -> Result<Arc<Service>> {
    let config = &ctx.config.soulseek;
    let slskd = Slskd::new(ctx.http.clone(), &config.slskd_url, config.slskd_api_key.clone())?;
    // With the `secrets` switch on, slskd gets the account in its environment when the hub starts it
    // (ADR-0091), and the file lull owns is kept empty: slskd reads it over its environment.
    let in_env = ctx.config.flags.on(&lull_core::flags::SECRETS);
    let in_file = config.account.as_ref().filter(|_| !in_env);
    if let Some(file) = &config.config_file
        && account::write(file, in_file).await?
        && !in_env
    {
        ctx.spawn(account::apply(slskd.clone(), config.account.clone()));
    }
    // Subscribed before collection starts, so its start-up `settings` arrives (ADR-0050).
    let rules = Current::default();
    ctx.spawn(rules.follow(&ctx.bus));
    let peers = Peers::load(ctx.db.clone()).await?;
    let sources = Sources::new(peers.clone(), config.preferred_formats.clone());
    let cache = Cache::new(
        ctx.db.clone(),
        ctx.status.clone(),
        config.transfers_dir.clone(),
        ctx.config.cache.max_bytes,
    );
    let searches = Searches::new(slskd.clone(), ctx.bus.clone(), ctx.db.clone(), sources.clone());
    let layout = Arc::new(RwLock::new(Layout::new(&config.transfers_dir, None)));
    let fetcher = Fetcher::new(
        slskd.clone(),
        layout.clone(),
        cache.clone(),
        peers,
        sources.clone(),
        searches.clone(),
        ctx.bus.clone(),
        rules.clone(),
        youtube,
    );
    let svc = Arc::new(Service {
        ctx: ctx.clone(),
        slskd: slskd.clone(),
        online: AtomicBool::new(false),
        searches,
        sources,
        fetcher,
        cache,
        rules,
        relogged: Mutex::default(),
        saving: Mutex::default(),
        prefetch: prefetch::Lists::default(),
        album_checks: tokio::sync::Mutex::new(()),
        song_checks: tokio::sync::Mutex::new(()),
    });
    ctx.status
        .set("soulseek", State::Down, "Connecting to slskd", Value::Null);
    svc.cache.report().await?;
    // These loops end when the hub stops this set of crates; searches and fetches finish on
    // their own, so a stream playing across a restart isn't cut.
    ctx.spawn(watch_connection(svc.clone(), layout));
    ctx.spawn(housekeeping(svc.clone()));
    ctx.spawn(release_saves(svc.clone()));
    ctx.spawn(prewarm::follow_catalog(svc.clone()));
    ctx.spawn(prefetch::run(svc.clone()));
    if config.share == lull_core::config::Share::Library {
        ctx.spawn(share::rescan_after_saves(slskd, ctx.bus.clone()));
    }
    Ok(svc)
}

/// `/api/soulseek/search`, `/api/tracks/{id}/sources`, `/api/stream/{id}`, `/api/transfers/...`.
pub fn router(svc: Arc<Service>, library: Arc<dyn Library>) -> Router {
    svc.searches.set_library(library);
    api::router(svc)
}

impl Service {
    fn online(&self) -> bool {
        self.online.load(Ordering::Relaxed)
    }

    /// No Soulseek peer shares the song: a YouTube copy (AAC, which the save pipeline can tag),
    /// when the listener lets saves use YouTube. It counts as meeting the saving minimum, which
    /// is about Soulseek copies; auto-upgrade still replaces it once a peer has a good copy.
    async fn youtube_save(&self, track: &Track) -> Result<Option<SaveCandidate>> {
        let settings = self.rules.get().youtube;
        if !settings.saves || settings.mode == Mode::Off {
            return Ok(None);
        }
        let found = match self.fetcher.youtube.find(track, YoutubeWant::Save).await {
            Ok(Some(found)) => found,
            Ok(None) => return Ok(None),
            Err(e) => {
                tracing::info!(error = %e, track = %track.id, "no YouTube copy to save");
                return Ok(None);
            }
        };
        let kbps = youtube::Copy::of(&found.source).map_or(0, |c| c.kbps(found.source.size, track.duration));
        Ok(Some(SaveCandidate {
            source: found.source,
            quality: Quality { lossless: false, kbps },
            meets: true,
        }))
    }

    async fn track(&self, id: &TrackId) -> Result<Track> {
        let lookup = id.clone();
        self.ctx
            .db
            .call(move |c| catalog::track(c, &lookup))
            .await?
            .ok_or_else(|| Error::NotFound(format!("track {id} is unknown")))
    }
}

#[async_trait]
impl lull_core::seams::Copies for Service {
    async fn finished(&self, track: &Track) -> Vec<(SourceRef, PathBuf)> {
        match self.cache.complete(&track.id).await {
            Ok(entries) => entries
                .into_iter()
                .filter(|e| !youtube::is_youtube(&e.source))
                .map(|e| (e.source, e.path))
                .collect(),
            Err(e) => {
                tracing::debug!(error = %e, "the song's finished copies");
                Vec::new()
            }
        }
    }
}

#[async_trait]
impl Transfers for Service {
    async fn complete(&self, track: &TrackId, source: &SourceRef) -> Result<PathBuf> {
        let t = self.track(track).await?;
        let path = self.fetcher.complete(t, source.clone()).await?;
        if let Ok(mut saving) = self.saving.lock() {
            saving.insert(track.clone(), self.cache.pin(track));
        }
        Ok(path)
    }

    /// The saving profile's copy. One already here (a better copy on its way, the one playing,
    /// else the best in the cache) is reused when it passes and the profile wouldn't rank
    /// another copy higher: no second transfer.
    async fn save_candidate(&self, id: &TrackId, fresh: bool) -> Result<Option<SaveCandidate>> {
        let track = self.track(id).await?;
        if (fresh || !self.sources.searched(&track.id)) && self.online.load(Ordering::Relaxed) {
            self.searches.resolve(&track).await?;
        }
        let rules = self.rules.get();
        let ordered = self.sources.ordered(&track, &rules, Purpose::Save);
        let picks = ordered.iter().filter(|r| !r.queue_too_long);
        let best = picks.clone().next();
        let mut here = self.fetcher.copies_here(id);
        if here.is_empty() {
            here.extend(
                self.fetcher
                    .best_cached(&track, Purpose::Save, &rules)
                    .await?
                    .map(|e| e.source),
            );
        }
        let reusable = |r: &&Ranked| {
            r.meets
                && best.is_some_and(|b| match rules.save.pick {
                    Pick::Fastest => true,
                    Pick::Highest => !b
                        .source
                        .quality(track.duration)
                        .beats(&r.source.quality(track.duration)),
                })
        };
        let reuse = here
            .iter()
            .filter_map(|h| picks.clone().find(|r| r.source.is(h)))
            .find(reusable);
        if let Some(r) = reuse.or(best) {
            return Ok(Some(SaveCandidate {
                source: r.source.source_ref(),
                quality: r.source.quality(track.duration),
                meets: r.meets,
            }));
        }
        self.youtube_save(&track).await
    }

    async fn album_available(&self, artist: &str, album: &str) -> Result<Option<bool>> {
        albums::available(self, artist, album).await
    }

    async fn album_known(&self, artist: &str, album: &str) -> Result<Option<bool>> {
        albums::known(self, artist, album).await
    }

    async fn song_available(&self, track: &Track) -> Result<Option<bool>> {
        songs::available(self, track).await
    }
}

/// Keeps the `soulseek` status current: connected, reconnecting or down. On each (re)connect
/// it learns where slskd keeps its files and checks that lull can see them and that sharing
/// matches `LULL_SHARE`.
async fn watch_connection(svc: Arc<Service>, layout: Arc<RwLock<Layout>>) {
    let config = &svc.ctx.config.soulseek;
    let mut was_online = false;
    let mut stuck = recover::Stuck::default();
    let mut silent = 0; // the server's unanswered status checks in a row
    loop {
        let mut extra = Value::Null;
        let server = svc.slskd.server().await;
        // Only a logged-out slskd mid-transition can be stuck; anything else starts the count over.
        if !server.as_ref().is_ok_and(|s| s.is_transitioning && !s.is_logged_in) {
            stuck.settle();
        }
        let (state, detail) = match server {
            Ok(server) if server.is_logged_in => {
                let mut problems = Vec::new();
                if !was_online {
                    match svc.slskd.options().await {
                        Ok(options) => {
                            let next = Layout::new(&config.transfers_dir, options.map(|o| o.directories));
                            if let Ok(mut l) = layout.write() {
                                *l = next;
                            }
                        }
                        Err(e) => tracing::warn!(error = %e, "reading slskd's folders"),
                    }
                }
                if let Some(missing) = layout
                    .read()
                    .ok()
                    .and_then(|l| l.missing().map(|p| p.display().to_string()))
                {
                    problems.push(format!(
                        "lull cannot see slskd's files at {missing}; check LULL_TRANSFERS_DIR"
                    ));
                }
                problems.extend(share::mismatch(&svc.slskd, config.share).await);
                problems.extend(liveness(&svc, server.username.as_deref(), &mut silent).await);
                let who = server.username.map(|u| format!(" as {u}")).unwrap_or_default();
                match problems.is_empty() {
                    true => (State::Ok, format!("Connected{who}")),
                    false => (State::Warn, format!("Connected{who}, but {}", problems.join("; "))),
                }
            }
            Ok(server) if server.is_transitioning => {
                let login = config.web_login.as_ref();
                match recover::unstick(&svc.slskd, login, &server, &mut stuck, std::time::Instant::now()).await {
                    Some((state, detail, why)) => {
                        extra = why;
                        (state, detail)
                    }
                    None => (State::Warn, "Reconnecting to Soulseek".to_owned()),
                }
            }
            Ok(_) => (
                State::Down,
                "slskd is not connected to Soulseek; it retries by itself, so if this lasts check its Soulseek login"
                    .to_owned(),
            ),
            Err(e) => (State::Down, e.to_string()),
        };
        let online = state != State::Down && detail.starts_with("Connected");
        if !online {
            silent = 0; // a session that isn't logged in has nothing to answer yet
        }
        svc.online.store(online, Ordering::Relaxed);
        was_online = online;
        svc.ctx.status.set("soulseek", state, detail, extra);
        tokio::time::sleep(if online { CHECK_OK } else { CHECK_DOWN }).await;
    }
}

/// Whether the Soulseek server answers this session (`Slskd::server_answers`): the one sign of a
/// session gone stale while slskd still says it's logged in (docs/context/soulseek.md#stale-sessions).
/// Every search lane but the listener's and the player's waits while it doesn't. When it stays
/// silent `recover::SILENT_CHECKS` checks in a row, lull asks slskd to log in again, spaced out and
/// never mid-transfer. `None` while it answers; otherwise what the status says.
async fn liveness(svc: &Service, username: Option<&str>, silent: &mut u32) -> Option<String> {
    let me = username?;
    match svc.slskd.server_answers(me).await {
        Ok(true) => {
            *silent = 0;
            svc.searches.set_answering(true);
            None
        }
        Ok(false) => {
            *silent += 1;
            svc.searches.set_answering(false);
            Some(silence(svc, *silent).await)
        }
        Err(e) => {
            // slskd failed to ask, which says nothing about the server: searches go on as they were
            tracing::warn!(error = %e, "asking slskd whether the Soulseek server answers");
            Some(format!(
                "lull couldn't ask slskd whether the Soulseek server answers: {e}"
            ))
        }
    }
}

/// The server let `silent` status checks in a row go unanswered: log in again when it's due, and
/// say so, with slskd's own latest warning.
async fn silence(svc: &Service, silent: u32) -> String {
    let last = svc.relogged.lock().ok().and_then(|l| *l);
    if recover::reconnect_due(silent, last.map(|(when, _)| when.elapsed()), svc.fetcher.transferring()) {
        tracing::warn!(
            checks = silent,
            "the Soulseek server doesn't answer; asking slskd to log in again"
        );
        return match svc
            .slskd
            .relogin("lull: the Soulseek server stopped answering; logging in again")
            .await
        {
            Ok(()) => {
                let at = lull_core::now();
                if let Ok(mut l) = svc.relogged.lock() {
                    *l = Some((std::time::Instant::now(), at));
                }
                format!(
                    "the Soulseek server stopped answering, so lull logged in again at {}",
                    recover::clock(at)
                )
            }
            Err(e) => format!("the Soulseek server stopped answering, and logging in again failed: {e}"),
        };
    }
    let next = if silent < recover::SILENT_CHECKS {
        "lull checks again in a moment".to_owned()
    } else if svc.fetcher.transferring() {
        "lull logs in again once nothing is transferring".to_owned()
    } else {
        match last {
            Some((_, at)) => format!(
                "lull logged in again at {} and tries again at {}",
                recover::clock(at),
                recover::clock(at + recover::EVERY.as_secs() as i64)
            ),
            None => "lull logs in again shortly".to_owned(),
        }
    };
    let said = match svc.slskd.recent_log().await {
        Ok(log) => log
            .into_iter()
            .rev()
            .find(|l| l.is_problem())
            .map(|l| format!("; slskd's latest warning: {}", l.summary())),
        Err(e) => Some(format!("; slskd's log can't be read: {e}")),
    };
    format!(
        "the Soulseek server isn't answering ({next}){}",
        said.unwrap_or_default()
    )
}

async fn housekeeping(svc: Arc<Service>) {
    loop {
        tokio::time::sleep(HOUSEKEEPING).await;
        svc.fetcher.sweep();
        if let Err(e) = svc.cache.make_room(0).await {
            tracing::warn!(error = %e, "stream cache housekeeping");
        }
    }
}

/// A save holds its song in the cache until the save ends, however it ends.
async fn release_saves(svc: Arc<Service>) {
    let mut events = svc.ctx.bus.subscribe();
    loop {
        match events.recv().await {
            Ok(e) if e.topic == "save" && save_over(&e.data) => {
                let Some(track) = e.data.get("trackId").and_then(Value::as_str) else {
                    continue;
                };
                if let Ok(mut saving) = svc.saving.lock() {
                    saving.remove(&TrackId(track.to_owned()));
                }
            }
            Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
        }
    }
}

/// The song is in the library: the last step is done, or only Jellyfin's rescan is pending.
fn save_finished(save: &Value) -> bool {
    match save.get("status").and_then(Value::as_str) {
        Some("pending") => true,
        Some("done") => save.get("step").and_then(Value::as_u64) == Some(5),
        _ => false,
    }
}

fn save_over(save: &Value) -> bool {
    save_finished(save) || save.get("status").and_then(Value::as_str) == Some("failed")
}
