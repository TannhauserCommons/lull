//! The fast sources' answers for each query, as the source readers returned them, kept in the
//! eval folder so a rerun calls nothing it already has.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::search::merge::{Found, SongHit};

const FILE: &str = "answers.json";

/// What each fast source said to one query. YouTube Music is asked from three characters on, as
/// search asks it (`SLOW_SOURCES_FROM`); for a shorter query its answer is empty.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Answer {
    pub itunes: Found,
    pub deezer_artists: Found,
    pub youtube_music: Found,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Answers(BTreeMap<String, Answer>);

impl Answers {
    pub fn load(dir: &Path) -> Result<Answers, String> {
        let path = path(dir);
        match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Answers::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    /// Written beside the file and renamed over it, so a stop in the middle leaves the last save.
    pub fn save(&self, dir: &Path) -> Result<(), String> {
        let path = path(dir);
        let part = path.with_extension("json.part");
        let text = serde_json::to_string(self).map_err(|e| e.to_string())?;
        std::fs::write(&part, text).map_err(|e| format!("{}: {e}", part.display()))?;
        std::fs::rename(&part, &path).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn get(&self, query: &str) -> Option<&Answer> {
        self.0.get(query)
    }

    pub fn insert(&mut self, query: String, answer: Answer) {
        self.0.insert(query, answer);
    }

    /// Every song row of every recorded answer.
    pub fn songs(&self) -> impl Iterator<Item = &SongHit> {
        self.0
            .values()
            .flat_map(|a| [&a.itunes, &a.deezer_artists, &a.youtube_music])
            .flat_map(|found| &found.songs)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }
}

fn path(dir: &Path) -> PathBuf {
    dir.join(FILE)
}
