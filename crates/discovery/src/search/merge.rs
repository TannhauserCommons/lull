//! One row per song, album and artist across the catalogs, and their order (ADR-0008).
//!
//! - **Same song:** a shared ISRC, or the same loose artist and title (`ids::loose_*`: feat.,
//!   remaster, version, edit and live tags dropped) with lengths within 3 s. A differing ISRC
//!   doesn't keep two rows apart: remasters and compilations get new ones.
//! - **A row keeps** the identity it was first shown with, and gains the facts others add.
//! - **Order:** rows another source confirms before Last.fm-only ones (scrobbles carry
//!   misspellings), then how well the row matches the query, then popularity (in tenths), then
//!   the listener's own (saved, liked, followed), then exact popularity.
//! - **Fit:** exact, then every word typed, then most of them (a soundtrack named without the
//!   "season" the listener typed), then loose. Words found only in a "(from …)" note count as
//!   most: such a song is from the work the query names, not that work.
//! - **Misspelt artists:** a Last.fm-only artist a typo away from a confirmed one, with under 1%
//!   of its audience, is dropped ("Radiohed" next to Radiohead).

use std::cmp::Ordering;

use lull_core::ids::{AlbumId, ArtistId, TrackId, loose_artist, loose_title, normalize};
use lull_core::model::Track;
use serde::{Deserialize, Serialize};

use crate::sources::{ArtistInfo, Release, Song};
use crate::store;

/// Lengths within this many seconds are the same recording.
const SAME_LENGTH_S: u32 = 3;
/// How far the top result leans toward an artist or a full-length album (an album, a
/// compilation, a soundtrack), then an EP or single, over a track of about the same popularity
/// ("radiohed" is Radiohead, not their best known song; a soundtrack's name is the soundtrack).
const LEAN_ARTIST: f64 = 0.15;
const LEAN_FULL_LENGTH: f64 = 0.15;
const LEAN_RELEASE: f64 = 0.05;
/// Words shorter than this (but not numbers) don't count toward "most of the words": "of" and
/// "the" make "Sign of the Times" no answer to "skronk of the times".
const SIGNIFICANT_LETTERS: usize = 4;
/// A Last.fm-only artist with less than this share of a near-namesake's audience is a typo.
const MISSPELT_SHARE: u64 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Deezer,
    Itunes,
    Library,
    MusicBrainz,
    /// MusicBrainz's release groups by title, for submitted queries.
    #[serde(rename = "musicbrainz-albums")]
    MusicBrainzAlbums,
    LastFm,
}

impl Source {
    /// Its name in messages.
    pub fn service(self) -> &'static str {
        match self {
            Source::Deezer => "Deezer",
            Source::Itunes => "iTunes",
            Source::Library => "The library",
            Source::MusicBrainz | Source::MusicBrainzAlbums => "MusicBrainz",
            Source::LastFm => "Last.fm",
        }
    }
}

