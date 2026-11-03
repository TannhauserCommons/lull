//! Playlists for link imports. YouTube and YouTube Music list flat (yt-dlp asks for a hundred
//! songs a request). SoundCloud's flat listing names no song, so each one is read: its metadata
//! only (no stream formats, one request a song), a second apart, since SoundCloud answers 403 to
//! a burst (seen after about 150 songs read with their formats). A song blocked in this country
//! is left out rather than failing the page.

use std::time::Duration;

use lull_core::youtube::{FoundPlaylist, Listed, Listing};
use serde::Deserialize;

use crate::Service;
use crate::matching::read_upload;
use crate::search::plain_id;
use crate::ytdlp::Problem;

const FLAT_TIME: Duration = Duration::from_secs(90);
const FULL_TIME: Duration = Duration::from_secs(240);

/// Up to `count` songs from position `from` (0 = the first).
pub async fn page(svc: &Service, url: &str, from: usize, count: usize) -> Result<Listing, Problem> {
    let soundcloud = reqwest::Url::parse(url)
        .ok()
        .and_then(|u| {
            u.host_str()
                .map(|h| h == "soundcloud.com" || h.ends_with(".soundcloud.com"))
        })
        .unwrap_or(false);
    let items = format!("{}:{}", from + 1, from + count.max(1));
    let mut args = vec!["-J", "-I", items.as_str()];
    if soundcloud {
        args.extend([
            "-i",
            "--ignore-no-formats-error",
            "--extractor-args",
            "soundcloud:formats=none",
            "--sleep-requests",
            "1",
        ]);
    } else {
        args.push("--flat-playlist");
    }
    // `--`: the link is never read as an option.
    args.extend(["--", url]);
    let limit = if soundcloud { FULL_TIME } else { FLAT_TIME };
    let out = svc.run_with(&args, limit, soundcloud).await?;
    parse(&out, soundcloud)
}

/// yt-dlp's `-J` of a playlist as a page of songs.
pub fn parse(json: &[u8], soundcloud: bool) -> Result<Listing, Problem> {
    // `-i` turns a failed playlist into `null` on stdout.
    let raw: Option<Raw> = serde_json::from_slice(json).map_err(|e| Problem::Other(format!("yt-dlp's answer: {e}")))?;
    let raw = raw.ok_or_else(|| Problem::Other("the site didn't give the playlist".into()))?;
    if raw.kind.as_deref() != Some("playlist") {
        return Err(Problem::Other("this link is a single upload, not a playlist".into()));
    }
    // Largest first; one without a width (SoundCloud's original) comes last.
    let mut thumbnails: Vec<Thumbnail> = raw
        .thumbnails
        .into_iter()
        .filter(|t| t.url.starts_with("https://"))
        .collect();
    thumbnails.sort_by_key(|t| std::cmp::Reverse(t.width.unwrap_or(0)));
    Ok(Listing {
        title: raw.title.unwrap_or_default(),
        total: raw.playlist_count,
        owner: raw.channel.or(raw.uploader).filter(|o| !o.trim().is_empty()),
        thumbnails: thumbnails.into_iter().map(|t| t.url).collect(),
        description: raw.description.map(|d| d.trim().to_owned()).filter(|d| !d.is_empty()),
        songs: raw
            .entries
            .into_iter()
            .flatten()
            .filter_map(|e| e.listed(soundcloud))
            .collect(),
    })
}

/// YouTube's search, filtered to playlists (`sp=EgIQAw==`), first `limit` results, flat.
pub async fn search(svc: &Service, query: &str, limit: usize) -> Result<Vec<FoundPlaylist>, Problem> {
    let url = reqwest::Url::parse_with_params(
        "https://www.youtube.com/results",
        [("search_query", query), ("sp", "EgIQAw==")],
    )
    .map_err(|e| Problem::Other(format!("search link: {e}")))?;
    let items = format!("1:{}", limit.max(1));
    let out = svc
        .run(&["-J", "--flat-playlist", "-I", &items, "--", url.as_str()], FLAT_TIME)
        .await?;
    parse_search(&out)
}

