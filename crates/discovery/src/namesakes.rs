//! ADR-0035 in discovery, while the `namesakes` switch is on: search's rows and the listener's
//! follows go to the artist their anchors say (`lull_core::identity`), never to every artist that
//! shares a name. With the switch off nothing here runs.

use std::collections::HashSet;

use lull_core::Result;
use lull_core::flags;
use lull_core::identity::{self, Anchor, Came, Place};
use lull_core::ids::{AlbumId, ArtistId};
use rusqlite::Connection;

use crate::Service;
use crate::search::merge::{ArtistHit, Merged, SongHit};
use crate::sources::ArtistSource;

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

#[cfg(test)]
mod tests;