/// One source's answer.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Found {
    pub songs: Vec<SongHit>,
    pub albums: Vec<AlbumHit>,
    pub artists: Vec<ArtistHit>,
    /// Part of the answer is missing, and why (the rest is still used).
    pub problem: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SongHit {
    /// The library's record of a saved song: shown as it is, under its own id.
    pub saved: Option<Track>,
    pub title: String,
    pub artist: String,
    /// Empty when the source names no release (Last.fm).
    pub album: String,
    /// The release's artist when it differs from the song's.
    pub album_artist: Option<String>,
    /// "album", "EP", "single", "compilation".
    pub album_kind: Option<String>,
    pub year: Option<u16>,
    pub duration: Option<u32>,
    pub isrcs: Vec<String>,
    pub mbid: Option<String>,
    /// 0..1 (`popularity`).
    pub popularity: f64,
    /// The album's cover: an outside URL where `{size}` is the size in pixels.
    pub art: Option<String>,
    /// Only Last.fm knows it so far.
    #[serde(default)]
    pub lastfm_only: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AlbumHit {
    pub title: String,
    pub artist: String,
    /// "album", "EP", "single", "compilation".
    pub kind: Option<String>,
    /// ISO date, as precise as known.
    pub date: Option<String>,
    /// The release group's MBID.
    pub mbid: Option<String>,
    pub popularity: f64,
    pub art: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ArtistHit {
    pub name: String,
    pub mbid: Option<String>,
    pub popularity: f64,
    /// A photo: an outside URL where `{size}` is the size in pixels.
    pub image: Option<String>,
    /// Fans or listeners, the larger known.
    #[serde(default)]
    pub audience: Option<u64>,
    /// Only Last.fm knows it so far.
    #[serde(default)]
    pub lastfm_only: bool,
}

/// A count (fans, listeners, Deezer rank) on a 0..1 log scale where `full` reaches 1.
pub fn popularity(count: u64, full: f64) -> f64 {
    ((count as f64 + 1.0).log10() / full.log10()).clamp(0.0, 1.0)
}

impl SongHit {
    pub fn song(&self) -> Song {
        Song {
            mbid: self.mbid.clone(),
            title: self.title.clone(),
            artist: self.artist.clone(),
            artist_mbid: None,
            album: self.album.clone(),
            album_artist: self.album_artist.clone(),
            album_mbid: None,
            year: self.year,
            no: None,
            disc: None,
            duration_s: self.duration,
            tags: Vec::new(),
        }
    }

    pub fn id(&self) -> TrackId {
        match &self.saved {
            Some(t) => t.id.clone(),
            None => store::track(self.song()).id,
        }
    }

    /// The album this song is on, as an album row (for its cover and its page). None for a
    /// saved song: the library has its album and cover.
    pub fn album_hit(&self) -> Option<AlbumHit> {
        (self.saved.is_none() && !self.album.is_empty()).then(|| AlbumHit {
            title: self.album.clone(),
            artist: self.album_artist.clone().unwrap_or_else(|| self.artist.clone()),
            kind: self.album_kind.clone(),
            date: self.year.map(|y| y.to_string()),
            mbid: None,
            popularity: 0.0,
            art: self.art.clone(),
        })
    }
}

impl AlbumHit {
    pub fn release(&self) -> Release {
        Release {
            mbid: self.mbid.clone(),
            title: self.title.clone(),
            artist: self.artist.clone(),
            artist_mbid: None,
            kind: self.kind.clone(),
            date: self.date.clone(),
            tags: Vec::new(),
        }
    }

    pub fn id(&self) -> AlbumId {
        store::album(self.release()).id
    }
}

impl ArtistHit {
    pub fn info(&self) -> ArtistInfo {
        ArtistInfo {
            mbid: self.mbid.clone(),
            name: self.name.clone(),
        }
    }

    pub fn id(&self) -> ArtistId {
        ArtistId::of(&self.name)
    }
}

/// Every source's rows merged so far.
#[derive(Debug, Default)]
pub struct Merged {
    pub songs: Vec<SongHit>,
    pub albums: Vec<AlbumHit>,
    pub artists: Vec<ArtistHit>,
    /// Songs before this position have been shown: their identity (and so their id) stays.
    shown_songs: usize,
}

/// Positions of the rows one answer added or improved.
#[derive(Debug, Default, PartialEq)]
pub struct Touched {
    pub songs: Vec<usize>,
    pub albums: Vec<usize>,
    pub artists: Vec<usize>,
}

impl Merged {
    pub fn add(&mut self, found: Found) -> Touched {
        let mut touched = Touched::default();
        for hit in found.songs {
            let at = match self.songs.iter().position(|s| same_song(s, &hit)) {
                Some(i) => {
                    let may_rename = i >= self.shown_songs;
                    fill_song(&mut self.songs[i], hit, may_rename).then_some(i)
                }
                None => {
                    self.songs.push(hit);
                    Some(self.songs.len() - 1)
                }
            };
            touched.songs.extend(at.filter(|i| !touched.songs.contains(i)));
        }
        for hit in found.albums {
            let at = match self.albums.iter().position(|a| same_album(a, &hit)) {
                Some(i) => fill_album(&mut self.albums[i], hit).then_some(i),
                None => {
                    self.albums.push(hit);
                    Some(self.albums.len() - 1)
                }
            };
            touched.albums.extend(at.filter(|i| !touched.albums.contains(i)));
        }
        for hit in found.artists {
            let at = match self.artists.iter().position(|a| same_artist(a, &hit)) {
                Some(i) => fill_artist(&mut self.artists[i], hit).then_some(i),
                None => {
                    self.artists.push(hit);
                    Some(self.artists.len() - 1)
                }
            };
            touched.artists.extend(at.filter(|i| !touched.artists.contains(i)));
        }
        touched
    }

    pub fn is_empty(&self) -> bool {
        self.songs.is_empty() && self.albums.is_empty() && self.artists.is_empty()
    }

    /// Every row so far has been shown: none may change id any more.
    pub fn freeze(&mut self) {
        self.shown_songs = self.songs.len();
    }
}

pub fn same_song(a: &SongHit, b: &SongHit) -> bool {
    let isrc = a.isrcs.iter().any(|i| b.isrcs.contains(i));
    let length = match (a.duration, b.duration) {
        (Some(x), Some(y)) => x.abs_diff(y) <= SAME_LENGTH_S,
        _ => true,
    };
    isrc || (length
        && loose_artist(&a.artist) == loose_artist(&b.artist)
        && loose_title(&a.title) == loose_title(&b.title))
}

fn same_album(a: &AlbumHit, b: &AlbumHit) -> bool {
    loose_artist(&a.artist) == loose_artist(&b.artist) && loose_title(&a.title) == loose_title(&b.title)
}

fn same_artist(a: &ArtistHit, b: &ArtistHit) -> bool {
    loose_artist(&a.name) == loose_artist(&b.name)
}

/// Adds what `hit` knows and `row` doesn't; returns whether anything changed. While the row
/// hasn't been shown, a saved copy or a named release replaces its identity.
fn fill_song(row: &mut SongHit, hit: SongHit, may_rename: bool) -> bool {
    let before = row.clone();
    if may_rename && row.saved.is_none() && hit.saved.is_some() {
        row.saved = hit.saved;
        row.title = hit.title;
        row.artist = hit.artist;
        row.album = hit.album;
        row.album_artist = hit.album_artist;
    } else if may_rename && row.saved.is_none() && row.album.is_empty() && !hit.album.is_empty() {
        row.album = hit.album;
        row.album_artist = hit.album_artist;
        row.album_kind = hit.album_kind.clone();
    }
    row.album_kind = row.album_kind.take().or(hit.album_kind);
    row.year = row.year.or(hit.year);
    row.duration = row.duration.or(hit.duration);
    row.mbid = row.mbid.take().or(hit.mbid);
    row.art = row.art.take().or(hit.art);
    row.popularity = row.popularity.max(hit.popularity);
    row.lastfm_only &= hit.lastfm_only;
    for isrc in hit.isrcs {
        if !row.isrcs.contains(&isrc) {
            row.isrcs.push(isrc);
        }
    }
    *row != before
}

fn fill_album(row: &mut AlbumHit, hit: AlbumHit) -> bool {
    let before = row.clone();
    row.kind = row.kind.take().or(hit.kind);
    row.date = row.date.take().or(hit.date);
    row.mbid = row.mbid.take().or(hit.mbid);
    row.art = row.art.take().or(hit.art);
    row.popularity = row.popularity.max(hit.popularity);
    *row != before
}

fn fill_artist(row: &mut ArtistHit, hit: ArtistHit) -> bool {
    let before = row.clone();
    row.mbid = row.mbid.take().or(hit.mbid);
    row.image = row.image.take().or(hit.image);
    row.popularity = row.popularity.max(hit.popularity);
    row.audience = row.audience.max(hit.audience);
    row.lastfm_only &= hit.lastfm_only;
    *row != before
}

/// How well a row answers the query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Fit {
    /// The source found it (a typo, an alias), but the words differ.
    Loose,
    /// At least two of the typed words, and two thirds of them (not counting short ones like
    /// "of"), start a word of its artist, title or album; or every word does, but only thanks
    /// to a "(from …)" note.
    Most,
    /// Every word typed starts a word of its artist, title or album.
    Words,
    /// The title or name is the query, or the artist is (an artist's songs and albums), or
    /// artist and title together are.
    Exact,
}

/// The query, in the forms rows are compared with.
pub struct Query {
    /// `loose_title` of the query, and the same without a leading "the".
    loose: [String; 2],
    words: Vec<String>,
}

impl Query {
    pub fn new(q: &str) -> Self {
        let loose = loose_title(q);
        let bare = loose.strip_prefix("the ").unwrap_or(&loose).to_owned();
        Query {
            words: normalize(q).split_whitespace().map(str::to_owned).collect(),
            loose: [loose, bare],
        }
    }

    fn is(&self, s: &str) -> bool {
        self.loose.iter().any(|l| l == s)
    }

    fn fit(&self, exact: &[String], text: &[&str]) -> Fit {
        if exact.iter().any(|e| self.is(e)) {
            return Fit::Exact;
        }
        let words_of = |texts: &mut dyn Iterator<Item = String>| -> Vec<String> {
            texts
                .flat_map(|t| normalize(&t).split_whitespace().map(str::to_owned).collect::<Vec<_>>())
                .collect()
        };
        let own = words_of(&mut text.iter().map(|t| without_from_notes(t)));
        let all = words_of(&mut text.iter().map(|t| (*t).to_owned()));
        let holds = |have: &[String], w: &str| have.iter().any(|h| h.starts_with(w));
        if self.words.is_empty() {
            return Fit::Loose;
        }
        if self.words.iter().all(|w| holds(&own, w)) {
            return Fit::Words;
        }
        let significant: Vec<&String> = self.words.iter().filter(|w| is_significant(w)).collect();
        let held = significant.iter().filter(|w| holds(&all, w)).count();
        if self.words.iter().all(|w| holds(&all, w)) || (held >= 2 && held * 3 >= significant.len() * 2) {
            Fit::Most
        } else {
            Fit::Loose
        }
    }

    pub fn song(&self, s: &SongHit) -> Fit {
        let (artist, title) = (loose_artist(&s.artist), loose_title(&s.title));
        let exact = [format!("{artist} {title}"), format!("{title} {artist}"), title, artist];
        self.fit(&exact, &[&s.artist, &s.title, &s.album])
    }

    pub fn album(&self, a: &AlbumHit) -> Fit {
        let (artist, title) = (loose_artist(&a.artist), loose_title(&a.title));
        let exact = [format!("{artist} {title}"), format!("{title} {artist}"), title, artist];
        self.fit(&exact, &[&a.artist, &a.title])
    }

    pub fn artist(&self, a: &ArtistHit) -> Fit {
        self.fit(&[loose_artist(&a.name)], &[&a.name])
    }
}

/// A word that counts toward "most of the words": a number, or four letters or more.
fn is_significant(word: &str) -> bool {
    word.chars().count() >= SIGNIFICANT_LETTERS || word.chars().all(|c| c.is_numeric())
}

/// The text without brackets that say where a song is from: "You Can't Run From Me (from
/// Cyberpunk: Edgerunners Season 2)" → "You Can't Run From Me ".
fn without_from_notes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(open) = rest.find(['(', '[']) {
        let close = if rest[open..].starts_with('(') { ')' } else { ']' };
        let Some(len) = rest[open + 1..].find(close) else { break };
        let inner = rest[open + 1..open + 1 + len].trim_start_matches(|c: char| !c.is_alphanumeric());
        let is_note = inner.get(..5).is_some_and(|w| w.eq_ignore_ascii_case("from "));
        out.push_str(&rest[..open]);
        if !is_note {
            out.push_str(&rest[open..open + len + 2]);
        }
        rest = &rest[open + len + 2..];
    }
    out.push_str(rest);
    out
}

