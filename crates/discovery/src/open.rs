//! `GET /api/open?link=`: a link to a song, album or artist, opened as this hub's own page
//! (docs/context/discovery.md#opening-links).
//! - **lull's links** (`share.rs`) name what the sender's lull knows: names and public ids, never a
//!   hub's address. Since ADR-0034 they are pages on the share host (`share_host.rs`, `share/`):
//!   `/1/<kind>/<names>?<ids>`. Before it (ADR-0032) they were lull's open page (`OPEN_PAGE`, `site/`)
//!   followed by `#lull?`, the names, ids and the item's public page (`page=`: song.link, album.link,
//!   Last.fm); browsers never send the part after `#`, and any lull reads it here, whatever page
//!   precedes it (the oldest links had the public page itself in front). Both are read for good.
//! - **Other services' links** are read by their ids: song.link, album.link and Apple Music
//!   (iTunes ids), Deezer, MusicBrainz release groups; Last.fm's by the names in them.
//! - **Recorded like a search row**, its album with the catalog album behind it, so the page
//!   shows its tracklist and the song plays from Soulseek like any other.
//!
//! Playlist links aren't read here: the UI opens them as an outside playlist (`/api/outside`).

use std::borrow::Cow;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Query as Params, State};
use lull_core::model::{SongQuery, Track};
use lull_core::seams::Catalog;
use lull_core::{Error, Result};
use serde::{Deserialize, Serialize};
use url::{Url, form_urlencoded};

use crate::Service;
use crate::art::{self, Pictured};
use crate::search::merge::{AlbumHit, Found, SongHit};
use crate::share_host;
use crate::sources::{AlbumSource, ArtistInfo, Release, Song};
use crate::store::{self, entities};

/// lull's open page (site/open.html, deployed by `.github/workflows/pages.yml`).
const OPEN_PAGE: &str = "https://tannhausercommons.github.io/lull/open";
/// What follows `#` in a lull link, before its ids.
const MARK: &str = "lull?";
const READS: &str = "lull opens links to songs, albums and artists from lull, song.link, Apple Music, Deezer, \
     MusicBrainz and Last.fm.";

/// What a link names, as lull records it.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Track {
        artist: String,
        title: String,
        /// Empty when the link names no album (Last.fm's song pages).
        album: String,
        /// The album's artist when it isn't the song's (a soundtrack's "Various Artists").
        album_artist: Option<String>,
        duration: Option<u32>,
        album_source: Option<AlbumSource>,
    },
    Album {
        artist: String,
        title: String,
        mbid: Option<String>,
        source: Option<AlbumSource>,
    },
    Artist {
        name: String,
        mbid: Option<String>,
    },
}

/// A link read, before what it names is known.
#[derive(Debug, PartialEq)]
enum Link {
    Named(Item),
    Itunes(u64),
    Deezer(DeezerKind, u64),
    ReleaseGroup(String),
}

#[derive(Debug, PartialEq)]
enum DeezerKind {
    Track,
    Album,
    Artist,
}

#[derive(Deserialize)]
pub struct OpenParams {
    link: String,
}

#[derive(Serialize)]
pub struct Opened {
    /// `track`, `album` or `artist`.
    kind: &'static str,
    id: String,
    entities: Pictured,
}

pub async fn open(State(svc): State<Arc<Service>>, Params(p): Params<OpenParams>) -> Result<Json<Opened>> {
    let item = match parse(&p.link)? {
        Link::Named(item) => item,
        link => resolve(&svc, link).await?,
    };
    Ok(Json(record(&svc, item).await?))
}

/// ADR-0032's lull link to `item`, whose public page is `page`: the open page, then `#lull?kind=song&…`.
/// Written until the share host has a canonical address (`share_host::link`).
pub fn link(item: &Item, page: &str) -> String {
    format!("{OPEN_PAGE}#{}", fragment(item, page))
}

