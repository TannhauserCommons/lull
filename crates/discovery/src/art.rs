//! `GET /api/catalog/art/{id}?size=96|256|512|1024|2048`: covers and artist photos, served by the
//! hub's picture service (`lull_core::pictures`, ADR-0033) from lull, so the UI never loads an
//! outside image and the CSP stays same-origin. This is where they come from (`Covers`, the
//! service's finder) and how entities point at them:
//! - an album's cover: the source the catalog recorded (Deezer, iTunes), else found by name:
//!   Last.fm, then iTunes from what search leaves spare, then the Cover Art Archive by
//!   release-group MBID; the source found is recorded, so every size reuses it;
//! - an artist's photo: only a vetted one (`artist::photo`);
//! - a search playlist's picture: its source, remembered a month.
//!
//! Entities point at this route as `art` (albums) whenever lull can name a source or look one
//! up, and as `image` (artists) only for a vetted photo.

use std::collections::BTreeMap;
use std::sync::{Arc, Weak};

use async_trait::async_trait;
use axum::extract::{Path, Query, State};
use axum::response::Response;
use lull_core::ids::{AlbumId, ArtistId, TrackId};
use lull_core::model::{Album, Artist, Entities, Saved, Track};
use lull_core::pictures::{Finder, Found, Source};
use lull_core::{Error, Result, catalog};
use serde::{Deserialize, Serialize};

use crate::Service;

const DEFAULT_SIZE: u32 = 256;
/// Covers fetched for the songs after the playing one.
const UP_NEXT: usize = 3;
/// The picture service's lanes for looking a cover up by name: one per outside service.
const LASTFM: &str = "lastfm";
const ITUNES: &str = "itunes";
/// The finder's steps, in order.
const BY_LASTFM: u8 = 0;
const BY_ITUNES: u8 = 1;

/// An album as the UI gets it from discovery: with `art` when lull serves its cover.
#[derive(Debug, Clone, Serialize)]
pub struct PicturedAlbum {
    #[serde(flatten)]
    pub album: Album,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub art: Option<String>,
}

/// An artist with `image` when lull serves a photo.
#[derive(Debug, Clone, Serialize)]
pub struct PicturedArtist {
    #[serde(flatten)]
    pub artist: Artist,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
}

/// `Entities` with pictures (docs/context/api.md#conventions).
#[derive(Debug, Clone, Default, Serialize)]
pub struct Pictured {
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub tracks: BTreeMap<TrackId, Track>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub albums: BTreeMap<AlbumId, PicturedAlbum>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub artists: BTreeMap<ArtistId, PicturedArtist>,
}

/// Same-origin picture URLs for what lull can picture, except where it learnt lately that no
/// source has one (a new release nobody has a cover for yet): the UI shows its placeholder at
/// once instead of asking for a 404 on every load. Nothing is fetched here: what the page then
/// shows is, as its screen reports it (ADR-0033).
pub async fn pictured(svc: &Service, e: Entities) -> Result<Pictured> {
    let mut out = svc.ctx.db.call(move |c| picture(c, e)).await?;
    let pictures = &svc.ctx.pictures;
    for (id, album) in &mut out.albums {
        if album.art.is_some() && pictures.none_left(id.as_str()).await.is_some() {
            album.art = None;
        }
    }
    for (id, artist) in &mut out.artists {
        if artist.image.is_some() && pictures.none_left(id.as_str()).await.is_some() {
            artist.image = None;
        }
    }
    let from_library: Vec<AlbumId> = out
        .albums
        .iter()
        .filter(|(_, a)| a.art.is_none() && a.album.saved != Saved::None)
        .map(|(id, _)| id.clone())
        .collect();
    svc.store.warm_covers(&from_library);
    Ok(out)
}

