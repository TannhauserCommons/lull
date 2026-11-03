//! YouTube, YouTube Music and SoundCloud playlists, listed by yt-dlp through the `Youtube` seam.
//! Uploads name songs loosely ("Artist - Title (Official Video)" on a label's channel), so each
//! is read into an artist and a title before it is matched:
//! - music metadata, when the site gives it, wins;
//! - a YouTube "Artist - Topic" channel is official audio: the channel is the artist, and the
//!   length is the song's;
//! - else "Artist - Title" splits at the first dash, and else the channel is the artist;
//! - video tags ("(Official Music Video)", "[Lyrics]", "| 4K") are dropped. A video's length
//!   isn't the song's (intros, outros), so it isn't compared; a SoundCloud upload's is.

use lull_core::Result;
use lull_core::youtube::Listed;

use super::parse::{Link, Site};
use super::{Listing, MAX_SONGS, Song};
use crate::Service;

/// Songs asked of yt-dlp at once: a flat YouTube page is one quick run; SoundCloud reads each
/// song (about 1 s), so its pages are small and progress shows.
fn page_size(site: Site) -> usize {
    if site == Site::Soundcloud { 40 } else { 200 }
}

/// The playlist page by page; `on_page` hears how many songs are read so far.
pub(crate) async fn read(svc: &Service, link: &Link, mut on_page: impl FnMut(usize, Option<usize>)) -> Result<Listing> {
    let (url, n) = (link.url(), page_size(link.site));
    let mut listing = Listing::default();
    let mut from = 0;
    loop {
        let page = match svc.youtube.playlist(&url, from, n).await {
            Ok(page) => page,
            // The songs read so far are worth importing; the note says the rest are missing.
            Err(e) if !listing.songs.is_empty() => {
                listing.note = Some(format!(
                    "{} stopped answering after {} songs ({e}). Import the link again later for the rest.",
                    if link.site == Site::Soundcloud {
                        "SoundCloud"
                    } else {
                        "YouTube"
                    },
                    listing.songs.len()
                ));
                break;
            }
            Err(e) => return Err(e),
        };
        if from == 0 {
            listing.name = page.title.clone();
            listing.covers = page.thumbnails.clone();
            listing.description = page.description.clone();
            listing.owner = page.owner.clone();
        }
        from += n;
        listing
            .songs
            .extend(page.songs.iter().filter_map(|l| song(l, link.site)));
        on_page(listing.songs.len(), page.total);
        let more = page.total.map_or(page.songs.len() >= n, |t| from < t);
        if !more || listing.songs.len() >= MAX_SONGS {
            break;
        }
    }
    listing.songs.truncate(MAX_SONGS);
    if listing.name.is_empty() {
        listing.name = format!(
            "{} playlist",
            if link.site == Site::Soundcloud {
                "SoundCloud"
            } else {
                "YouTube"
            }
        );
    }
    Ok(listing)
}

/// An upload as a song to match, `None` when it names none.
pub(crate) fn song(l: &Listed, site: Site) -> Option<Song> {
    let key = format!("{}:{}", site.name(), l.id);
    let exact = site == Site::Soundcloud;
    let uploader = l.uploader.trim();
    let (artist, title, exact_length) = match (&l.artist, &l.track) {
        (Some(a), Some(t)) if *t != l.title && !a.trim().is_empty() => (a.trim().to_owned(), clean(t), exact),
        _ => match uploader.strip_suffix(" - Topic") {
            Some(artist) => (artist.to_owned(), clean(&l.title), true),
            None => match split_dash(&l.title) {
                Some((a, t)) => {
                    let title = clean(&without_artist(&t, &a));
                    (a, title, exact)
                }
                None => (channel_artist(uploader), clean(&l.title), exact),
            },
        },
    };
    (!artist.is_empty() && !title.is_empty()).then(|| Song {
        key,
        artist,
        title,
        album: None,
        duration: l.duration.filter(|_| exact_length),
        isrc: None,
    })
}

/// "Artist - Title" (any dash, or SoundCloud's "Artist • Title"), when both sides have words.
fn split_dash(title: &str) -> Option<(String, String)> {
    [" - ", " – ", " — ", " -- ", " • "]
        .iter()
        .filter_map(|d| title.split_once(d))
        .min_by_key(|(a, _)| a.len())
        .map(|(a, t)| (a.trim().to_owned(), t.trim().to_owned()))
        .filter(|(a, t)| !a.is_empty() && !t.is_empty())
}

/// "Dominic Fike \"Babydoll\"" after "Dominic Fike - ": the artist said again before the title.
fn without_artist(title: &str, artist: &str) -> String {
    match title.get(..artist.len()) {
        Some(head) if head.eq_ignore_ascii_case(artist) && title[artist.len()..].starts_with(' ') => {
            title[artist.len()..].trim().to_owned()
        }
        _ => title.to_owned(),
    }
}

/// A channel name as an artist: "AdeleVEVO" is Adele, "Adele Official" is Adele.
fn channel_artist(channel: &str) -> String {
    let mut name = channel.trim();
    for suffix in ["VEVO", "Vevo", " Official", " official", " Music"] {
        if let Some(rest) = name.strip_suffix(suffix).filter(|r| !r.trim().is_empty()) {
            name = rest.trim();
        }
    }
    name.to_owned()
}

