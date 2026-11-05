//! lull links on the share host (ADR-0034): `https://<host>/1/<kind>/<names>?<ids>`, a page chat apps show
//! as a card (`share/`, a Cloudflare Worker). Names are exact, each one percent-encoded path segment; ids are
//! public catalogue ids. Never the hub's address, the listener or the time.
//!
//! `share/hosts.json` lists lull's own share hosts and names the canonical one, where links are written.
//! Until it names one (the Worker isn't deployed yet), `link` answers `None` and lull keeps writing
//! ADR-0032's links (`open::link`). Links are read by their shape on any host, so a share host someone runs
//! for themselves from `share/` opens in every lull too.

use std::sync::LazyLock;

use lull_core::{Error, Result};
use serde::Deserialize;
use url::Url;

use crate::open::Item;
use crate::sources::AlbumSource;

/// The links' version: the first segment of every path. The Worker answers every version it ever handed out.
const VERSION: &str = "1";
const MAX_NAME: usize = 200;

#[derive(Debug, Deserialize)]
struct Hosts {
    canonical: Option<String>,
    hosts: Vec<String>,
}

static HOSTS: LazyLock<std::result::Result<Hosts, String>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../../share/hosts.json")).map_err(|e| format!("share/hosts.json: {e}"))
});

fn hosts() -> Result<&'static Hosts> {
    HOSTS.as_ref().map_err(|e| Error::internal(e.clone()))
}

/// What a link carries beside `Item`: the item's own iTunes id (a song's track, an album's collection;
/// its song.link or album.link page), and a song's album's MBID.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Extra {
    pub itunes: Option<u64>,
    pub album_mbid: Option<String>,
}

/// The share host's link to `item`, or `None` while no host is canonical.
pub fn link(item: &Item, extra: &Extra) -> Result<Option<String>> {
    Ok(hosts()?.canonical.as_deref().map(|host| link_on(host, item, extra)))
}

/// The item a share host's link names; `None` for a link that isn't one (another service's).
pub fn read(url: &Url) -> Result<Option<Item>> {
    read_on(url, &hosts()?.hosts)
}

fn link_on(host: &str, item: &Item, extra: &Extra) -> String {
    let mut ids: Vec<(&str, String)> = Vec::new();
    let mut put = |key, value: Option<String>| {
        if let Some(v) = value.filter(|v| !v.is_empty()) {
            ids.push((key, v));
        }
    };
    let (kind, names): (&str, Vec<&str>) = match item {
        Item::Track {
            artist,
            title,
            album,
            album_artist,
            duration,
            album_source,
        } => {
            put("al", Some(album.clone()));
            put("aa", album_artist.clone());
            put("len", duration.map(|d| d.to_string()));
            put("it", extra.itunes.map(|id| id.to_string()));
            put("ic", itunes(album_source));
            put("dz", deezer(album_source));
            put("mb", extra.album_mbid.clone());
            ("song", vec![artist, title])
        }
        Item::Album {
            artist,
            title,
            mbid,
            source,
        } => {
            put("it", itunes(source).or_else(|| extra.itunes.map(|id| id.to_string())));
            put("dz", deezer(source));
            put("mb", mbid.clone());
            ("album", vec![artist, title])
        }
        Item::Artist { name, mbid } => {
            put("mb", mbid.clone());
            ("artist", vec![name])
        }
    };
    let path: String = names.iter().map(|n| format!("/{}", encode(n))).collect();
    let query: Vec<String> = ids.iter().map(|(k, v)| format!("{k}={}", encode(v))).collect();
    let query = if query.is_empty() {
        String::new()
    } else {
        format!("?{}", query.join("&"))
    };
    format!("https://{host}/{VERSION}/{kind}{path}{query}")
}

fn itunes(source: &Option<AlbumSource>) -> Option<String> {
    match source {
        Some(AlbumSource::Itunes(id)) => Some(id.to_string()),
        _ => None,
    }
}

fn deezer(source: &Option<AlbumSource>) -> Option<String> {
    match source {
        Some(AlbumSource::Deezer(id)) => Some(id.to_string()),
        _ => None,
    }
}

fn read_on(url: &Url, hosts: &[String]) -> Result<Option<Item>> {
    let segments: Vec<&str> = url.path_segments().map(|s| s.collect()).unwrap_or_default();
    // On one of lull's hosts every /1/ address is a link; on any other, one shaped as a link to a song,
    // album or artist is (a share host someone runs for themselves).
    let listed = url.host_str().is_some_and(|h| hosts.iter().any(|known| known == h));
    let shaped = matches!(segments.get(1), Some(&("song" | "album" | "artist")));
    if url.scheme() != "https" || segments.first() != Some(&VERSION) || !(listed || shaped) {
        return Ok(None);
    }
    let broken = || Error::BadRequest("That lull link is cut short or names nothing lull knows.".into());
    let name = |i: usize| {
        segments
            .get(i)
            .and_then(|s| decode(s))
            .and_then(|s| checked(&s))
            .ok_or_else(broken)
    };
    let query: Vec<(String, String)> = url.query_pairs().into_owned().collect();
    let get = |key: &str| query.iter().find(|(k, _)| k == key).map(|(_, v)| v.trim().to_owned());
    let id = |key: &str| get(key).and_then(|v| v.parse::<u64>().ok());
    let mbid = || {
        get("mb")
            .map(|m| m.to_ascii_lowercase())
            .filter(|m| crate::open::is_mbid(m))
    };
    let source = |itunes_key: &str| {
        id("dz")
            .map(AlbumSource::Deezer)
            .or_else(|| id(itunes_key).map(AlbumSource::Itunes))
    };
    let kind = segments.get(1).copied().unwrap_or_default();
    let item = match (kind, segments.len()) {
        ("song", 4) => Item::Track {
            artist: name(2)?,
            title: name(3)?,
            album: get("al").and_then(|a| checked(&a)).unwrap_or_default(),
            album_artist: get("aa").and_then(|a| checked(&a)),
            duration: id("len").and_then(|d| u32::try_from(d).ok()).filter(|d| *d > 0),
            album_source: source("ic"),
        },
        ("album", 4) => Item::Album {
            artist: name(2)?,
            title: name(3)?,
            mbid: mbid(),
            source: source("it"),
        },
        ("artist", 3) => Item::Artist {
            name: name(2)?,
            mbid: mbid(),
        },
        _ => return Err(broken()),
    };
    Ok(Some(item))
}

