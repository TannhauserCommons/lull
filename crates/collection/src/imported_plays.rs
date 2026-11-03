//! Listening history brought from elsewhere (Last.fm, ListenBrainz): `POST /api/import/history`.
//! The plays join the history and feed the taste model, but they are marked imported and
//! nothing is published for them, so they never scrobble and no `play.*` event goes out.
//! The sources parse their own formats into `Listen`s; this module stores them and reports
//! progress as a `job` row.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use lull_core::ids::TrackId;
use lull_core::{Error, Result, catalog};
use rusqlite::{Connection, params};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{Service, lastfm_import, listenbrainz_import};

pub(crate) fn routes() -> Router<Arc<Service>> {
    Router::new()
        .route("/api/import/history", post(start))
        // A ListenBrainz export is years of listens, far past axum's 2 MB default.
        .layer(DefaultBodyLimit::max(128 * 1024 * 1024))
}

/// Listens stored per transaction (and per progress row).
pub(crate) const BATCH: usize = 500;

/// One listen from another service.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Listen {
    pub artist: String,
    pub album: String,
    pub title: String,
    /// Unix seconds.
    pub started_at: i64,
    /// Seconds, when the source knows it.
    pub duration: Option<u32>,
}

/// What imports did: listens that are in the history now, ones that already were, and ones that
/// don't make a play (no artist or title, a time in the future).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Counts {
    pub added: u32,
    pub duplicates: u32,
    pub invalid: u32,
}

/// `{ source: "lastfm", user }`, or the body of a ListenBrainz export. Answers 202 with the job
/// id once the source is known to work (Last.fm knows the user, the export parses).
async fn start(State(svc): State<Arc<Service>>, body: Bytes) -> Result<(StatusCode, Json<Value>)> {
    let claim = Claim::new(&svc)?;
    let job_id = match lastfm_request(&body)? {
        Some(user) => {
            let pages = lastfm_import::Pages::open(&svc, &user).await?;
            let mut progress = Progress::start(&svc, claim, pages.name());
            let job_id = progress.job_id.clone();
            tokio::spawn(async move {
                let result = pages.import(&mut progress).await;
                progress.finish(result);
            });
            job_id
        }
        None => {
            let listens = listenbrainz_import::parse(&body)?;
            let mut progress = Progress::start(&svc, claim, "ListenBrainz history".into());
            let job_id = progress.job_id.clone();
            tokio::spawn(async move {
                let result = listenbrainz_import::import(&mut progress, listens).await;
                progress.finish(result);
            });
            job_id
        }
    };
    Ok((StatusCode::ACCEPTED, Json(json!({ "jobId": job_id }))))
}

