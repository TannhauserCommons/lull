//! Which playlist a pasted link names. Each supported link has one canonical key
//! (`spotify:playlist:<id>`), so pasting it again, with other tracking parameters or from another
//! app, updates the same import.

use lull_core::{Error, Result};
use url::Url;

/// A playlist (or album) on a service lull imports from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Link {
    pub(crate) site: Site,
    pub(crate) kind: Kind,
    /// The service's id: a Spotify or Deezer id, a YouTube list id, `user/sets/name` on SoundCloud.
    pub(crate) id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Site {
    Spotify,
    Deezer,
    Youtube,
    Soundcloud,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Playlist,
    Album,
}

/// A link that redirects to the real one: the app's share sheets hand these out.
pub(crate) enum Parsed {
    Link(Link),
    Short(Url),
}

const SUPPORTED: &str = "lull imports playlists from Spotify, Deezer, YouTube, YouTube Music and SoundCloud. \
     Paste the playlist's share link.";

impl Link {
    /// `spotify:playlist:<id>`: the import's key.
    pub(crate) fn key(&self) -> String {
        format!("{}:{}:{}", self.site.name(), self.kind.name(), self.id)
    }

    /// The link lull reads from, and shows.
    pub(crate) fn url(&self) -> String {
        let (kind, id) = (self.kind.name(), &self.id);
        match self.site {
            Site::Spotify => format!("https://open.spotify.com/{kind}/{id}"),
            Site::Deezer => format!("https://www.deezer.com/{kind}/{id}"),
            Site::Youtube => format!("https://www.youtube.com/playlist?list={id}"),
            Site::Soundcloud => format!("https://soundcloud.com/{id}"),
        }
    }

    /// The key back as a link, for an import already stored.
    pub(crate) fn from_key(key: &str) -> Result<Link> {
        let mut parts = key.splitn(3, ':');
        let (site, kind, id) = (parts.next(), parts.next(), parts.next());
        let site = match site {
            Some("spotify") => Site::Spotify,
            Some("deezer") => Site::Deezer,
            Some("youtube") => Site::Youtube,
            Some("soundcloud") => Site::Soundcloud,
            _ => return Err(Error::NotFound(format!("No import {key}."))),
        };
        let kind = match kind {
            Some("playlist") => Kind::Playlist,
            Some("album") => Kind::Album,
            _ => return Err(Error::NotFound(format!("No import {key}."))),
        };
        let id = id
            .filter(|id| valid_id(site, id))
            .ok_or_else(|| Error::NotFound(format!("No import {key}.")))?;
        Ok(Link {
            site,
            kind,
            id: id.to_owned(),
        })
    }
}

impl Site {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Site::Spotify => "spotify",
            Site::Deezer => "deezer",
            Site::Youtube => "youtube",
            Site::Soundcloud => "soundcloud",
        }
    }
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Playlist => "playlist",
            Kind::Album => "album",
        }
    }
}

/// The playlist a pasted link names: 400 in plain words for anything else.
pub(crate) fn parse(text: &str) -> Result<Parsed> {
    let text = text.trim();
    if let Some(link) = spotify_uri(text) {
        return Ok(Parsed::Link(link));
    }
    let with_scheme = if text.contains("://") {
        text.to_owned()
    } else {
        format!("https://{text}")
    };
    let url = Url::parse(&with_scheme).map_err(|_| Error::BadRequest(format!("That isn't a link. {SUPPORTED}")))?;
    if !matches!(url.scheme(), "https" | "http") {
        return Err(Error::BadRequest(SUPPORTED.into()));
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
        "spotify.link" | "spotify.app.link" | "link.deezer.com" | "deezer.page.link" | "on.soundcloud.com" => {
            return Ok(Parsed::Short(url));
        }
        "open.spotify.com" | "play.spotify.com" => kind_and_id(&path, Site::Spotify),
        "deezer.com" => kind_and_id(&path, Site::Deezer),
        "youtube.com" | "m.youtube.com" | "music.youtube.com" | "youtu.be" => youtube(&url),
        "soundcloud.com" | "m.soundcloud.com" => soundcloud(&path),
        _ => return Err(Error::BadRequest(SUPPORTED.into())),
    };
    found.map(Parsed::Link).ok_or_else(|| {
        Error::BadRequest(format!(
            "That link isn't a playlist or an album lull can read. {SUPPORTED}"
        ))
    })
}