fn fragment(item: &Item, page: &str) -> String {
    let mut f = form_urlencoded::Serializer::new(String::new());
    let mut put = |key: &str, value: Option<&str>| {
        if let Some(v) = value.filter(|v| !v.is_empty()) {
            f.append_pair(key, v);
        }
    };
    let source = |s: &Option<AlbumSource>| match s {
        Some(AlbumSource::Deezer(id)) => ("deezer", id.to_string()),
        Some(AlbumSource::Itunes(id)) => ("itunes", id.to_string()),
        None => ("", String::new()),
    };
    match item {
        Item::Track {
            artist,
            title,
            album,
            album_artist,
            duration,
            album_source,
        } => {
            put("kind", Some("song"));
            put("artist", Some(artist));
            put("title", Some(title));
            put("album", Some(album));
            put("by", album_artist.as_deref());
            put("length", duration.map(|d| d.to_string()).as_deref());
            let (key, id) = source(album_source);
            put(key, Some(&id));
        }
        Item::Album {
            artist,
            title,
            mbid,
            source: album_source,
        } => {
            put("kind", Some("album"));
            put("artist", Some(artist));
            put("title", Some(title));
            put("mbid", mbid.as_deref());
            let (key, id) = source(album_source);
            put(key, Some(&id));
        }
        Item::Artist { name, mbid } => {
            put("kind", Some("artist"));
            put("artist", Some(name));
            put("mbid", mbid.as_deref());
        }
    }
    put("page", Some(page));
    format!("{MARK}{}", f.finish())
}

/// What a pasted link names: 400 in plain words for one lull can't read.
fn parse(text: &str) -> Result<Link> {
    let text = text.trim();
    let with_scheme = if text.contains("://") {
        Cow::Borrowed(text)
    } else {
        Cow::Owned(format!("https://{text}"))
    };
    let url = Url::parse(&with_scheme).map_err(|_| Error::BadRequest(format!("That isn't a link. {READS}")))?;
    if let Some(ids) = url.fragment().and_then(|f| f.strip_prefix(MARK)) {
        return named(ids).map(Link::Named);
    }
    if !matches!(url.scheme(), "https" | "http") {
        return Err(Error::BadRequest(READS.into()));
    }
    if let Some(item) = share_host::read(&url)? {
        return Ok(Link::Named(item));
    }
    let host = url
        .host_str()
        .unwrap_or_default()
        .trim_start_matches("www.")
        .to_ascii_lowercase();
    let path: Vec<&str> = url
        .path_segments()
        .map(|s| s.filter(|p| !p.is_empty()).collect())
        .unwrap_or_default();
    let found = match host.as_str() {
        "song.link" | "album.link" => match path.as_slice() {
            ["i", id] => id.parse().ok().map(Link::Itunes),
            _ => None,
        },
        "music.apple.com" | "itunes.apple.com" | "geo.music.apple.com" => apple(&url, &path),
        "deezer.com" => deezer(&path),
        "musicbrainz.org" => match path.as_slice() {
            ["release-group", mbid, ..] if is_mbid(mbid) => Some(Link::ReleaseGroup((*mbid).to_owned())),
            _ => None,
        },
        "last.fm" | "lastfm.com" => lastfm(&path).map(Link::Named),
        _ => return Err(Error::BadRequest(READS.into())),
    };
    found.ok_or_else(|| Error::BadRequest(format!("That link isn't a song, an album or an artist. {READS}")))
}

