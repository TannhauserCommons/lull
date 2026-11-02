//! A collection on an in-memory database with no library, and a way to see what it published.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use lull_core::config::Config;
use lull_core::db::{CORE, Db};
use lull_core::devices::{self, Kind};
use lull_core::events::{Bus, Event};
use lull_core::http::Http;
use lull_core::ids::TrackId;
use lull_core::model::{LatePlay, Track};
use lull_core::seams::{Library, Scrobbles};
use lull_core::status::Status;
use lull_core::{Ctx, Result, catalog};
use tokio::sync::broadcast::Receiver;

use crate::{MIGRATIONS, Service};

struct NoLibrary;

#[async_trait]
impl Library for NoLibrary {
    async fn search(&self, _: &str, _: usize) -> Result<Vec<Track>> {
        Ok(Vec::new())
    }
    async fn saved(&self, _: &[TrackId]) -> Result<Vec<Track>> {
        Ok(Vec::new())
    }
}

/// Last.fm as lastfm's rules would have it, minus the account: 14 days, half a track, and a
/// switch for whether Last.fm is connected. Records what it was asked to queue.
#[derive(Default)]
pub(crate) struct FakeLastfm {
    pub(crate) disconnected: AtomicBool,
    pub(crate) queued: std::sync::Mutex<Vec<LatePlay>>,
}

#[async_trait]
impl Scrobbles for FakeLastfm {
    fn refusal(&self, play: &LatePlay, now: i64) -> Option<String> {
        if now - play.started_at > 14 * 86_400 {
            Some("it is older than 14 days.".into())
        } else if play.played_s * 2 < play.duration_s {
            Some("it wasn't played for half its length.".into())
        } else {
            None
        }
    }

    async fn submit(&self, plays: Vec<LatePlay>) -> Result<()> {
        if self.disconnected.load(Ordering::Relaxed) {
            return Err(lull_core::Error::Conflict("Connect Last.fm first.".into()));
        }
        let mut queued = self.queued.lock().map_err(|_| lull_core::Error::internal("poisoned"))?;
        if let Some(p) = plays.iter().find(|p| queued.iter().any(|q| q.play_id == p.play_id)) {
            return Err(lull_core::Error::Conflict(format!("{} is already queued.", p.play_id)));
        }
        queued.extend(plays);
        Ok(())
    }
}

/// A paired browser, as the gate would have identified it; `private` is its listening switch.
pub(crate) async fn device(svc: &Service, private: bool) -> Result<lull_core::devices::Device> {
    let (mut device, _) = devices::create(&svc.ctx.db, "Test browser", Kind::Browser).await?;
    if private {
        devices::set_private(&svc.ctx.db, &device.id, true).await?;
        device.private = true;
    }
    Ok(device)
}

pub(crate) async fn service() -> Result<(Arc<Service>, Receiver<Event>)> {
    let (svc, rx, _) = service_with_lastfm().await?;
    Ok((svc, rx))
}

pub(crate) async fn service_with_lastfm() -> Result<(Arc<Service>, Receiver<Event>, Arc<FakeLastfm>)> {
    let vars: BTreeMap<String, String> = [("LULL_SLSKD_URL", "http://slskd"), ("LULL_SLSKD_API_KEY", "k")]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
    let config = Config::from_vars(vars).map_err(|e| lull_core::Error::internal(e.to_string()))?;
    let db = Db::memory()?;
    for m in [&CORE, &catalog::MIGRATIONS, &MIGRATIONS] {
        db.migrate(m)?;
    }
    let bus = Bus::new();
    let rx = bus.subscribe();
    let ctx = Ctx {
        config: Arc::new(config),
        db,
        http: Http::new()?,
        status: Status::new(bus.clone()),
        bus,
        shutdown: Default::default(),
    };
    let lastfm = Arc::new(FakeLastfm::default());
    let svc = Arc::new(Service {
        ctx,
        library: Arc::new(NoLibrary),
        scrobbles: lastfm.clone(),
        sent: Default::default(),
        importing: AtomicBool::new(false),
    });
    Ok((svc, rx, lastfm))
}

/// Records a song in the catalog, as search would have, and returns its id.
pub(crate) async fn known_song(svc: &Service, artist: &str, title: &str) -> Result<TrackId> {
    let track = crate::import::unsaved_track(artist, "", title, Some(200));
    let id = track.id.clone();
    svc.ctx.db.call(move |c| catalog::remember(c, &[track])).await?;
    Ok(id)
}

/// The topics published so far, in order.
pub(crate) fn published(rx: &mut Receiver<Event>) -> Vec<Event> {
    std::iter::from_fn(|| rx.try_recv().ok()).collect()
}
