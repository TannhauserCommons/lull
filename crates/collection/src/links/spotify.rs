//! Spotify playlists and albums, from the public embed page (`open.spotify.com/embed/…`): no key
//! and no sign-in, any public link. Spotify's Web API reads only the playlists the app's own
//! user owns since February 2026, so it can't import a link someone shares. The embed page names
//! each song's title, artists and length, but no album or ISRC, and lists at most 100 songs
//! (PROJECT.md records the user's OK to read it).

use lull_core::{Error, Result};
use serde::Deserialize;
use serde_json::Value;

use super::parse::{Kind, Link};
use super::{Listing, Song};
use crate::Service;

const SERVICE: &str = "Spotify";
/// The most songs the embed page lists.
const EMBED_MAX: usize = 100;

pub(crate) async fn read(svc: &Service, link: &Link) -> Result<Listing> {
    let kind = match link.kind {
        Kind::Playlist => "playlist",
        Kind::Album => "album",
    };
    let url = format!("https://open.spotify.com/embed/{kind}/{}", link.id);
    let resp = svc.ctx.http.send(SERVICE, |c| c.get(&url)).await?;
    let status = resp.status();
    let page = resp
        .text()
        .await
        .map_err(|e| Error::upstream(SERVICE, format!("reading the page: {e}")))?;
    if status.as_u16() == 404 {
        return Err(not_found());
    }
    if !status.is_success() {
        return Err(Error::upstream(SERVICE, format!("answered {status}")));
    }
    embedded(&page, link.kind)
}

fn not_found() -> Error {
    Error::NotFound(
        "Spotify shows no public playlist at that link. A private playlist can't be read: make it public, \
         or export it with Exportify and import the .csv."
            .into(),
    )
}

/// The songs in the page's `__NEXT_DATA__`.
pub(crate) fn embedded(page: &str, kind: Kind) -> Result<Listing> {
    let start = page
        .find("<script id=\"__NEXT_DATA__\"")
        .and_then(|i| page[i..].find('>').map(|j| i + j + 1))
        .ok_or_else(|| Error::upstream(SERVICE, "its page has changed: no song data in it"))?;
    let end = page[start..]
        .find("</script>")
        .map(|j| start + j)
        .ok_or_else(|| Error::upstream(SERVICE, "its page has changed: no song data in it"))?;
    let data: Value = serde_json::from_str(&page[start..end])
        .map_err(|e| Error::upstream(SERVICE, format!("its page has changed: {e}")))?;
    let props = &data["props"]["pageProps"];
    if props["status"].as_u64() == Some(404) {
        return Err(not_found());
    }
    let entity: Entity = serde_json::from_value(props["state"]["data"]["entity"].clone())
        .map_err(|e| Error::upstream(SERVICE, format!("its page has changed: {e}")))?;
    let name = entity
        .name
        .or(entity.title)
        .unwrap_or_else(|| "Spotify playlist".into());
    let album = (kind == Kind::Album).then(|| name.clone());
    let songs: Vec<Song> = entity
        .track_list
        .into_iter()
        .filter_map(|t| {
            let artist = t.subtitle.replace('\u{a0}', " ").trim().to_owned();
            (!artist.is_empty() && !t.title.trim().is_empty()).then(|| Song {
                key: t.uri,
                artist,
                title: t.title.trim().to_owned(),
                album: album.clone(),
                duration: t
                    .duration
                    .map(|ms| u32::try_from(ms / 1000).unwrap_or(u32::MAX))
                    .filter(|d| *d > 0),
                isrc: None,
            })
        })
        .collect();
    let note = (songs.len() >= EMBED_MAX).then(|| {
        "Spotify lets lull read only the first 100 songs of a playlist. If this one has more, export it with \
         Exportify and import the .csv."
            .to_owned()
    });
    Ok(Listing { name, songs, note })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entity {
    name: Option<String>,
    title: Option<String>,
    #[serde(default)]
    track_list: Vec<Embedded>,
}

#[derive(Deserialize)]
struct Embedded {
    uri: String,
    title: String,
    /// The artists, joined with ",\u{a0}".
    subtitle: String,
    /// Milliseconds.
    duration: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(entity: &str) -> String {
        format!(
            r#"<html><script id="__NEXT_DATA__" type="application/json">{{"props":{{"pageProps":{{"state":{{"data":{{"entity":{entity}}}}}}}}}}}</script></html>"#
        )
    }

    #[test]
    fn an_embedded_playlist_lists_its_songs_with_every_artist() -> Result<()> {
        let listing = embedded(
            &page(
                r#"{"name":"All Out 80s","trackList":[
                {"uri":"spotify:track:2LlQb7Uoj1kKyGhlkBf9aC","title":"Thriller","subtitle":"Michael Jackson","duration":358807},
                {"uri":"spotify:track:6OmhkSOpvYBokMKQxpIGx2","title":"Global Warming (feat. Sensato)","subtitle":"Pitbull, Sensato","duration":85400}]}"#,
            ),
            Kind::Playlist,
        )?;
        assert_eq!(listing.name, "All Out 80s");
        assert_eq!(listing.songs.len(), 2);
        assert_eq!(listing.songs[0].duration, Some(358));
        assert_eq!(listing.songs[1].artist, "Pitbull, Sensato");
        assert_eq!(listing.songs[1].album, None);
        assert_eq!(listing.note, None);
        Ok(())
    }

    #[test]
    fn an_album_names_its_songs_album() -> Result<()> {
        let listing = embedded(
            &page(
                r#"{"name":"Global Warming","trackList":[{"uri":"spotify:track:x","title":"Back in Time","subtitle":"Pitbull","duration":207000}]}"#,
            ),
            Kind::Album,
        )?;
        assert_eq!(listing.songs[0].album.as_deref(), Some("Global Warming"));
        Ok(())
    }

    #[test]
    fn a_full_embed_says_spotify_shows_only_the_first_hundred() -> Result<()> {
        let tracks: Vec<String> = (0..100)
            .map(|i| format!(r#"{{"uri":"spotify:track:{i}","title":"Song {i}","subtitle":"A","duration":1000}}"#))
            .collect();
        let listing = embedded(
            &page(&format!(r#"{{"name":"Big","trackList":[{}]}}"#, tracks.join(","))),
            Kind::Playlist,
        )?;
        assert!(listing.note.is_some_and(|n| n.contains("Exportify")));
        Ok(())
    }

    #[test]
    fn spotifys_not_found_page_is_not_found() {
        let html =
            r#"<script id="__NEXT_DATA__" type="application/json">{"props":{"pageProps":{"status":404}}}</script>"#;
        assert!(matches!(embedded(html, Kind::Playlist), Err(Error::NotFound(_))));
        assert!(matches!(
            embedded("<html>changed</html>", Kind::Playlist),
            Err(Error::Upstream { .. })
        ));
    }
}