/// `spotify:playlist:<id>`, as Spotify's desktop app copies it.
fn spotify_uri(text: &str) -> Option<Link> {
    let rest = text.strip_prefix("spotify:")?;
    let (kind, id) = rest.split_once(':')?;
    kind_and_id(&[kind, id], Site::Spotify)
}

/// `…/playlist/<id>` or `…/album/<id>`, after optional locale segments (`intl-de`, `en`).
fn kind_and_id(path: &[&str], site: Site) -> Option<Link> {
    let at = path.iter().position(|p| matches!(*p, "playlist" | "album"))?;
    let kind = if path[at] == "playlist" {
        Kind::Playlist
    } else {
        Kind::Album
    };
    let id = path.get(at + 1)?;
    valid_id(site, id).then(|| Link {
        site,
        kind,
        id: (*id).to_owned(),
    })
}

/// Any YouTube or YouTube Music link with a `list`: a playlist page, or a song played from one.
fn youtube(url: &Url) -> Option<Link> {
    let list = url
        .query_pairs()
        .find(|(k, _)| k == "list")
        .map(|(_, v)| v.into_owned())?;
    valid_id(Site::Youtube, &list).then_some(Link {
        site: Site::Youtube,
        kind: Kind::Playlist,
        id: list,
    })
}

/// `soundcloud.com/<user>/sets/<name>`: a SoundCloud playlist or album.
fn soundcloud(path: &[&str]) -> Option<Link> {
    match path {
        [user, "sets", name, ..] => {
            let id = format!("{}/sets/{}", user.to_ascii_lowercase(), name.to_ascii_lowercase());
            valid_id(Site::Soundcloud, &id).then_some(Link {
                site: Site::Soundcloud,
                kind: Kind::Playlist,
                id,
            })
        }
        _ => None,
    }
}

/// Ids go into URLs and yt-dlp's arguments, so only their own alphabet is let through.
fn valid_id(site: Site, id: &str) -> bool {
    let word = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
    !id.is_empty()
        && id.len() <= 128
        && match site {
            Site::Spotify => id.len() == 22 && id.chars().all(|c| c.is_ascii_alphanumeric()),
            Site::Deezer => id.len() <= 20 && id.chars().all(|c| c.is_ascii_digit()),
            Site::Youtube => id.chars().all(word),
            Site::Soundcloud => {
                let parts: Vec<&str> = id.split('/').collect();
                parts.len() == 3 && parts[1] == "sets" && parts.iter().all(|p| !p.is_empty() && p.chars().all(word))
            }
        }
}

