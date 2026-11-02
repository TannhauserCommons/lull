//! `GET /api/albums/{id}`, `/api/artists/{id}`, `/api/tags/{id}`. An id is resolved through
//! what discovery has shown before or the catalog; the page is then filled from MusicBrainz,
//! ListenBrainz and (when configured) Last.fm. A section whose source failed is empty and named
//! in `errors`, so the rest of the page still shows.
//!
//! An album a catalog found (iTunes, Deezer) takes its tracklist from that catalog in one call;
//! MusicBrainz, paced and shared with every lull crate, then only adds its facts in the
//! background. The album page answers within `ALBUM_DEADLINE` with what lull has, and
//! announces the full tracklist as `album.updated` when it arrives later.

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::{Path, State};
use lull_core::ids::{AlbumId, ArtistId, TrackId};
use lull_core::model::Track;
use lull_core::model::{Album, Artist};
use lull_core::{Error, Result};
use serde::Serialize;
use tokio::sync::oneshot;
use tokio::time::Instant;

use crate::art::{self, Pictured, PicturedArtist};
use crate::sources::listenbrainz::Mode;
use crate::sources::{AlbumSource, ArtistInfo, Release, Song, top_tags};
use crate::store::{self, entities};
use crate::{Service, taste};

const SIMILAR: usize = 12;
const TOP_TRACKS: usize = 10;
const TAG_ARTISTS: usize = 12;
const TAG_RELEASES: usize = 12;
/// "New" on a tag page: first released within this many days.
const TAG_NEW_DAYS: i64 = 90;

/// How long the album page waits for its tracklist before answering with what lull has.
const ALBUM_DEADLINE: Duration = Duration::from_secs(3);
/// How long the artist and tag pages wait for their sections.
const PAGE_DEADLINE: Duration = Duration::from_secs(4);

type Errors = BTreeMap<&'static str, String>;

#[derive(Serialize)]
pub struct AlbumPage {
    album: Album,
    /// The tracklist wasn't in by the deadline: the tracks lull had are shown, and
    /// `album.updated` follows when the rest arrives.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    partial: bool,
    entities: Pictured,
}