/// Words that make a bracket a video's label rather than part of the song's title.
const VIDEO_WORDS: &[&str] = &[
    "official",
    "video",
    "audio",
    "lyric",
    "lyrics",
    "visualizer",
    "visualiser",
    "hd",
    "hq",
    "4k",
    "mv",
    "m/v",
    "clip",
    "full song",
    "color coded",
];

/// The title without video labels, a "| channel" tail or quotes around it.
pub(crate) fn clean(title: &str) -> String {
    let title = title.split(" | ").next().unwrap_or(title);
    let mut out = String::with_capacity(title.len());
    let mut rest = title;
    while let Some(open) = rest.find(['(', '[']) {
        let close = if rest.as_bytes()[open] == b'(' { ')' } else { ']' };
        let Some(len) = rest[open + 1..].find(close) else { break };
        let inner = rest[open + 1..open + 1 + len].to_lowercase();
        out.push_str(&rest[..open]);
        let words: Vec<&str> = inner.split(|c: char| !c.is_alphanumeric() && c != '/').collect();
        let label = VIDEO_WORDS.iter().any(|w| {
            if w.contains(' ') {
                inner.contains(w)
            } else {
                words.contains(w)
            }
        });
        if !label {
            out.push_str(&rest[open..open + len + 2]);
        }
        rest = &rest[open + len + 2..];
    }
    out.push_str(rest);
    let out = out.split_whitespace().collect::<Vec<_>>().join(" ");
    // A title quoted whole ("\"Song\""), not an apostrophe at its end ("Breezin'").
    let quoted = ['"', '“', '”'];
    match out.strip_prefix(quoted).and_then(|o| o.strip_suffix(quoted)) {
        Some(inner) if !inner.trim().is_empty() => inner.trim().to_owned(),
        _ => out,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listed(title: &str, uploader: &str, duration: Option<u32>) -> Listed {
        Listed {
            id: "78wrful9cVU".into(),
            title: title.into(),
            uploader: uploader.into(),
            duration,
            ..Listed::default()
        }
    }

    fn named(song: Option<Song>) -> (String, String, Option<u32>) {
        song.map(|s| (s.artist, s.title, s.duration)).unwrap_or_default()
    }

    #[test]
    fn a_music_video_title_splits_into_artist_and_title_without_its_label() {
        let s = song(
            &listed(
                "Olivia Rodrigo - drop dead (Official Music Video)",
                "Olivia Rodrigo",
                Some(251),
            ),
            Site::Youtube,
        );
        assert_eq!(named(s), ("Olivia Rodrigo".into(), "drop dead".into(), None));
    }

    #[test]
    fn an_artist_named_again_before_a_quoted_title_is_dropped() {
        let s = song(
            &listed(
                "Dominic Fike - Dominic Fike \"Babydoll\" (Official Video)",
                "Dominic Fike",
                None,
            ),
            Site::Youtube,
        );
        assert_eq!(named(s), ("Dominic Fike".into(), "Babydoll".into(), None));
    }

    #[test]
    fn a_topic_channel_is_official_audio_with_the_songs_length() {
        let s = song(
            &listed("Patient Zero", "Taylor Swift - Topic", Some(313)),
            Site::Youtube,
        );
        assert_eq!(named(s), ("Taylor Swift".into(), "Patient Zero".into(), Some(313)));
    }

    #[test]
    fn without_a_dash_the_channel_is_the_artist() {
        let s = song(&listed("Hello [Lyrics]", "AdeleVEVO", Some(300)), Site::Youtube);
        assert_eq!(named(s), ("Adele".into(), "Hello".into(), None));
    }

    #[test]
    fn version_brackets_stay_in_the_title() {
        assert_eq!(
            clean("Song (Live at Wembley) (Official Video) | Band"),
            "Song (Live at Wembley)"
        );
        assert_eq!(clean("Song (feat. X) [4K Remaster]"), "Song (feat. X)");
        assert_eq!(clean("\"Quoted\" (Audio)"), "Quoted");
    }

    #[test]
    fn music_metadata_wins_over_the_upload_title() {
        let mut l = listed("Sweet Medicine w/ ODYSSEE - Breezin' [lofi]", "Label", Some(158));
        (l.artist, l.track) = (Some("Sweet Medicine".into()), Some("Breezin'".into()));
        assert_eq!(
            named(song(&l, Site::Soundcloud)),
            ("Sweet Medicine".into(), "Breezin'".into(), Some(158))
        );
    }

    #[test]
    fn a_bullet_splits_artist_and_title_too() {
        let s = song(&listed("Juan RIOS • praia", "Juan RIOS", Some(203)), Site::Soundcloud);
        assert_eq!(named(s), ("Juan RIOS".into(), "praia".into(), Some(203)));
    }

    #[test]
    fn a_soundcloud_upload_keeps_its_length() {
        let s = song(
            &listed("Sweet Medicine - Dreamin'", "Sweet Medicine", Some(144)),
            Site::Soundcloud,
        );
        assert_eq!(named(s), ("Sweet Medicine".into(), "Dreamin'".into(), Some(144)));
    }
}