/// A lull link's ids (`fragment`); a song or album needs an artist and a title.
fn named(ids: &str) -> Result<Item> {
    let pairs: Vec<(String, String)> = form_urlencoded::parse(ids.as_bytes()).into_owned().collect();
    let get = |key: &str| {
        pairs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.trim().to_owned())
            .filter(|v| !v.is_empty())
    };
    let id = |key: &str| get(key).and_then(|v| v.parse::<u64>().ok());
    let source = id("deezer")
        .map(AlbumSource::Deezer)
        .or_else(|| id("itunes").map(AlbumSource::Itunes));
    let mbid = get("mbid").filter(|m| is_mbid(m));
    let broken = || Error::BadRequest("That lull link is missing its song's or album's name.".into());
    let artist = get("artist").ok_or_else(broken)?;
    Ok(match get("kind").as_deref() {
        Some("song") => Item::Track {
            artist,
            title: get("title").ok_or_else(broken)?,
            album: get("album").unwrap_or_default(),
            album_artist: get("by"),
            duration: id("length").and_then(|d| u32::try_from(d).ok()),
            album_source: source,
        },
        Some("album") => Item::Album {
            artist,
            title: get("title").ok_or_else(broken)?,
            mbid,
            source,
        },
        Some("artist") => Item::Artist { name: artist, mbid },
        _ => {
            return Err(Error::BadRequest(format!(
                "This lull link names nothing lull knows. {READS}"
            )));
        }
    })
}

/// `music.apple.com/<cc>/album/<name>/<id>?i=<song id>`, `…/song/<name>/<id>`,
/// `…/artist/<name>/<id>`, and the older `itunes.apple.com/…/id<id>`.
fn apple(url: &Url, path: &[&str]) -> Option<Link> {
    let song = url
        .query_pairs()
        .find(|(k, _)| k == "i")
        .and_then(|(_, v)| v.parse().ok());
    let last = path.last()?.trim_start_matches("id").parse().ok();
    song.or(last).map(Link::Itunes)
}

/// `deezer.com/[<lang>/]track|album|artist/<id>`.
fn deezer(path: &[&str]) -> Option<Link> {
    let at = path.iter().position(|p| matches!(*p, "track" | "album" | "artist"))?;
    let kind = match path[at] {
        "track" => DeezerKind::Track,
        "album" => DeezerKind::Album,
        _ => DeezerKind::Artist,
    };
    Some(Link::Deezer(kind, path.get(at + 1)?.parse().ok()?))
}

/// `last.fm/music/<artist>[/_/<song> | /<album>]`; Last.fm writes spaces as `+`.
fn lastfm(path: &[&str]) -> Option<Item> {
    let name = |segment: &str| {
        form_urlencoded::parse(format!("n={segment}").as_bytes())
            .next()
            .map(|(_, v)| v.trim().to_owned())
            .filter(|v| !v.is_empty())
    };
    match path {
        ["music", artist] => Some(Item::Artist {
            name: name(artist)?,
            mbid: None,
        }),
        ["music", artist, "_", title, ..] => Some(Item::Track {
            artist: name(artist)?,
            title: name(title)?,
            album: String::new(),
            album_artist: None,
            duration: None,
            album_source: None,
        }),
        ["music", artist, album, ..] if !album.starts_with('+') => Some(Item::Album {
            artist: name(artist)?,
            title: name(album)?,
            mbid: None,
            source: None,
        }),
        _ => None,
    }
}

