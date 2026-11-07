//! ADR-0035 in discovery, while the `namesakes` switch is on: search's rows and the listener's
//! follows go to the artist their anchors say (`lull_core::identity`), never to every artist that
//! shares a name. With the switch off nothing here runs.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use axum::Json;
use axum::extract::{Query as Params, State};
use lull_core::identity::{self, Anchor, Came, Place};
use lull_core::ids::{AlbumId, ArtistId, TrackId};
use lull_core::model::Artist;
use lull_core::{Error, Result, catalog, flags};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::Service;
use crate::art::{self, Pictured};
use crate::artist::identity::resources;
use crate::search::merge::{ArtistHit, Merged, SongHit};
use crate::sources::ArtistSource;
use crate::sources::musicbrainz::Namesake;
use crate::store;

pub fn on(svc: &Service) -> bool {
    svc.ctx.config.flags.on(&flags::NAMESAKES)
}

pub fn source_anchor(s: ArtistSource) -> Anchor {
    match s {
        ArtistSource::Itunes(id) => Anchor::Itunes(id),
        ArtistSource::Deezer(id) => Anchor::Deezer(id),
    }
}

/// An artist row's exact ids: its MBID and the catalogues' ids it came with.
pub fn anchors(hit: &ArtistHit) -> Vec<Anchor> {
    hit.mbid
        .iter()
        .map(|m| Anchor::Mbid(m.clone()))
        .chain(hit.sources.iter().map(|&s| source_anchor(s)))
        .collect()
}

/// A song row's anchors: its catalogue artist's id, the song and its album.
fn song_anchors(hit: &SongHit) -> Vec<Anchor> {
    let album = match &hit.saved {
        Some(t) => Some(t.album_id.clone()),
        None => {
            (!hit.album.is_empty()).then(|| AlbumId::of(hit.album_artist.as_deref().unwrap_or(&hit.artist), &hit.album))
        }
    };
    hit.artist_source
        .map(source_anchor)
        .into_iter()
        .chain([Anchor::Track(hit.id())])
        .chain(album.map(Anchor::Album))
        .collect()
}

/// Places every row search hasn't placed yet, in one transaction.
pub async fn place_rows(svc: &Service, m: &mut Merged) -> Result<()> {
    let jobs = Jobs::of(m);
    if jobs.is_empty() {
        return Ok(());
    }
    let placed = svc.ctx.db.call(move |c| jobs.run(c)).await?;
    placed.apply(m);
    Ok(())
}

/// The artists the listener follows, each as the artist it is now (`followed_in`).
pub async fn followed(svc: &Service, ids: HashSet<ArtistId>) -> Result<HashSet<ArtistId>> {
    svc.ctx.db.call(move |c| followed_in(c, ids)).await
}

/// Rows not placed yet, with what each brings.
struct Jobs {
    artists: Vec<(usize, String, Vec<Anchor>)>,
    /// Whether the song came with its artist's id.
    songs: Vec<(usize, String, Vec<Anchor>, bool)>,
    albums: Vec<(usize, String, AlbumId)>,
}

/// Where each row went.
struct Placed {
    artists: Vec<(usize, ArtistId)>,
    songs: Vec<(usize, ArtistId)>,
    albums: Vec<(usize, ArtistId)>,
}

impl Jobs {
    fn of(m: &Merged) -> Jobs {
        Jobs {
            artists: m
                .artists
                .iter()
                .enumerate()
                .filter(|(_, a)| a.placed.is_none())
                .map(|(i, a)| (i, a.name.clone(), anchors(a)))
                .collect(),
            songs: m
                .songs
                .iter()
                .enumerate()
                .filter(|(_, s)| s.placed.is_none())
                .map(|(i, s)| (i, s.artist.clone(), song_anchors(s), s.artist_source.is_some()))
                .collect(),
            albums: m
                .albums
                .iter()
                .enumerate()
                .filter(|(_, a)| a.placed.is_none())
                .map(|(i, a)| (i, a.artist.clone(), a.id()))
                .collect(),
        }
    }

    fn is_empty(&self) -> bool {
        self.artists.is_empty() && self.songs.is_empty() && self.albums.is_empty()
    }