/// The Last.fm user when the body is `{ source: "lastfm", user }`; `None` when it is something
/// else (a ListenBrainz export has no `source`).
fn lastfm_request(body: &[u8]) -> Result<Option<String>> {
    #[derive(Deserialize)]
    #[serde(rename_all = "lowercase")]
    enum Source {
        Lastfm,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Request {
        #[allow(dead_code)] // only "lastfm" parses: that is the check
        source: Source,
        user: String,
    }
    let is_request = matches!(serde_json::from_slice::<Value>(body), Ok(Value::Object(o)) if o.contains_key("source"));
    if !is_request {
        return Ok(None);
    }
    let request: Request = serde_json::from_slice(body).map_err(|e| {
        Error::BadRequest(format!(
            "Import history with {{ \"source\": \"lastfm\", \"user\" }}: {e}."
        ))
    })?;
    Ok(Some(request.user))
}

/// Holds the one history import slot until dropped.
struct Claim(Arc<Service>);

impl Claim {
    fn new(svc: &Arc<Service>) -> Result<Self> {
        if svc.importing.swap(true, Ordering::SeqCst) {
            return Err(Error::Conflict("A history import is already running.".into()));
        }
        Ok(Claim(svc.clone()))
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        self.0.importing.store(false, Ordering::SeqCst);
    }
}

/// One import's job row: stores batches and tells Activity how far it got.
pub(crate) struct Progress {
    svc: Arc<Service>,
    _claim: Claim,
    job_id: String,
    name: String,
    started: i64,
    counts: Counts,
}

impl Progress {
    fn start(svc: &Arc<Service>, claim: Claim, name: String) -> Self {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let progress = Progress {
            svc: svc.clone(),
            _claim: claim,
            job_id: format!("job-history-{nanos}"),
            name,
            started: lull_core::now(),
            counts: Counts::default(),
        };
        progress.publish("running", None, None);
        progress
    }

    /// Stores a batch and reports. `pct` is how far through the source this batch is, when known.
    pub(crate) async fn add(&mut self, listens: Vec<Listen>, pct: Option<u8>) -> Result<()> {
        let batch = self.svc.ctx.db.call(move |conn| store(conn, &listens)).await?;
        self.counts.added += batch.added;
        self.counts.duplicates += batch.duplicates;
        self.counts.invalid += batch.invalid;
        self.publish("running", pct, None);
        Ok(())
    }

    fn finish(self, result: Result<()>) {
        match result {
            Ok(()) => self.publish("done", Some(100), None),
            Err(e) => {
                tracing::warn!(error = %e, job = %self.job_id, "history import failed");
                self.publish("failed", None, Some(e.to_string()));
            }
        }
    }

    /// The job row, as `GET /api/jobs` lists it (api.md#events `job`).
    fn publish(&self, state: &str, pct: Option<u8>, error: Option<String>) {
        self.svc.ctx.bus.publish(
            "job",
            json!({
                "jobId": self.job_id, "kind": "import", "name": self.name, "state": state, "started": self.started,
                "pct": pct, "added": self.counts.added, "duplicates": self.counts.duplicates,
                "invalid": self.counts.invalid, "error": error,
            }),
        );
    }
}

/// Adds the listens as imported plays. Listening to the same song at the same second is the
/// same play: that is how a re-run import, and the scrobbles lull itself sent to Last.fm,
/// are recognised.
fn store(conn: &mut Connection, listens: &[Listen]) -> Result<Counts> {
    let (tx, now) = (conn.transaction()?, lull_core::now());
    let mut counts = Counts::default();
    for listen in listens {
        let (artist, title) = (listen.artist.trim(), listen.title.trim());
        if artist.is_empty() || title.is_empty() || listen.started_at <= 0 || listen.started_at > now {
            counts.invalid += 1;
            continue;
        }
        let album = listen.album.trim();
        let id = TrackId::of(artist, album, title);
        let duplicate: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM plays WHERE started_at = ?1 AND (track_id = ?2 OR lower(title) = lower(?3)))",
            params![listen.started_at, id.as_str(), title],
            |r| r.get(0),
        )?;
        if duplicate {
            counts.duplicates += 1;
            continue;
        }
        // As the listen names it (a richer record from search or the library wins); discovery
        // resolves it to the catalog's song when History shows it.
        let track = catalog::remember_named(
            &tx,
            catalog::Named {
                artist,
                album: Some(album).filter(|a| !a.is_empty()),
                title,
                duration: listen.duration,
                ..catalog::Named::default()
            },
        )?;
        // A listen counts as heard: that is what a scrobble means, and the taste model reads it so.
        let duration = track.duration.unwrap_or(0);
        tx.execute(
            "INSERT INTO plays (track_id, artist, title, album, duration_s, started_at, played_s, completed)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?5, 1)",
            params![
                track.id.as_str(),
                track.artist,
                track.title,
                track.album,
                duration,
                listen.started_at
            ],
        )?;
        tx.execute(
            "INSERT INTO collection_plays (play_id, ended_at, imported) VALUES (?1, ?2, 1)",
            params![tx.last_insert_rowid(), now],
        )?;
        counts.added += 1;
    }
    tx.commit()?;
    Ok(counts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing;

    fn listen(artist: &str, title: &str, started_at: i64) -> Listen {
        Listen {
            artist: artist.into(),
            album: String::new(),
            title: title.into(),
            started_at,
            duration: None,
        }
    }

    #[tokio::test]
    async fn imported_plays_never_publish_a_play_event() -> Result<()> {
        let (svc, mut rx) = testing::service().await?;
        let claim = Claim::new(&svc)?;
        let mut progress = Progress::start(&svc, claim, "Last.fm history (moth)".into());
        let at = lull_core::now() - 86_400;
        progress
            .add(vec![listen("Velvet Orchard", "Paper Moon Static", at)], Some(100))
            .await?;
        progress.finish(Ok(()));

        let topics: Vec<String> = testing::published(&mut rx).into_iter().map(|e| e.topic).collect();
        assert!(!topics.is_empty() && topics.iter().all(|t| t == "job"), "{topics:?}");
        let (imported, ended): (i64, Option<i64>) = svc
            .ctx
            .db
            .call(|c| {
                Ok(c.query_row("SELECT imported, ended_at FROM collection_plays", [], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })?)
            })
            .await?;
        assert_eq!(imported, 1);
        assert!(ended.is_some(), "an imported play is over: nothing can end it again");
        Ok(())
    }

    #[tokio::test]
    async fn importing_twice_adds_a_listen_once_and_skips_what_lull_already_has() -> Result<()> {
        let (svc, _rx) = testing::service().await?;
        let day_ago = lull_core::now() - 86_400;
        // lull's own play of this song, which it scrobbled to Last.fm at this very second.
        let own = testing::known_song(&svc, "Low Tide Radio", "Harbour Lights").await?;
        svc.ctx
            .db
            .call(move |c| {
                c.execute(
                    "INSERT INTO plays (track_id, artist, title, album, duration_s, started_at) VALUES (?1, 'Low Tide Radio', 'Harbour Lights', '', 200, ?2)",
                    params![own.as_str(), day_ago],
                )?;
                Ok(())
            })
            .await?;
        let listens = vec![
            listen("Velvet Orchard", "Paper Moon Static", day_ago - 300),
            listen("Low Tide Radio", "Harbour Lights (Remastered)", day_ago),
            listen("Low Tide Radio", "harbour lights", day_ago),
            listen("", "No Artist", day_ago),
            listen("Future Band", "Tomorrow", lull_core::now() + 3600),
        ];
        let first = svc.ctx.db.call({
            let l = listens.clone();
            move |c| store(c, &l)
        });
        let second = svc.ctx.db.call(move |c| store(c, &listens));
        let (first, second) = (first.await?, second.await?);
        assert_eq!(
            first,
            Counts {
                added: 2,
                duplicates: 1,
                invalid: 2
            }
        );
        assert_eq!(second.added, 0, "a re-run adds nothing");
        Ok(())
    }
}
