//! The catalog ids of the listener's artists, as the eval derives them. The export has none
//! (`discovery_artist_ids` isn't in it), so they are **derived from the recorded rows, not the
//! hub's own**: for each song the listener played or liked, the `artist_source` of the recorded
//! catalog rows that are the same take (`merge::same_take`), weighed by the hub's own rule
//! (`identity::decide`). The song ids alone are iTunes artists (the recorded song rows are
//! iTunes's) while artist rows are Deezer's, so for the eval's artists the lookups are the hub's
//! (`links.rs`: MusicBrainz's url and artist links, recorded), and the Deezer ids come as the hub
//! gets them. Everything else asks nothing. These are ids **derived as the hub derives them, from
//! recorded rows plus MusicBrainz**, not the hub's own: it also has the albums' tracklists, the
//! library's MBIDs and ISRC credits.

use std::collections::{HashMap, HashSet};

use lull_core::ids::{ArtistId, loose_artist, loose_title};
use lull_core::model::Track;

use super::answers::Answers;
use super::links::ArtistLinks;
use crate::artist::identity::{self, Evidence, Listened, Source};
use crate::search::merge::{SongHit, same_take};
use crate::search::personal::score::CatalogId;

/// The trusted catalog ids per artist, as the hub's `listener_artist_ids` gives them.
#[derive(Default)]
pub struct DerivedIds {
    pub by_artist: HashMap<ArtistId, Vec<CatalogId>>,
}

/// The ids of the artists of `tracks` (the songs played or liked), from the rows in `answers`,
/// and for the artists named in `expand` (loose first credits) as the hub expands them through
/// MusicBrainz's links (`links`). Derived as the hub derives them, from recorded rows plus
/// MusicBrainz: not the hub's own.
pub fn derive(
    tracks: &[Track],
    answers: &Answers,
    links: &ArtistLinks,
    expand: &HashSet<String>,
) -> Result<DerivedIds, String> {
    let rows = recorded_rows(answers);
    let mut listened: HashMap<ArtistId, Listened> = HashMap::new();
    let mut names: HashMap<ArtistId, String> = HashMap::new();
    for track in tracks {
        names.insert(
            ArtistId::of(&track.artist),
            loose_artist(super::listening::first_credit(&track.artist)),
        );
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
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let mut by_artist = HashMap::new();
    for (artist, listened) in listened.into_iter().filter(|(_, l)| !l.catalog.is_empty()) {
        let asked = if names.get(&artist).is_some_and(|n| expand.contains(n)) {
            Some(links)
        } else {
            None
        };
        let ids = runtime
            .block_on(identity::decide(
                &listened,
                "US",
                |u| async move {
                    match asked {
                        Some(links) => links.url_artists(u).await,
                        None => Ok(HashMap::new()),
                    }
                },
                |m| async move {
                    match asked {
                        Some(links) => links.links(m).await,
                        None => Ok(Default::default()),
                    }
                },
                |_| async { Ok(Vec::new()) },
            ))
            .map_err(|e| e.to_string())?;
        // What the hub's `listener_artist_ids` keeps: everything but a recommender's or a
        // search row's id, none of which come out here.
        let trusted: Vec<CatalogId> = ids
            .rows()
            .iter()
            .filter(|r| !matches!(r.evidence, Evidence::Recommended { .. } | Evidence::Searched))
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
    /// Whether the artist named (loose, first credit) has a trusted id through any song of theirs,
    /// of `source` when one is given.
    pub fn holds(&self, tracks: &[Track], name: &str, source: Option<Source>) -> bool {
        tracks
            .iter()
            .filter(|t| loose_artist(super::listening::first_credit(&t.artist)) == name)
            .filter_map(|t| self.by_artist.get(&ArtistId::of(&t.artist)))
            .flatten()
            .any(|(s, _)| source.is_none_or(|only| *s == only))
    }
}