    /// Artist rows, and songs that come with their artist's id, keep what they bring (an artist
    /// row with the ids of a second artist of its name is a new artist). Songs without one and
    /// albums are only read, so search writes nothing for them.
    fn run(self, c: &mut Connection) -> Result<Placed> {
        let tx = c.transaction()?;
        let mut placed = Placed {
            artists: Vec::new(),
            songs: Vec::new(),
            albums: Vec::new(),
        };
        for (_, name, anchors) in &self.artists {
            identity::place(&tx, name, anchors, Came::Credit)?;
        }
        for (_, name, anchors, _) in self.songs.iter().filter(|s| s.3) {
            identity::place(&tx, name, anchors, Came::Credit)?;
        }
        // Read once everything is kept: a later row can prove a second artist of an earlier one's
        // name, which moves the earlier one to its own id.
        for (i, name, anchors) in self.artists {
            placed
                .artists
                .push((i, identity::artist_of(&tx, &name, &anchors)?.id(&name)));
        }
        for (i, name, anchors, _) in self.songs {
            placed
                .songs
                .push((i, identity::artist_of(&tx, &name, &anchors)?.id(&name)));
        }
        for (i, name, album) in self.albums {
            let place = identity::artist_of(&tx, &name, &[Anchor::Album(album)])?;
            placed.albums.push((i, place.id(&name)));
        }
        tx.commit()?;
        Ok(placed)
    }
}

impl Placed {
    fn apply(self, m: &mut Merged) {
        for (i, id) in self.artists {
            m.artists[i].placed = Some(id);
        }
        for (i, id) in self.songs {
            m.songs[i].placed = Some(id);
        }
        for (i, id) in self.albums {
            m.albums[i].placed = Some(id);
        }
    }
}

/// Each followed id as the artist it is now: an artist's own id is that artist; a follow kept under
/// a name's id from before is the one artist of that name, or the one the listener's songs say
/// (`owner`), or nobody's until they pick. A namesake never gets it.
fn followed_in(c: &Connection, ids: HashSet<ArtistId>) -> Result<HashSet<ArtistId>> {
    let mut out = HashSet::new();
    for id in ids {
        match identity::resolve(c, &id)? {
            Place::One(artist) => {
                out.insert(artist);
            }
            Place::Several(_) => out.extend(identity::owner(c, &id)?),
        }
    }
    Ok(out)
}

/// What an artist id opens with the switch on.
pub enum Opens {
    Artist(ArtistId),
    /// The name's page: several artists share it.
    Named(String),
}

pub async fn opens(svc: &Service, id: ArtistId) -> Result<Opens> {
    svc.ctx
        .db
        .call(move |c| {
            Ok(match identity::resolve(c, &id)? {
                Place::One(artist) => Opens::Artist(artist),
                Place::Several(_) => Opens::Named(identity::name_of(c, &id)?.unwrap_or_default()),
            })
        })
        .await
}

fn switched_on(svc: &Service) -> Result<()> {
    if on(svc) {
        Ok(())
    } else {
        Err(Error::NotFound(
            "Artists sharing a name are told apart only with the namesakes switch on.".into(),
        ))
    }
}

fn checked_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 200 || name.chars().any(char::is_control) {
        return Err(Error::BadRequest("Name an artist.".into()));
    }
    Ok(name.to_owned())
}

#[derive(Deserialize)]
pub struct NameParams {
    name: String,
}

/// `GET /api/artists/named?name=`: every artist called `name` (ADR-0035's name page).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NamedPage {
    name: String,
    /// Oldest first.
    artists: Vec<ArtistId>,
    /// What tells each apart: MusicBrainz's comment, else its type, country and years.
    about: BTreeMap<ArtistId, String>,
    /// One album of each that lull knows.
    albums: BTreeMap<ArtistId, AlbumId>,
    /// Those the listener has songs of, or follows.
    yours: Vec<ArtistId>,
    /// The listener's songs of the name that no anchor places.
    unplaced: Vec<TrackId>,
    /// Whose the name's id is, for what the listener did under it before; absent: which one?
    #[serde(skip_serializing_if = "Option::is_none")]
    owner: Option<ArtistId>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    errors: BTreeMap<&'static str, String>,
    entities: Pictured,
}

