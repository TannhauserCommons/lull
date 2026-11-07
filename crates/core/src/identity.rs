//! Who an artist is (ADR-0035). **An artist is a set of anchors. Two things are the same artist
//! when they share an anchor, or when they have the same name and no anchor tells them apart.** A
//! name is never an anchor.
//!
//! - **Anchors** are exact ids (an MBID, a Deezer or Apple artist id, the listener's own `you:<n>`)
//!   and works (an album or song record). Two different exact ids of one source tell artists apart;
//!   works only join.
//! - **Anything that names an artist goes to every artist it could be** (`place`): the artist that
//!   holds its anchors, else every artist of its name that none of its anchors tells apart. One is
//!   that artist; several is the name's page.
//! - **An artist keeps every name it was called by** ("Nirvana (UK)" on Deezer, "Nirvana" on
//!   MusicBrainz), so it is among the artists of each.
//! - **A name's id** (`ArtistId::of`) is the artist while the name has one. Once a name has several,
//!   the one that had its id gets its own (`ArtistId::own`) and the name's id means the name. What
//!   the listener kept under a name's id before is their artist's (`owner`), never the name's.
//!
//! The tables exist only while the `namesakes` switch is on (ADR-0096), so with it off lull is what
//! it was. Rows are this module's own: no other table is rewritten.

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::{Connection, OptionalExtension, params};

use crate::db::Migrations;
use crate::error::{Error, Result};
use crate::ids::{AlbumId, ArtistId, TrackId, normalize_artist};

pub const MIGRATIONS: Migrations = Migrations {
    owner: "identity",
    steps: &[r#"
        -- Every artist this module knows: by a name's id while it is the one artist of that name, else its own.
        CREATE TABLE identity_artists (
            id          TEXT PRIMARY KEY,
            kind        TEXT NOT NULL CHECK (kind IN ('name', 'own')),
            name        TEXT NOT NULL       -- the name it was first known by
        );
        -- Every name an artist has been called by ("Nirvana (UK)" on Deezer, "Nirvana" on MusicBrainz).
        CREATE TABLE identity_called (
            name_id     TEXT NOT NULL,      -- ArtistId::of(name)
            artist_id   TEXT NOT NULL,
            name        TEXT NOT NULL,
            PRIMARY KEY (name_id, artist_id)
        );
        CREATE INDEX identity_called_artist ON identity_called(artist_id);
        -- Each anchor belongs to one artist.
        CREATE TABLE identity_anchors (
            anchor      TEXT PRIMARY KEY,   -- Anchor::key: "mb:…", "dz:…", "it:…", "you:…", "album:…", "track:…"
            artist_id   TEXT NOT NULL,
            came        TEXT NOT NULL,      -- Came::as_str
            at          INTEGER NOT NULL
        );
        CREATE INDEX identity_anchors_artist ON identity_anchors(artist_id);
        -- An artist's own id joined into another (a MusicBrainz merge, two catalogue profiles proven one).
        CREATE TABLE identity_aliases (
            from_id     TEXT PRIMARY KEY,
            to_id       TEXT NOT NULL
        );
        -- Whose a name's id is, for what the listener kept under it before the name had several artists.
        CREATE TABLE identity_names (
            name_id     TEXT PRIMARY KEY,
            artist_id   TEXT NOT NULL,
            by          TEXT NOT NULL CHECK (by IN ('songs', 'listener')),
            at          INTEGER NOT NULL
        );
    "#],
};

/// What holds an artist together.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Anchor {
    /// The listener's own word: "this is another Nirvana".
    Listener(u64),
    Mbid(String),
    /// Deezer's artist id.
    Deezer(u64),
    /// Apple's artistId.
    Itunes(u64),
    /// An album credited to the artist.
    Album(AlbumId),
    /// A song credited to the artist.
    Track(TrackId),
}

impl Anchor {
    pub fn key(&self) -> String {
        match self {
            Anchor::Listener(n) => format!("you:{n}"),
            Anchor::Mbid(m) => format!("mb:{m}"),
            Anchor::Deezer(id) => format!("dz:{id}"),
            Anchor::Itunes(id) => format!("it:{id}"),
            Anchor::Album(a) => format!("album:{a}"),
            Anchor::Track(t) => format!("track:{t}"),
        }
    }

