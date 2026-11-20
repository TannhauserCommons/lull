//! The listener's music in memory: every song they saved, played, liked or put in a playlist,
//! with its album, year and genre, and the albums those are on. It answers search in
//! the shape the catalogs do (`merge::Found`), matched by search's own matcher (`query::Query`),
//! so "ra" finds their Radiohead from the first letters without a call to anyone.
//!
//! A query matches a row by its names (artist, title, album) or, failing that, by its facts
//! (year, genre): "1997" or "trip hop" find the listener's songs from then or of that kind. A
//! row found only by its facts is kept below the rows found by name (`Match`).

use std::collections::{HashMap, HashSet};

use lull_core::ids::{loose_artist, loose_title};
use lull_core::model::Track;

use crate::search::merge::{AlbumHit, SongHit};
use crate::search::query::{Query, folded_words, version_words, word_fits};

/// A song as one take of itself: its loose artist and title and the version words of its title
/// (`merge::same_take`'s rule), so "Creep (Acoustic)" is not "Creep" and a remaster is.
pub type Take = (String, String, String);

/// An album by its loose artist and title (`merge::same_album`'s rule).
pub type AlbumKey = (String, String);

pub fn take_of(artist: &str, title: &str) -> Take {
    (loose_artist(artist), loose_title(title), version_words(title).join(" "))
}

pub fn album_key(artist: &str, title: &str) -> AlbumKey {
    (loose_artist(artist), loose_title(title))
}

/// How a query matched a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Match {
    /// Every word typed starts a word of its facts or names, but not all by names alone.
    Facts,
    Name,
}

/// Every distinct word of the index once, so a query judges each word typed against each word
/// there is once (one edit-distance table per distinct word, not per song).
#[derive(Default)]
struct Vocabulary {
    ids: HashMap<String, u32>,
    words: Vec<String>,
}

impl Vocabulary {
    fn id(&mut self, word: String) -> u32 {
        if let Some(&id) = self.ids.get(&word) {
            return id;
        }
        let id = u32::try_from(self.words.len()).unwrap_or(u32::MAX);
        self.ids.insert(word.clone(), id);
        self.words.push(word);
        id
    }

    /// For each word typed, which words of the index it fits (`word_fits`).
    fn fits(&self, q: &Query) -> Vec<Vec<bool>> {
        q.words()
            .iter()
            .map(|typed| self.words.iter().map(|w| word_fits(typed, w)).collect())
            .collect()
    }
}

/// The words a row is searched by: its names first, then its facts.
#[derive(Debug, Default)]
struct Words {
    all: Vec<u32>,
    names: usize,
}

impl Words {
    fn of(vocabulary: &mut Vocabulary, names: &[&str], facts: &[String]) -> Words {
        let mut all: Vec<u32> = names
            .iter()
            .flat_map(|n| folded_words(n))
            .map(|w| vocabulary.id(w))
            .collect();
        let names = all.len();
        all.extend(facts.iter().flat_map(|f| folded_words(f)).map(|w| vocabulary.id(w)));
        Words { all, names }
    }

    /// Every word typed fits one of `have` (`Query::holds_every_word`'s rule).
    fn holds_every(fits: &[Vec<bool>], have: &[u32]) -> bool {
        !fits.is_empty() && fits.iter().all(|f| have.iter().any(|&w| f[w as usize]))
    }

    fn matched_by(&self, fits: &[Vec<bool>]) -> Option<Match> {
        if Words::holds_every(fits, &self.all[..self.names]) {
            Some(Match::Name)
        } else {
            Words::holds_every(fits, &self.all).then_some(Match::Facts)
        }
    }
}

/// What the listener has of one song.
pub struct Song {
    pub track: Track,
    /// The saved library's own record of it: shown as it is, under its own id.
    pub saved: bool,
    /// The album's artist when it differs from the song's.
    pub album_artist: Option<String>,
}

pub struct Entry {
    pub hit: SongHit,
    pub take: Take,
    words: Words,
}

pub struct AlbumEntry {
    pub hit: AlbumHit,
    pub key: AlbumKey,
    /// The songs of it the listener has (positions in `Index::songs`).
    pub songs: Vec<usize>,
    words: Words,
}