pub async fn named(State(svc): State<Arc<Service>>, Params(p): Params<NameParams>) -> Result<Json<NamedPage>> {
    switched_on(&svc)?;
    let name = checked_name(&p.name)?;
    let mut errors = BTreeMap::new();
    let n = name.clone();
    let known = svc.ctx.db.call(move |c| identity::artists_named(c, &n)).await?;
    // MusicBrainz's other artists of the name join only a name several already share: listing
    // them for a name with one artist would make it ambiguous.
    let about_by_mbid: HashMap<String, String> = if known.len() > 1 {
        match add_musicbrainz(&svc, &name).await {
            Ok(found) => found.into_iter().filter_map(|f| Some((f.mbid, f.about?))).collect(),
            Err(e) => {
                errors.insert("musicbrainz", e.to_string());
                HashMap::new()
            }
        }
    } else {
        HashMap::new()
    };
    let n = name.clone();
    let (artists, about, albums, yours, unplaced, owner, tracks, album_rows) = svc
        .ctx
        .db
        .call(move |c| {
            let artists = identity::artists_named(c, &n)?;
            let (mut about, mut albums) = (BTreeMap::new(), BTreeMap::new());
            for artist in &artists {
                for anchor in identity::anchors(c, artist)? {
                    match anchor {
                        Anchor::Mbid(m) => {
                            if let Some(d) = about_by_mbid.get(&m) {
                                about.entry(artist.clone()).or_insert_with(|| d.clone());
                            }
                        }
                        Anchor::Album(al) if !albums.contains_key(artist) && catalog::album(c, &al)?.is_some() => {
                            albums.insert(artist.clone(), al);
                        }
                        _ => {}
                    }
                }
            }
            let (mut yours, mut unplaced) = (Vec::new(), Vec::new());
            for (track, place) in identity::listener_songs(c, &n)? {
                match place {
                    Place::One(a) if !yours.contains(&a) => yours.push(a),
                    Place::One(_) => {}
                    Place::Several(_) => unplaced.push(track),
                }
            }
            let follows: HashSet<ArtistId> = {
                let mut stmt = c.prepare("SELECT id FROM follows WHERE kind = 'artist'")?;
                let ids = stmt.query_map([], |r| r.get::<_, String>(0))?;
                ids.map(|r| r.map(ArtistId)).collect::<rusqlite::Result<_>>()?
            };
            for a in followed_in(c, follows)? {
                if artists.contains(&a) && !yours.contains(&a) {
                    yours.push(a);
                }
            }
            let owner = identity::owner(c, &ArtistId::of(&n))?;
            let mut tracks = Vec::new();
            for t in &unplaced {
                tracks.extend(catalog::track(c, t)?);
            }
            let mut album_rows = Vec::new();
            for al in albums.values() {
                album_rows.extend(catalog::album(c, al)?);
            }
            Ok((artists, about, albums, yours, unplaced, owner, tracks, album_rows))
        })
        .await?;
    let records = svc
        .store
        .artists(
            artists
                .iter()
                .map(|id| Artist {
                    id: id.clone(),
                    name: name.clone(),
                    tags: Vec::new(),
                    listeners: None,
                    bio: None,
                    musicbrainz_id: None,
                })
                .collect(),
        )
        .await?;
    let entities = art::pictured(&svc, store::entities(tracks, album_rows, records)).await?;
    Ok(Json(NamedPage {
        name,
        artists,
        about,
        albums,
        yours,
        unplaced,
        owner,
        errors,
        entities,
    }))
}

