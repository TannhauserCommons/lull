//! M3U import preview: each line matched to the library first, else to a song Soulseek will be
//! searched for when it is played, else not found. Nothing is created; the UI makes the
//! playlist from the ids it keeps (`POST /api/playlists`).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use futures_util::{StreamExt, stream};
use lull_core::ids::{AlbumId, ArtistId, TrackId, normalize, normalize_artist};
use lull_core::model::Track;
use lull_core::seams::Library;
use lull_core::{Error, Result, catalog};
use serde::Serialize;
use serde_json::{Value, json};

use crate::m3u::{self, Entry};
use crate::{Service, entities};

const MAX_LINES: usize = 5000;
/// Library searches in flight at once: quick on a big file, gentle on Jellyfin.
const PARALLEL: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum Status {
    Library,
    WillSearch,
    NotFound,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Line {
    n: usize,
    location: String,
    artist: Option<String>,
    title: Option<String>,
    status: Status,
    track_id: Option<TrackId>,
}

/// `{ name, lines: [{ n, location, artist, title, status, trackId }], counts, libraryChecked, entities }`.
pub(crate) async fn preview(svc: &Service, body: &[u8]) -> Result<Value> {
    let playlist = m3u::parse(&m3u::decode(body));
    if playlist.entries.is_empty() {
        return Err(Error::BadRequest("This file has no songs in it.".into()));
    }
    if playlist.entries.len() > MAX_LINES {
        return Err(Error::BadRequest(format!("This file has more than {MAX_LINES} songs.")));
    }

    let library_ok = Arc::new(AtomicBool::new(true));
    let matches: Vec<(Entry, Match)> = stream::iter(playlist.entries)
        .map(|entry| {
            let (library, library_ok) = (svc.library.clone(), library_ok.clone());
            async move {
                let found = match_entry(svc, &*library, &library_ok, &entry).await;
                found.map(|m| (entry, m))
            }
        })
        .buffered(PARALLEL)
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<_>>()?;

    let library_checked = library_ok.load(Ordering::Relaxed);
    svc.ctx
        .db
        .call(move |conn| {
            let mut lines = Vec::with_capacity(matches.len());
            let mut counts = [0usize; 3];
            for (i, (entry, m)) in matches.into_iter().enumerate() {
                let (status, track_id) = match m {
                    Match::Known(track, saved) => {
                        let id = track.id.clone();
                        catalog::remember(conn, &[track])?;
                        (if saved { Status::Library } else { Status::WillSearch }, Some(id))
                    }
                    Match::New(track) => {
                        let id = track.id.clone();
                        // A richer record from search or the library wins over what a line says.
                        if catalog::track(conn, &id)?.is_none() {
                            catalog::remember(conn, &[track])?;
                        }
                        (Status::WillSearch, Some(id))
                    }
                    Match::None => (Status::NotFound, None),
                };
                counts[status as usize] += 1;
                lines.push(Line {
                    n: i + 1,
                    location: entry.location,
                    artist: entry.artist,
                    title: entry.title,
                    status,
                    track_id,
                });
            }
            let entities = entities(conn, lines.iter().filter_map(|l| l.track_id.as_ref()))?;
            Ok(json!({
                "name": playlist.name,
                "lines": lines,
                "counts": { "library": counts[0], "willSearch": counts[1], "notFound": counts[2] },
                "libraryChecked": library_checked,
                "entities": entities,
            }))
        })
        .await
}

enum Match {
    /// A track lull already has a record of, and whether it is in the library.
    Known(Track, bool),
    /// A song lull hasn't seen: playing it searches Soulseek for artist + title.
    New(Track),
    None,
}

async fn match_entry(svc: &Service, library: &dyn Library, library_ok: &AtomicBool, entry: &Entry) -> Result<Match> {
    if let Some(id) = &entry.track_id {
        // A lull link: the catalog knows it unless it came from another lull server.
        let known = svc.ctx.db.call({
            let id = id.clone();
            move |conn| catalog::track(conn, &id)
        });
        if let Some(track) = known.await? {
            let saved = in_library(library, library_ok, &track).await || track.saved;
            return Ok(Match::Known(track, saved));
        }
    }
    let Some(title) = entry.title.as_deref() else {
        return Ok(Match::None);
    };
    if let Some(track) = search_library(library, library_ok, entry.artist.as_deref(), title).await {
        return Ok(Match::Known(track, true));
    }
    Ok(match entry.artist.as_deref() {
        Some(artist) => Match::New(unsaved_track(
            artist,
            entry.album.as_deref().unwrap_or(""),
            title,
            entry.duration,
        )),
        None => Match::None,
    })
}

/// The library's copy of a song with this title (and artist, when the line names one).
async fn search_library(library: &dyn Library, ok: &AtomicBool, artist: Option<&str>, title: &str) -> Option<Track> {
    if !ok.load(Ordering::Relaxed) {
        return None;
    }
    let query = match artist {
        Some(artist) => format!("{artist} {title}"),
        None => title.to_owned(),
    };
    match library.search(&query, 10).await {
        Ok(hits) => hits.into_iter().find(|t| {
            normalize(&t.title) == normalize(title)
                && artist.is_none_or(|a| normalize_artist(&t.artist) == normalize_artist(a))
        }),
        Err(e) => {
            library_unreachable(ok, &e);
            None
        }
    }
}

async fn in_library(library: &dyn Library, ok: &AtomicBool, track: &Track) -> bool {
    if !ok.load(Ordering::Relaxed) {
        return false;
    }
    match library.saved(std::slice::from_ref(&track.id)).await {
        Ok(saved) => !saved.is_empty(),
        Err(e) => {
            library_unreachable(ok, &e);
            false
        }
    }
}

/// One failure stops library lookups for the rest of the file; the preview says so
/// (`libraryChecked: false`) instead of failing the whole import.
fn library_unreachable(ok: &AtomicBool, e: &Error) {
    if ok.swap(false, Ordering::Relaxed) {
        tracing::warn!(error = %e, "M3U import continues without the library");
    }
}

pub(crate) fn unsaved_track(artist: &str, album: &str, title: &str, duration: Option<u32>) -> Track {
    Track {
        id: TrackId::of(artist, album, title),
        title: title.to_owned(),
        artist_id: ArtistId::of(artist),
        artist: artist.to_owned(),
        album_id: AlbumId::of(artist, album),
        album: album.to_owned(),
        year: None,
        no: None,
        disc: None,
        duration,
        tags: Vec::new(),
        saved: false,
        file: None,
    }
}
