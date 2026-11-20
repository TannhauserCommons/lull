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

use super::personal::score::Affinity;
use super::query::{Fit, Query, edits, names_a_version, release_versions, version_words, without_versions};
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
/// An artist whose name holds another's, with less than 1/this of its audience, borrows it.
const BORROWED_SHARE: u64 = 100;
/// An artist with less than 1/this of a famous near match's audience ranks with it.
const OVERSHADOWED_SHARE: u64 = 10;
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
    /// YouTube's video uploads read as songs, the last gap-filler (`Youtube::search_songs`).
    #[serde(rename = "youtube-uploads")]
    YoutubeUploads,
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
            Source::YoutubeUploads => "YouTube",
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
    /// The artist ADR-0035's rule placed the song on (`namesakes::place_rows`), switch on only.
    #[serde(skip)]
    pub placed: Option<ArtistId>,
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
    /// The artist ADR-0035's rule placed the album on, switch on only.
    #[serde(skip)]
    pub placed: Option<ArtistId>,
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
    /// The artist ADR-0035's rule placed the row on, switch on only.
    #[serde(skip)]
    pub placed: Option<ArtistId>,
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
            placed: None,
        })
    }

    /// The song's artist: where ADR-0035's rule placed it, else its name's id.
    pub fn artist_id(&self) -> ArtistId {
        self.placed.clone().unwrap_or_else(|| ArtistId::of(&self.artist))
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

    /// The album's artist: where ADR-0035's rule placed it, else its name's id.
    pub fn artist_id(&self) -> ArtistId {
        self.placed.clone().unwrap_or_else(|| ArtistId::of(&self.artist))
    }
}

impl ArtistHit {
    pub fn info(&self) -> ArtistInfo {
        ArtistInfo {
            mbid: self.mbid.clone(),
            name: self.name.clone(),
        }
    }

    /// Where ADR-0035's rule placed the row, else its name's id.
    pub fn id(&self) -> ArtistId {
        self.placed.clone().unwrap_or_else(|| ArtistId::of(&self.name))
    }
}

/// Every source's rows merged so far.
#[derive(Debug, Default)]
pub struct Merged {
    /// ADR-0035's switch: rows whose ids tell them apart stay apart, whatever their names.
    pub namesakes: bool,
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
    /// An empty merge; `namesakes` is ADR-0035's switch.
    pub fn new(namesakes: bool) -> Merged {
        Merged {
            namesakes,
            ..Merged::default()
        }
    }

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
            let at = match self.artists.iter().position(|a| same_artist(a, &hit, self.namesakes)) {
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
    /// An answer as its kind asks: a gap-filler's (one carrying `fillers`) only adds what the
    /// rows lack (`fill_gaps`); any other merges as it is.
    pub fn absorb(&mut self, found: Found) -> Touched {
        if found.fillers.is_some() {
            fill_gaps(self, found)
        } else {
            self.add(found)
        }
    }

    /// A row holds every word typed (or is them): no gap left for the last gap-filler to fill.
    pub fn answers(&self, q: &Query) -> bool {
        self.songs.iter().any(|s| q.song(s) >= Fit::Words) || self.albums.iter().any(|a| q.album(a) >= Fit::Words)
    }

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

fn same_artist(a: &ArtistHit, b: &ArtistHit, namesakes: bool) -> bool {
    loose_artist(&a.name) == loose_artist(&b.name)
        && !(namesakes
            && lull_core::identity::tells_apart(&crate::namesakes::anchors(a), &crate::namesakes::anchors(b)))
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

/// How far the listener's own music lifts a row's popularity (`Score::with_affinity`), per unit of
/// affinity (0 to 1). Chosen by the personal case set and the query set (docs/context/discovery.md).
pub const AFFINITY_WEIGHT: f64 = 0.5;

impl Score {
    /// The row as the listener's music lifts it: popularity plus `AFFINITY_WEIGHT` of its affinity
    /// (so the top result, which weighs popularity, leans the same way), and a take they have
    /// counts as the song itself, not another version of it ("Creep (Acoustic)" for a listener
    /// who plays it).
    pub fn with_affinity(self, a: Affinity) -> Score {
        Score {
            version: self.version && !a.own,
            popularity: self.popularity + AFFINITY_WEIGHT * a.score,
            ..self
        }
    }
}

/// Positions of `rows` best first when their popularity already holds the listener's affinity
/// (`Score::with_affinity`): fit, then the song before its other versions, then the catalog's
/// rows before a gap-filler's, then that one blended value. With no affinity anywhere this is
/// `rank`'s order, as popularity in tenths then exact popularity is popularity.
pub fn rank_blended(rows: &[Score]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..rows.len()).collect();
    order.sort_by(|&a, &b| {
        let key = |i: usize| (rows[i].fit, !rows[i].version, !rows[i].filler);
        key(b).cmp(&key(a)).then_with(|| {
            rows[b]
                .popularity
                .partial_cmp(&rows[a].popularity)
                .unwrap_or(Ordering::Equal)
        })
    });
    order
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
