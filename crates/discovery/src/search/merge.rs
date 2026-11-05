//! One row per song, album and artist across the catalogs, and their order (ADR-0008).
//!
//! - **Same song:** a shared ISRC, or the same loose artist and title (`ids::loose_*`: feat.,
//!   remaster, version, edit and live tags dropped) with lengths within 3 s. A differing ISRC
//!   doesn't keep two rows apart: remasters and compilations get new ones.
//! - **A row keeps** the identity it was first shown with, and gains the facts others add.
//! - **Order:** how well the row matches the query, then the song itself before its
//!   live takes, remixes and covers' karaoke, then popularity (in tenths), then the listener's
//!   own (saved, liked, followed), then exact popularity.
//! - **Fit:** exact, then every word typed, then most of them (a soundtrack named without the
//!   "season" the listener typed), then one of them, then loose; accents folded and typos
//!   forgiven ("beyonse halo"). Words found only in a "(from …)" note count as most: such a song
//!   is from the work the query names, not that work.
//! - **Well-known songs:** where the catalog gives no counts (iTunes), a song its own artists
//!   have on more releases and takes in the answers is the better known (`known`).
//! - **Borrowed names:** an artist whose name holds another's ("Radiohead - Topic", "SZA with
//!   Kendrick Lamar"), with under 1% of its audience, is dropped.
//! - **Famous near matches:** an artist with under a tenth of the audience of another artist
//!   that holds every word typed (or is a typo away) ranks by popularity with it, its better fit
//!   aside: "mozart" is Wolfgang Amadeus Mozart before a pop act called Mozart.

use std::cmp::Ordering;

use lull_core::ids::{AlbumId, ArtistId, TrackId, loose_artist, loose_title, normalize};
use lull_core::model::Track;
use serde::{Deserialize, Serialize};

use crate::sources::{AlbumSource, ArtistInfo, ArtistSource, Release, Song};
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
/// An artist whose name holds another's, with less than 1/this of its audience, borrows it.
const BORROWED_SHARE: u64 = 100;
/// An artist with less than 1/this of a famous near match's audience ranks with it.
const OVERSHADOWED_SHARE: u64 = 10;
/// A typed word may hold one typo in this many letters (and is this long at least); the whole
/// query one in this many, so a short word never passes for another.
const MISSPELT_WORD_LETTERS: usize = 5;
const MISSPELT_PHRASE_LETTERS: usize = 8;
/// Words in a bracket or " - " suffix that make a title another performance or mix of the song
/// it names ("(Live)", "[Daft Punk Remix]", "(Acoustic)", "- Radio Edit", "[Midland Version]",
/// "(Originally Performed By Adele)", "(Lullaby Rendition of Queen)"). Remasters and "feat."
/// are the same recording (`ids::loose_title` drops them). Search and radio read this one list.
const VERSION_TAGS: &[&str] = &[
    "live",
    "remix",
    "remixes",
    "mashup",
    "remixed",
    "rmx",
    "mix",
    "dub",
    "edit",
    "acoustic",
    "unplugged",
    "instrumental",
    "demo",
    "karaoke",
    "cover",
    "performed",
    "tribute",
    "rendition",
    "lullaby",
    "acapella",
    "cappella",
    "reprise",
    "rework",
    "slowed",
    "sped",
    "lofi",
    "take",
    "rehearsal",
    "version",
];
/// Each further take of a song by its own artist in the answers (a deluxe edition, a best-of, a
/// live take, an official remix) closes this share of the gap between its popularity and full
/// (`known`).
const TAKE_SHARE: f64 = 0.15;

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
    /// Deezer's playlist search (keyless, in every country).
    #[serde(rename = "deezer-playlists")]
    DeezerPlaylists,
    /// YouTube's playlist search, through yt-dlp.
    #[serde(rename = "youtube-playlists")]
    YoutubePlaylists,
    /// YouTube Music's search, for what the catalog lacks (`youtube_music`).
    #[serde(rename = "youtube-music")]
    YoutubeMusic,
}