/// MusicBrainz's artists of the name, each its own artist unless MusicBrainz links it to a
/// catalogue id lull holds for one of the name's artists (one call for those links).
async fn add_musicbrainz(svc: &Service, name: &str) -> Result<Vec<Namesake>> {
    let found = svc.mb.named_artists(name).await?;
    let (n, country) = (name.to_owned(), svc.itunes.country().to_owned());
    let held: Vec<ArtistSource> = svc
        .ctx
        .db
        .call(move |c| {
            let mut held = Vec::new();
            for artist in identity::artists_named(c, &n)? {
                for anchor in identity::anchors(c, &artist)? {
                    match anchor {
                        Anchor::Deezer(id) => held.push(ArtistSource::Deezer(id)),
                        Anchor::Itunes(id) => held.push(ArtistSource::Itunes(id)),
                        _ => {}
                    }
                }
            }
            Ok(held)
        })
        .await?;
    let urls: Vec<(String, ArtistSource)> = held
        .into_iter()
        .flat_map(|s| resources(s, &country).into_iter().map(move |u| (u, s)))
        .collect();
    let linked = svc
        .mb
        .url_artists(urls.iter().map(|(u, _)| u.clone()).collect())
        .await?;
    let mut links: Vec<(ArtistSource, String)> = urls
        .into_iter()
        .filter_map(|(u, s)| match linked.get(&u)?.as_slice() {
            [one] => Some((s, one.mbid.clone()?)),
            _ => None,
        })
        .collect();
    links.sort_by_key(|(s, m)| (format!("{s:?}"), m.clone()));
    links.dedup();
    let (n, mbids) = (
        name.to_owned(),
        found.iter().map(|f| f.mbid.clone()).collect::<Vec<_>>(),
    );
    svc.ctx
        .db
        .call(move |c| {
            let tx = c.transaction()?;
            for (source, mbid) in links {
                identity::place(&tx, &n, &[source_anchor(source), Anchor::Mbid(mbid)], Came::Link)?;
            }
            for mbid in mbids {
                identity::add_distinct(&tx, &n, &[Anchor::Mbid(mbid)], Came::Credit)?;
            }
            tx.commit()?;
            Ok(())
        })
        .await?;
    Ok(found)
}

/// The listener's word (ADR-0035): works by name that are another artist's, or this one's.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Word {
    /// `split`: the name the works are credited to.
    #[serde(default)]
    name: Option<String>,
    /// `place` and `choose`: the artist.
    #[serde(default)]
    artist: Option<ArtistId>,
    #[serde(default)]
    albums: Vec<AlbumId>,
    #[serde(default)]
    tracks: Vec<TrackId>,
}

#[derive(Serialize)]
pub struct Said {
    artist: ArtistId,
}

fn works(w: &Word) -> Vec<Anchor> {
    w.albums
        .iter()
        .cloned()
        .map(Anchor::Album)
        .chain(w.tracks.iter().cloned().map(Anchor::Track))
        .collect()
}

/// `POST /api/artists/named/split`: "This is another …": the works become a new artist of the name.
pub async fn split(State(svc): State<Arc<Service>>, Json(w): Json<Word>) -> Result<Json<Said>> {
    switched_on(&svc)?;
    let name = checked_name(w.name.as_deref().unwrap_or_default())?;
    let anchors = works(&w);
    if anchors.is_empty() {
        return Err(Error::BadRequest(
            "Say which albums or songs are by another artist.".into(),
        ));
    }
    let artist = svc
        .ctx
        .db
        .call(move |c| {
            let tx = c.transaction()?;
            let id = identity::listener_splits(&tx, &name, &anchors)?;
            tx.commit()?;
            Ok(id)
        })
        .await?;
    Ok(Json(Said { artist }))
}

/// `POST /api/artists/place`: "This is by …": the works are that artist's.
pub async fn place(State(svc): State<Arc<Service>>, Json(w): Json<Word>) -> Result<Json<Said>> {
    switched_on(&svc)?;
    let artist = w
        .artist
        .clone()
        .ok_or_else(|| Error::BadRequest("Say whose they are.".into()))?;
    let anchors = works(&w);
    let said = artist.clone();
    svc.ctx
        .db
        .call(move |c| {
            let tx = c.transaction()?;
            for a in &anchors {
                identity::listener_places(&tx, a, &said)?;
            }
            tx.commit()?;
            Ok(())
        })
        .await?;
    Ok(Json(Said { artist }))
}

/// `POST /api/artists/named/choose`: which artist a follow or hide of the name from before meant.
pub async fn choose(State(svc): State<Arc<Service>>, Json(w): Json<Word>) -> Result<Json<Said>> {
    switched_on(&svc)?;
    let name = checked_name(w.name.as_deref().unwrap_or_default())?;
    let artist = w
        .artist
        .clone()
        .ok_or_else(|| Error::BadRequest("Say which one.".into()))?;
    let said = artist.clone();
    svc.ctx
        .db
        .call(move |c| identity::listener_chooses(c, &ArtistId::of(&name), &said))
        .await?;
    Ok(Json(Said { artist }))
}

#[cfg(test)]
mod tests;