#[derive(Default)]
pub struct Index {
    pub songs: Vec<Entry>,
    pub albums: Vec<AlbumEntry>,
    vocabulary: Vocabulary,
}

impl Index {
    /// The index of `songs` (a take held twice is one, the saved copy first). It holds no artist
    /// rows: an artist is reached through a catalog's row and its ids (`score::by_catalog_id`),
    /// never by a name this index could lend to a namesake's row.
    pub fn build(songs: Vec<Song>) -> Index {
        let mut index = Index::default();
        let mut at: HashSet<Take> = HashSet::new();
        let mut songs = songs;
        songs.sort_by_key(|s| !s.saved);
        for song in songs {
            let entry = entry(song, &mut index.vocabulary);
            if at.insert(entry.take.clone()) {
                index.songs.push(entry);
            }
        }
        index.albums = albums(&index.songs, &mut index.vocabulary);
        index
    }

    /// The songs the query matches (positions in `songs`), and how.
    pub fn songs(&self, q: &Query) -> Vec<(usize, Match)> {
        let fits = self.vocabulary.fits(q);
        self.songs
            .iter()
            .enumerate()
            .filter_map(|(i, e)| Some((i, e.words.matched_by(&fits)?)))
            .collect()
    }

    pub fn albums(&self, q: &Query) -> Vec<(usize, Match)> {
        let fits = self.vocabulary.fits(q);
        self.albums
            .iter()
            .enumerate()
            .filter_map(|(i, e)| Some((i, e.words.matched_by(&fits)?)))
            .collect()
    }
}

fn entry(song: Song, vocabulary: &mut Vocabulary) -> Entry {
    let Song {
        track,
        saved,
        album_artist,
    } = song;
    let facts: Vec<String> = track
        .year
        .map(|y| y.to_string())
        .into_iter()
        .chain(track.tags.clone())
        .collect();
    let words = Words::of(
        vocabulary,
        &[
            &track.artist,
            &track.title,
            &track.album,
            album_artist.as_deref().unwrap_or_default(),
        ],
        &facts,
    );
    let hit = SongHit {
        title: track.title.clone(),
        artist: track.artist.clone(),
        album: track.album.clone(),
        album_artist: album_artist.filter(|a| !a.is_empty() && loose_artist(a) != loose_artist(&track.artist)),
        year: track.year,
        duration: track.duration,
        saved: saved.then(|| track.clone()),
        ..SongHit::default()
    };
    Entry {
        take: take_of(&track.artist, &track.title),
        hit,
        words,
    }
}

/// The albums the songs are on, each with the songs of it the listener has.
fn albums(songs: &[Entry], vocabulary: &mut Vocabulary) -> Vec<AlbumEntry> {
    let mut found: Vec<AlbumEntry> = Vec::new();
    let mut at: HashMap<AlbumKey, usize> = HashMap::new();
    for (i, s) in songs.iter().enumerate().filter(|(_, s)| !s.hit.album.is_empty()) {
        let artist = s.hit.album_artist.as_deref().unwrap_or(&s.hit.artist);
        let key = album_key(artist, &s.hit.album);
        match at.get(&key) {
            Some(&a) => {
                let album = &mut found[a];
                album.songs.push(i);
                album.hit.date = album.hit.date.take().or_else(|| s.hit.year.map(|y| y.to_string()));
            }
            None => {
                let facts: Vec<String> = s.hit.year.map(|y| y.to_string()).into_iter().collect();
                at.insert(key.clone(), found.len());
                found.push(AlbumEntry {
                    hit: AlbumHit {
                        title: s.hit.album.clone(),
                        artist: artist.to_owned(),
                        date: s.hit.year.map(|y| y.to_string()),
                        ..AlbumHit::default()
                    },
                    key,
                    songs: vec![i],
                    words: Words::of(vocabulary, &[artist, &s.hit.album], &facts),
                });
            }
        }
    }
    found
}

#[cfg(test)]
pub(in crate::search) mod tests {
    use super::*;
    use lull_core::ids::{AlbumId, ArtistId, TrackId};