/// A name as links carry it: trimmed, not empty, not too long, no control characters.
fn checked(name: &str) -> Option<String> {
    let name = name.trim();
    (!name.is_empty() && name.chars().count() <= MAX_NAME && !name.chars().any(char::is_control))
        .then(|| name.to_owned())
}

/// Percent-encoding as JavaScript's `encodeURIComponent` does it, so the hub writes the same address the
/// Worker names as canonical.
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// A path segment's text; `None` for an escape that isn't one or bytes that aren't UTF-8.
fn decode(segment: &str) -> Option<String> {
    let bytes = segment.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The share host tests use until the owner has picked the real one (ADR-0034).
    const HOST: &str = "lull.share-host.invalid";

    fn read(link: &str) -> Option<Item> {
        let url = Url::parse(link).ok()?;
        read_on(&url, &[HOST.to_owned()]).ok().flatten()
    }

    #[test]
    fn hosts_json_is_well_formed_and_its_canonical_host_is_listed() {
        let hosts = hosts().map_err(|e| e.to_string());
        assert!(hosts.is_ok(), "{hosts:?}");
        if let Ok(Hosts {
            canonical: Some(canonical),
            hosts,
        }) = hosts
        {
            assert!(
                hosts.contains(canonical),
                "{canonical} isn't in share/hosts.json's hosts"
            );
        }
    }

    #[test]
    fn a_share_link_carries_exact_names_and_public_ids_and_reads_back() {
        let song = Item::Track {
            artist: "Sigur Rós".into(),
            title: "Hoppípolla / Með blóðnasir + more".into(),
            album: "Takk...".into(),
            album_artist: None,
            duration: Some(268),
            album_source: Some(AlbumSource::Itunes(1_440_000_001)),
        };
        let extra = Extra {
            itunes: Some(1_440_000_123),
            album_mbid: Some("0c0b1cc8-36e2-3c8e-9b22-6e1ab3e1c1b5".into()),
        };
        let link = link_on(HOST, &song, &extra);
        assert_eq!(
            link,
            "https://lull.share-host.invalid/1/song/Sigur%20R%C3%B3s/Hopp%C3%ADpolla%20%2F%20Me%C3%B0%20bl%C3%B3%C3%B0nasir%20%2B%20more\
             ?al=Takk...&len=268&it=1440000123&ic=1440000001&mb=0c0b1cc8-36e2-3c8e-9b22-6e1ab3e1c1b5",
            "the Worker's canonical form: ids in its order, encoded as encodeURIComponent does"
        );
        assert_eq!(read(&link), Some(song), "names come back exactly, '+' and '/' included");
        let album = Item::Album {
            artist: "Portishead".into(),
            title: "Dummy".into(),
            mbid: Some("48140466-cff6-3222-bd55-63c27e43190d".into()),
            source: Some(AlbumSource::Deezer(302_127)),
        };
        assert_eq!(read(&link_on(HOST, &album, &Extra::default())), Some(album));
        let artist = Item::Artist {
            name: "AC/DC".into(),
            mbid: None,
        };
        assert_eq!(
            link_on(HOST, &artist, &Extra::default()),
            "https://lull.share-host.invalid/1/artist/AC%2FDC"
        );
        assert_eq!(read("https://lull.share-host.invalid/1/artist/AC%2FDC"), Some(artist));
    }

    #[test]
    fn any_share_hosts_links_are_read_and_a_broken_one_says_so() {
        let elsewhere = |link: &str| {
            Url::parse(link)
                .ok()
                .map(|u| read_on(&u, &[HOST.to_owned()]).ok().flatten())
        };
        assert_eq!(
            elsewhere("https://share.example.org/1/artist/Portishead"),
            Some(Some(Item::Artist {
                name: "Portishead".into(),
                mbid: None
            })),
            "a share host someone runs for themselves"
        );
        assert_eq!(
            elsewhere("https://example.org/1/news/today"),
            Some(None),
            "not shaped as a link"
        );
        assert_eq!(
            elsewhere("http://example.org/1/artist/Portishead"),
            Some(None),
            "share hosts are https"
        );
        for broken in [
            "https://lull.share-host.invalid/1/song/OnlyAnArtist",
            "https://lull.share-host.invalid/1/podcast/a/b",
            "https://lull.share-host.invalid/1/artist/%FF",
            "https://lull.share-host.invalid/1/artist/%20",
        ] {
            let url = Url::parse(broken).ok();
            assert!(
                url.is_some_and(|u| matches!(read_on(&u, &[HOST.to_owned()]), Err(Error::BadRequest(_)))),
                "{broken}"
            );
        }
        assert_eq!(
            read("https://lull.share-host.invalid/1/album/Portishead/Dummy?mb=nope&it=x1"),
            Some(Item::Album {
                artist: "Portishead".into(),
                title: "Dummy".into(),
                mbid: None,
                source: None,
            }),
            "a malformed id is dropped, not trusted"
        );
    }
}