impl Source {
    /// Its name in messages.
    pub fn service(self) -> &'static str {
        match self {
            Source::Deezer => "Deezer",
            Source::Itunes => "iTunes",
            Source::Library => "The library",
            Source::MusicBrainz | Source::MusicBrainzAlbums => "MusicBrainz",
            Source::DeezerPlaylists => "Deezer's playlists",
            Source::YoutubePlaylists => "YouTube's playlists",
            Source::YoutubeMusic => "YouTube Music",
        }
    }
}

/// One source's answer.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Found {
    pub songs: Vec<SongHit>,
    pub albums: Vec<AlbumHit>,
    pub artists: Vec<ArtistHit>,
    #[serde(default)]
    pub playlists: Vec<PlaylistHit>,
    /// Part of the answer is missing, and why (the rest is still used).
    pub problem: Option<String>,
    /// A gap-filler's rows beyond its own best answer (YouTube Music's list after its top
    /// result): they rank after the catalog's rows of the same fit (`fill_gaps`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fillers: Option<Box<Found>>,
}

/// A playlist someone made on YouTube or Deezer (search's Playlists section, `playlists.rs`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PlaylistHit {
    /// `youtube` or `deezer`.
    pub site: String,
    /// The site's id for it.
    pub id: String,
    pub title: String,
    pub owner: String,
    /// Its picture: an outside URL.
    pub cover: Option<String>,
    /// How many songs, and how many follow it (Deezer's fans), when the site says.
    pub tracks: Option<u32>,
    pub fans: Option<u64>,
    /// Its place in the site's own order (0 = first), which weighs relevance and popularity.
    pub rank: u32,
}

