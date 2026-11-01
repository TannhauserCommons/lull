//! The saved library in memory, keyed by track id whichever source it comes from (ADR-0003).
//! Every lookup, list and search (the UI's and other crates', through the `Library` seam) is
//! answered from here, never by a call to the source per request. `sync` (Jellyfin) and
//! `folder` (the music folder) refresh it and persist it.

use std::collections::{BTreeMap, HashMap};

use lull_core::ids::{AlbumId, ArtistId, TrackId, normalize};
use lull_core::model::{Album, Artist, Saved as SavedState, Track};
use lull_core::{Error, Result};
use serde::Serialize;

use crate::items::{Art, Quality, Saved};

#[derive(Default)]
pub struct Index {
    /// By location key (Jellyfin item id or file path): what everything else is derived from.
    items: HashMap<String, Saved>,
    /// Content id → location key of its best copy (a song saved twice is one song).
    best: HashMap<TrackId, String>,
    albums: BTreeMap<AlbumId, AlbumEntry>,
    artists: BTreeMap<ArtistId, Artist>,
    /// Ids that name a song by another id: a save's asked id (docs/context/save-flow.md#ids), or
    /// the other source's id for the same file (docs/context/library.md#switching).
    aliases: HashMap<TrackId, TrackId>,
}

pub struct AlbumEntry {
    pub album: Album,
    /// Newest file's date added (ISO 8601).
    pub added: String,
    pub art: Option<Art>,
    pub mb_release_group: Option<String>,
}

/// A genre (tag) in the library, for the Genres segment.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Genre {
    pub id: String,
    pub name: String,
    pub artists: usize,
    pub songs: usize,
    /// A few songs for the card's mosaic.
    pub track_ids: Vec<TrackId>,
}

impl Index {
    pub fn new(items: Vec<Saved>, aliases: HashMap<TrackId, TrackId>) -> Self {
        let mut index = Index {
            aliases,
            ..Index::default()
        };
        index.replace(items);
        index
    }

    /// The whole library, as a full read of the source found it.
    pub fn replace(&mut self, items: Vec<Saved>) {
        self.items = items.into_iter().map(|s| (s.location.key().to_owned(), s)).collect();
        self.rebuild();
    }

    /// Adds or updates items (a save's rescan); returns those that were new or changed.
    pub fn merge(&mut self, items: Vec<Saved>) -> Vec<Saved> {
        let changed: Vec<Saved> = items
            .into_iter()
            .filter(|s| self.items.get(s.location.key()) != Some(s))
            .collect();
        if !changed.is_empty() {
            for s in &changed {
                self.items.insert(s.location.key().to_owned(), s.clone());
            }
            self.rebuild();
        }
        changed
    }

    /// Drops items by location key (files gone from the music folder).
    pub fn remove(&mut self, keys: &[String]) {
        let before = self.items.len();
        for key in keys {
            self.items.remove(key);
        }
        if self.items.len() != before {
            self.rebuild();
        }
    }

    /// An item by location key.
    pub fn item(&self, key: &str) -> Option<&Saved> {
        self.items.get(key)
    }

    /// Every item, best copies or not: what reconciliation matches by path.
    pub fn items(&self) -> impl Iterator<Item = &Saved> {
        self.items.values()
    }

    pub fn aliases(&self) -> &HashMap<TrackId, TrackId> {
        &self.aliases
    }

    pub fn alias(&mut self, asked: TrackId, saved: TrackId) {
        self.aliases.insert(asked, saved);
    }

    pub fn len(&self) -> usize {
        self.best.len()
    }

    /// The saved copy of a song, by its own id or an alias. A song's own id wins over an alias
    /// from it, so switching back to a source finds its songs under their ids.
    pub fn saved(&self, id: &TrackId) -> Option<&Saved> {
        let key = self.best.get(id).or_else(|| self.best.get(self.aliases.get(id)?))?;
        self.items.get(key)
    }

    /// The saved track under the id it was asked by, so callers can match their own ids.
    pub fn track(&self, id: &TrackId) -> Option<Track> {
        self.saved(id).map(|s| Track {
            id: id.clone(),
            ..s.track.clone()
        })
    }

    pub fn album(&self, id: &AlbumId) -> Option<&AlbumEntry> {
        self.albums.get(id)
    }

