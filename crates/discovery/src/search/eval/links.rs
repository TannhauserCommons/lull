//! MusicBrainz's answers for the artists' ids, as the hub asks them (`identity::decide`'s
//! `url_artists` and `links` lookups), kept in the eval folder like `answers.json` so a rerun
//! calls nothing it already has. Recorded only with `LULL_SEARCH_EVAL_LINKS` set, through the
//! crate's own reader (`sources::musicbrainz`), paced as the hub paces it (one request a second).
//! A lookup nobody recorded answers nothing, as a hub whose MusicBrainz is away would.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use lull_core::Result;
use lull_core::db::Db;
use lull_core::http::Http;
use serde::{Deserialize, Serialize};

use crate::sources::ArtistInfo;
use crate::sources::cache::Cache;
use crate::sources::musicbrainz::{Links, MusicBrainz};

const FILE: &str = "artist-links.json";
/// Requests one recording run may make: past this something is wrong, or the artists are too many.
const MAX_REQUESTS: usize = 100;

#[derive(Default, Serialize, Deserialize)]
struct Recorded {
    /// By the page URLs asked, sorted and joined by a space.
    url_artists: BTreeMap<String, HashMap<String, Vec<ArtistInfo>>>,
    /// By MBID.
    links: BTreeMap<String, Links>,
}

pub struct ArtistLinks {
    recorded: Mutex<Recorded>,
    /// The reader, when this run records what it lacks.
    mb: Option<MusicBrainz>,
    /// Requests made this run.
    asked: Mutex<usize>,
}

impl ArtistLinks {
    /// What is recorded in `dir`, asking nothing.
    pub fn load(dir: &Path) -> std::result::Result<ArtistLinks, String> {
        let path = path(dir);
        let recorded = match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Recorded::default(),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        Ok(ArtistLinks {
            recorded: Mutex::new(recorded),
            mb: None,
            asked: Mutex::new(0),
        })
    }

    /// What is recorded in `dir`, and MusicBrainz for what is not.
    pub fn recording(dir: &Path) -> std::result::Result<ArtistLinks, String> {
        let http = Http::new().map_err(|e| e.to_string())?;
        let db = Db::memory().map_err(|e| e.to_string())?;
        db.migrate(&crate::MIGRATIONS).map_err(|e| e.to_string())?;
        Ok(ArtistLinks {
            mb: Some(MusicBrainz::new(http, Cache::new(db))),
            ..ArtistLinks::load(dir)?
        })
    }

    /// `identity::decide`'s `url_artists`.
    pub async fn url_artists(&self, mut urls: Vec<String>) -> Result<HashMap<String, Vec<ArtistInfo>>> {
        urls.sort();
        urls.dedup();
        let key = urls.join(" ");
        if let Some(known) = self.recorded().url_artists.get(&key) {
            return Ok(known.clone());
        }
        let Some(mb) = &self.mb else { return Ok(HashMap::new()) };
        self.count_request()?;
        let answer = mb.url_artists(urls).await?;
        self.recorded().url_artists.insert(key, answer.clone());
        Ok(answer)
    }

    /// `identity::decide`'s `links`.
    pub async fn links(&self, mbid: String) -> Result<Links> {
        if let Some(known) = self.recorded().links.get(&mbid) {
            return Ok(known.clone());
        }
        let Some(mb) = &self.mb else {
            return Ok(Links::default());
        };
        self.count_request()?;
        let answer = mb.artist_links(&mbid).await?;
        self.recorded().links.insert(mbid, answer.clone());
        Ok(answer)
    }

    /// Written beside the file and renamed over it, so a stop in the middle leaves the last save.
    pub fn save(&self, dir: &Path) -> std::result::Result<(), String> {
        let path = path(dir);
        let part = path.with_extension("json.part");
        let text = serde_json::to_string(&*self.recorded()).map_err(|e| e.to_string())?;
        std::fs::write(&part, text).map_err(|e| format!("{}: {e}", part.display()))?;
        std::fs::rename(&part, &path).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// How many requests this run made.
    pub fn requests(&self) -> usize {
        *self.asked.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn count_request(&self) -> Result<()> {
        let mut asked = self.asked.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if *asked >= MAX_REQUESTS {
            return Err(lull_core::Error::internal(format!(
                "more than {MAX_REQUESTS} MusicBrainz requests needed: stopping"
            )));
        }
        *asked += 1;
        Ok(())
    }

    fn recorded(&self) -> std::sync::MutexGuard<'_, Recorded> {
        self.recorded.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

fn path(dir: &Path) -> PathBuf {
    dir.join(FILE)
}