impl PlaylistHit {
    /// `youtube:playlist:<id>`: the key link imports and bookmarks use.
    pub fn link(&self) -> String {
        format!("{}:playlist:{}", self.site, self.id)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SongHit {
    /// The library's record of a saved song: shown as it is, under its own id.
    pub saved: Option<Track>,
    pub title: String,
    pub artist: String,
    /// Empty when the source names no release.
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
    /// The album in the catalog that found the song, for its tracklist.
    #[serde(default)]
    pub album_source: Option<AlbumSource>,
    /// The catalog's own id for the song's artist (Apple's `artistId`, Deezer's artist id), from
    /// the row the source gave, never from the name.
    #[serde(default)]
    pub artist_source: Option<ArtistSource>,
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
    /// The album in the catalog that found it, for its tracklist.
    #[serde(default)]
    pub source: Option<AlbumSource>,
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
    /// The catalogs' own ids for the artist, from the rows they gave, never from the name.
    #[serde(default)]
    pub sources: Vec<ArtistSource>,
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
            artist_source: self.artist_source,
            isrc: self.isrcs.first().cloned(),
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
            source: self.album_source,
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
    pub playlists: Vec<PlaylistHit>,
    /// Songs before this position have been shown: their identity (and so their id) stays.
    shown_songs: usize,
    /// Songs and albums only a gap-filler's list has (`fill_gaps`).
    filler_songs: Vec<usize>,
    filler_albums: Vec<usize>,
}

/// Positions of the rows one answer added or improved.
#[derive(Debug, Default, PartialEq)]
pub struct Touched {
    pub songs: Vec<usize>,
    pub albums: Vec<usize>,
    pub artists: Vec<usize>,
    pub playlists: Vec<usize>,
}

impl Touched {
    /// Both answers' positions.
    pub fn and(mut self, other: Touched) -> Touched {
        self.songs.extend(other.songs);
        self.albums.extend(other.albums);
        self.artists.extend(other.artists);
        self.playlists.extend(other.playlists);
        self
    }
}

impl Merged {
    pub fn add(&mut self, found: Found) -> Touched {
        let mut touched = Touched::default();
        for hit in found.songs {
            let at = match self.songs.iter().position(|s| same_take(s, &hit)) {
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
        for hit in found.playlists {
            if !self.playlists.iter().any(|p| p.link() == hit.link()) {
                self.playlists.push(hit);
                touched.playlists.push(self.playlists.len() - 1);
            }
        }
        touched
    }

    /// Every row so far has been shown: none may change id any more.
    pub fn freeze(&mut self) {
        self.shown_songs = self.songs.len();
    }
}

/// What a gap-filler (YouTube Music) brings: its songs and albums that aren't the same song or
/// album as one the catalog already gave are added; for one that is, the catalog's row keeps its
/// identity and facts, and only takes the gap-filler's play count when that is higher, as the
/// catalog's own order is a guess (iTunes) and plays are a count ("hello" is Adele's, 2B plays,
/// not a namesake YouTube Music put first for the region asking). Artists and playlists are left
/// out.
pub fn fill_gaps(m: &mut Merged, mut found: Found) -> Touched {
    let fillers = found.fillers.take();
    let best = add_gaps(m, found);
    let Some(fillers) = fillers else { return best };
    let (songs, albums) = (m.songs.len(), m.albums.len());
    let listed = add_gaps(m, *fillers);
    m.filler_songs.extend(songs..m.songs.len());
    m.filler_albums.extend(albums..m.albums.len());
    best.and(listed)
}

fn add_gaps(m: &mut Merged, mut found: Found) -> Touched {
    found
        .songs
        .retain(|s| match m.songs.iter_mut().find(|row| same_take(row, s)) {
            Some(row) => {
                row.popularity = row.popularity.max(s.popularity);
                false
            }
            None => true,
        });
    found.albums.retain(|a| !m.albums.iter().any(|row| same_album(row, a)));
    found.artists.clear();
    found.playlists.clear();
    m.add(found)
}

impl Merged {
    /// Only a gap-filler's list has this song (`fill_gaps`).
    pub fn filler_song(&self, i: usize) -> bool {
        self.filler_songs.contains(&i)
    }

    pub fn filler_album(&self, i: usize) -> bool {
        self.filler_albums.contains(&i)
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

/// `same_song`, and where a length is missing (YouTube Music gives none, a playlist line may
/// not), the same take too: a remix, an edit or a live take is never the song itself ("Shelter
/// (Them Jeans drum edit)" is not "Shelter"), so it neither merges into its row, lends it its
/// plays, nor matches it for a link import. A shared ISRC is the same recording whatever the tags.
pub fn same_take(a: &SongHit, b: &SongHit) -> bool {
    let isrc = a.isrcs.iter().any(|i| b.isrcs.contains(i));
    isrc || (same_song(a, b)
        && ((a.duration.is_some() && b.duration.is_some()) || version_words(&a.title) == version_words(&b.title)))
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
    // The catalog's album id goes with the release it names.
    if row.album_source.is_none() && loose_title(&row.album) == loose_title(&hit.album) {
        row.album_source = hit.album_source;
    }
    let names_version = |s: &SongHit| names_a_version(&s.title) || !release_versions(&s.album).is_empty();
    if may_rename && row.saved.is_none() && hit.saved.is_some() {
        row.saved = hit.saved;
        row.title = hit.title;
        row.artist = hit.artist;
        row.album = hit.album;
        row.album_artist = hit.album_artist;
    } else if may_rename && row.saved.is_none() && names_version(row) && !names_version(&hit) {
        // The song itself, not its live or remixed release ("Dynamite (Live)" on a concert).
        row.title = hit.title;
        row.artist = hit.artist;
        row.artist_source = hit.artist_source.or(row.artist_source);
        row.album = hit.album;
        row.album_artist = hit.album_artist;
        row.album_kind = hit.album_kind.clone();
        row.album_source = hit.album_source;
        row.art = hit.art.clone();
    } else if may_rename && row.saved.is_none() && row.album.is_empty() && !hit.album.is_empty() {
        row.album = hit.album;
        row.album_artist = hit.album_artist;
        row.album_kind = hit.album_kind.clone();
        row.album_source = hit.album_source;
    }
    row.album_kind = row.album_kind.take().or(hit.album_kind);
    // The same song by the same loose artist: another catalog's id for that artist.
    row.artist_source = row.artist_source.or(hit.artist_source);
    row.year = row.year.or(hit.year);
    row.duration = row.duration.or(hit.duration);
    row.mbid = row.mbid.take().or(hit.mbid);
    row.art = row.art.take().or(hit.art);
    row.popularity = row.popularity.max(hit.popularity);
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
    row.source = row.source.or(hit.source);
    row.popularity = row.popularity.max(hit.popularity);
    *row != before
}

fn fill_artist(row: &mut ArtistHit, hit: ArtistHit) -> bool {
    let before = row.clone();
    row.mbid = row.mbid.take().or(hit.mbid);
    row.image = row.image.take().or(hit.image);
    row.popularity = row.popularity.max(hit.popularity);
    row.audience = row.audience.max(hit.audience);
    for source in hit.sources {
        if !row.sources.contains(&source) {
            row.sources.push(source);
        }
    }
    *row != before
}

/// How well a row answers the query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Fit {
    /// The source found it (a typo, an alias, a name in another script), but the words differ.
    Loose,
    /// One of the words that count (four letters or more, or a number) starts a word of it:
    /// "lemon" of "米津玄師 lemon", whose artist is written in another script.
    Partly,
    /// At least two of the typed words, and two thirds of them (not counting short ones like
    /// "of"), start a word of its artist, title or album; or every word does, but only thanks
    /// to a "(from …)" note.
    Most,
    /// Every word typed starts a word of its artist, title or album (or is one with a typo).
    Words,
    /// The title or name is the query (or it with a typo), or the artist is (an artist's songs
    /// and albums), or artist and title together are; or the title begins with the words typed
    /// ("smells like teen"), or they are an artist's name and words of the title ("nirvana
    /// unplugged").
    Exact,
}

/// The query, in the forms rows are compared with.
pub struct Query {
    /// `loose_title` of the query, and the same without a leading "the".
    loose: [String; 2],
    /// The words typed, accents folded.
    words: Vec<String>,
}

impl Query {
    pub fn new(q: &str) -> Self {
        let loose = loose_title(q);
        let bare = loose.strip_prefix("the ").unwrap_or(&loose).to_owned();
        Query {
            words: folded_words(q),
            loose: [loose, bare],
        }
    }

    /// The query is `s`, a leading "the" aside on either side ("dark side of the moon" is
    /// "The Dark Side of the Moon"), or a typo away from it ("radiohed creep").
    fn is(&self, s: &str) -> bool {
        let bare = s.strip_prefix("the ").unwrap_or(s);
        self.loose
            .iter()
            .any(|l| l == s || l == bare || misspelt_phrase(l, s) || misspelt_phrase(l, bare))
    }

    /// `s` begins with the whole words typed: a title being typed ("smells like teen", "stairway
    /// to"). Only for two words or more, as one word starts too many titles.
    fn begins(&self, s: &str) -> bool {
        self.words.len() >= 2
            && self.loose.iter().any(|l| {
                s.strip_prefix(l.as_str())
                    .or_else(|| s.strip_prefix("the ").and_then(|b| b.strip_prefix(l.as_str())))
                    .is_some_and(|rest| rest.starts_with(' '))
            })
    }

    /// `exact`: the forms that fit exactly when they are the words typed; `titled`: the forms
    /// that also fit exactly when they begin with them.
    fn fit(&self, exact: &[String], titled: &[String], text: &[&str]) -> Fit {
        if exact.iter().any(|e| self.is(e)) || titled.iter().any(|t| self.is(t) || self.begins(t)) {
            return Fit::Exact;
        }
        let own: Vec<String> = text.iter().flat_map(|t| folded_words(&without_from_notes(t))).collect();
        let all: Vec<String> = text.iter().flat_map(|t| folded_words(t)).collect();
        let holds = |have: &[String], w: &str| have.iter().any(|h| h.starts_with(w) || misspelt_word(w, h));
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
        } else if held >= 1 {
            Fit::Partly
        } else {
            Fit::Loose
        }
    }

    pub fn song(&self, s: &SongHit) -> Fit {
        self.named(&s.artist, &s.title, &[&s.artist, &s.title, &s.album])
    }

    pub fn album(&self, a: &AlbumHit) -> Fit {
        self.named(&a.artist, &a.title, &[&a.artist, &a.title])
    }

    /// A song's or album's fit: exact also when the words are a credited artist's name and words
    /// of the title ("nirvana unplugged" is Nirvana's "MTV Unplugged In New York").
    fn named(&self, artist: &str, title: &str, text: &[&str]) -> Fit {
        let names: Vec<String> = crate::radio::credited_names(artist)
            .iter()
            .map(|n| loose_artist(n))
            .collect();
        let (exact, titled) = forms(&names, title);
        if self.artist_and_title_words(&names, title) {
            Fit::Exact
        } else {
            self.fit(&exact, &titled, text)
        }
    }

    /// The words are a song's credited artist and words of its title ("lose yourself eminem"):
    /// they name that song.
    pub fn names_artist_and_title(&self, s: &SongHit) -> bool {
        let names: Vec<String> = crate::radio::credited_names(&s.artist)
            .iter()
            .map(|n| loose_artist(n))
            .collect();
        self.artist_and_title_words(&names, &s.title)
    }

    fn artist_and_title_words(&self, names: &[String], title: &str) -> bool {
        let title_words = folded_words(title);
        names.iter().filter(|n| !n.is_empty()).any(|n| {
            self.loose.iter().any(|l| {
                let rest = l
                    .strip_prefix(n.as_str())
                    .and_then(|r| r.strip_prefix(' '))
                    .or_else(|| l.strip_suffix(n.as_str()).and_then(|r| r.strip_suffix(' ')));
                rest.is_some_and(|r| {
                    folded_words(r).iter().all(|w| {
                        title_words
                            .iter()
                            .any(|h| h.starts_with(w.as_str()) || misspelt_word(w, h))
                    })
                })
            })
        })
    }

    pub fn artist(&self, a: &ArtistHit) -> Fit {
        self.fit(&[loose_artist(&a.name)], &[], &[&a.name])
    }

    /// The words (of a title or a release, `version_words`) make another version than the
    /// words typed name: a live take, a remix, an acoustic or instrumental one, a cover's
    /// "(Originally Performed By …)", unless the listener typed that word ("hotel california
    /// live").
    fn other_version(&self, words: &[String]) -> bool {
        let typed = |w: &String| {
            self.words.contains(w)
                || (self.words.iter().any(|t| t == "unplugged") && matches!(w.as_str(), "live" | "acoustic"))
        };
        words.iter().any(|w| !typed(w))
    }
}

/// The forms of a song or an album that are what someone types for it: each credited artist
/// (`names`: "Daft Punk, Pharrell Williams & Nile Rodgers" is also Daft Punk's) with the title
/// either side, the artist alone (their songs and albums), the title alone; titles also without
/// the tags of another version, as the version is ranked apart (`Score::version`). The second
/// list holds the forms that start with the title, which fit when they begin with the words.
fn forms(names: &[String], title: &str) -> (Vec<String>, Vec<String>) {
    let full = loose_title(title);
    let plain = loose_title(&without_versions(title).0);
    let titles: Vec<&String> = if plain == full {
        vec![&full]
    } else {
        vec![&full, &plain]
    };
    let mut exact = Vec::new();
    let mut titled: Vec<String> = titles.iter().map(|t| (*t).clone()).collect();
    for name in names {
        for t in &titles {
            titled.push(format!("{name} {t}"));
            exact.push(format!("{t} {name}"));
        }
        exact.push(name.clone());
    }
    (exact, titled)
}

/// A word that counts toward "most of the words": a number, or four letters or more.
fn is_significant(word: &str) -> bool {
    word.chars().count() >= SIGNIFICANT_LETTERS || word.chars().all(|c| c.is_numeric())
}

/// The words of `s`, accents folded ("Beyoncé" holds "beyonce").
fn folded_words(s: &str) -> Vec<String> {
    normalize(s)
        .split(' ')
        .map(loose_title)
        .filter(|w| !w.is_empty())
        .collect()
}

/// A word typed with a typo of `word` ("beyonse", "fleetwod", "elish"): one edit in five
/// letters, two at most, and never for words under five letters ("halo" isn't "hall").
fn misspelt_word(typed: &str, word: &str) -> bool {
    let len = typed.chars().count();
    len >= MISSPELT_WORD_LETTERS && {
        let d = edits(typed, word);
        d > 0 && d <= 2 && d * MISSPELT_WORD_LETTERS <= len.max(word.chars().count())
    }
}

/// What was typed is `s` with a typo ("bohemian rapsody", "radiohed creep"): one edit in eight
/// letters, two at most, so short words never pass for others ("creep" isn't "creed").
fn misspelt_phrase(typed: &str, s: &str) -> bool {
    let d = edits(typed, s);
    d > 0 && d <= 2 && d * MISSPELT_PHRASE_LETTERS <= typed.chars().count()
}

/// The words that make a title another version of the song it names: the version tags of its
/// brackets and " - " suffix ("Talamak (Tom Lown Remix)", "So Many Details - Live at KEXP"),
/// never its own words ("Live and Let Die", "Dub Be Good to Me").
fn version_words(title: &str) -> Vec<String> {
    without_versions(title).1
}

/// Whether a title names another take of a song (`version_words`).
pub fn names_a_version(title: &str) -> bool {
    !version_words(title).is_empty()
}

/// The words that make a release another version of its songs: its tags, and "live" for a
/// concert ("Live at Reading", "Live At SoFi Stadium").
fn release_versions(release: &str) -> Vec<String> {
    let mut words = version_words(release);
    if normalize(release).starts_with("live ") {
        words.push("live".into());
    }
    words
}

/// The text without its brackets and " - " suffixes that name another version of the song, and
/// the version words they held: "Blinding Lights (Live)" is ("Blinding Lights", ["live"]).
fn without_versions(s: &str) -> (String, Vec<String>) {
    let mut found = Vec::new();
    let mut names_version = |part: &str| {
        let before = found.len();
        found.extend(
            normalize(part)
                .split(' ')
                .filter(|w| VERSION_TAGS.contains(w))
                .map(str::to_owned),
        );
        found.len() > before
    };
    let s = s.replace(" – ", " - ").replace(" — ", " - ");
    let mut kept = String::with_capacity(s.len());
    let mut rest = s.as_str();
    while let Some(open) = rest.find(['(', '[', '{']) {
        let close = match rest.as_bytes()[open] {
            b'(' => ')',
            b'[' => ']',
            _ => '}',
        };
        let Some(len) = rest[open + 1..].find(close) else {
            break;
        };
        kept.push_str(&rest[..open]);
        if !names_version(&rest[open + 1..open + 1 + len]) {
            kept.push_str(&rest[open..open + len + 2]);
        }
        rest = &rest[open + len + 2..];
    }
    kept.push_str(rest);
    let mut parts = kept.split(" - ");
    let mut plain = parts.next().unwrap_or_default().to_owned();
    for part in parts {
        if names_version(part) {
            break;
        }
        plain.push_str(" - ");
        plain.push_str(part);
    }
    (plain, found)
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

/// Songs found for their album's name ("Brain Damage" for "dark side of the moon": Apple's song
/// search matches album names too) lend that album their popularity, as the album is what the
/// catalog found; an album only known from a song otherwise has a share of it.
pub fn credit_albums(q: &Query, found: &mut Found) {
    for s in &found.songs {
        let Some(album) = s.album_hit() else { continue };
        let album_fit = q.album(&album);
        let own_fit = q.song(&SongHit {
            album: String::new(),
            ..s.clone()
        });
        if album_fit >= Fit::Words && album_fit > own_fit {
            for a in found.albums.iter_mut().filter(|a| same_album(a, &album)) {
                a.popularity = a.popularity.max(s.popularity);
            }
        }
    }
}

/// What a row is ranked by.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Score {
    pub fit: Fit,
    /// Another version than the words name (`Query::other_version`): it goes after the song.
    pub version: bool,
    /// Only a gap-filler's list has it: it goes after the catalog's rows of the same fit.
    pub filler: bool,
    pub popularity: f64,
    /// How far the top result leans toward it over the other kinds (`top`).
    pub lean: f64,
}

impl Score {
    pub fn song(q: &Query, s: &SongHit) -> Self {
        Score {
            fit: q.song(s),
            version: q.other_version(&[version_words(&s.title), release_versions(&s.album)].concat()),
            filler: false,
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
            fit: q.album(a),
            version: q.other_version(&release_versions(&a.title)),
            filler: false,
            popularity: a.popularity,
            lean: if full_length { LEAN_FULL_LENGTH } else { LEAN_RELEASE },
        }
    }

    pub fn artist(q: &Query, a: &ArtistHit) -> Self {
        Score {
            fit: q.artist(a),
            version: false,
            filler: false,
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
        (
            r.fit,
            !r.version,
            !r.filler,
            (r.popularity * 10.0).floor() as i64,
            own(i),
        )
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

/// Raises each song's popularity by the other takes of it its own artists have in the answers:
/// the catalogs' sign of a well-known song where they give no counts (iTunes). The Weeknd's
/// "Blinding Lights" is on After Hours, its deluxe edition, The Highlights, and has official
/// remixes and an instrumental; a cover has its single. Takes are the song's and the album rows'
/// titles without version tags, by an artist the song credits, counted once per release (a
/// single is its song).
pub fn known(m: &Merged, scores: &mut [Score]) {
    struct Take {
        names: Vec<String>,
        title: String,
        release: String,
    }
    let take = |artist: &str, title: &str, release: &str| Take {
        names: crate::radio::credited_names(artist)
            .iter()
            .map(|n| loose_artist(n))
            .collect(),
        title: loose_title(&without_versions(title).0),
        release: normalize(release),
    };
    let takes: Vec<Take> = m
        .songs
        .iter()
        .map(|s| {
            take(
                &s.artist,
                &s.title,
                if s.album.is_empty() { &s.title } else { &s.album },
            )
        })
        .chain(m.albums.iter().map(|a| take(&a.artist, &a.title, &a.title)))
        .collect();
    for (i, score) in scores.iter_mut().enumerate() {
        let song = &takes[i];
        let mut releases: Vec<&str> = takes
            .iter()
            .filter(|t| t.title == song.title && t.names.iter().any(|n| song.names.contains(n)))
            .map(|t| t.release.as_str())
            .collect();
        releases.sort_unstable();
        releases.dedup();
        let further = i32::try_from(releases.len().saturating_sub(1)).unwrap_or(i32::MAX);
        score.popularity = 1.0 - (1.0 - score.popularity) * (1.0 - TAKE_SHARE).powi(further);
    }
}

/// Which artists to leave out: a name holding another artist's ("Radiohead - Topic", "We Love
/// Taylor Swift", "Radiohead & Portishead") with under 1% of its audience (none known counts as
/// none). A typo of a known artist stays, ranked by `overshadow`: it may be the band typed.
pub fn borrowed(artists: &[ArtistHit]) -> Vec<bool> {
    let names: Vec<String> = artists.iter().map(|a| loose_artist(&a.name)).collect();
    artists
        .iter()
        .enumerate()
        .map(|(i, a)| {
            artists
                .iter()
                .enumerate()
                .any(|(j, c)| i != j && smaller_than(a, c, BORROWED_SHARE) && holds_name(&names[i], &names[j]))
        })
        .collect()
}

/// Lowers each artist's fit to that of an artist with ten times its audience that holds
/// every word typed or is a typo away, so popularity decides between them.
pub fn overshadow(artists: &[ArtistHit], scores: &mut [Score]) {
    let names: Vec<String> = artists.iter().map(|a| loose_artist(&a.name)).collect();
    let fits: Vec<Fit> = scores.iter().map(|s| s.fit).collect();
    for (i, a) in artists.iter().enumerate() {
        let near = artists.iter().enumerate().filter(|&(j, c)| {
            i != j
                && (fits[j] >= Fit::Words || typo_apart(&names[i], &names[j]))
                && smaller_than(a, c, OVERSHADOWED_SHARE)
        });
        if let Some(lowest) = near.map(|(j, _)| fits[j]).min() {
            scores[i].fit = scores[i].fit.min(lowest);
        }
    }
}

/// `a`'s audience (none known counts as none) is under 1/`share` of `c`'s.
fn smaller_than(a: &ArtistHit, c: &ArtistHit, share: u64) -> bool {
    c.audience
        .is_some_and(|big| a.audience.unwrap_or(0).saturating_mul(share) < big)
}

/// `name` holds `other` as whole words, and more: "radiohead topic" holds "radiohead".
fn holds_name(name: &str, other: &str) -> bool {
    !other.is_empty() && name != other && format!(" {name} ").contains(&format!(" {other} "))
}

/// One or two edits apart, and no more than one per four letters ("radiohed", "radiohead").
fn typo_apart(a: &str, b: &str) -> bool {
    let d = edits(a, b);
    d > 0 && d <= 2 && d * 4 <= a.chars().count().max(b.chars().count())
}

/// Edits between two strings, a swap of neighbours counting as one ("indain", "indian").
/// Lengths more than two apart count as three, as no rule allows more.
fn edits(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    if a.len().abs_diff(b.len()) > 2 {
        return 3;
    }
    // Rows i-2, i-1 and i of the table.
    let mut before: Vec<usize> = vec![0; b.len() + 1];
    let mut last: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut row = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            row[j] = (last[j - 1] + cost).min(last[j] + 1).min(row[j - 1] + 1);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                row[j] = row[j].min(before[j - 2] + 1);
            }
        }
        before = std::mem::replace(&mut last, row);
    }
    last[b.len()]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Artist,
    Album,
    Track,
}

/// The one top result among the best artist, album and track: the best fit, then the song
/// itself (or its release) before another version of it, then the most popular, leaning toward artists and albums
/// (full-length ones more than EPs and singles).
pub fn top(artist: Option<Score>, album: Option<Score>, track: Option<Score>) -> Option<Kind> {
    type Candidate = (Kind, (Fit, bool), f64);
    let better = |a: &Candidate, b: &Candidate| {
        a.1.cmp(&b.1)
            .then_with(|| a.2.partial_cmp(&b.2).unwrap_or(Ordering::Equal))
            == Ordering::Greater
    };
    [(Kind::Artist, artist), (Kind::Album, album), (Kind::Track, track)]
        .into_iter()
        .filter_map(|(kind, row)| row.map(|r| (kind, (r.fit, !r.version), r.popularity + r.lean)))
        .fold(None, |best, c| match best {
            Some(b) if !better(&c, &b) => Some(b),
            _ => Some(c),
        })
        .map(|(kind, ..)| kind)
}
