//! `GET /api/albums/{id}`, `/api/artists/{id}`, `/api/tags/{id}`. An id is resolved through
//! what discovery has shown before or the catalog; the page is then filled from MusicBrainz,
//! ListenBrainz and (when configured) Last.fm. A section whose source failed is empty and named
//! in `errors`, so the rest of the page still shows.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use lull_core::ids::{AlbumId, ArtistId, TrackId};
use lull_core::model::{Album, Artist, Entities};
use lull_core::{Error, Result};
use serde::Serialize;

use crate::art::{self, Pictured, PicturedArtist};
use crate::sources::listenbrainz::Mode;
use crate::sources::{ArtistInfo, Release, Song, top_tags};
use crate::store::{self, entities};
use crate::{Service, taste};

const SIMILAR: usize = 12;
const TOP_TRACKS: usize = 10;
const TAG_ARTISTS: usize = 12;
const TAG_RELEASES: usize = 12;
/// "New" on a tag page: first released within this many days.
const TAG_NEW_DAYS: i64 = 90;

type Errors = BTreeMap<&'static str, String>;

#[derive(Serialize)]
pub struct AlbumPage {
    album: Album,
    entities: Pictured,
}

pub async fn album(State(svc): State<Arc<Service>>, Path(id): Path<String>) -> Result<Json<AlbumPage>> {
    let id = AlbumId(checked(id, AlbumId::PREFIX)?);
    let known = svc
        .store
        .album_ref(id.clone())
        .await?
        .ok_or_else(|| Error::NotFound(format!("Album {id} is unknown to lull.")))?;

    let (mut album, songs) = match from_musicbrainz(&svc, &known).await {
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
    let album = svc.store.full_album(album, &tracks).await?;
    let artist = svc
        .store
        .artists(vec![store::artist(ArtistInfo {
            mbid: None,
            name: album.artist.clone(),
        })])
        .await?;
    Ok(Json(AlbumPage {
        entities: art::pictured(&svc, entities(tracks, vec![album.clone()], artist)).await?,
        album,
    }))
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
    let mut errors = Errors::new();
    if artist.musicbrainz_id.is_none() {
        artist.musicbrainz_id = section(&mut errors, "artist", svc.mb.find_artist(&artist.name).await)
            .flatten()
            .and_then(|a| a.mbid);
    }
    let info = ArtistInfo {
        mbid: artist.musicbrainz_id.clone(),
        name: artist.name.clone(),
    };

    if let Some(mbid) = &info.mbid
        && let Some(tags) = section(&mut errors, "tags", svc.mb.artist_tags(mbid).await)
    {
        artist.tags = tags;
    }
    if let Some(lastfm) = svc.lastfm()
        && let Some(profile) = section(&mut errors, "bio", lastfm.artist(&artist.name).await)
    {
        artist.listeners = profile.listeners;
        artist.bio = profile.bio;
        if artist.tags.is_empty() {
            artist.tags = profile.tags;
        }
    }

    let releases = section(&mut errors, "albums", svc.mb.discography(&info).await).unwrap_or_default();
    let albums = svc.store.albums(releases).await?;
    let songs = section(&mut errors, "topTracks", top_songs(&svc, &info).await).unwrap_or_default();
    let tracks = svc.store.tracks(songs).await?;
    let similar = section(&mut errors, "similar", similar_artists(&svc, &info).await).unwrap_or_default();
    let similar = svc
        .store
        .artists(similar.into_iter().map(store::artist).collect())
        .await?;

    artist.id = id;
    let mut all_artists = svc.store.artists(vec![artist.clone()]).await?;
    remember_photo(&svc, &artist).await;
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
        (Some(lastfm), _) => lastfm.similar_artists(&artist.name).await?,
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
    entities: Entities,
}

pub async fn tag(State(svc): State<Arc<Service>>, Path(id): Path<String>) -> Result<Json<TagPage>> {
    let tag = id.trim().to_lowercase();
    if tag.is_empty() {
        return Err(Error::BadRequest("a tag needs a name".into()));
    }
    let mut errors = Errors::new();
    let since = iso_date(lull_core::now() - TAG_NEW_DAYS * 86_400);
    let mut releases =
        section(&mut errors, "newReleases", svc.mb.tagged_releases(&tag, &since).await).unwrap_or_default();
    let co_tagged = co_tags(&tag, &releases);
    releases.truncate(TAG_RELEASES);

    let (desc, related, artists) = match svc.lastfm() {
        Some(lastfm) => (
            section(&mut errors, "desc", lastfm.tag_description(&tag).await).flatten(),
            section(&mut errors, "related", lastfm.similar_tags(&tag).await).unwrap_or_default(),
            section(&mut errors, "artists", lastfm.tag_artists(&tag).await).unwrap_or_default(),
        ),
        None => (
            None,
            Vec::new(),
            section(&mut errors, "artists", tag_artists_from_radio(&svc, &tag).await).unwrap_or_default(),
        ),
    };
    let related = if related.is_empty() { co_tagged } else { related };

    let albums = svc.store.albums(releases).await?;
    let artists = svc
        .store
        .artists(artists.into_iter().take(TAG_ARTISTS).map(store::artist).collect())
        .await?;
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
        entities: entities(Vec::new(), albums, artists),
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
