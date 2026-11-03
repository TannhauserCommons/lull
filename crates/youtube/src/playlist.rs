//! Playlists for link imports. YouTube and YouTube Music list flat (yt-dlp asks for a hundred
//! songs a request); SoundCloud's flat listing names no song, so each one is read, about a second
//! apiece, and a song blocked in this country is left out rather than failing the page.

use std::time::Duration;

use lull_core::youtube::{Listed, Listing};
use serde::Deserialize;

use crate::Service;
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
    args.push(if soundcloud { "-i" } else { "--flat-playlist" });
    // `--`: the link is never read as an option.
    args.extend(["--", url]);
    let limit = if soundcloud { FULL_TIME } else { FLAT_TIME };
    let out = svc.run_with(&args, limit, soundcloud).await?;
    parse(&out)
}

/// yt-dlp's `-J` of a playlist as a page of songs.
pub fn parse(json: &[u8]) -> Result<Listing, Problem> {
    let raw: Raw = serde_json::from_slice(json).map_err(|e| Problem::Other(format!("yt-dlp's answer: {e}")))?;
    if raw.kind.as_deref() != Some("playlist") {
        return Err(Problem::Other("this link is a single upload, not a playlist".into()));
    }
    Ok(Listing {
        title: raw.title.unwrap_or_default(),
        total: raw.playlist_count,
        songs: raw.entries.into_iter().flatten().filter_map(Entry::listed).collect(),
    })
}

#[derive(Deserialize)]
struct Raw {
    #[serde(rename = "_type")]
    kind: Option<String>,
    title: Option<String>,
    playlist_count: Option<usize>,
    /// `null` for a song `-i` skipped.
    #[serde(default)]
    entries: Vec<Option<Entry>>,
}

#[derive(Deserialize)]
struct Entry {
    id: Option<String>,
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
    fn listed(self) -> Option<Listed> {
        let id = self.id.filter(|id| plain_id(id))?;
        let title = self.title.filter(|t| !(t.starts_with('[') && t.ends_with(" video]")))?;
        Some(Listed {
            id,
            title,
            uploader: self.channel.or(self.uploader).unwrap_or_default(),
            duration: self.duration.filter(|d| *d > 0.0).map(|d| d.round() as u32),
            artist: self.artists.and_then(|a| a.into_iter().next()).or(self.artist),
            track: self.track,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flat_youtube_playlist_lists_its_songs_without_the_gone_ones() -> Result<(), Problem> {
        let json = br#"{"_type": "playlist", "title": "The Hit List", "playlist_count": 52, "entries": [
            {"id": "78wrful9cVU", "title": "Olivia Rodrigo - drop dead (Official Music Video)", "channel": "Olivia Rodrigo", "duration": 251},
            {"id": "aaaaaaaaaaa", "title": "[Private video]", "duration": null},
            {"id": "bbbbbbbbbbb", "title": "Patient Zero", "channel": "Taylor Swift - Topic", "duration": 313.4}
        ]}"#;
        let page = parse(json)?;
        assert_eq!((page.title.as_str(), page.total), ("The Hit List", Some(52)));
        assert_eq!(page.songs.len(), 2);
        assert_eq!(page.songs[1].uploader, "Taylor Swift - Topic");
        assert_eq!(page.songs[1].duration, Some(313));
        Ok(())
    }

    #[test]
    fn a_soundcloud_page_skips_the_songs_yt_dlp_could_not_read() -> Result<(), Problem> {
        let json = br#"{"_type": "playlist", "title": "Late", "entries": [
            {"id": "917326303", "title": "Breezin'", "uploader": "Sweet Medicine", "artist": "Sweet Medicine", "track": "Breezin'", "duration": 157.7},
            null
        ]}"#;
        let page = parse(json)?;
        assert_eq!(page.songs.len(), 1);
        assert_eq!(page.songs[0].artist.as_deref(), Some("Sweet Medicine"));
        assert_eq!(page.songs[0].duration, Some(158));
        Ok(())
    }

    #[test]
    fn a_single_upload_is_not_a_playlist() {
        assert!(parse(br#"{"_type": "video", "id": "78wrful9cVU", "title": "x"}"#).is_err());
    }
}