/// The link a short link leads to: where it redirected, else the first supported link in the
/// page it answered with (Spotify's short links answer with a page, not a redirect).
pub(crate) fn resolved(final_url: &str, page: &str) -> Option<Link> {
    if let Ok(Parsed::Link(link)) = parse(final_url) {
        return Some(link);
    }
    [
        "https://open.spotify.com/",
        "https://www.deezer.com/",
        "https://soundcloud.com/",
    ]
    .iter()
    .flat_map(|start| page.match_indices(start).map(|(i, _)| i))
    .filter_map(|i| {
        let end = page[i..]
            .find(|c: char| c == '"' || c == '\'' || c == '<' || c.is_whitespace())
            .map_or(page.len(), |n| i + n);
        match parse(&page[i..end]) {
            Ok(Parsed::Link(link)) => Some(link),
            _ => None,
        }
    })
    .next()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(text: &str) -> String {
        match parse(text) {
            Ok(Parsed::Link(l)) => l.key(),
            Ok(Parsed::Short(u)) => format!("short {u}"),
            Err(e) => format!("error {e}"),
        }
    }

    #[test]
    fn spotify_links_in_every_shape_name_one_playlist() {
        let want = "spotify:playlist:37i9dQZF1DXcBWIGoYBM5M";
        assert_eq!(
            key("https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M?si=abc123"),
            want
        );
        assert_eq!(key("open.spotify.com/intl-de/playlist/37i9dQZF1DXcBWIGoYBM5M"), want);
        assert_eq!(key(" spotify:playlist:37i9dQZF1DXcBWIGoYBM5M "), want);
        assert_eq!(
            key("https://open.spotify.com/album/4aawyAB9vmqN3uQ7FjRGTy"),
            "spotify:album:4aawyAB9vmqN3uQ7FjRGTy"
        );
    }

    #[test]
    fn deezer_links_with_or_without_a_language_name_one_playlist() {
        assert_eq!(
            key("https://www.deezer.com/en/playlist/908622995"),
            "deezer:playlist:908622995"
        );
        assert_eq!(
            key("https://deezer.com/playlist/908622995?utm_source=x"),
            "deezer:playlist:908622995"
        );
        assert_eq!(key("https://www.deezer.com/fr/album/12047956"), "deezer:album:12047956");
    }

    #[test]
    fn youtube_and_youtube_music_links_name_their_list() {
        let want = "youtube:playlist:PLFgquLnL59alCl_2TQvOiD5Vgm1hCaGSI";
        assert_eq!(
            key("https://www.youtube.com/playlist?list=PLFgquLnL59alCl_2TQvOiD5Vgm1hCaGSI"),
            want
        );
        assert_eq!(
            key("https://music.youtube.com/playlist?list=PLFgquLnL59alCl_2TQvOiD5Vgm1hCaGSI&si=x"),
            want
        );
        assert_eq!(
            key("https://www.youtube.com/watch?v=dQw4w9WgXcQ&list=PLFgquLnL59alCl_2TQvOiD5Vgm1hCaGSI"),
            want
        );
        assert_eq!(
            key("https://youtu.be/dQw4w9WgXcQ?list=PLFgquLnL59alCl_2TQvOiD5Vgm1hCaGSI"),
            want
        );
    }

    #[test]
    fn soundcloud_sets_are_playlists() {
        assert_eq!(
            key("https://soundcloud.com/Lofi-Hip-Hop-Music/sets/lofi-hip-hop-beats?si=1"),
            "soundcloud:playlist:lofi-hip-hop-music/sets/lofi-hip-hop-beats"
        );
    }

    #[test]
    fn short_links_are_followed_first() {
        assert!(key("https://spotify.link/ZBcBOZtOSDb").starts_with("short "));
        assert!(key("https://link.deezer.com/s/30abc").starts_with("short "));
        assert!(key("https://on.soundcloud.com/xyz").starts_with("short "));
    }

    #[test]
    fn other_links_are_refused_in_plain_words() {
        for text in [
            "https://open.spotify.com/track/6OmhkSOpvYBokMKQxpIGx2",
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://soundcloud.com/sweet-medicine/breezin",
            "https://example.com/playlist/1",
            "https://open.spotify.com/playlist/../../etc",
            "hello there",
            "file:///etc/passwd",
        ] {
            assert!(key(text).starts_with("error "), "{text}: {}", key(text));
        }
    }

    #[test]
    fn a_key_reads_back_as_its_link() -> Result<()> {
        for text in [
            "https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M",
            "https://www.deezer.com/playlist/908622995",
            "https://www.youtube.com/playlist?list=PLFgquLnL59alCl_2TQvOiD5Vgm1hCaGSI",
            "https://soundcloud.com/lofi-hip-hop-music/sets/lofi-hip-hop-beats",
        ] {
            let Parsed::Link(link) = parse(text)? else {
                return Err(Error::internal(format!("{text} is a short link")));
            };
            assert_eq!(Link::from_key(&link.key())?, link);
            assert_eq!(link.url(), text);
        }
        assert!(Link::from_key("spotify:playlist:../x").is_err());
        Ok(())
    }

    #[test]
    fn a_short_link_resolves_by_redirect_or_by_the_page_it_answers() {
        let page = r#"<html><a href="https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M?si=1">open</a>"#;
        assert_eq!(
            resolved("https://spotify.link/x", page).map(|l| l.key()).as_deref(),
            Some("spotify:playlist:37i9dQZF1DXcBWIGoYBM5M")
        );
        assert_eq!(
            resolved("https://www.deezer.com/us/playlist/908622995", "")
                .map(|l| l.key())
                .as_deref(),
            Some("deezer:playlist:908622995")
        );
        assert_eq!(resolved("https://spotify.link/x", "nothing here"), None);
    }
}
