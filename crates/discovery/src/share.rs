//! `GET /api/share/{kind}/{id}`: a public link to a track, album or artist that opens in any
//! streaming service, with a cover anyone on the internet can load (the desktop app shows it
//! as Discord Rich Presence art). Songs and albums are found in the iTunes Store and linked
//! through Odesli's pages (`song.link/i/<trackId>`, `album.link/i/<collectionId>`): Odesli
//! closed its keyless API (401 `PUBLIC_API_ACCESS_DEPRECATED`, 2026-11-02), but its pages
//! still resolve an iTunes id to every service. Artists, and anything the store doesn't
//! list, get their Last.fm page. Answers are kept in the database; one is resolved at a time,
//! so the same link asked for twice at once costs one iTunes call.
//!
//! `link`, what "Copy link" copies, is that page followed by `#lull?` and the names and public
//! ids lull knows (`open::fragment`): any lull opens it as its own page (`open.rs`), and the
//! page ignores it. Never the hub's address.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use lull_core::ids::{AlbumId, ArtistId, TrackId, loose_artist, loose_title, normalize};
use lull_core::{Error, Result, catalog};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::Service;
use crate::art;
use crate::open::{self, Item};
use crate::sources::cache::DAY;
use crate::sources::itunes::{Entity, Listing};

/// A link found in the store holds; a song or album may reach the store later, so a Last.fm
/// fallback is looked for again sooner.
const KEEP_FOUND_S: i64 = 30 * DAY;
const KEEP_FALLBACK_S: i64 = DAY;
/// Covers handed out for sharing: big enough for Discord, small enough to load fast.
const ARTWORK_PX: u32 = 600;

/// `GET /api/share/{kind}/{id}` answer: the public page, and the lull link to it.
#[derive(Debug, Serialize)]
pub struct Answer {
    #[serde(flatten)]
    share: Share,
    link: String,
}

/// The public page found for an item, kept.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Share {
    pub url: String,
    pub source: Source,
    /// A public cover or photo URL; `None` when lull knows of none.
    pub artwork: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Songlink,
    Lastfm,
}

/// One resolution at a time: a second request for the same link waits and finds it cached.
#[derive(Default)]
pub struct Shares {
    one: Mutex<()>,
}

/// What a share link is for: what lull knows of it (`item`), and the album or artist whose
/// picture it shows.
enum Shared {
    Track { item: Item, album: AlbumId },
    Album { item: Item, id: AlbumId },
    Artist { item: Item, id: ArtistId },
}

impl Shared {
    fn item(&self) -> &Item {
        match self {
            Shared::Track { item, .. } | Shared::Album { item, .. } | Shared::Artist { item, .. } => item,
        }
    }

    /// The artist, and the song's or album's title (empty for an artist).
    fn names(&self) -> (&str, &str) {
        match self.item() {
            Item::Track { artist, title, .. } | Item::Album { artist, title, .. } => (artist, title),
            Item::Artist { name, .. } => (name, ""),
        }
    }
}

pub async fn get(State(svc): State<Arc<Service>>, Path((kind, id)): Path<(String, String)>) -> Result<Json<Answer>> {
    let shared = known(&svc, &kind, id.clone())
        .await?
        .ok_or_else(|| Error::NotFound(format!("{kind} {id} is unknown to lull.")))?;
    let _one = svc.shares.one.lock().await;
    let key = format!("share:{kind}:{id}");
    let ttl = |s: &Share| match s.source {
        Source::Songlink => KEEP_FOUND_S,
        Source::Lastfm => KEEP_FALLBACK_S,
    };
    let share = svc
        .cache
        .get_with(key, ttl, || resolve(&svc, &shared))
        .await
        .map_err(|e| match e {
            Error::Upstream { service, reason } => Error::Unavailable { service, reason },
            e => e,
        })?;
    let link = format!("{}#{}", share.url, open::fragment(shared.item()));
    Ok(Json(Answer { share, link }))
}

