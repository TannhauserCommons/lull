//! A collection on an in-memory database with no library, and a way to see what it published.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use async_trait::async_trait;
use lull_core::config::Config;
use lull_core::db::{CORE, Db};
use lull_core::events::{Bus, Event};
use lull_core::http::Http;
use lull_core::ids::TrackId;
use lull_core::model::Track;
use lull_core::seams::Library;
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

pub(crate) async fn service() -> Result<(Arc<Service>, Receiver<Event>)> {
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
    let svc = Arc::new(Service {
        ctx,
        library: Arc::new(NoLibrary),
        importing: AtomicBool::new(false),
    });
    Ok((svc, rx))
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
