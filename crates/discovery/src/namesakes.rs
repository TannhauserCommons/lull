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
use crate::open::ArtistIds;
use crate::search::merge::{ArtistHit, Merged, SongHit};
use crate::sources::ArtistSource;
use crate::sources::musicbrainz::Namesake;
use crate::store;
use crate::taste::{Placed, Signals};

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
struct Rows {
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
    fn run(self, c: &mut Connection) -> Result<Rows> {
        let tx = c.transaction()?;
        let mut placed = Rows {
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

impl Rows {
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
        out.extend(identity::now_is(c, &id)?);
    }
    Ok(out)
}

/// Who each taste signal's artist is (ADR-0035): a song by its anchors (the song, its album), a
/// name alone by the artists called it (one, else nobody), a follow by the artist it is now; and
/// the hidden artists as the artists they are now. A namesake never gets another's.
pub fn placed(c: &Connection, s: &Signals) -> Result<(Placed, HashSet<ArtistId>)> {
    let mut placed = Placed::default();
    let songs = s
        .plays
        .iter()
        .map(|p| (&p.track, &p.artist))
        .chain(s.likes.iter().chain(&s.saves).map(|m| (&m.track, &m.artist)))
        .chain(s.feedback.iter().filter_map(|f| Some((f.track.as_ref()?, &f.artist))));
    for (track, artist) in songs {
        if placed.tracks.contains_key(track) {
            continue;
        }
        let mut anchors = vec![Anchor::Track(track.clone())];
        anchors.extend(catalog::track(c, track)?.map(|t| Anchor::Album(t.album_id)));
        let who = match identity::artist_of(c, artist, &anchors)? {
            Place::One(a) => Some(a),
            Place::Several(_) => None,
        };
        placed.tracks.insert(track.clone(), who);
    }
    let names = s
        .history
        .iter()
        .chain(&s.library)
        .map(|(a, _)| a)
        .chain(s.feedback.iter().filter(|f| f.track.is_none()).map(|f| &f.artist));
    for name in names {
        if !placed.names.contains_key(name) {
            let mut all = identity::artists_named(c, name)?;
            let one = (all.len() == 1).then(|| all.remove(0));
            placed.names.insert(name.clone(), one);
        }
    }
    let mut stmt = c.prepare("SELECT id FROM follows WHERE kind = 'artist' ORDER BY rowid")?;
    let follows: Vec<String> = stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    for id in follows {
        placed.follows.push(identity::now_is(c, &ArtistId(id))?);
    }
    let mut hidden = HashSet::new();
    for id in &s.hidden {
        hidden.extend(identity::now_is(c, id)?);
    }
    Ok((placed, hidden))
}

/// Where an artist link goes (ADR-0035): the artist holding its ids or its album, else every
/// artist of its name (`Place::id`: the name's id opens their page). What it brings is kept.
pub async fn place_link(svc: &Service, name: &str, mbid: Option<&str>, ids: &ArtistIds) -> Result<ArtistId> {
    let anchors = link_anchors(name, mbid, ids);
    let name = name.to_owned();
    svc.ctx
        .db
        .call(move |c| Ok(identity::place(c, &name, &anchors, Came::Link)?.id(&name)))
        .await
}

/// A link's anchors: its ids, and the album it names as the album of that name lull would know.
fn link_anchors(name: &str, mbid: Option<&str>, ids: &ArtistIds) -> Vec<Anchor> {
    let mut anchors: Vec<Anchor> = mbid.map(|m| Anchor::Mbid(m.to_owned())).into_iter().collect();
    anchors.extend(ids.deezer.map(Anchor::Deezer));
    anchors.extend(ids.itunes.map(Anchor::Itunes));
    anchors.extend(ids.album.as_deref().map(|al| Anchor::Album(AlbumId::of(name, al))));
    anchors
}

/// What a link to `id` carries (ADR-0035): every exact id the hub holds for the artist (the rule's
/// anchors, else what `artist::identity` keeps), one album of theirs, and, for a name several
/// artists share, MusicBrainz's words for which one it is.
pub async fn link_ids(svc: &Service, id: &ArtistId, name: &str) -> Result<(Option<String>, ArtistIds)> {
    let (at, n) = (id.clone(), name.to_owned());
    let (mut mbid, mut ids, shared) = svc
        .ctx
        .db
        .call(move |c| {
            let artist = identity::now_is(c, &at)?.unwrap_or_else(|| at.clone());
            let (mut mbid, mut ids) = (None, ArtistIds::default());
            for anchor in identity::anchors(c, &artist)? {
                match anchor {
                    Anchor::Mbid(m) => mbid = mbid.or(Some(m)),
                    Anchor::Deezer(d) => ids.deezer = ids.deezer.or(Some(d)),
                    Anchor::Itunes(i) => ids.itunes = ids.itunes.or(Some(i)),
                    Anchor::Album(al) if ids.album.is_none() => ids.album = catalog::album(c, &al)?.map(|a| a.title),
                    _ => {}
                }
            }
            Ok((mbid, ids, identity::artists_named(c, &n)?.len() > 1))
        })
        .await?;
    let held = svc.store.artist_ids(id.clone()).await?;
    mbid = mbid.or_else(|| held.mbid().map(str::to_owned));
    ids.deezer = ids.deezer.or(held.deezer());
    ids.itunes = ids.itunes.or(held.itunes());
    if shared && let Some(m) = &mbid {
        // Words for the card only: without them the link still opens the right artist by its ids.
        match svc.mb.named_artists(name).await {
            Ok(found) => ids.about = found.into_iter().find(|f| f.mbid == *m).and_then(|f| f.about),
            Err(e) => tracing::info!(artist = name, error = %e, "a link without its description"),
        }
    }
    Ok((mbid, ids))
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
    /// The listener follows the name from before and nothing says which artist they meant: ask.
    ask_follow: bool,
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
    let (artists, about, albums, yours, unplaced, owner, ask_follow, tracks, album_rows) = svc
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
            let ask_follow = follows.contains(&ArtistId::of(&n)) && identity::owner(c, &ArtistId::of(&n))?.is_none();
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
            Ok((
                artists, about, albums, yours, unplaced, owner, ask_follow, tracks, album_rows,
            ))
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
        ask_follow,
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