/// What a row is ranked by.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Score {
    /// A source other than Last.fm knows it.
    pub confirmed: bool,
    pub fit: Fit,
    pub popularity: f64,
    /// How far the top result leans toward it over the other kinds (`top`).
    pub lean: f64,
}

impl Score {
    pub fn song(q: &Query, s: &SongHit) -> Self {
        Score {
            confirmed: !s.lastfm_only,
            fit: q.song(s),
            popularity: s.popularity,
            lean: 0.0,
        }
    }

    pub fn album(q: &Query, a: &AlbumHit) -> Self {
        let full_length = a
            .kind
            .as_deref()
            .is_some_and(|k| k.eq_ignore_ascii_case("album") || k.eq_ignore_ascii_case("compilation"));
        Score {
            confirmed: true,
            fit: q.album(a),
            popularity: a.popularity,
            lean: if full_length { LEAN_FULL_LENGTH } else { LEAN_RELEASE },
        }
    }

    pub fn artist(q: &Query, a: &ArtistHit) -> Self {
        Score {
            confirmed: !a.lastfm_only,
            fit: q.artist(a),
            popularity: a.popularity,
            lean: LEAN_ARTIST,
        }
    }
}

/// Positions of `rows` best first. `own` says whether a row is the listener's (saved, liked,
/// a followed artist's).
pub fn rank(rows: &[Score], own: impl Fn(usize) -> bool) -> Vec<usize> {
    let mut order: Vec<usize> = (0..rows.len()).collect();
    let key = |i: usize| {
        let r = rows[i];
        (r.confirmed, r.fit, (r.popularity * 10.0).floor() as i64, own(i))
    };
    order.sort_by(|&a, &b| {
        key(b).cmp(&key(a)).then_with(|| {
            rows[b]
                .popularity
                .partial_cmp(&rows[a].popularity)
                .unwrap_or(Ordering::Equal)
        })
    });
    order
}

