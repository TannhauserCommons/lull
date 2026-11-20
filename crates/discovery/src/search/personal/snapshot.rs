//! What search reads of the listener, built once and shared: the index of their music and the
//! scores (`score`) of its songs, albums and artists. A search takes no database call: it asks
//! the snapshot, which answers in the catalogs' shape (`Found`) and says how much each row
//! (`Affinity`) is the listener's.

use std::collections::{HashMap, HashSet};

use lull_core::ids::{ArtistId, TrackId};

use super::index::{AlbumKey, Index, Match, Song, Take, album_key, take_of};
use super::score::{self, Affinity, CatalogId, Keep, Kept};
use crate::artist::identity::Source;
use crate::search::merge::{AlbumHit, ArtistHit, Found, SongHit};
use crate::search::query::Query;
use crate::sources::ArtistSource;
use crate::taste::{Signals, Taste};

/// The most songs and albums the index answers with: the board shows 50 songs and 20 albums.
const SONGS: usize = 50;
const ALBUMS: usize = 20;

/// A take's two scores: its own, and its artist's.
#[derive(Debug, Clone, Copy, Default)]
struct TakeScore {
    song: f64,
    artist: f64,
}

#[derive(Default)]
pub struct Snapshot {
    index: Index,
    /// Each indexed song's affinity, in `index.songs`' order.
    songs: Vec<Affinity>,
    albums: HashMap<AlbumKey, Affinity>,
    /// Every take the listener has a signal for, kept or not in the index.
    takes: HashMap<Take, TakeScore>,
    /// Their artists' scores by each catalog id lull holds for them.
    artists: HashMap<CatalogId, f64>,
}

impl Snapshot {
    /// The snapshot of what the listener did (`signals`), has (`songs`: saved, played, liked or in
    /// a playlist, as their records) and the catalog ids lull holds for their artists (`ids`).
    pub fn from_signals(
        signals: Signals,
        songs: Vec<Song>,
        playlisted: &HashSet<TrackId>,
        ids: &HashMap<ArtistId, Vec<CatalogId>>,
        now: i64,
    ) -> Snapshot {
        let artist_scores = score::artists(&Taste::build(&signals, now));
        let kept = kept(&songs, playlisted);
        let song_scores = score::songs(&signals, &kept, now);
        let takes = take_scores(&songs, &song_scores, &artist_scores);
        let index = Index::build(songs);
        Snapshot {
            songs: index
                .songs
                .iter()
                .map(|s| affinity(takes.get(&s.take).copied()))
                .collect(),
            albums: album_scores(&index, &takes),
            artists: score::by_catalog_id(&artist_scores, ids),
            takes,
            index,
        }
    }

    /// The listener's songs and albums the words match, as a source's answer: the best match
    /// first (a name before a year or a genre), then the most theirs.
    pub fn found(&self, q: &Query) -> Found {
        let mut songs: Vec<(usize, Match)> = self.index.songs(q);
        songs.sort_by(|&(a, ma), &(b, mb)| {
            mb.cmp(&ma)
                .then_with(|| self.songs[b].score.total_cmp(&self.songs[a].score))
        });
        let mut albums: Vec<(usize, Match)> = self.index.albums(q);
        albums.sort_by(|&(a, ma), &(b, mb)| {
            mb.cmp(&ma).then_with(|| {
                let of = |i: usize| self.albums.get(&self.index.albums[i].key).map_or(0.0, |a| a.score);
                of(b).total_cmp(&of(a))
            })
        });
        Found {
            songs: songs
                .into_iter()
                .take(SONGS)
                .map(|(i, _)| self.index.songs[i].hit.clone())
                .collect(),
            albums: albums
                .into_iter()
                .take(ALBUMS)
                .map(|(i, _)| self.index.albums[i].hit.clone())
                .collect(),
            ..Found::default()
        }
    }

    /// How much a song row is the listener's: its take's own signals, with its artist's (reached by the
    /// catalog id the row came with, or the listener's own record of the take) at half.
    pub fn song(&self, s: &SongHit) -> Affinity {
        let take = self
            .takes
            .get(&take_of(&s.artist, &s.title))
            .copied()
            .unwrap_or_default();
        let by_id = s.artist_source.and_then(|a| self.artists.get(&catalog_id(a))).copied();
        affinity(Some(TakeScore {
            artist: take.artist.max(by_id.unwrap_or(0.0)),
            ..take
        }))
    }

    pub fn album(&self, a: &AlbumHit) -> Affinity {
        self.albums
            .get(&album_key(&a.artist, &a.title))
            .copied()
            .unwrap_or_default()
    }

    /// Only through the ids the row came with: Apple's, Deezer's, MusicBrainz's.
    pub fn artist(&self, a: &ArtistHit) -> Affinity {
        let mbid = a.mbid.iter().map(|m| (Source::Musicbrainz, m.clone()));
        let score = a
            .sources
            .iter()
            .map(|&s| catalog_id(s))
            .chain(mbid)
            .filter_map(|id| self.artists.get(&id).copied())
            .fold(0.0, f64::max);
        Affinity { score, own: false }
    }
}

/// An artist row's own catalog id, as `discovery_artist_ids` holds it.
fn catalog_id(source: ArtistSource) -> CatalogId {
    match source {
        ArtistSource::Itunes(id) => (Source::Itunes, id.to_string()),
        ArtistSource::Deezer(id) => (Source::Deezer, id.to_string()),
    }
}