/// The track, album or artist an id stands for, with the album's catalog source; `None` for an
/// id lull never handed out.
async fn known(svc: &Service, kind: &str, id: String) -> Result<Option<Shared>> {
    Ok(match kind {
        "track" if id.starts_with(TrackId::PREFIX) => match svc.store.track(TrackId(id)).await? {
            Some(t) => {
                let on = svc.store.album_ref(t.album_id.clone()).await?;
                let album_artist = on.map(|a| a.artist).filter(|by| *by != t.artist);
                let album_source = svc.store.album_source(t.album_id.clone()).await?;
                Some(Shared::Track {
                    item: Item::Track {
                        artist: t.artist,
                        title: t.title,
                        album: t.album,
                        album_artist,
                        duration: t.duration,
                        album_source,
                    },
                    album: t.album_id,
                })
            }
            None => None,
        },
        "album" if id.starts_with(AlbumId::PREFIX) => {
            let id = AlbumId(id);
            match svc.store.album_ref(id.clone()).await? {
                Some(a) => Some(Shared::Album {
                    item: Item::Album {
                        artist: a.artist,
                        title: a.title,
                        mbid: a.musicbrainz_id,
                        source: svc.store.album_source(id.clone()).await?,
                    },
                    id,
                }),
                None => None,
            }
        }
        "artist" if id.starts_with(ArtistId::PREFIX) => {
            let id = ArtistId(id);
            svc.store.artist_ref(id.clone()).await?.map(|a| Shared::Artist {
                item: Item::Artist {
                    name: a.name,
                    mbid: a.musicbrainz_id,
                },
                id,
            })
        }
        _ => None,
    })
}

/// The iTunes Store's song or album behind an Odesli page, else the Last.fm page. Artwork is
/// the store's cover, else the picture the catalog already knows a public source for.
async fn resolve(svc: &Service, shared: &Shared) -> Result<Share> {
    let (artist, title) = shared.names();
    let found = match shared {
        Shared::Track { .. } => {
            let listings = svc.itunes.listings(&format!("{artist} {title}"), Entity::Song).await?;
            best(&listings, artist, title, |l| (l.track_id, l.track_name.as_deref()))
                .map(|(id, l)| (format!("https://song.link/i/{id}"), cover(l)))
        }
        Shared::Album { .. } => {
            let listings = svc.itunes.listings(&format!("{artist} {title}"), Entity::Album).await?;
            best(&listings, artist, title, |l| {
                (l.collection_id, l.collection_name.as_deref())
            })
            .map(|(id, l)| (format!("https://album.link/i/{id}"), cover(l)))
        }
        Shared::Artist { .. } => None,
    };
    if let Some((url, cover)) = found {
        let artwork = match cover {
            Some(cover) => Some(cover),
            None => known_picture(svc, shared).await?,
        };
        return Ok(Share {
            url,
            source: Source::Songlink,
            artwork,
        });
    }
    Ok(Share {
        url: lastfm_page(shared),
        source: Source::Lastfm,
        artwork: known_picture(svc, shared).await?,
    })
}

/// The listing (Apple orders by relevance) that is this song or album, by the same artist (or
/// a credit including them): the first with the same title, else the first with the same title
/// loosely (`ids::loose_title`, which also drops "live" and "remaster" tags).
fn best<'a>(
    listings: &'a [Listing],
    artist: &str,
    title: &str,
    id_and_name: impl Fn(&Listing) -> (Option<u64>, Option<&str>),
) -> Option<(u64, &'a Listing)> {
    let artist = loose_artist(artist);
    let by_artist = |l: &Listing| {
        l.artist_name
            .as_deref()
            .map(loose_artist)
            .is_some_and(|by| by == artist || (!artist.is_empty() && by.contains(&artist)))
    };
    let first = |same: &dyn Fn(&str) -> bool| {
        listings.iter().find_map(|l| {
            let (id, name) = id_and_name(l);
            (by_artist(l) && same(name?)).then_some((id?, l))
        })
    };
    first(&|name| normalize(name) == normalize(title))
        .or_else(|| first(&|name| loose_title(name) == loose_title(title)))
}

/// The listing's cover at `ARTWORK_PX`: the store's CDN scales to the size in the name.
fn cover(listing: &Listing) -> Option<String> {
    let cover100 = listing.artwork_url100.as_deref()?;
    cover100
        .contains("100x100bb")
        .then(|| cover100.replace("100x100bb", &format!("{ARTWORK_PX}x{ARTWORK_PX}bb")))
}