/// A playlist search's `-J` as playlists; videos and channels among the results are left out.
pub fn parse_search(json: &[u8]) -> Result<Vec<FoundPlaylist>, Problem> {
    let raw: Option<Raw> = serde_json::from_slice(json).map_err(|e| Problem::Other(format!("yt-dlp's answer: {e}")))?;
    let Some(raw) = raw else { return Ok(Vec::new()) };
    Ok(raw
        .entries
        .into_iter()
        .flatten()
        .filter(|e| e.url.as_deref().is_some_and(|u| u.contains("playlist?list=")))
        .filter_map(|e| {
            let id = e.id.filter(|id| plain_id(id))?;
            let title = e.title.filter(|t| !t.trim().is_empty())?;
            let mut thumbnails = e.thumbnails;
            thumbnails.retain(|t| t.url.starts_with("https://"));
            thumbnails.sort_by_key(|t| std::cmp::Reverse(t.width.unwrap_or(0)));
            Some(FoundPlaylist {
                id,
                title,
                owner: e.channel.or(e.uploader).unwrap_or_default(),
                thumbnails: thumbnails.into_iter().map(|t| t.url).collect(),
            })
        })
        .collect())
}

#[derive(Deserialize)]
struct Raw {
    #[serde(rename = "_type")]
    kind: Option<String>,
    title: Option<String>,
    playlist_count: Option<usize>,
    description: Option<String>,
    channel: Option<String>,
    uploader: Option<String>,
    #[serde(default)]
    thumbnails: Vec<Thumbnail>,
    /// `null` for a song `-i` skipped.
    #[serde(default)]
    entries: Vec<Option<Entry>>,
}

#[derive(Deserialize)]
struct Thumbnail {
    url: String,
    width: Option<u32>,
}

#[derive(Deserialize)]
struct Entry {
    id: Option<String>,
    url: Option<String>,
    #[serde(default)]
    thumbnails: Vec<Thumbnail>,
    title: Option<String>,
    channel: Option<String>,
    uploader: Option<String>,
    duration: Option<f64>,
    artist: Option<String>,
    artists: Option<Vec<String>>,
    track: Option<String>,
}