/// `pictured` on a connection. An album gets `art` when its source is known, or when it isn't
/// saved and has a name to look one up by (a saved album's own cover is library's `/api/art`);
/// it is recorded, so the route can name it.
pub fn picture(c: &rusqlite::Connection, e: Entities) -> Result<Pictured> {
    let mut out = Pictured {
        tracks: e.tracks,
        ..Pictured::default()
    };
    for (id, album) in e.albums {
        let known = catalog::album_art(c, &id)?.is_some();
        let findable = !known && album.saved == Saved::None && !album.title.trim().is_empty();
        if findable {
            catalog::remember_album(c, &album, None)?;
        }
        let art = (known || findable).then(|| url(id.as_str()));
        out.albums.insert(id, PicturedAlbum { album, art });
    }
    for (id, artist) in e.artists {
        // Only a vetted photo: what search recorded may be a cover (artist/photo.rs). Its version
        // makes a browser holding an old picture ask again.
        let image = crate::artist::photo::image_of(c, &id)?.map(|source| image_url(&id, &source));
        out.artists.insert(id, PicturedArtist { artist, image });
    }
    Ok(out)
}

/// The route of an artist's vetted photo, versioned by its source.
pub fn image_url(id: &ArtistId, source: &str) -> String {
    format!("{}?v={}", url(id.as_str()), crate::artist::photo::version(source))
}

/// The Cover Art Archive's front cover of a release group, as a picture source.
pub fn cover_archive(release_group: &str) -> String {
    format!("https://coverartarchive.org/release-group/{release_group}/front-{{size}}")
}

/// The prefix of a playlist picture's id: `pls-` and the playlist's link key in hex, so the id
/// says which picture it is and stays a plain path segment.
const PLAYLIST_PICTURE: &str = "pls-";

/// The route for an outside playlist's picture (search's Playlists section), its source
/// remembered for the route a month.
pub async fn playlist_picture(svc: &Service, link: &str, source: &str) -> Result<String> {
    let id = format!("{PLAYLIST_PICTURE}{}", hex_of(link));
    svc.cache
        .put(format!("playlist-picture:{id}"), &source.to_owned())
        .await?;
    Ok(url(&id))
}

fn hex_of(s: &str) -> String {
    s.bytes().map(|b| format!("{b:02x}")).collect()
}

/// The route for an album's or artist's picture; the UI adds `?size=`. Relative to the page, as
/// the UI's own requests are, so lull also works behind a reverse-proxy sub-path.
pub fn url(id: &str) -> String {
    format!("api/catalog/art/{id}")
}

#[derive(Deserialize)]
pub struct Size {
    #[serde(default = "default_size")]
    size: u32,
}

fn default_size() -> u32 {
    DEFAULT_SIZE
}

pub async fn get(State(svc): State<Arc<Service>>, Path(id): Path<String>, Query(q): Query<Size>) -> Result<Response> {
    Ok(svc.ctx.pictures.get(&id, q.size).await?.into_response(&id))
}

/// Where pictures come from, for the picture service: discovery's catalog, artist photos and
/// search playlists. Holds the service weakly: the service holds the context that holds this.
pub struct Covers(pub Weak<Service>);

#[async_trait]
impl Finder for Covers {
    async fn source(&self, id: &str) -> Result<Source> {
        let Some(svc) = self.0.upgrade() else {
            return Ok(Source::Unknown);
        };
        if id.starts_with(AlbumId::PREFIX) {
            let album = AlbumId(id.to_owned());
            let (art, named) = svc
                .ctx
                .db
                .call(move |c| Ok((catalog::album_art(c, &album)?, name_of(c, &album)?.is_some())))
                .await?;
            return Ok(match (art, named) {
                (Some(url), _) => Source::Url(url),
                (None, true) => first_step(&svc),
                (None, false) => Source::Unknown,
            });
        }
        if id.starts_with(ArtistId::PREFIX) {
            let artist = ArtistId(id.to_owned());
            let photo = svc
                .ctx
                .db
                .call(move |c| crate::artist::photo::image_of(c, &artist))
                .await?;
            return Ok(photo.map_or(Source::Unknown, Source::Url));
        }
        if id.starts_with(PLAYLIST_PICTURE) {
            let source = svc
                .cache
                .fresh::<String>(format!("playlist-picture:{id}"), 30 * crate::sources::cache::DAY)
                .await?;
            return Ok(source.map_or(Source::Unknown, Source::Url));
        }
        Ok(Source::Unknown)
    }