/// The cover (or artist photo) the catalog knows a public source for (Deezer, iTunes, the
/// Cover Art Archive), at `ARTWORK_PX`.
async fn known_picture(svc: &Service, shared: &Shared) -> Result<Option<String>> {
    let source = match shared {
        Shared::Track { album: id, .. } | Shared::Album { id, .. } => {
            let id = id.clone();
            svc.ctx.db.call(move |c| catalog::album_art(c, &id)).await?
        }
        Shared::Artist { id, .. } => {
            let id = id.clone();
            svc.ctx.db.call(move |c| crate::artist::photo::image_of(c, &id)).await?
        }
    };
    Ok(source.map(|s| art::sized(&s, ARTWORK_PX)))
}

/// Last.fm's page for it: `/music/<artist>[/_/<track> | /<album>]`, each name one encoded
/// path segment.
fn lastfm_page(shared: &Shared) -> String {
    let (artist, title) = shared.names();
    let segments: Vec<&str> = match shared {
        Shared::Track { .. } => vec![artist, "_", title],
        Shared::Album { .. } => vec![artist, title],
        Shared::Artist { .. } => vec![artist],
    };
    const BASE: &str = "https://www.last.fm/music";
    let Ok(mut url) = reqwest::Url::parse(BASE) else {
        return BASE.to_owned();
    };
    if let Ok(mut path) = url.path_segments_mut() {
        path.extend(segments);
    }
    url.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listing(id: u64, artist: &str, name: &str) -> Listing {
        Listing {
            track_id: Some(id),
            track_name: Some(name.into()),
            collection_id: None,
            collection_name: None,
            artist_name: Some(artist.into()),
            artwork_url100: Some(format!(
                "https://is1-ssl.mzstatic.com/image/thumb/x/{id}.jpg/100x100bb.jpg"
            )),
        }
    }

    #[test]
    fn a_share_link_is_the_store_listing_that_is_this_song() {
        fn song(l: &Listing) -> (Option<u64>, Option<&str>) {
            (l.track_id, l.track_name.as_deref())
        }
        let listings = [
            listing(1, "Portishead", "Glory Box (Live)"),
            listing(2, "Tricky", "Glory Box"),
            listing(3, "Portishead & Friends", "Glory Box"),
        ];
        let (id, l) = best(&listings, "Portishead", "Glory Box", song).unwrap_or((0, &listings[0]));
        assert_eq!(
            id, 3,
            "the same title by a credit that includes the artist beats a live take"
        );
        assert_eq!(
            cover(l).as_deref(),
            Some("https://is1-ssl.mzstatic.com/image/thumb/x/3.jpg/600x600bb.jpg")
        );
        let only_loose = [
            listing(4, "Tricky", "Glory Box"),
            listing(5, "Portishead", "Glory Box (Remastered)"),
        ];
        assert_eq!(
            best(&only_loose, "Portishead", "Glory Box", song).map(|(id, _)| id),
            Some(5),
            "no exact title: the same song loosely, never another artist's"
        );
        assert!(best(&listings, "Massive Attack", "Glory Box", song).is_none());
    }

    #[test]
    fn the_lastfm_fallback_encodes_each_name() {
        let track = Shared::Track {
            item: Item::Track {
                artist: "Sigur Rós".into(),
                title: "Hoppípolla / Með blóðnasir".into(),
                album: String::new(),
                album_artist: None,
                duration: None,
                album_source: None,
            },
            album: AlbumId("al-x".into()),
        };
        assert_eq!(
            lastfm_page(&track),
            "https://www.last.fm/music/Sigur%20R%C3%B3s/_/Hopp%C3%ADpolla%20%2F%20Me%C3%B0%20bl%C3%B3%C3%B0nasir"
        );
        let artist = Shared::Artist {
            item: Item::Artist {
                name: "AC/DC".into(),
                mbid: None,
            },
            id: ArtistId("ar-x".into()),
        };
        assert_eq!(lastfm_page(&artist), "https://www.last.fm/music/AC%2FDC");
    }
}