    pub fn parse(key: &str) -> Result<Anchor> {
        let bad = || Error::internal(format!("anchor {key:?}"));
        let (kind, value) = key.split_once(':').ok_or_else(bad)?;
        let number = || value.parse::<u64>().map_err(|_| bad());
        Ok(match kind {
            "you" => Anchor::Listener(number()?),
            "mb" => Anchor::Mbid(value.to_owned()),
            "dz" => Anchor::Deezer(number()?),
            "it" => Anchor::Itunes(number()?),
            "album" => Anchor::Album(AlbumId(value.to_owned())),
            "track" => Anchor::Track(TrackId(value.to_owned())),
            _ => return Err(bad()),
        })
    }

    /// The source of an exact id: two different ids of one source are two artists. Works are no source.
    fn source(&self) -> Option<&'static str> {
        match self {
            Anchor::Listener(_) => Some("you"),
            Anchor::Mbid(_) => Some("mb"),
            Anchor::Deezer(_) => Some("dz"),
            Anchor::Itunes(_) => Some("it"),
            Anchor::Album(_) | Anchor::Track(_) => None,
        }
    }

    /// Which anchor wins when a reference's anchors name artists that can't be one: the listener's
    /// word, then MusicBrainz, then the catalogues, then works.
    fn weight(&self) -> u8 {
        match self {
            Anchor::Listener(_) => 0,
            Anchor::Mbid(_) => 1,
            Anchor::Deezer(_) | Anchor::Itunes(_) => 2,
            Anchor::Album(_) | Anchor::Track(_) => 3,
        }
    }
}

/// Where an anchor came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Came {
    /// A catalogue's or a file's credit (a search row, a song's catalogue artist, a recommender's id).
    Credit,
    /// MusicBrainz's links between ids.
    Link,
    /// A look-up of a work lull had by name only.
    Lookup,
    /// The library's tags or Jellyfin's ids.
    Library,
    /// The listener's word. Nothing else replaces it.
    Listener,
}

impl Came {
    pub fn as_str(self) -> &'static str {
        match self {
            Came::Credit => "credit",
            Came::Link => "link",
            Came::Lookup => "lookup",
            Came::Library => "library",
            Came::Listener => "listener",
        }
    }
}

/// Where a reference goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    /// The one artist it can be.
    One(ArtistId),
    /// Every artist it could be, oldest first: the name's page.
    Several(Vec<ArtistId>),
}

impl Place {
    /// The artist id a song or row carries: the artist's, or the name's id when it could be several
    /// (which opens the name's page).
    pub fn id(&self, name: &str) -> ArtistId {
        match self {
            Place::One(id) => id.clone(),
            Place::Several(_) => ArtistId::of(name),
        }
    }
}

/// Where a reference named `name` with `anchors` goes, without writing anything: for readers.
pub fn artist_of(conn: &Connection, name: &str, anchors: &[Anchor]) -> Result<Place> {
    Ok(decide(conn, name, anchors)?.place)
}

/// Where a reference goes. What it brings is kept: its anchors on the one artist it lands on (a
/// new one when its exact ids tell it apart from every artist of its name), its name among that
/// artist's names, and artists it proves to be that one are joined.
pub fn place(conn: &Connection, name: &str, anchors: &[Anchor], came: Came) -> Result<Place> {
    let decided = decide(conn, name, anchors)?;
    if anchors.is_empty() {
        // A bare name brings nothing to keep.
        return Ok(decided.place);
    }
    let landed = match &decided.place {
        Place::Several(_) if !decided.new => return Ok(decided.place),
        Place::Several(_) => new_artist(conn, name, anchors)?,
        Place::One(id) => id.clone(),
    };
    for other in &decided.join {
        join(conn, &landed, other)?;
    }
    if decided.record {
        keep(conn, &landed, anchors, came)?;
    }
    let landed = alias(conn, &landed)?;
    called(conn, name, &landed, true)?;
    Ok(Place::One(alias(conn, &landed)?))
}