    /// Looks an album's cover up by its name, one service a step; the first found is recorded.
    async fn find(&self, id: &str, step: u8) -> Result<Found> {
        let Some(svc) = self.0.upgrade() else {
            return Ok(Found::Missing);
        };
        let album = AlbumId(id.to_owned());
        let asked = album.clone();
        let Some(name) = svc.ctx.db.call(move |c| name_of(c, &asked)).await? else {
            return Ok(Found::Missing);
        };
        let found = match step {
            BY_LASTFM => match svc.lastfm() {
                Some(lastfm) => lastfm.album_cover(&name.artist, &name.title).await?,
                None => None,
            },
            BY_ITUNES => svc.itunes.album_cover(&name.artist, &name.title).await?,
            _ => None,
        };
        let found = match (found, step) {
            (Some(url), _) => url,
            (None, BY_LASTFM) => {
                return Ok(Found::Next {
                    lane: ITUNES,
                    step: BY_ITUNES,
                });
            }
            (None, _) => match name.release_group {
                Some(group) => cover_archive(&group),
                None => return Ok(Found::Missing),
            },
        };
        let recorded = found.clone();
        svc.ctx
            .db
            .call(move |c| catalog::found_album_art(c, &album, &recorded))
            .await?;
        Ok(Found::Url(found))
    }
}

/// The first lane an album's cover is looked up in: Last.fm when lull may call it.
fn first_step(svc: &Service) -> Source {
    if svc.lastfm().is_some() {
        Source::Find {
            lane: LASTFM,
            step: BY_LASTFM,
        }
    } else {
        Source::Find {
            lane: ITUNES,
            step: BY_ITUNES,
        }
    }
}

/// What an album is called, to look its cover up by.
struct Name {
    artist: String,
    title: String,
    release_group: Option<String>,
}

/// The album's name from its catalog record, else from a song on it (a song lull knows only by
/// its source's words). `None` without a title to search for.
fn name_of(c: &rusqlite::Connection, id: &AlbumId) -> Result<Option<Name>> {
    let name = match catalog::album(c, id)? {
        Some(a) => Name {
            artist: a.artist,
            title: a.title,
            release_group: a.musicbrainz_id,
        },
        None => match catalog::track_on_album(c, id)? {
            Some(t) => Name {
                artist: t.artist,
                title: t.album,
                release_group: None,
            },
            None => return Ok(None),
        },
    };
    Ok((!name.title.trim().is_empty() && !name.artist.trim().is_empty()).then_some(name))
}

