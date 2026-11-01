//! Which fast catalog this hub searches (ADR-0008 update). Deezer is closed in some countries
//! (in some countries): its track and album searches come back empty there, while artist search
//! still answers. So the hub asks Deezer's `/infos` at start, then daily: where Deezer is open it
//! answers tracks, albums and artists; where it is closed, iTunes Search answers tracks and
//! albums, and Deezer only artists and their photos. The choice and any failure show as the
//! `catalog` status.

use std::sync::{Arc, RwLock};
use std::time::Duration;

use lull_core::status::State as Health;

use crate::Service;

const RECHECK: Duration = Duration::from_secs(24 * 3600);
/// After a failed check: until then Deezer is used, as where it is open.
const RETRY: Duration = Duration::from_secs(3600);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    /// Deezer for tracks, albums and artists.
    Deezer,
    /// iTunes for tracks and albums, Deezer for artists.
    Itunes,
}

/// The choice, `None` until Deezer has answered once (Deezer is used meanwhile).
#[derive(Default)]
pub struct Catalog(RwLock<Option<Choice>>);

impl Catalog {
    pub fn choice(&self) -> Choice {
        self.0.read().ok().and_then(|c| *c).unwrap_or(Choice::Deezer)
    }

    fn set(&self, choice: Choice) {
        if let Ok(mut c) = self.0.write() {
            *c = Some(choice);
        }
    }
}

/// Asks Deezer where it is open, at start and daily.
pub async fn keep_choosing(svc: Arc<Service>) {
    loop {
        let wait = match svc.deezer.is_open().await {
            Ok(open) => {
                let choice = if open { Choice::Deezer } else { Choice::Itunes };
                tracing::info!(?choice, "catalog search source chosen");
                svc.catalog.set(choice);
                report(&svc, None);
                RECHECK
            }
            Err(e) => {
                report(
                    &svc,
                    Some(format!("Deezer couldn't say where it is open ({e}); using it as is")),
                );
                RETRY
            }
        };
        tokio::time::sleep(wait).await;
    }
}

/// The `catalog` status: the sources in use, and the latest failure if `problem`.
pub fn report(svc: &Service, problem: Option<String>) {
    let using = match svc.catalog.choice() {
        Choice::Deezer => "catalog: Deezer".to_owned(),
        Choice::Itunes => format!(
            "catalog: iTunes ({}) + Deezer artists, as Deezer isn't open in this country",
            svc.itunes.country()
        ),
    };
    let (state, detail) = match problem {
        Some(p) => (Health::Warn, format!("{using}. {p}")),
        None => (Health::Ok, format!("{using}.")),
    };
    svc.ctx.status.set("catalog", state, detail, serde_json::Value::Null);
}