fn affinity(take: Option<TakeScore>) -> Affinity {
    let t = take.unwrap_or_default();
    Affinity {
        score: score::with_artist(t.song, t.artist),
        own: t.song > 0.0,
    }
}

fn kept(songs: &[Song], playlisted: &HashSet<TrackId>) -> Vec<Kept> {
    let keep = |s: &Song, how| Kept {
        track: s.track.id.clone(),
        artist: s.track.artist.clone(),
        how,
    };
    let saved = songs.iter().filter(|s| s.saved).map(|s| keep(s, Keep::Saved));
    let listed = songs
        .iter()
        .filter(|s| playlisted.contains(&s.track.id))
        .map(|s| keep(s, Keep::Playlisted));
    saved.chain(listed).collect()
}

/// Each take's best song score among the records the listener has of it, and its artist's.
fn take_scores(
    songs: &[Song],
    song_scores: &HashMap<TrackId, f64>,
    artist_scores: &HashMap<ArtistId, f64>,
) -> HashMap<Take, TakeScore> {
    let mut takes: HashMap<Take, TakeScore> = HashMap::new();
    for s in songs {
        let t = takes.entry(take_of(&s.track.artist, &s.track.title)).or_default();
        t.song = t.song.max(song_scores.get(&s.track.id).copied().unwrap_or(0.0));
        t.artist = t
            .artist
            .max(artist_scores.get(&s.track.artist_id).copied().unwrap_or(0.0));
    }
    takes
}

/// An album's score from the songs of it the listener has, with its best artist's.
fn album_scores(index: &Index, takes: &HashMap<Take, TakeScore>) -> HashMap<AlbumKey, Affinity> {
    index
        .albums
        .iter()
        .map(|album| {
            let of: Vec<TakeScore> = album
                .songs
                .iter()
                .filter_map(|&i| takes.get(&index.songs[i].take).copied())
                .collect();
            let songs = score::album(of.iter().map(|t| t.song));
            let artist = of.iter().map(|t| t.artist).fold(0.0, f64::max);
            let a = Affinity {
                score: score::with_artist(songs, artist),
                own: songs > 0.0,
            };
            (album.key.clone(), a)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::personal::index::tests::{song, track};
    use crate::taste::{Mark, Play};
    use std::time::Instant;

    const DAY: i64 = 86_400;
    const NOW: i64 = 1_800_000_000;

    /// A library of `n` songs: artists of two made-up words, four albums each.
    fn library(n: usize) -> (Signals, Vec<Song>) {
        const WORDS: [&str; 24] = [
            "radio", "velvet", "orchard", "glow", "tape", "pale", "harbor", "quiet", "ferns", "silver", "static",
            "river", "paper", "moon", "garden", "echo", "amber", "north", "winter", "signal", "ocean", "dust",
            "violet", "stone",
        ];
        let word = |i: usize| WORDS[i % WORDS.len()];
        let (mut signals, mut songs) = (Signals::default(), Vec::with_capacity(n));
        for i in 0..n {
            let artist = format!("{} {} {}", word(i / 48), word(i / 1152 + 7), i / 48);
            let album = format!("{} {}", word(i / 12 + 3), (i / 12) % 4);
            let title = format!("{} {} {}", word(i), word(i / 7 + 11), i % 12);
            let t = track(&artist, &title, &album, Some(1990 + (i / 12 % 34) as u16), &["rock"]);
            if i % 10 == 0 {
                signals.plays.push(Play {
                    track: t.id.clone(),
                    artist: artist.clone(),
                    at: NOW - (i as i64 % 300) * DAY,
                    played_s: 200,
                    duration_s: 200,
                    completed: true,
                    skipped: false,
                });
            }
            if i % 100 == 0 {
                signals.likes.push(Mark {
                    track: t.id.clone(),
                    artist: artist.clone(),
                    at: NOW - DAY,
                });
            }
            songs.push(song(t, true));
        }
        (signals, songs)
    }

    /// Numbers for docs/context/discovery.md: `cargo test -p lull-discovery --release -- --ignored --nocapture`.
    #[test]
    #[ignore = "a measurement, not a check"]
    fn fifty_thousand_songs_measured() {
        let (signals, songs) = library(50_000);
        let started = Instant::now();
        let snapshot = Snapshot::from_signals(signals, songs, &HashSet::new(), &HashMap::new(), NOW);
        println!(
            "build: {:?} for {} songs",
            started.elapsed(),
            snapshot.index.songs.len()
        );
        for q in [
            "ra",
            "radio velvet",
            "velvet orchard 2003",
            "the",
            "zzzzz",
            "silvr static",
        ] {
            let query = Query::new(q);
            let runs = 20;
            let started = Instant::now();
            let mut found = Found::default();
            for _ in 0..runs {
                found = snapshot.found(&query);
            }
            let asked = started.elapsed() / runs;
            let started = Instant::now();
            let lifted: f64 = found.songs.iter().map(|s| snapshot.song(s).score).sum();
            println!(
                "search {q:?}: {asked:?} for {} songs and {} albums, lifting them {:?} (sum {lifted:.2})",
                found.songs.len(),
                found.albums.len(),
                started.elapsed()
            );
        }
    }
}
