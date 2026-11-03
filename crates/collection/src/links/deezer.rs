//! Deezer playlists and albums through its public API: no key, every song with its ISRC. A
//! playlist or album id answers even where Deezer's catalog is closed. Pages of 100,
//! inside lull's one Deezer window (50 requests per 5 s, `http::DEEZER`), which discovery's
//! searches share.

use lull_core::http::DEEZER;
use lull_core::{Error, Result};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::parse::{Kind, Link};
use super::{Listing, MAX_SONGS, Song};
use crate::Service;

const SERVICE: &str = "Deezer";
const API: &str = "https://api.deezer.com";
const PAGE: usize = 100;
/// Deezer's `error.code` for "quota limit exceeded".
const QUOTA: u64 = 4;

/// The playlist's or album's name: also how a link to nothing is found out before the import.
pub(crate) async fn name(svc: &Service, link: &Link) -> Result<String> {
    Ok(get::<About>(svc, &format!("{}/{}", kind(link), link.id)).await?.title)
}

pub(crate) async fn read(svc: &Service, link: &Link) -> Result<Listing> {
    let kind = kind(link);
    let about: About = get(svc, &format!("{kind}/{}", link.id)).await?;
    let album = (link.kind == Kind::Album).then(|| about.title.clone());
    let album_cover = album.as_ref().and(about.cover_xl.clone());
    let mut songs = Vec::new();
    loop {
        let path = format!("{kind}/{}/tracks?index={}&limit={PAGE}", link.id, songs.len());
        let page: Page = get(svc, &path).await?;
        let got = page.data.len();
        songs.extend(
            page.data
                .into_iter()
                .map(|t| t.song(album.as_deref(), album_cover.as_deref())),
        );
        if got < PAGE || page.next.is_none() || songs.len() >= MAX_SONGS {
            break;
        }
    }
    songs.truncate(MAX_SONGS);
    Ok(Listing {
        name: about.title,
        songs,
        note: None,
        covers: about.picture_xl.into_iter().chain(about.cover_xl).collect(),
        description: about.description.map(|d| d.trim().to_owned()).filter(|d| !d.is_empty()),
        owner: about.creator.or(about.artist).map(|n| n.name),
    })
}

fn kind(link: &Link) -> &'static str {
    match link.kind {
        Kind::Playlist => "playlist",
        Kind::Album => "album",
    }
}

async fn get<T: DeserializeOwned>(svc: &Service, path: &str) -> Result<T> {
    svc.ctx.http.window(DEEZER).await;
    let url = format!("{API}/{path}");
    let body: Value = svc.ctx.http.json(SERVICE, |c| c.get(&url)).await?;
    if body["error"]["code"].as_u64() == Some(QUOTA) {
        svc.ctx.http.close_window(DEEZER);
    }
    decode(body)
}

/// Deezer answers errors as HTTP 200 with an `error` object; "no data" (800) is a link to
/// nothing, or a private playlist.
fn decode<T: DeserializeOwned>(body: Value) -> Result<T> {
    if let Some(error) = body.get("error") {
        return Err(match error["code"].as_u64() {
            Some(800) => Error::NotFound(
                "Deezer has no public playlist or album at that link. A private playlist can't be read: make it public first."
                    .into(),
            ),
            Some(QUOTA) => Error::unavailable(SERVICE, "is limiting lull's requests; try again in a minute"),
            code => Error::upstream(
                SERVICE,
                format!("error {}: {}", code.unwrap_or(0), error["message"].as_str().unwrap_or("no message")),
            ),
        });
    }
    serde_json::from_value(body).map_err(|e| Error::upstream(SERVICE, format!("unexpected answer: {e}")))
}

#[derive(Deserialize)]
struct About {
    title: String,
    description: Option<String>,
    /// A playlist's picture, an album's cover (1000 px).
    picture_xl: Option<String>,
    cover_xl: Option<String>,
    /// A playlist's maker, an album's artist.
    creator: Option<Named>,
    artist: Option<Named>,
}

#[derive(Deserialize)]
struct Page {
    data: Vec<Track>,
    next: Option<String>,
}

#[derive(Deserialize)]
struct Track {
    id: u64,
    title: String,
    duration: Option<u32>,
    isrc: Option<String>,
    artist: Named,
    album: Option<Named2>,
    /// When it was added to the playlist (unix seconds); albums have none.
    time_add: Option<i64>,
}

#[derive(Deserialize)]
struct Named {
    name: String,
}

#[derive(Deserialize)]
struct Named2 {
    title: String,
    cover_xl: Option<String>,
}

impl Track {
    /// `album` and `cover` are the album's when reading an album (its tracks don't repeat them).
    fn song(self, album: Option<&str>, cover: Option<&str>) -> Song {
        let (own_album, own_cover) = match self.album {
            Some(a) => (Some(a.title), a.cover_xl),
            None => (None, None),
        };
        Song {
            key: format!("deezer:track:{}", self.id),
            artist: self.artist.name,
            title: self.title,
            album: album.map(str::to_owned).or(own_album),
            duration: self.duration.filter(|d| *d > 0),
            isrc: self.isrc.filter(|i| !i.is_empty()),
            added: self.time_add.filter(|t| *t > 0),
            picture: cover.map(str::to_owned).or(own_cover),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_playlist_page_keeps_each_songs_isrc_and_album() -> Result<()> {
        let page: Page = decode(serde_json::json!({ "data": [{
            "id": 116348632, "title": "Hey Jude (Remastered 2015)", "duration": 429, "isrc": "GBUM71505902",
            "artist": { "name": "The Beatles" }, "album": { "title": "1 (Remastered 2015)", "cover_xl": "https://cdn-images.dzcdn.net/images/cover/x/1000x1000.jpg" }
        }], "next": null }))?;
        let song = page.data.into_iter().next().map(|t| t.song(None, None));
        let song = song.ok_or_else(|| Error::internal("no song"))?;
        assert_eq!(song.key, "deezer:track:116348632");
        assert_eq!(song.isrc.as_deref(), Some("GBUM71505902"));
        assert_eq!(song.album.as_deref(), Some("1 (Remastered 2015)"));
        assert_eq!(
            song.picture.as_deref(),
            Some("https://cdn-images.dzcdn.net/images/cover/x/1000x1000.jpg")
        );
        Ok(())
    }

    #[test]
    fn deezers_no_data_error_is_not_found() {
        let r: Result<About> =
            decode(serde_json::json!({ "error": { "type": "DataException", "message": "no data", "code": 800 } }));
        assert!(matches!(r, Err(Error::NotFound(_))));
    }
}