pub(crate) fn is_mbid(s: &str) -> bool {
    s.len() == 36 && s.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

/// What an outside id names, asked of its service. 404 when it names nothing.
async fn resolve(svc: &Service, link: Link) -> Result<Item> {
    let gone = || Error::NotFound("That song, album or artist isn't there any more.".into());
    match link {
        Link::Named(item) => Ok(item),
        Link::Itunes(id) => {
            let found = svc.itunes.lookup(id).await?;
            first(found).ok_or_else(gone)
        }
        Link::Deezer(DeezerKind::Track, id) => svc.deezer.track(id).await?.map(song).ok_or_else(gone),
        Link::Deezer(DeezerKind::Album, id) => svc.deezer.album(id).await?.map(album).ok_or_else(gone),
        Link::Deezer(DeezerKind::Artist, id) => {
            let artist = svc.deezer.artist(id).await?.ok_or_else(gone)?;
            Ok(Item::Artist {
                name: artist.name,
                mbid: None,
            })
        }
        Link::ReleaseGroup(mbid) => {
            let r = svc.mb.release(&mbid).await?;
            Ok(Item::Album {
                artist: r.artist,
                title: r.title,
                mbid: r.mbid,
                source: None,
            })
        }
    }
}

fn first(found: Found) -> Option<Item> {
    let Found {
        songs, albums, artists, ..
    } = found;
    if let Some(s) = songs.into_iter().next() {
        return Some(song(s));
    }
    if let Some(a) = albums.into_iter().next() {
        return Some(album(a));
    }
    artists.into_iter().next().map(|a| Item::Artist {
        name: a.name,
        mbid: a.mbid,
    })
}

fn song(s: SongHit) -> Item {
    Item::Track {
        artist: s.artist,
        title: s.title,
        album: s.album,
        album_artist: s.album_artist,
        duration: s.duration,
        album_source: s.album_source,
    }
}

fn album(a: AlbumHit) -> Item {
    Item::Album {
        artist: a.artist,
        title: a.title,
        mbid: a.mbid,
        source: a.source,
    }
}

/// Records the item as this hub's own (a song without an album is looked for in the catalog
/// first, for its album), with its album's catalog source; its id and entities.
async fn record(svc: &Service, item: Item) -> Result<Opened> {
    let (mut tracks, mut albums, mut artists) = (Vec::new(), Vec::new(), Vec::new());
    let (kind, id) = match item {
        Item::Track {
            artist,
            title,
            album,
            album_artist,
            duration,
            album_source,
        } => {
            let track = if album.is_empty() {
                find(svc, &artist, &title, duration).await
            } else {
                None
            };
            let track = match track {
                Some(t) => t,
                None => {
                    let song = Song {
                        mbid: None,
                        title,
                        artist,
                        artist_mbid: None,
                        album,
                        album_artist,
                        album_mbid: None,
                        year: None,
                        no: None,
                        disc: None,
                        duration_s: duration,
                        tags: Vec::new(),
                        artist_source: None,
                        isrc: None,
                    };
                    let release = (!song.album.is_empty()).then(|| Release {
                        mbid: None,
                        title: song.album.clone(),
                        artist: song.album_artist.clone().unwrap_or_else(|| song.artist.clone()),
                        artist_mbid: None,
                        kind: None,
                        date: None,
                        tags: Vec::new(),
                    });
                    if let Some(release) = release {
                        albums = svc.store.albums(vec![release]).await?;
                        remember_source(svc, &albums, album_source).await?;
                    }
                    let mut recorded = svc.store.tracks(vec![song]).await?;
                    recorded
                        .pop()
                        .ok_or_else(|| Error::internal("a song recorded came back empty"))?
                }
            };
            let id = track.id.0.clone();
            tracks.push(track);
            ("track", id)
        }
        Item::Album {
            artist,
            title,
            mbid,
            source,
        } => {
            albums = svc
                .store
                .albums(vec![Release {
                    mbid,
                    title,
                    artist,
                    artist_mbid: None,
                    kind: None,
                    date: None,
                    tags: Vec::new(),
                }])
                .await?;
            remember_source(svc, &albums, source).await?;
            let id = albums.first().map(|a| a.id.0.clone()).unwrap_or_default();
            ("album", id)
        }
        Item::Artist { name, mbid } => {
            artists = svc
                .store
                .artists(vec![store::artist(ArtistInfo { mbid, name })])
                .await?;
            let id = artists.first().map(|a| a.id.0.clone()).unwrap_or_default();
            ("artist", id)
        }
    };
    let entities = art::pictured(svc, entities(tracks, albums, artists)).await?;
    Ok(Opened { kind, id, entities })
}

/// The catalog's song for a link that names only artist and title, when it has one.
async fn find(svc: &Service, artist: &str, title: &str, duration: Option<u32>) -> Option<Track> {
    let query = SongQuery {
        artist: artist.to_owned(),
        title: title.to_owned(),
        duration,
        ..SongQuery::default()
    };
    match svc.find_song(&query).await {
        Ok(found) => found,
        Err(e) => {
            tracing::info!(error = %e, "a linked song kept as the link names it");
            None
        }
    }
}

async fn remember_source(svc: &Service, albums: &[lull_core::model::Album], source: Option<AlbumSource>) -> Result<()> {
    let sources = albums.iter().zip(source).map(|(a, s)| (a.id.clone(), s)).collect();
    svc.store.remember_sources(sources).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track() -> Item {
        Item::Track {
            artist: "Petite Meller".into(),
            title: "High Forever".into(),
            album: "Cyberpunk: Edgerunners 2 (Original Series Soundtrack)".into(),
            album_artist: Some("Various Artists".into()),
            duration: Some(187),
            album_source: Some(AlbumSource::Itunes(1_789_000_123)),
        }
    }

    #[test]
    fn a_lull_link_carries_names_and_public_ids_and_reads_back() {
        let link = link(&track(), "https://song.link/i/1789000777");
        assert!(link.starts_with("https://tannhausercommons.github.io/lull/open#lull?kind=song&artist=Petite+Meller"));
        assert!(link.ends_with("&page=https%3A%2F%2Fsong.link%2Fi%2F1789000777"));
        assert!(!link.contains("ts.net") && !link.contains("token"));
        assert_eq!(
            parse(&link).ok(),
            Some(Link::Named(track())),
            "the ids win over the page's"
        );
        let artist = Item::Artist {
            name: "AC/DC & Friends".into(),
            mbid: Some("66c662b6-6e2f-4930-8610-912e24c63ed1".into()),
        };
        let older = format!(
            "https://www.last.fm/music/AC%2FDC#{}",
            fragment(&artist, "https://www.last.fm/music/AC%2FDC")
        );
        assert_eq!(
            parse(&older).ok(),
            Some(Link::Named(artist)),
            "a link from before the open page"
        );
    }

    #[test]
    fn other_services_links_are_read_by_their_ids() {
        let read = |s: &str| parse(s).ok();
        assert_eq!(
            read("https://song.link/i/1440818839"),
            Some(Link::Itunes(1_440_818_839))
        );
        assert_eq!(read("album.link/i/1440818664"), Some(Link::Itunes(1_440_818_664)));
        assert_eq!(
            read("https://music.apple.com/us/album/ok-computer/1097861387?i=1097861696"),
            Some(Link::Itunes(1_097_861_696)),
            "a song on its album"
        );
        assert_eq!(
            read("https://music.apple.com/gb/album/ok-computer/1097861387"),
            Some(Link::Itunes(1_097_861_387))
        );
        assert_eq!(
            read("https://www.deezer.com/en/track/3135556?utm_source=x"),
            Some(Link::Deezer(DeezerKind::Track, 3_135_556))
        );
        assert_eq!(
            read("https://deezer.com/artist/27"),
            Some(Link::Deezer(DeezerKind::Artist, 27))
        );
        assert_eq!(
            read("https://musicbrainz.org/release-group/b1392450-e666-3926-a536-22c65f834433"),
            Some(Link::ReleaseGroup("b1392450-e666-3926-a536-22c65f834433".into()))
        );
        assert_eq!(
            read("https://www.last.fm/music/Sigur+R%C3%B3s/_/Hopp%C3%ADpolla"),
            Some(Link::Named(Item::Track {
                artist: "Sigur Rós".into(),
                title: "Hoppípolla".into(),
                album: String::new(),
                album_artist: None,
                duration: None,
                album_source: None,
            }))
        );
        assert_eq!(
            read("https://www.last.fm/music/Radiohead/OK+Computer"),
            Some(Link::Named(Item::Album {
                artist: "Radiohead".into(),
                title: "OK Computer".into(),
                mbid: None,
                source: None,
            }))
        );
    }

    #[test]
    fn links_lull_cant_read_say_what_it_can() {
        for bad in [
            "https://example.com/song",
            "https://www.youtube.com/playlist?list=PL123",
            "https://www.last.fm/music/Radiohead/+wiki",
            "https://song.link/x",
            "https://song.link/i/1#lull?kind=song&title=No+artist",
        ] {
            assert!(matches!(parse(bad), Err(Error::BadRequest(_))), "{bad} is a 400");
        }
    }
}