/// One of a list of artists sharing `name` (MusicBrainz's): the artist holding its anchors, else a
/// new artist of its own. A list says they are different artists, not which one lull already has,
/// so it never joins one by name.
pub fn add_distinct(conn: &Connection, name: &str, anchors: &[Anchor], came: Came) -> Result<ArtistId> {
    if let Some((artist, _)) = holders(conn, anchors)?.into_iter().min_by_key(|(_, w)| *w) {
        return called(conn, name, &artist, false);
    }
    let first = anchors
        .iter()
        .min_by_key(|a| a.weight())
        .ok_or_else(|| Error::internal(format!("a {name:?} needs an anchor")))?;
    let id = ArtistId::own(name, &first.key());
    ensure_artist(conn, &id, name)?;
    keep(conn, &id, anchors, came)?;
    called(conn, name, &id, false)
}

/// What an id lull handed out opens: an artist's own id is that artist; a name's id is the one
/// artist of that name, or the name's page.
pub fn resolve(conn: &Connection, id: &ArtistId) -> Result<Place> {
    let own = alias(conn, id)?;
    let kind: Option<String> = conn
        .query_row("SELECT kind FROM identity_artists WHERE id = ?1", [own.as_str()], |r| {
            r.get(0)
        })
        .optional()?;
    if own != *id || kind.as_deref() == Some("own") {
        return Ok(Place::One(own));
    }
    let mut named = called_by(conn, id)?;
    Ok(match named.len() {
        0 => Place::One(id.clone()),
        1 => Place::One(named.remove(0)),
        _ => Place::Several(named),
    })
}

/// The artists of a name, oldest first; the name's own id when this module knows none.
pub fn artists_named(conn: &Connection, name: &str) -> Result<Vec<ArtistId>> {
    let name_id = ArtistId::of(name);
    let named = called_by(conn, &name_id)?;
    Ok(if named.is_empty() { vec![name_id] } else { named })
}