/// Which artists are misspellings: Last.fm-only, a typo away from a confirmed artist, and with
/// under 1% of its audience.
pub fn misspelt(artists: &[ArtistHit]) -> Vec<bool> {
    let names: Vec<String> = artists.iter().map(|a| loose_artist(&a.name)).collect();
    artists
        .iter()
        .enumerate()
        .map(|(i, a)| {
            a.lastfm_only
                && artists.iter().enumerate().any(|(j, c)| {
                    !c.lastfm_only
                        && typo_apart(&names[i], &names[j])
                        && match (a.audience, c.audience) {
                            (Some(small), Some(big)) => small.saturating_mul(MISSPELT_SHARE) < big,
                            _ => false,
                        }
                })
        })
        .collect()
}

/// One or two edits apart, and no more than one per four letters ("radiohed", "radiohead").
fn typo_apart(a: &str, b: &str) -> bool {
    let d = edits(a, b);
    d > 0 && d <= 2 && d * 4 <= a.chars().count().max(b.chars().count())
}

/// Levenshtein distance.
fn edits(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let next = (prev + usize::from(ca != *cb)).min(row[j] + 1).min(row[j + 1] + 1);
            prev = row[j + 1];
            row[j + 1] = next;
        }
    }
    row[b.len()]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Artist,
    Album,
    Track,
}

/// The one top result among the best artist, album and track: a confirmed row before a
/// Last.fm-only one, then the best fit, then the most popular, leaning toward artists and albums
/// (full-length ones more than EPs and singles).
pub fn top(artist: Option<Score>, album: Option<Score>, track: Option<Score>) -> Option<Kind> {
    let better = |a: &(Kind, (bool, Fit), f64), b: &(Kind, (bool, Fit), f64)| {
        a.1.cmp(&b.1)
            .then_with(|| a.2.partial_cmp(&b.2).unwrap_or(Ordering::Equal))
            == Ordering::Greater
    };
    [(Kind::Artist, artist), (Kind::Album, album), (Kind::Track, track)]
        .into_iter()
        .filter_map(|(kind, row)| row.map(|r| (kind, (r.confirmed, r.fit), r.popularity + r.lean)))
        .fold(None, |best, c| match best {
            Some(b) if !better(&c, &b) => Some(b),
            _ => Some(c),
        })
        .map(|(kind, ..)| kind)
}
