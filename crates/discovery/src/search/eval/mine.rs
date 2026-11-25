//! The listener's folder as the personal snapshot is built from it: the plays, likes, library,
//! playlists and follows the hub's database would hold, in the shape `Snapshot::from_signals`
//! takes. The export has no artist ids (`discovery_artist_ids` isn't in it), so the caller gives
//! the ones `ids` derives from recorded rows, or none.

use std::collections::{HashMap, HashSet};

use lull_core::ids::{ArtistId, TrackId};
use lull_core::model::Track;
use serde_json::Value;

use crate::search::personal::first_met;
use crate::search::personal::index::Song;
use crate::search::personal::score::CatalogId;
use crate::search::personal::snapshot::Snapshot;
use crate::taste::{Mark, Play, Signals};

/// The pages of the export the snapshot reads, and the track records they share.
pub struct Pages<'a> {
    pub history: &'a Value,
    pub likes: &'a Value,
    pub library: &'a Value,
    pub playlists: &'a Value,
    pub follows: &'a Value,
    /// Every track record of every page, by id.
    pub known: &'a HashMap<&'a str, &'a Value>,
}

impl Pages<'_> {
    /// The snapshot, with `ids` as the catalog ids of the listener's artists.
    pub fn snapshot(&self, ids: &HashMap<ArtistId, Vec<CatalogId>>) -> Result<Snapshot, String> {
        let plays = self.plays()?;
        // The export gives the day of a play but not of a like, so the likes count as made at the
        // last play: as new as anything the listener did.
        let now = plays.iter().map(|p| p.at).max().unwrap_or_default();
        let likes = self.liked(now)?;
        let saved: Vec<Track> = self.tracks(&self.library["songs"])?;
        let playlisted: Vec<TrackId> = self.playlisted();
        let signals = Signals {
            followed_artists: self.followed_names(),
            plays,
            likes,
            ..Signals::default()
        };
        let songs = self.songs(&signals, &saved, &playlisted)?;
        let playlisted: HashSet<TrackId> = playlisted.into_iter().collect();
        Ok(Snapshot::from_signals(signals, songs, &playlisted, ids, now))
    }

    /// The songs played or liked, once each: what the hub's identity rule reads
    /// (`Store::listener_songs`).
    pub fn listened(&self) -> Result<Vec<Track>, String> {
        let mut seen = HashSet::new();
        let ids = strings(&self.likes["items"]).chain(
            self.history["days"]
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(|day| day["items"].as_array().into_iter().flatten())
                .filter_map(|p| p["trackId"].as_str()),
        );
        ids.filter(|id| seen.insert(*id))
            .filter_map(|id| self.track(id).transpose())
            .collect()
    }

    fn plays(&self) -> Result<Vec<Play>, String> {
        let mut plays = Vec::new();
        for item in self.history["days"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|day| day["items"].as_array().into_iter().flatten())
        {
            let Some(track) = item["trackId"].as_str().and_then(|id| self.track(id).transpose()) else {
                continue;
            };
            let track = track?;
            plays.push(Play {
                artist: track.artist.clone(),
                duration_s: track.duration.unwrap_or_default(),
                track: track.id,
                at: item["at"].as_i64().unwrap_or_default(),
                played_s: item["playedS"]
                    .as_u64()
                    .and_then(|s| u32::try_from(s).ok())
                    .unwrap_or_default(),
                completed: item["completed"].as_bool().unwrap_or_default(),
                skipped: item["skipped"].as_bool().unwrap_or_default(),
            });
        }
        Ok(plays)
    }

    fn liked(&self, at: i64) -> Result<Vec<Mark>, String> {
        Ok(self
            .tracks(&self.likes["items"])?
            .into_iter()
            .map(|t| Mark {
                artist: t.artist,
                track: t.id,
                at,
            })
            .collect())
    }

    /// The songs in the playlists, once each, in the playlists' order (as the hub reads them).
    fn playlisted(&self) -> Vec<TrackId> {
        let listed: Vec<TrackId> = self.playlists["items"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|list| list["items"].as_array().into_iter().flatten())
            .filter_map(|t| t["trackId"].as_str())
            .map(|id| TrackId(id.to_owned()))
            .collect();
        first_met(&listed).into_iter().cloned().collect()
    }

    fn followed_names(&self) -> Vec<String> {
        self.follows["entities"]["artists"]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(_, a)| a["name"].as_str())
            .map(str::to_owned)
            .collect()
    }

    /// Every song the listener saved, played, liked or listed, once, as the hub's build holds them.
    fn songs(&self, signals: &Signals, saved: &[Track], playlisted: &[TrackId]) -> Result<Vec<Song>, String> {
        let kept: HashSet<&TrackId> = saved.iter().map(|t| &t.id).collect();
        // In the order the hub meets them (`personal::first_met`): plays, likes, playlists.
        let met = first_met(
            signals
                .plays
                .iter()
                .map(|p| &p.track)
                .chain(signals.likes.iter().map(|m| &m.track))
                .chain(playlisted),
        );
        let mut songs: Vec<Song> = saved
            .iter()
            .map(|t| Song {
                track: t.clone(),
                saved: true,
                album_artist: None,
            })
            .collect();
        for id in met.into_iter().filter(|id| !kept.contains(id)) {
            if let Some(track) = self.track(&id.0)? {
                songs.push(Song {
                    track,
                    saved: false,
                    album_artist: None,
                });
            }
        }
        Ok(songs)
    }

    fn tracks(&self, ids: &Value) -> Result<Vec<Track>, String> {
        strings(ids).filter_map(|id| self.track(id).transpose()).collect()
    }

    /// The record of a track, when the export has one.
    fn track(&self, id: &str) -> Result<Option<Track>, String> {
        self.known
            .get(id)
            .map(|t| serde_json::from_value::<Track>((*t).clone()).map_err(|e| format!("a track record: {e}")))
            .transpose()
    }
}

fn strings(list: &Value) -> impl Iterator<Item = &str> {
    list.as_array().into_iter().flatten().filter_map(Value::as_str)
}