/// An artist's anchors, oldest first.
pub fn anchors(conn: &Connection, artist: &ArtistId) -> Result<Vec<Anchor>> {
    let mut stmt = conn.prepare_cached("SELECT anchor FROM identity_anchors WHERE artist_id = ?1 ORDER BY rowid")?;
    let keys: Vec<String> = stmt
        .query_map([alias(conn, artist)?.as_str()], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    keys.iter().map(|k| Anchor::parse(k)).collect()
}

/// The artist an id now is: itself, or the artist it was joined into.
pub fn alias(conn: &Connection, id: &ArtistId) -> Result<ArtistId> {
    let to: Option<String> = conn
        .prepare_cached("SELECT to_id FROM identity_aliases WHERE from_id = ?1")?
        .query_row([id.as_str()], |r| r.get(0))
        .optional()?;
    Ok(to.map_or_else(|| id.clone(), ArtistId))
}

/// MusicBrainz answered a look-up of `old` under `new`: one artist, whatever else either holds.
pub fn merged_mbid(conn: &Connection, old: &str, new: &str) -> Result<()> {
    let (old, new) = (Anchor::Mbid(old.to_owned()), Anchor::Mbid(new.to_owned()));
    match (holder(conn, &old)?, holder(conn, &new)?) {
        (Some(a), Some(b)) if a != b => {
            let (keep, gone) = if age(conn, &a)? <= age(conn, &b)? {
                (a, b)
            } else {
                (b, a)
            };
            join(conn, &keep, &gone)
        }
        (Some(a), None) => put(conn, &new.key(), &a, Came::Link),
        _ => Ok(()),
    }
}

/// The listener says `work` is by `artist`: their word replaces whatever placed it.
pub fn listener_places(conn: &Connection, work: &Anchor, artist: &ArtistId) -> Result<()> {
    put(conn, &work.key(), &alias(conn, artist)?, Came::Listener)
}

/// The listener says `works` are by another artist called `name`: a new artist, anchored by their
/// word. An artist of that name with no exact id gets the listener's word too, so the two are told
/// apart from now on.
pub fn listener_splits(conn: &Connection, name: &str, works: &[Anchor]) -> Result<ArtistId> {
    for artist in artists_named(conn, name)? {
        if exact(conn, &artist)?.is_empty() {
            ensure_artist(conn, &artist, name)?;
            put(
                conn,
                &Anchor::Listener(next_listener(conn)?).key(),
                &artist,
                Came::Listener,
            )?;
            called(conn, name, &artist, false)?;
        }
    }
    let mine = Anchor::Listener(next_listener(conn)?);
    let id = new_artist(conn, name, std::slice::from_ref(&mine))?;
    put(conn, &mine.key(), &id, Came::Listener)?;
    for w in works {
        put(conn, &w.key(), &id, Came::Listener)?;
    }
    let id = called(conn, name, &id, false)?;
    settle(conn, name)?;
    Ok(id)
}

/// Whose a name's id is, for what the listener kept under it before: the listener's choice, else
/// the one artist their own songs of that name are on. None while nothing decides ("which one?").
/// A name with one artist needs none: its id opens that artist (`resolve`).
pub fn owner(conn: &Connection, name_id: &ArtistId) -> Result<Option<ArtistId>> {
    let owner: Option<String> = conn
        .query_row(
            "SELECT artist_id FROM identity_names WHERE name_id = ?1",
            [name_id.as_str()],
            |r| r.get(0),
        )
        .optional()?;
    owner.map(|o| alias(conn, &ArtistId(o))).transpose()
}

/// The artist an id kept from before is now: an artist's own id that artist; a name's id the one
/// artist of the name, else the listener's (`owner`); None while nothing says which ("which one?").
pub fn now_is(conn: &Connection, id: &ArtistId) -> Result<Option<ArtistId>> {
    Ok(match resolve(conn, id)? {
        Place::One(a) => Some(a),
        Place::Several(_) => owner(conn, id)?,
    })
}

/// Whether `artist` is called `name`: how a follow of an artist's own id checks the name it comes with.
pub fn is_called(conn: &Connection, artist: &ArtistId, name: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM identity_called WHERE artist_id = ?1 AND name_id = ?2",
            params![alias(conn, artist)?.as_str(), ArtistId::of(name).as_str()],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// The listener picks which artist a name's id meant (a follow or hide from before).
pub fn listener_chooses(conn: &Connection, name_id: &ArtistId, artist: &ArtistId) -> Result<()> {
    conn.execute(
        "INSERT INTO identity_names (name_id, artist_id, by, at) VALUES (?1, ?2, 'listener', ?3)
         ON CONFLICT(name_id) DO UPDATE SET artist_id = excluded.artist_id, by = 'listener', at = excluded.at",
        params![name_id.as_str(), alias(conn, artist)?.as_str(), crate::now()],
    )?;
    Ok(())
}

// ----- deciding -----

struct Decided {
    place: Place,
    /// Artists the reference proves are the one it lands on.
    join: Vec<ArtistId>,
    /// Whether its anchors may be kept on the artist it lands on (a new one included).
    record: bool,
    /// Its exact ids tell it apart from every artist of its name: it is a new one.
    new: bool,
}

fn decide(conn: &Connection, name: &str, anchors: &[Anchor]) -> Result<Decided> {
    let held = holders(conn, anchors)?;
    if !held.is_empty() {
        return decide_held(conn, anchors, held);
    }
    let all = artists_named(conn, name)?;
    let mut fits = Vec::new();
    for artist in &all {
        if !tells_apart(&exact(conn, artist)?, anchors) {
            fits.push(artist.clone());
        }
    }
    let (place, record, new) = match fits.len() {
        1 => (Place::One(fits.remove(0)), true, false),
        0 => (Place::Several(all), true, true),
        _ => (Place::Several(fits), false, false),
    };
    Ok(Decided {
        place,
        join: Vec::new(),
        record,
        new,
    })
}

/// Its anchors are held: by one artist, that one; by several, one artist when nothing tells them
/// apart, else the holder of its weightiest anchor, and nothing is kept.
fn decide_held(conn: &Connection, anchors: &[Anchor], held: BTreeMap<ArtistId, u8>) -> Result<Decided> {
    let mut by_age = Vec::new();
    for artist in held.keys() {
        by_age.push((age(conn, artist)?, artist.clone()));
    }
    by_age.sort();
    let mut artists = by_age.into_iter().map(|(_, a)| a);
    let first = artists.next().ok_or_else(|| Error::internal("no holder"))?;
    let others: Vec<ArtistId> = artists.collect();
    let mut all = exact(conn, &first)?;
    let mut one = true;
    for other in &others {
        let theirs = exact(conn, other)?;
        one &= !tells_apart(&all, &theirs);
        all.extend(theirs);
    }
    if one && !tells_apart(&all, anchors) {
        return Ok(Decided {
            place: Place::One(first),
            join: others,
            record: true,
            new: false,
        });
    }
    let weightiest = held.iter().min_by_key(|(_, w)| **w).map_or(first, |(a, _)| a.clone());
    Ok(Decided {
        place: Place::One(weightiest),
        join: Vec::new(),
        record: false,
        new: false,
    })
}

/// The artists holding any of `anchors`, each with the weight of its weightiest one.
fn holders(conn: &Connection, anchors: &[Anchor]) -> Result<BTreeMap<ArtistId, u8>> {
    let mut held = BTreeMap::new();
    for a in anchors {
        if let Some(artist) = holder(conn, a)? {
            let w = held.entry(artist).or_insert(u8::MAX);
            *w = (*w).min(a.weight());
        }
    }
    Ok(held)
}

fn holder(conn: &Connection, anchor: &Anchor) -> Result<Option<ArtistId>> {
    let artist: Option<String> = conn
        .prepare_cached("SELECT artist_id FROM identity_anchors WHERE anchor = ?1")?
        .query_row([anchor.key()], |r| r.get(0))
        .optional()?;
    artist.map(|a| alias(conn, &ArtistId(a))).transpose()
}

fn exact(conn: &Connection, artist: &ArtistId) -> Result<Vec<Anchor>> {
    Ok(anchors(conn, artist)?
        .into_iter()
        .filter(|a| a.source().is_some())
        .collect())
}

/// Two sets of exact anchors are two artists when they share none and hold different ids of one
/// source, or when either holds the listener's word (theirs joins only by the listener's word).
pub fn tells_apart(ours: &[Anchor], theirs: &[Anchor]) -> bool {
    let exact = |a: &&Anchor| a.source().is_some();
    if ours.iter().filter(exact).any(|a| theirs.contains(a)) {
        return false;
    }
    let listeners = |v: &[Anchor]| v.iter().any(|a| matches!(a, Anchor::Listener(_)));
    let both_exact = ours.iter().any(|a| exact(&a)) && theirs.iter().any(|a| exact(&a));
    (both_exact && (listeners(ours) || listeners(theirs)))
        || ours
            .iter()
            .filter(exact)
            .any(|a| theirs.iter().any(|b| b.source() == a.source() && b != a))
}

// ----- writing -----

/// Keeps `anchors` on `artist`, except an id that would tell it apart from itself, and a work the
/// listener placed elsewhere.
fn keep(conn: &Connection, artist: &ArtistId, anchors: &[Anchor], came: Came) -> Result<()> {
    let have = exact(conn, artist)?;
    for a in anchors {
        if a.source().is_some() && tells_apart(&have, std::slice::from_ref(a)) {
            continue;
        }
        conn.execute(
            "INSERT INTO identity_anchors (anchor, artist_id, came, at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(anchor) DO NOTHING",
            params![a.key(), artist.as_str(), came.as_str(), crate::now()],
        )?;
    }
    Ok(())
}

/// Sets an anchor's artist, replacing what held it.
fn put(conn: &Connection, key: &str, artist: &ArtistId, came: Came) -> Result<()> {
    conn.execute(
        "INSERT INTO identity_anchors (anchor, artist_id, came, at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(anchor) DO UPDATE SET artist_id = excluded.artist_id, came = excluded.came",
        params![key, artist.as_str(), came.as_str(), crate::now()],
    )?;
    Ok(())
}

/// A new artist called `name`: the name's id while nothing else is called that, else its own id,
/// made from its weightiest exact anchor.
fn new_artist(conn: &Connection, name: &str, anchors: &[Anchor]) -> Result<ArtistId> {
    let id = if called_by(conn, &ArtistId::of(name))?.is_empty() {
        ArtistId::of(name)
    } else {
        let first = anchors
            .iter()
            .min_by_key(|a| a.weight())
            .ok_or_else(|| Error::internal(format!("a new {name:?} needs an anchor")))?;
        ArtistId::own(name, &first.key())
    };
    ensure_artist(conn, &id, name)?;
    Ok(id)
}

fn ensure_artist(conn: &Connection, id: &ArtistId, name: &str) -> Result<()> {
    let kind = if *id == ArtistId::of(name) { "name" } else { "own" };
    conn.execute(
        "INSERT INTO identity_artists (id, kind, name) VALUES (?1, ?2, ?3) ON CONFLICT(id) DO NOTHING",
        params![id.as_str(), kind, name],
    )?;
    Ok(())
}

/// `artist` is called `name`. Another artist of that name that nothing tells apart from it is it
/// (`join_by_name`; a candidate from a list of namesakes is no evidence of that).
/// Once a name has several artists, the one that had the name's id gets its own, so the name's id
/// means the name, and whose it was before is settled. Returns the artist's id now.
fn called(conn: &Connection, name: &str, artist: &ArtistId, join_by_name: bool) -> Result<ArtistId> {
    let name_id = ArtistId::of(name);
    ensure_artist(conn, artist, name)?;
    let mine = exact(conn, artist)?;
    let mut fits = Vec::new();
    for other in called_by(conn, &name_id)?.into_iter().filter(|_| join_by_name) {
        if other != *artist && !tells_apart(&mine, &exact(conn, &other)?) {
            fits.push(other);
        }
    }
    let mut artist = artist.clone();
    if let [other] = fits.as_slice() {
        let (keep, gone) = if age(conn, other)? <= age(conn, &artist)? {
            (other.clone(), artist.clone())
        } else {
            (artist.clone(), other.clone())
        };
        join(conn, &keep, &gone)?;
        artist = keep;
    }
    conn.execute(
        "INSERT INTO identity_called (name_id, artist_id, name) VALUES (?1, ?2, ?3) ON CONFLICT DO NOTHING",
        params![name_id.as_str(), artist.as_str(), name],
    )?;
    let named = called_by(conn, &name_id)?;
    if named.len() > 1 && named.contains(&name_id) {
        let first = anchors(conn, &name_id)?
            .into_iter()
            .min_by_key(Anchor::weight)
            .ok_or_else(|| Error::internal(format!("{name:?} has no anchor")))?;
        let own = ArtistId::own(name, &first.key());
        rekey(conn, &name_id, &own, name)?;
        if artist == name_id {
            artist = own;
        }
    }
    if called_by(conn, &name_id)?.len() > 1 {
        settle(conn, name)?;
    }
    Ok(artist)
}

/// The artists called `name_id`'s name, oldest first.
fn called_by(conn: &Connection, name_id: &ArtistId) -> Result<Vec<ArtistId>> {
    let mut stmt = conn.prepare_cached("SELECT artist_id FROM identity_called WHERE name_id = ?1")?;
    let ids: Vec<String> = stmt
        .query_map([name_id.as_str()], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let mut aged = Vec::new();
    for id in ids {
        let id = alias(conn, &ArtistId(id))?;
        if !aged.iter().any(|(_, a)| *a == id) {
            aged.push((age(conn, &id)?, id));
        }
    }
    aged.sort();
    Ok(aged.into_iter().map(|(_, a)| a).collect())
}

/// An artist moves from a name's id to its own: anchors, names and choices follow. No alias is
/// left, so the name's id means the name from now on.
fn rekey(conn: &Connection, from: &ArtistId, to: &ArtistId, name: &str) -> Result<()> {
    conn.execute("DELETE FROM identity_artists WHERE id = ?1", [from.as_str()])?;
    ensure_artist(conn, to, name)?;
    move_rows(conn, from, to)
}

/// `gone` is `keep`: its anchors, names and choices move, and its own id answers as `keep`.
fn join(conn: &Connection, keep: &ArtistId, gone: &ArtistId) -> Result<()> {
    if keep == gone {
        return Ok(());
    }
    let kind: Option<String> = conn
        .query_row(
            "SELECT kind FROM identity_artists WHERE id = ?1",
            [gone.as_str()],
            |r| r.get(0),
        )
        .optional()?;
    conn.execute("DELETE FROM identity_artists WHERE id = ?1", [gone.as_str()])?;
    move_rows(conn, gone, keep)?;
    if kind.as_deref() == Some("own") {
        conn.execute(
            "INSERT INTO identity_aliases (from_id, to_id) VALUES (?1, ?2)
             ON CONFLICT(from_id) DO UPDATE SET to_id = excluded.to_id",
            params![gone.as_str(), keep.as_str()],
        )?;
    }
    Ok(())
}

fn move_rows(conn: &Connection, from: &ArtistId, to: &ArtistId) -> Result<()> {
    let (f, t) = (from.as_str(), to.as_str());
    conn.execute(
        "UPDATE identity_anchors SET artist_id = ?1 WHERE artist_id = ?2",
        params![t, f],
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO identity_called (name_id, artist_id, name)
         SELECT name_id, ?1, name FROM identity_called WHERE artist_id = ?2",
        params![t, f],
    )?;
    conn.execute("DELETE FROM identity_called WHERE artist_id = ?1", [f])?;
    conn.execute("UPDATE identity_aliases SET to_id = ?1 WHERE to_id = ?2", params![t, f])?;
    conn.execute(
        "UPDATE identity_names SET artist_id = ?1 WHERE artist_id = ?2",
        params![t, f],
    )?;
    Ok(())
}

/// When a name has several artists, whose its id was: the one artist the listener's own songs of
/// that name are on, when exactly one. The listener's choice stays.
fn settle(conn: &Connection, name: &str) -> Result<()> {
    let name_id = ArtistId::of(name);
    let owners: BTreeSet<ArtistId> = listener_songs(conn, name)?
        .into_iter()
        .filter_map(|(_, place)| match place {
            Place::One(a) => Some(a),
            Place::Several(_) => None,
        })
        .collect();
    if owners.len() == 1 {
        let owner = owners.into_iter().next().unwrap_or_else(|| name_id.clone());
        conn.execute(
            "INSERT INTO identity_names (name_id, artist_id, by, at) VALUES (?1, ?2, 'songs', ?3)
             ON CONFLICT(name_id) DO UPDATE SET artist_id = excluded.artist_id, at = excluded.at
             WHERE identity_names.by = 'songs'",
            params![name_id.as_str(), owner.as_str(), crate::now()],
        )?;
    } else {
        conn.execute(
            "DELETE FROM identity_names WHERE name_id = ?1 AND by = 'songs'",
            [name_id.as_str()],
        )?;
    }
    Ok(())
}

/// The listener's own songs credited to `name` (played, liked or saved), each with where the rule
/// places it by its anchors (the song, its album): one artist, or several when nothing decides.
pub fn listener_songs(conn: &Connection, name: &str) -> Result<Vec<(TrackId, Place)>> {
    let key = normalize_artist(name);
    let mut songs: BTreeMap<String, String> = BTreeMap::new();
    for sql in [
        "SELECT p.track_id, p.artist, json_extract(c.json, '$.albumId')
           FROM (SELECT DISTINCT track_id, artist FROM plays) p LEFT JOIN catalog_tracks c ON c.id = p.track_id",
        "SELECT l.track_id, l.artist, json_extract(c.json, '$.albumId')
           FROM likes l LEFT JOIN catalog_tracks c ON c.id = l.track_id",
        "SELECT id, json_extract(json, '$.artist'), json_extract(json, '$.albumId')
           FROM catalog_tracks WHERE json_extract(json, '$.saved') = 1",
    ] {
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })?;
        for row in rows {
            let (track, artist, album) = row?;
            if artist.is_some_and(|a| normalize_artist(&a) == key) {
                songs.entry(track).or_insert(album.unwrap_or_default());
            }
        }
    }
    let mut out = Vec::with_capacity(songs.len());
    for (track, album) in songs {
        let mut work = vec![Anchor::Track(TrackId(track.clone()))];
        if !album.is_empty() {
            work.push(Anchor::Album(AlbumId(album)));
        }
        out.push((TrackId(track), artist_of(conn, name, &work)?));
    }
    Ok(out)
}

/// The name a name's id stands for, as an artist was called by it; None for a name this module
/// has never seen.
pub fn name_of(conn: &Connection, name_id: &ArtistId) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT name FROM identity_called WHERE name_id = ?1 LIMIT 1",
            [name_id.as_str()],
            |r| r.get(0),
        )
        .optional()?)
}

fn next_listener(conn: &Connection) -> Result<u64> {
    let n: Option<i64> = conn.query_row(
        "SELECT MAX(CAST(substr(anchor, 5) AS INTEGER)) FROM identity_anchors WHERE anchor LIKE 'you:%'",
        [],
        |r| r.get(0),
    )?;
    Ok(n.map_or(1, |n| n.unsigned_abs() + 1))
}

/// When an artist was first known: its first anchor's row (one with none is the newest).
fn age(conn: &Connection, artist: &ArtistId) -> Result<i64> {
    let at: Option<i64> = conn
        .prepare_cached("SELECT MIN(rowid) FROM identity_anchors WHERE artist_id = ?1")?
        .query_row([artist.as_str()], |r| r.get(0))?;
    Ok(at.unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests;