/// Songs played next, from `device.state`: the covers of the playing song's album (at every size
/// Now Playing may want) and the next songs' (the player bar's and Now Playing's) go ahead of
/// everything a screen shows. Saved songs' albums are library's. A state that names the same
/// songs again changes nothing.
pub async fn playing(svc: &Service, data: serde_json::Value) -> Result<()> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Playing {
        track_id: Option<TrackId>,
        #[serde(default)]
        queue: Vec<TrackId>,
        queue_index: Option<usize>,
    }
    let p: Playing = serde_json::from_value(data).map_err(|e| Error::internal(format!("device.state payload: {e}")))?;
    let next = p.queue_index.map_or(0, |i| i + 1);
    let songs: Vec<TrackId> = p
        .track_id
        .into_iter()
        .chain(p.queue.into_iter().skip(next).take(UP_NEXT))
        .collect();
    let albums = svc
        .ctx
        .db
        .call(move |c| {
            let mut albums: Vec<AlbumId> = Vec::new();
            for song in &songs {
                if let Some(track) = catalog::track(c, song)?
                    && !track.saved
                    && !albums.contains(&track.album_id)
                {
                    albums.push(track.album_id);
                }
            }
            Ok(albums)
        })
        .await?;
    let sized = albums
        .into_iter()
        .enumerate()
        .map(|(i, id)| {
            let sizes = if i == 0 { vec![256, 512, 1024] } else { vec![256, 512] };
            (id.0, sizes)
        })
        .collect();
    svc.ctx.pictures.set_playing("discovery", sized).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use lull_core::db::Db;
    use lull_core::ids::{AlbumId, ArtistId};
    use lull_core::model::{Album, Artist, Entities, Saved};
    use lull_core::{Result, catalog};

    use super::picture;

    fn album(title: &str, saved: Saved) -> Album {
        Album {
            id: AlbumId::of("Velvet Orchard", title),
            title: title.into(),
            artist_id: ArtistId::of("Velvet Orchard"),
            artist: "Velvet Orchard".into(),
            year: None,
            kind: Some("album".into()),
            released: None,
            label: None,
            tags: vec![],
            saved,
            track_ids: vec![],
            duration: None,
            musicbrainz_id: Some("0f3c0c64-5b0c-4d0b-9c0c-9a3a2c5b1e11".into()),
        }
    }

    #[tokio::test]
    async fn an_artist_picture_search_recorded_is_not_advertised_until_vetted() -> Result<()> {
        // Live, BIJI's first page visit advertised Deezer's "photo" (the Edgerunners cover).
        let db = Db::memory()?;
        db.migrate(&catalog::MIGRATIONS)?;
        db.migrate(&crate::MIGRATIONS)?;
        let biji = Artist {
            id: ArtistId::of("BIJI"),
            name: "BIJI".into(),
            tags: vec![],
            listeners: None,
            bio: None,
            musicbrainz_id: None,
        };
        let id = biji.id.clone();
        let e = || Entities {
            artists: [(biji.id.clone(), biji.clone())].into(),
            ..Entities::default()
        };
        let (first, e1) = (id.clone(), e());
        let unvetted = db
            .call(move |c| {
                catalog::remember_artist(c, &e1.artists[&first], Some("https://cdn.example/cover/{size}.jpg"))?;
                Ok(picture(c, e1)?.artists[&first].image.clone())
            })
            .await?;
        assert_eq!(unvetted, None, "search's picture may be a cover");
        let (then, e2) = (id.clone(), e());
        let vetted = db
            .call(move |c| {
                c.execute(
                    "INSERT INTO discovery_artist_photos (artist_id, image, checked_at) VALUES (?1, ?2, 0)",
                    [then.as_str(), "https://commons.example/biji.jpg?width={size}"],
                )?;
                Ok(picture(c, e2)?.artists[&then].image.clone())
            })
            .await?;
        assert!(vetted.is_some_and(|u| u.starts_with("api/catalog/art/ar-") && u.contains("?v=")));
        Ok(())
    }

    #[tokio::test]
    async fn an_album_without_a_known_source_gets_a_picture_to_look_up_unless_saved() -> Result<()> {
        // Live, Home's new releases had no `art`, and imported songs showed no cover until each
        // was resolved, at about 12 a minute: now the cover is looked up by name.
        let db = Db::memory()?;
        db.migrate(&catalog::MIGRATIONS)?;
        let new = album("Satellite Porch", Saved::None);
        let saved = album("Paper Moon Static", Saved::All);
        let (new_id, saved_id) = (new.id.clone(), saved.id.clone());
        let e = Entities {
            albums: [(new.id.clone(), new), (saved.id.clone(), saved)].into(),
            ..Entities::default()
        };
        let asked = new_id.clone();
        let (out, recorded) = db
            .call(move |c| Ok((picture(c, e)?, catalog::album(c, &asked)?.is_some())))
            .await?;
        assert_eq!(
            out.albums[&new_id].art.as_deref(),
            Some(super::url(new_id.as_str()).as_str())
        );
        assert!(recorded, "the route can name it");
        assert!(out.albums[&saved_id].art.is_none(), "library's own cover");
        Ok(())
    }
}
