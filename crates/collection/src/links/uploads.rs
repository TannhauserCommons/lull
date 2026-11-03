//! YouTube, YouTube Music and SoundCloud playlists, listed by yt-dlp through the `Youtube` seam.
//! Uploads name songs loosely ("Artist - Title (Official Video)" on a label's channel); the
//! youtube crate reads each into an artist and a title, the same reading its own matching uses
//! (youtube.md#matching), and says whether its length is the song's (not a music video's).

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

/// An upload as a song to match, as the youtube crate read it; `None` when it names none.
pub(crate) fn song(l: &Listed, site: Site) -> Option<Song> {
    let (artist, title) = (l.artist.clone()?, l.track.clone()?);
    Some(Song {
        key: format!("{}:{}", site.name(), l.id),
        artist,
        title,
        album: None,
        duration: l.duration.filter(|_| l.exact_length),
        isrc: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_listed_upload_is_a_song_with_its_length_only_when_it_is_the_songs() {
        let listed = |exact_length| Listed {
            id: "78wrful9cVU".into(),
            title: "Olivia Rodrigo - drop dead (Official Music Video)".into(),
            uploader: "Olivia Rodrigo".into(),
            duration: Some(251),
            artist: Some("Olivia Rodrigo".into()),
            track: Some("drop dead".into()),
            exact_length,
        };
        let video = song(&listed(false), Site::Youtube);
        assert_eq!(
            video
                .as_ref()
                .map(|s| (s.key.as_str(), s.artist.as_str(), s.title.as_str(), s.duration)),
            Some(("youtube:78wrful9cVU", "Olivia Rodrigo", "drop dead", None))
        );
        assert_eq!(
            song(&listed(true), Site::Soundcloud).and_then(|s| s.duration),
            Some(251)
        );
        let unread = Listed {
            artist: None,
            ..listed(false)
        };
        assert_eq!(song(&unread, Site::Youtube), None);
    }
}
