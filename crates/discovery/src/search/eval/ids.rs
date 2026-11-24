//! The catalog ids of the listener's artists, as the eval derives them. The export has none
//! (`discovery_artist_ids` isn't in it), so they are **derived from the recorded rows, not the
//! hub's own**: for each song the listener played or liked, the `artist_source` of the recorded
//! catalog rows that are the same take (`merge::same_take`), weighed by the hub's own rule
//! (`identity::decide`) with lookups that answer nothing. No MusicBrainz expansion happens, so
//! only `Evidence::Song` ids come out: a catalog artist that every such row of the artist names.
//! The hub's ids come from the tracklists of the albums its songs were found on, and add the
//! library's MBIDs and links, so this is a floor on what the hub holds, never more.

use std::collections::{HashMap, HashSet};

use lull_core::ids::{ArtistId, loose_artist, loose_title};
use lull_core::model::Track;

use super::answers::Answers;
use crate::artist::identity::{self, Evidence, Listened};
use crate::search::merge::{SongHit, same_take};
use crate::search::personal::score::CatalogId;

/// The trusted catalog ids per artist, as the hub's `listener_artist_ids` gives them.
#[derive(Default)]
pub struct DerivedIds {
    pub by_artist: HashMap<ArtistId, Vec<CatalogId>>,
}

/// The ids of the artists of `tracks` (the songs played or liked), from the rows in `answers`.
pub fn derive(tracks: &[Track], answers: &Answers) -> Result<DerivedIds, String> {
    let rows = recorded_rows(answers);
    let mut listened: HashMap<ArtistId, Listened> = HashMap::new();
    for track in tracks {
        let song = crate::search::saved(track.clone());
        let named = rows
            .get(&(loose_artist(&track.artist), loose_title(&track.title)))
            .into_iter()
            .flatten()
            .filter(|row| same_take(row, &song))
            .filter_map(|row| row.artist_source);
        listened
            .entry(ArtistId::of(&track.artist))
            .or_default()
            .catalog
            .extend(named.map(|source| (source, track.id.clone())));
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|e| e.to_string())?;
    let mut by_artist = HashMap::new();
    for (artist, listened) in listened.into_iter().filter(|(_, l)| !l.catalog.is_empty()) {
        let ids = runtime
            .block_on(identity::decide(
                &listened,
                "US",
                |_| async { Ok(HashMap::new()) },
                |_| async { Ok(Default::default()) },
                |_| async { Ok(Vec::new()) },
            ))
            .map_err(|e| e.to_string())?;
        let trusted: Vec<CatalogId> = ids
            .rows()
            .iter()
            .filter(|r| matches!(r.evidence, Evidence::Song { .. }))
            .map(|r| (r.source, r.id.clone()))
            .collect();
        if !trusted.is_empty() {
            by_artist.insert(artist, trusted);
        }
    }
    Ok(DerivedIds { by_artist })
}

/// The recorded rows that name their artist by a catalog id, by loose artist and title.
fn recorded_rows(answers: &Answers) -> HashMap<(String, String), Vec<&SongHit>> {
    let mut rows: HashMap<(String, String), Vec<&SongHit>> = HashMap::new();
    for row in answers.songs().filter(|s| s.artist_source.is_some()) {
        rows.entry((loose_artist(&row.artist), loose_title(&row.title)))
            .or_default()
            .push(row);
    }
    rows
}

impl DerivedIds {
    /// Whether the artist named (loose, first credit) has a trusted id through any song of theirs.
    pub fn holds(&self, tracks: &[Track], name: &str) -> bool {
        let named: HashSet<ArtistId> = tracks
            .iter()
            .filter(|t| loose_artist(super::listening::first_credit(&t.artist)) == name)
            .map(|t| ArtistId::of(&t.artist))
            .collect();
        named.iter().any(|a| self.by_artist.contains_key(a))
    }
}