impl Entry {
    /// `None` for an upload that is gone: YouTube lists it as "[Private video]" or "[Deleted video]".
    /// The upload as a song (`matching::read_upload`); a SoundCloud upload's length is always
    /// the track's. `None` for an upload that is gone ("[Private video]") or names no song.
    fn listed(self, soundcloud: bool) -> Option<Listed> {
        let id = self.id.filter(|id| plain_id(id))?;
        let title = self.title.filter(|t| !(t.starts_with('[') && t.ends_with(" video]")))?;
        let uploader = self.channel.or(self.uploader).unwrap_or_default();
        let music_artist = self.artists.and_then(|a| a.into_iter().next()).or(self.artist);
        let (artist, track, official) = read_upload(&title, &uploader, music_artist.as_deref(), self.track.as_deref())?;
        Some(Listed {
            id,
            title,
            uploader,
            duration: self.duration.filter(|d| *d > 0.0).map(|d| d.round() as u32),
            artist: Some(artist),
            track: Some(track),
            exact_length: official || soundcloud,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flat_youtube_playlist_lists_its_songs_without_the_gone_ones() -> Result<(), Problem> {
        let json = br#"{"_type": "playlist", "title": "The Hit List", "playlist_count": 52,
            "description": " Today's hits. ", "thumbnails": [
                {"url": "https://i9.ytimg.com/s_p/x/mqdefault.jpg", "width": 180},
                {"url": "https://i9.ytimg.com/s_p/x/maxresdefault.jpg", "width": 1200},
                {"url": "https://i9.ytimg.com/s_p/x/sddefault.jpg", "width": 640}
            ], "entries": [
            {"id": "78wrful9cVU", "title": "Olivia Rodrigo - drop dead (Official Music Video)", "channel": "Olivia Rodrigo", "duration": 251},
            {"id": "aaaaaaaaaaa", "title": "[Private video]", "duration": null},
            {"id": "bbbbbbbbbbb", "title": "Patient Zero", "channel": "Taylor Swift - Topic", "duration": 313.4}
        ]}"#;
        let page = parse(json, false)?;
        assert_eq!((page.title.as_str(), page.total), ("The Hit List", Some(52)));
        assert_eq!(
            page.thumbnails.first().map(String::as_str),
            Some("https://i9.ytimg.com/s_p/x/maxresdefault.jpg")
        );
        assert_eq!(page.thumbnails.len(), 3);
        assert_eq!(page.description.as_deref(), Some("Today's hits."));
        assert_eq!(page.songs.len(), 2);
        assert_eq!(page.songs[1].uploader, "Taylor Swift - Topic");
        assert_eq!(page.songs[1].duration, Some(313));
        assert_eq!(page.songs[0].artist.as_deref(), Some("Olivia Rodrigo"));
        assert_eq!(page.songs[0].track.as_deref(), Some("drop dead"));
        assert_eq!(
            (page.songs[0].exact_length, page.songs[1].exact_length),
            (false, true),
            "a music video's length isn't the song's; official audio's is"
        );
        Ok(())
    }

    #[test]
    fn a_soundcloud_page_skips_the_songs_yt_dlp_could_not_read() -> Result<(), Problem> {
        let json = br#"{"_type": "playlist", "title": "Late", "entries": [
            {"id": "917326303", "title": "Breezin'", "uploader": "Sweet Medicine", "artist": "Sweet Medicine", "track": "Breezin'", "duration": 157.7},
            null
        ]}"#;
        let page = parse(json, true)?;
        assert_eq!(page.songs.len(), 1);
        assert_eq!(page.songs[0].artist.as_deref(), Some("Sweet Medicine"));
        assert_eq!(page.songs[0].duration, Some(158));
        assert!(page.songs[0].exact_length, "a SoundCloud track's length is the song's");
        Ok(())
    }

    #[test]
    fn a_playlist_the_site_refused_is_an_error() {
        assert!(parse(b"null\n", true).is_err());
    }

    #[test]
    fn a_playlist_search_keeps_only_playlists_with_their_owner_and_biggest_picture() -> Result<(), Problem> {
        let json = br#"{"_type": "playlist", "title": "person of interest", "entries": [
            {"_type": "url", "ie_key": "YoutubeTab", "id": "PLrFSfgR7Y3I1yKrovCWPyGNlFR4eoEgCd",
             "url": "https://www.youtube.com/playlist?list=PLrFSfgR7Y3I1yKrovCWPyGNlFR4eoEgCd",
             "title": "Person of Interest : List of Songs", "channel": "young Jin", "thumbnails": [
                {"url": "https://i.ytimg.com/vi/x/hq720.jpg?a=1", "width": 360},
                {"url": "https://i.ytimg.com/vi/x/hq720.jpg?a=2", "width": 720}]},
            {"_type": "url", "ie_key": "Youtube", "id": "dQw4w9WgXcQ", "url": "https://www.youtube.com/watch?v=dQw4w9WgXcQ", "title": "a video"},
            {"_type": "url", "ie_key": "YoutubeTab", "id": "PLnoname", "url": "https://www.youtube.com/playlist?list=PLnoname", "title": null}
        ]}"#;
        let found = parse_search(json)?;
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].title, "Person of Interest : List of Songs");
        assert_eq!(found[0].owner, "young Jin");
        assert_eq!(found[0].thumbnails[0], "https://i.ytimg.com/vi/x/hq720.jpg?a=2");
        Ok(())
    }

    #[test]
    fn a_single_upload_is_not_a_playlist() {
        assert!(parse(br#"{"_type": "video", "id": "78wrful9cVU", "title": "x"}"#, false).is_err());
    }
}