    /// Saved songs whose artist, album and title contain every word of `query`, best first.
    pub fn search(&self, query: &str, limit: usize) -> Vec<Track> {
        let q = normalize(query);
        if q.is_empty() {
            return vec![];
        }
        let words: Vec<&str> = q.split(' ').collect();
        let mut hits: Vec<(u8, &Track)> = self
            .tracks()
            .filter_map(|t| {
                let hay = normalize(&format!("{} {} {}", t.artist, t.album, t.title));
                words.iter().all(|w| hay.contains(w)).then(|| {
                    let title = normalize(&t.title);
                    let rank = if title == q {
                        3
                    } else if title.starts_with(&q) {
                        2
                    } else {
                        u8::from(title.contains(&q))
                    };
                    (rank, t)
                })
            })
            .collect();
        hits.sort_by(|(ra, a), (rb, b)| rb.cmp(ra).then_with(|| album_order(a, b)));
        hits.into_iter().take(limit).map(|(_, t)| t.clone()).collect()
    }

    pub fn songs(&self, sort: &str, filter: &str) -> Result<Vec<&Track>> {
        let mut list: Vec<&Track> = self
            .tracks()
            .filter(|t| matches(filter, &[&t.title, &t.artist, &t.album]))
            .collect();
        match sort {
            "album" | "" => list.sort_by(|a, b| album_order(a, b)),
            "title" => list.sort_by_cached_key(|t| (normalize(&t.title), normalize(&t.artist))),
            "artist" => list.sort_by(|a, b| normalize(&a.artist).cmp(&normalize(&b.artist)).then(album_order(a, b))),
            "recent" => list.sort_by_cached_key(|t| std::cmp::Reverse(self.saved(&t.id).map(|s| s.added.clone()))),
            other => return Err(bad_sort(other, "album, title, artist, recent")),
        }
        Ok(list)
    }

    pub fn albums(&self, sort: &str, filter: &str) -> Result<Vec<&AlbumEntry>> {
        let mut list: Vec<&AlbumEntry> = self
            .albums
            .values()
            .filter(|e| matches(filter, &[&e.album.title, &e.album.artist]))
            .collect();
        match sort {
            "recent" | "" => list.sort_by(|a, b| b.added.cmp(&a.added)),
            "title" => list.sort_by_cached_key(|e| normalize(&e.album.title)),
            "artist" => list.sort_by_cached_key(|e| (normalize(&e.album.artist), e.album.year)),
            "year" => list.sort_by_key(|e| std::cmp::Reverse(e.album.year)),
            other => return Err(bad_sort(other, "recent, title, artist, year")),
        }
        Ok(list)
    }

    pub fn artists(&self, sort: &str, filter: &str) -> Result<Vec<&Artist>> {
        let mut list: Vec<&Artist> = self.artists.values().filter(|a| matches(filter, &[&a.name])).collect();
        match sort {
            "name" | "" => list.sort_by_cached_key(|a| normalize(&a.name)),
            other => return Err(bad_sort(other, "name")),
        }
        Ok(list)
    }

    pub fn genres(&self, sort: &str, filter: &str) -> Result<Vec<Genre>> {
        let mut by_name: BTreeMap<String, (Vec<&Track>, std::collections::BTreeSet<&ArtistId>)> = BTreeMap::new();
        for t in self.tracks() {
            for tag in &t.tags {
                let entry = by_name.entry(tag.clone()).or_default();
                entry.0.push(t);
                entry.1.insert(&t.artist_id);
            }
        }
        let mut list: Vec<Genre> = by_name
            .into_iter()
            .filter(|(name, _)| matches(filter, &[name]))
            .map(|(name, (tracks, artists))| Genre {
                id: name.clone(),
                name,
                artists: artists.len(),
                songs: tracks.len(),
                track_ids: tracks.iter().take(4).map(|t| t.id.clone()).collect(),
            })
            .collect();
        match sort {
            "songs" | "" => list.sort_by(|a, b| b.songs.cmp(&a.songs).then_with(|| a.name.cmp(&b.name))),
            "name" => list.sort_by_cached_key(|g| normalize(&g.name)),
            other => return Err(bad_sort(other, "songs, name")),
        }
        Ok(list)
    }

    fn tracks(&self) -> impl Iterator<Item = &Track> {
        self.best
            .values()
            .filter_map(|item| self.items.get(item))
            .map(|s| &s.track)
    }