    pub fn track(artist: &str, title: &str, album: &str, year: Option<u16>, tags: &[&str]) -> Track {
        Track {
            id: TrackId::of(artist, album, title),
            title: title.into(),
            artist_id: ArtistId::of(artist),
            artist: artist.into(),
            album_id: AlbumId::of(artist, album),
            album: album.into(),
            year,
            no: None,
            disc: None,
            duration: Some(200),
            tags: tags.iter().map(|t| (*t).to_owned()).collect(),
            saved: false,
            file: None,
        }
    }

    pub fn song(track: Track, saved: bool) -> Song {
        Song {
            track,
            saved,
            album_artist: None,
        }
    }

    fn titles(index: &Index, q: &str) -> Vec<(String, Match)> {
        let q = Query::new(q);
        index
            .songs(&q)
            .into_iter()
            .map(|(i, m)| (index.songs[i].hit.title.clone(), m))
            .collect()
    }

    fn listener() -> Index {
        Index::build(vec![
            song(
                track(
                    "Radiohead",
                    "Paranoid Android",
                    "OK Computer",
                    Some(1997),
                    &["alternative"],
                ),
                true,
            ),
            song(track("Radiohead", "Karma Police", "OK Computer", Some(1997), &[]), true),
            song(
                track("Radiohead", "Creep (Acoustic)", "Pablo Honey", Some(1993), &[]),
                false,
            ),
            song(
                track("Massive Attack", "Teardrop", "Mezzanine", Some(1998), &["trip hop"]),
                true,
            ),
            song(track("Björk", "Hyperballad", "Post", Some(1995), &[]), false),
        ])
    }

    #[test]
    fn the_first_letters_of_an_artist_find_their_songs() {
        let found = titles(&listener(), "ra");
        assert_eq!(found.len(), 3, "{found:?}");
        assert!(found.iter().all(|(_, m)| *m == Match::Name));
    }

    #[test]
    fn a_typo_and_an_accent_still_find_the_song() {
        assert_eq!(titles(&listener(), "radiohed karma").len(), 1);
        assert_eq!(
            titles(&listener(), "bjork").len(),
            1,
            "Björk is typed without the accent"
        );
    }

    #[test]
    fn a_year_or_a_genre_finds_songs_but_only_as_facts() {
        let by_year = titles(&listener(), "1997");
        assert_eq!(by_year.len(), 2);
        assert!(by_year.iter().all(|(_, m)| *m == Match::Facts));
        let by_genre = titles(&listener(), "trip hop");
        assert_eq!(by_genre, [("Teardrop".to_owned(), Match::Facts)]);
        let both = titles(&listener(), "radiohead 1997");
        assert_eq!(both.len(), 2, "the songs of 1997, not Pablo Honey's: {both:?}");
        assert!(both.iter().all(|(t, m)| t != "Creep (Acoustic)" && *m == Match::Facts));
    }

    #[test]
    fn a_take_held_twice_is_one_and_the_saved_copy_is_kept() {
        let t = track("Radiohead", "Karma Police", "OK Computer", Some(1997), &[]);
        let single = track("Radiohead", "Karma Police", "Karma Police", Some(1997), &[]);
        let index = Index::build(vec![song(single, false), song(t, true)]);
        assert_eq!(index.songs.len(), 1);
        assert!(index.songs[0].hit.saved.is_some());
    }

    #[test]
    fn an_album_holds_the_songs_of_it_the_listener_has() {
        let index = listener();
        let ok = index
            .albums
            .iter()
            .find(|a| a.hit.title == "OK Computer")
            .map(|a| (a.songs.len(), a.hit.artist.clone(), a.hit.date.clone()));
        assert_eq!(ok, Some((2, "Radiohead".into(), Some("1997".into()))));
        let q = Query::new("ok computer");
        assert_eq!(index.albums(&q).len(), 1);
    }

    #[test]
    fn a_take_is_its_artist_title_and_version_words() {
        assert_ne!(take_of("Radiohead", "Creep"), take_of("Radiohead", "Creep (Acoustic)"));
        assert_eq!(
            take_of("Radiohead", "Creep"),
            take_of("The Radiohead", "Creep (2009 Remaster)")
        );
    }
}