/// An album with its tracklist, and whether a catalog (not MusicBrainz) gave it.
struct Filled {
    album: Album,
    tracks: Vec<Track>,
    from_catalog: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AlbumUpdated {
    album_id: AlbumId,
}

pub async fn album(State(svc): State<Arc<Service>>, Path(id): Path<String>) -> Result<Json<AlbumPage>> {
    let id = AlbumId(checked(id, AlbumId::PREFIX)?);
    let mut known = svc
        .store
        .album_ref(id.clone())
        .await?
        .ok_or_else(|| Error::NotFound(format!("Album {id} is unknown to lull.")))?;
    known.id = id.clone();

    // Filled in a task of its own, so a tracklist that misses the deadline is still fetched,
    // remembered and announced.
    let started = Instant::now();
    let (tx, rx) = oneshot::channel();
    let task = (svc.clone(), known.clone());
    svc.ctx.spawn(async move {
        let (svc, known) = task;
        let filled = fill(&svc, known).await;
        let to_enrich = filled.as_ref().ok().filter(|f| f.from_catalog).map(|f| f.album.clone());
        if let Err(Ok(late)) = tx.send(filled) {
            tracing::info!(album = %late.album.id, ms = started.elapsed().as_millis() as u64, "album tracklist arrived after the page");
            svc.ctx.bus.publish("album.updated", AlbumUpdated { album_id: late.album.id });
        }
        if let Some(album) = to_enrich {
            enrich(&svc, album).await;
        }
    });
    let (album, tracks, partial) = match tokio::time::timeout(ALBUM_DEADLINE, rx).await {
        Ok(Ok(filled)) => {
            let f = filled?;
            (f.album, f.tracks, false)
        }
        Ok(Err(_)) => return Err(Error::unavailable("discovery", "is restarting")),
        Err(_) => {
            let tracks = svc.store.catalog_album_tracks(id.clone()).await?;
            (store::with_tracks(known, &tracks), tracks, true)
        }
    };
    tracing::info!(album = %id, ms = started.elapsed().as_millis() as u64, partial, "album page answered");
    let artist = svc
        .store
        .artists(vec![store::artist(ArtistInfo {
            mbid: None,
            name: album.artist.clone(),
        })])
        .await?;
    Ok(Json(AlbumPage {
        entities: art::pictured(&svc, entities(tracks, vec![album.clone()], artist)).await?,
        partial,
        album,
    }))
}

/// The album's tracklist, remembered with it: from the catalog that found it, else from
/// MusicBrainz, else the catalog tracks lull already has of it.
async fn fill(svc: &Service, known: Album) -> Result<Filled> {
    let id = known.id.clone();
    if let Some(source) = svc.store.album_source(id.clone()).await? {
        match from_catalog(svc, source, &known).await {
            Ok(songs) if !songs.is_empty() => {
                let tracks = svc.store.tracks(songs).await?;
                let album = svc.store.full_album(known, &tracks).await?;
                return Ok(Filled {
                    album,
                    tracks,
                    from_catalog: true,
                });
            }
            Ok(_) => tracing::info!(album = %id, ?source, "the catalog lists no tracks for it"),
            Err(e) => tracing::info!(error = %e, album = %id, ?source, "album tracklist from MusicBrainz instead"),
        }
    }
    let (mut album, songs) = match from_musicbrainz(svc, &known).await {
        Ok(Some(found)) => found,
        Ok(None) => (known, Vec::new()),
        Err(e) => {
            tracing::info!(error = %e, album = %id, "album from the catalog only");
            (known, Vec::new())
        }
    };
    album.id = id.clone();
    let tracks = if songs.is_empty() {
        svc.store.catalog_album_tracks(id).await?
    } else {
        svc.store.tracks(songs).await?
    };
    Ok(Filled {
        album: svc.store.full_album(album, &tracks).await?,
        tracks,
        from_catalog: false,
    })
}

/// The tracklist of the catalog album behind `known`, as songs of `known` (its title and artist,
/// so the tracks carry its id).
async fn from_catalog(svc: &Service, source: AlbumSource, known: &Album) -> Result<Vec<Song>> {
    let songs = match source {
        AlbumSource::Itunes(id) => svc.itunes.tracklist(id).await?,
        AlbumSource::Deezer(id) => svc.deezer.tracklist(id).await?,
    };
    Ok(songs
        .into_iter()
        .map(|s| Song {
            album: known.title.clone(),
            album_artist: Some(known.artist.clone()),
            album_mbid: known.musicbrainz_id.clone(),
            year: known.year.or(s.year),
            tags: known.tags.clone(),
            ..s
        })
        .collect())
}

/// Adds what MusicBrainz knows of an album a catalog gave (its MBID, tags, type, date).
async fn enrich(svc: &Service, album: Album) {
    if !svc.mb.switch.is_on() {
        return;
    }
    let group = match &album.musicbrainz_id {
        Some(mbid) => svc.mb.release(mbid).await.map(Some),
        None => svc.mb.find_release(&album.artist, &album.title).await,
    };
    let outcome = match group {
        Ok(Some(group)) => svc.store.enrich_album(album.id.clone(), group).await,
        Ok(None) => Ok(()),
        Err(e) => Err(e),
    };
    if let Err(e) = outcome {
        tracing::info!(error = %e, album = %album.id, "album not enriched from MusicBrainz");
    }
}

/// The album's release group and tracklist, when MusicBrainz knows it.
async fn from_musicbrainz(svc: &Service, known: &Album) -> Result<Option<(Album, Vec<Song>)>> {
    let group = match &known.musicbrainz_id {
        Some(mbid) => Some(svc.mb.release(mbid).await?),
        None => svc.mb.find_release(&known.artist, &known.title).await?,
    };
    let Some(group) = group else { return Ok(None) };
    let songs = svc.mb.tracklist(&group).await?;
    Ok(Some((store::album(group), songs)))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtistPage {
    artist: PicturedArtist,
    top_tracks: Vec<TrackId>,
    albums: Vec<AlbumId>,
    similar: Vec<ArtistId>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    errors: Errors,
    entities: Pictured,
}

pub async fn artist(State(svc): State<Arc<Service>>, Path(id): Path<String>) -> Result<Json<ArtistPage>> {
    let id = ArtistId(checked(id, ArtistId::PREFIX)?);
    let mut artist = svc
        .store
        .artist_ref(id.clone())
        .await?
        .ok_or_else(|| Error::NotFound(format!("Artist {id} is unknown to lull.")))?;
    let until = Instant::now() + PAGE_DEADLINE;
    let mut errors = Errors::new();

    // Who the artist is: the MBID a release named, else Last.fm's (by name, its best-known
    // artist of that name), else MusicBrainz's. Asked side by side.
    let (name, mbid) = (artist.name.clone(), artist.musicbrainz_id.clone());
    let profile = svc.lastfm().map(|_| {
        let (s, name, mbid) = (svc.clone(), name.clone(), mbid.clone());
        background(&svc, async move {
            match s.lastfm() {
                Some(lastfm) => lastfm.artist(&name, mbid.as_deref()).await,
                None => Err(Error::unavailable(
                    "Last.fm",
                    "it is switched off in Settings > Discovery",
                )),
            }
        })
    });
    let found = mbid.is_none().then(|| {
        let (s, name) = (svc.clone(), name.clone());
        background(&svc, async move { s.mb.find_artist(&name).await })
    });
    let profile = match profile {
        Some(rx) => section(&mut errors, "bio", by(until, rx).await),
        None => None,
    };
    if artist.musicbrainz_id.is_none() {
        artist.musicbrainz_id = profile.as_ref().and_then(|p| p.mbid.clone());
    }
    if let Some(rx) = found {
        let found = section(&mut errors, "artist", by(until, rx).await).flatten();
        if artist.musicbrainz_id.is_none() {
            artist.musicbrainz_id = found.and_then(|a| a.mbid);
        } else {
            errors.remove("artist");
        }
    }
    if let Some(profile) = profile {
        artist.listeners = profile.listeners;
        artist.bio = profile.bio;
        artist.tags = profile.tags;
    }
    artist.id = id;
    let info = ArtistInfo {
        mbid: artist.musicbrainz_id.clone(),
        name: artist.name.clone(),
    };

    // Every section at once, each in a task of its own: one that misses the deadline is named
    // in `errors` and is cached for the next visit.
    let tags = info.mbid.clone().map(|mbid| {
        let s = svc.clone();
        background(&svc, async move { s.mb.artist_tags(&mbid).await })
    });
    let releases = {
        let (s, i) = (svc.clone(), info.clone());
        background(&svc, async move { s.mb.discography(&i).await })
    };
    let top = {
        let (s, i) = (svc.clone(), info.clone());
        background(&svc, async move { top_songs(&s, &i).await })
    };
    let similar = {
        let (s, i) = (svc.clone(), info.clone());
        background(&svc, async move { similar_artists(&s, &i).await })
    };
    let photo = {
        let (s, a) = (svc.clone(), artist.clone());
        background(&svc, async move {
            remember_photo(&s, &a).await;
            Ok(())
        })
    };
    let tags = match tags {
        Some(rx) => section(&mut errors, "tags", by(until, rx).await),
        None => None,
    };
    if let Some(tags) = tags.filter(|t| !t.is_empty()) {
        artist.tags = tags;
    }
    let releases = section(&mut errors, "albums", by(until, releases).await).unwrap_or_default();
    let songs = section(&mut errors, "topTracks", by(until, top).await).unwrap_or_default();
    let similar = section(&mut errors, "similar", by(until, similar).await).unwrap_or_default();
    let _ = by(until, photo).await;

    let albums = svc.store.albums(releases).await?;
    let tracks = svc.store.tracks(songs).await?;
    let similar = svc
        .store
        .artists(similar.into_iter().map(store::artist).collect())
        .await?;
    let mut all_artists = svc.store.artists(vec![artist.clone()]).await?;
    all_artists.extend(similar.iter().cloned());
    let top_tracks = tracks.iter().map(|t| t.id.clone()).collect();
    let album_ids = albums.iter().map(|a| a.id.clone()).collect();
    let similar = similar.iter().map(|a| a.id.clone()).collect();
    let entities = art::pictured(&svc, entities(tracks, albums, all_artists)).await?;
    Ok(Json(ArtistPage {
        top_tracks,
        albums: album_ids,
        similar,
        artist: entities
            .artists
            .get(&artist.id)
            .cloned()
            .unwrap_or(PicturedArtist { artist, image: None }),
        errors,
        entities,
    }))
}

/// Runs a page section in a task of its own, so a fetch that misses the page's deadline still
/// ends and fills the cache for the next visit.
fn background<T: Send + 'static>(
    svc: &Service,
    section: impl Future<Output = Result<T>> + Send + 'static,
) -> oneshot::Receiver<Result<T>> {
    let (tx, rx) = oneshot::channel();
    svc.ctx.spawn(async move {
        // The page stopped waiting: the answer is cached all the same.
        let _ = tx.send(section.await);
    });
    rx
}

/// A section's answer by `until`, or why it is missing.
async fn by<T>(until: Instant, section: oneshot::Receiver<Result<T>>) -> Result<T> {
    match tokio::time::timeout_at(until, section).await {
        Ok(Ok(answer)) => answer,
        Ok(Err(_)) => Err(Error::unavailable("discovery", "is restarting")),
        Err(_) => Err(Error::unavailable(
            "this section",
            format!(
                "took over {} s; it shows when the page is opened again",
                PAGE_DEADLINE.as_secs()
            ),
        )),
    }
}

/// The artist's Deezer photo, found by name once and kept in the catalog. Pages show without one
/// when Deezer can't be asked.
async fn remember_photo(svc: &Service, artist: &Artist) {
    let id = artist.id.clone();
    let known = svc.ctx.db.call(move |c| lull_core::catalog::artist_image(c, &id)).await;
    if !matches!(known, Ok(None)) {
        return;
    }
    let image = match svc.deezer.artist_image(&artist.name).await {
        Ok(Some(image)) => image,
        Ok(None) => return,
        Err(e) => {
            tracing::info!(artist = %artist.name, error = %e, "no artist photo from Deezer");
            return;
        }
    };
    if let Err(e) = svc
        .store
        .remember_pictures(Vec::new(), vec![(artist.clone(), Some(image))])
        .await
    {
        tracing::warn!(error = %e, "could not record an artist photo");
    }
}

/// The artist's most played recordings on ListenBrainz (artist radio with no similar artists).
async fn top_songs(svc: &Service, artist: &ArtistInfo) -> Result<Vec<Song>> {
    let Some(mbid) = &artist.mbid else {
        return Ok(Vec::new());
    };
    let mut picks = svc.lb.artist_radio(mbid, Mode::Easy, 0, 30, (0, 100)).await?;
    picks.retain(|p| &p.artist_mbid == mbid);
    picks.sort_by_key(|p| std::cmp::Reverse(p.listens));
    let mbids: Vec<String> = picks.into_iter().map(|p| p.recording_mbid).collect();
    let mut songs = svc.lb.songs(&mbids).await?;
    // The same song on several releases is one row.
    let mut seen = std::collections::HashSet::new();
    songs.retain(|s| seen.insert(lull_core::ids::normalize(&s.title)));
    songs.truncate(TOP_TRACKS);
    Ok(songs)
}

/// Last.fm's similar artists when configured, else ListenBrainz's.
async fn similar_artists(svc: &Service, artist: &ArtistInfo) -> Result<Vec<ArtistInfo>> {
    let mut similar = match (svc.lastfm(), &artist.mbid) {
        (Some(lastfm), mbid) => lastfm.similar_artists(&artist.name, mbid.as_deref()).await?,
        (None, Some(mbid)) => svc.lb.similar_artists(mbid).await?,
        (None, None) => Vec::new(),
    };
    similar.truncate(SIMILAR);
    Ok(similar)
}

#[derive(Serialize)]
pub struct TagInfo {
    id: String,
    name: String,
    desc: Option<String>,
    related: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TagPage {
    tag: TagInfo,
    artists: Vec<ArtistId>,
    new_releases: Vec<AlbumId>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    errors: Errors,
    entities: Pictured,
}

pub async fn tag(State(svc): State<Arc<Service>>, Path(id): Path<String>) -> Result<Json<TagPage>> {
    let tag = id.trim().to_lowercase();
    if tag.is_empty() {
        return Err(Error::BadRequest("a tag needs a name".into()));
    }
    let until = Instant::now() + PAGE_DEADLINE;
    let mut errors = Errors::new();
    let since = iso_date(lull_core::now() - TAG_NEW_DAYS * 86_400);
    let releases = {
        let (s, t) = (svc.clone(), tag.clone());
        background(&svc, async move { s.mb.tagged_releases(&t, &since).await })
    };
    let lastfm = svc.lastfm().map(|_| {
        let (s, t) = (svc.clone(), tag.clone());
        background(&svc, async move {
            let Some(lastfm) = s.lastfm() else {
                return Err(Error::unavailable(
                    "Last.fm",
                    "it is switched off in Settings > Discovery",
                ));
            };
            let (desc, related, artists) = tokio::join!(
                lastfm.tag_description(&t),
                lastfm.similar_tags(&t),
                lastfm.tag_artists(&t)
            );
            Ok((desc, related, artists))
        })
    });
    let radio = lastfm.is_none().then(|| {
        let (s, t) = (svc.clone(), tag.clone());
        background(&svc, async move { tag_artists_from_radio(&s, &t).await })
    });

    let mut releases = section(&mut errors, "newReleases", by(until, releases).await).unwrap_or_default();
    let co_tagged = co_tags(&tag, &releases);
    releases.truncate(TAG_RELEASES);
    let (desc, related, artists) = match (lastfm, radio) {
        (Some(rx), _) => match section(&mut errors, "artists", by(until, rx).await) {
            Some((desc, related, artists)) => (
                section(&mut errors, "desc", desc).flatten(),
                section(&mut errors, "related", related).unwrap_or_default(),
                section(&mut errors, "artists", artists).unwrap_or_default(),
            ),
            None => (None, Vec::new(), Vec::new()),
        },
        (None, Some(rx)) => (
            None,
            Vec::new(),
            section(&mut errors, "artists", by(until, rx).await).unwrap_or_default(),
        ),
        (None, None) => (None, Vec::new(), Vec::new()),
    };
    let related = if related.is_empty() { co_tagged } else { related };
    if errors.is_empty() && desc.is_none() && releases.is_empty() && artists.is_empty() && related.is_empty() {
        return Err(Error::NotFound(format!("No source knows the tag {tag}.")));
    }

    let albums = svc.store.albums(releases).await?;
    let artists = svc
        .store
        .artists(artists.into_iter().take(TAG_ARTISTS).map(store::artist).collect())
        .await?;
    // Their photos, as on artist pages (found once, then kept); what misses the deadline shows
    // next time.
    let photos: Vec<_> = artists
        .iter()
        .map(|a| {
            let (s, a) = (svc.clone(), a.clone());
            background(&svc, async move {
                remember_photo(&s, &a).await;
                Ok(())
            })
        })
        .collect();
    for photo in photos {
        let _ = by(until, photo).await;
    }
    Ok(Json(TagPage {
        tag: TagInfo {
            id: tag.clone(),
            name: tag,
            desc,
            related: related.into_iter().take(8).collect(),
        },
        artists: artists.iter().map(|a| a.id.clone()).collect(),
        new_releases: albums.iter().map(|a| a.id.clone()).collect(),
        errors,
        entities: art::pictured(&svc, entities(Vec::new(), albums, artists)).await?,
    }))
}

/// Without Last.fm: the artists behind the tag's well-played recordings on ListenBrainz.
async fn tag_artists_from_radio(svc: &Service, tag: &str) -> Result<Vec<ArtistInfo>> {
    let mbids = svc.lb.tag_radio(tag, (50, 100), 50).await?;
    let songs = svc.lb.songs(&mbids).await?;
    let mut counts: HashMap<String, (usize, Option<String>)> = HashMap::new();
    for s in songs {
        let e = counts.entry(s.artist).or_insert((0, s.artist_mbid));
        e.0 += 1;
    }
    let mut ranked: Vec<_> = counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.0.cmp(&a.1.0).then_with(|| a.0.cmp(&b.0)));
    Ok(ranked
        .into_iter()
        .map(|(name, (_, mbid))| ArtistInfo { mbid, name })
        .collect())
}

/// Specific tags that appear alongside `tag` on at least two of its releases, most often first.
fn co_tags(tag: &str, releases: &[Release]) -> Vec<String> {
    let mut counts: HashMap<&str, i64> = HashMap::new();
    for t in releases.iter().flat_map(|r| &r.tags) {
        if t != tag && taste::is_specific(t) {
            *counts.entry(t).or_default() += 1;
        }
    }
    top_tags(
        counts
            .into_iter()
            .filter(|(_, n)| *n >= 2)
            .map(|(t, n)| (t.to_owned(), n))
            .collect(),
        8,
    )
}

/// Keeps a section's value, or records why it is missing.
fn section<T>(errors: &mut Errors, name: &'static str, result: Result<T>) -> Option<T> {
    match result {
        Ok(v) => Some(v),
        Err(e) => {
            errors.insert(name, e.to_string());
            None
        }
    }
}

fn checked(id: String, prefix: &str) -> Result<String> {
    if id.starts_with(prefix) {
        Ok(id)
    } else {
        Err(Error::BadRequest(format!("{id} is not an {prefix}… id")))
    }
}

/// Unix seconds to YYYY-MM-DD (UTC), by Howard Hinnant's civil-from-days.
fn iso_date(unix: i64) -> String {
    let z = unix.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    #[test]
    fn iso_date_matches_the_calendar() {
        assert_eq!(super::iso_date(0), "1970-01-01");
        assert_eq!(super::iso_date(1_793_404_800), "2026-10-31");
        assert_eq!(super::iso_date(951_782_400), "2000-02-29");
    }
}