    /// Derives songs, albums and artists from the items.
    fn rebuild(&mut self) {
        let mut best: HashMap<TrackId, &Saved> = HashMap::new();
        for s in self.items.values() {
            let better = best.get(&s.track.id).is_none_or(|cur| {
                (Quality::of(s.track.file.as_ref()), &s.added) > (Quality::of(cur.track.file.as_ref()), &cur.added)
            });
            if better {
                best.insert(s.track.id.clone(), s);
            }
        }
        let mut albums: BTreeMap<AlbumId, Vec<&Saved>> = BTreeMap::new();
        for s in best.values() {
            albums.entry(s.track.album_id.clone()).or_default().push(s);
        }
        self.albums = albums.into_iter().map(|(id, songs)| (id, album_entry(songs))).collect();
        self.artists = artists(best.values().copied());
        self.best = best
            .into_iter()
            .map(|(id, s)| (id, s.location.key().to_owned()))
            .collect();
    }
}

fn album_entry(mut songs: Vec<&Saved>) -> AlbumEntry {
    songs.sort_by(|a, b| album_order(&a.track, &b.track));
    let first = songs.first().map(|s| (&s.track, s.album_artist.as_str()));
    let (title, owner, year) = first
        .map(|(t, owner)| (t.album.clone(), owner.to_owned(), t.year))
        .unwrap_or_default();
    let tags = top_tags(songs.iter().flat_map(|s| &s.track.tags));
    let duration = songs.iter().map(|s| s.track.duration).sum::<Option<u32>>();
    AlbumEntry {
        album: Album {
            id: AlbumId::of(&owner, &title),
            artist_id: ArtistId::of(&owner),
            artist: owner,
            title,
            year,
            kind: None,
            released: year.map(|y| y.to_string()),
            label: None,
            tags,
            saved: SavedState::All,
            track_ids: songs.iter().map(|s| s.track.id.clone()).collect(),
            duration,
            musicbrainz_id: songs.iter().find_map(|s| s.mb_release.clone()),
        },
        added: songs.iter().map(|s| s.added.clone()).max().unwrap_or_default(),
        art: songs.iter().find_map(|s| s.art.clone()),
        mb_release_group: songs.iter().find_map(|s| s.mb_release_group.clone()),
    }
}

/// Every artist credited on a saved song, plus album owners (not "Various Artists").
fn artists<'a>(songs: impl Iterator<Item = &'a Saved>) -> BTreeMap<ArtistId, Artist> {
    let mut found: BTreeMap<ArtistId, (String, Vec<&String>, Option<String>)> = BTreeMap::new();
    for s in songs {
        let owns = s.album_artist == s.track.artist;
        let entry = found
            .entry(s.track.artist_id.clone())
            .or_insert_with(|| (s.track.artist.clone(), vec![], None));
        entry.1.extend(&s.track.tags);
        if owns && entry.2.is_none() {
            entry.2.clone_from(&s.mb_artist);
        }
    }
    found
        .into_iter()
        .map(|(id, (name, tags, mbid))| {
            let artist = Artist {
                id: id.clone(),
                name,
                tags: top_tags(tags.into_iter()),
                listeners: None,
                bio: None,
                musicbrainz_id: mbid,
            };
            (id, artist)
        })
        .collect()
}

/// The five most common tags, most common first.
fn top_tags<'a>(tags: impl Iterator<Item = &'a String>) -> Vec<String> {
    let mut counts: BTreeMap<&String, usize> = BTreeMap::new();
    for t in tags {
        *counts.entry(t).or_default() += 1;
    }
    let mut list: Vec<(&String, usize)> = counts.into_iter().collect();
    list.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    list.into_iter().take(5).map(|(t, _)| t.clone()).collect()
}

fn album_order(a: &Track, b: &Track) -> std::cmp::Ordering {
    (
        normalize(&a.album),
        a.disc.unwrap_or(1),
        a.no.unwrap_or(0),
        normalize(&a.title),
    )
        .cmp(&(
            normalize(&b.album),
            b.disc.unwrap_or(1),
            b.no.unwrap_or(0),
            normalize(&b.title),
        ))
}

fn matches(filter: &str, fields: &[&str]) -> bool {
    let f = normalize(filter);
    f.is_empty() || fields.iter().any(|x| normalize(x).contains(&f))
}

fn bad_sort(got: &str, allowed: &str) -> Error {
    Error::BadRequest(format!("Unknown sort {got:?}; use one of: {allowed}."))
}
