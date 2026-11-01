//! Browsing by ID3 tags: artists, albums, songs, album lists, random songs and genres. lull has
//! one library, so one music folder.

use std::collections::{BTreeMap, HashMap};

use lull_core::ids::{AlbumId, ArtistId, normalize, normalize_artist};
use lull_core::model::{Album, Artist};
use serde_json::{Value, json};

use crate::params::Params;
use crate::response::Fail;
use crate::{Reply, Service, shape, shuffle};

/// Articles `getArtists` files artists without ("The Cure" under C), as `normalize_artist` does.
const IGNORED_ARTICLES: &str = "The";

pub fn music_folders() -> Reply {
    Reply::of("musicFolders", json!({ "musicFolder": [{ "id": 1, "name": "Music" }] }))
}

pub async fn artists(svc: &Service) -> Result<Reply, Fail> {
    let index = index(svc).await?;
    Ok(Reply::of(
        "artists",
        json!({ "ignoredArticles": IGNORED_ARTICLES, "index": index }),
    ))
}

/// The pre-ID3 artist index; lull's library has no folders, so it is `getArtists` again.
pub async fn indexes(svc: &Service) -> Result<Reply, Fail> {
    let index = index(svc).await?;
    Ok(Reply::of(
        "indexes",
        json!({ "ignoredArticles": IGNORED_ARTICLES, "lastModified": lull_core::now() * 1000, "index": index }),
    ))
}

/// Artists under their first letter ("#" for anything else), each with its album count.
async fn index(svc: &Service) -> Result<Vec<Value>, Fail> {
    let artists = svc.library.artists().await?;
    let albums = svc.library.albums(None).await?;
    let by_artist = group_by_artist(&albums);
    let mut index: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for a in &artists {
        let theirs = by_artist.get(&a.id).map_or(&[][..], Vec::as_slice);
        index
            .entry(letter(&a.name))
            .or_default()
            .push(shape::artist(a, theirs.len(), theirs.first().copied()));
    }
    Ok(index
        .into_iter()
        .map(|(name, artist)| json!({ "name": name, "artist": artist }))
        .collect())
}

fn letter(name: &str) -> String {
    match normalize_artist(name).chars().next() {
        Some(c) if c.is_alphabetic() => c.to_uppercase().collect(),
        _ => "#".into(),
    }
}

pub fn group_by_artist(albums: &[Album]) -> HashMap<&ArtistId, Vec<&Album>> {
    let mut groups: HashMap<&ArtistId, Vec<&Album>> = HashMap::new();
    for album in albums {
        groups.entry(&album.artist_id).or_default().push(album);
    }
    groups
}

pub async fn artist(svc: &Service, p: &Params) -> Result<Reply, Fail> {
    let id = ArtistId(p.required("id")?.to_owned());
    let artist = find_artist(svc, &id).await?;
    let albums = svc.library.albums(Some(&id)).await?;
    let mut body = shape::artist(&artist, albums.len(), albums.first());
    body["album"] = albums.iter().map(shape::album).collect();
    Ok(Reply::of("artist", body))
}

async fn find_artist(svc: &Service, id: &ArtistId) -> Result<Artist, Fail> {
    svc.library
        .artists()
        .await?
        .into_iter()
        .find(|a| &a.id == id)
        .ok_or_else(|| Fail::not_found(format!("no artist {id}")))
}

pub async fn album(svc: &Service, p: &Params) -> Result<Reply, Fail> {
    let id = AlbumId(p.required("id")?.to_owned());
    let (album, tracks) = svc
        .library
        .album(&id)
        .await?
        .ok_or_else(|| Fail::not_found(format!("no album {id}")))?;
    let starred = svc.starred().await?;
    let mut body = shape::album(&album);
    body["song"] = tracks.iter().map(|t| shape::song(t, &starred)).collect();
    Ok(Reply::of("album", body))
}

pub async fn song(svc: &Service, p: &Params) -> Result<Reply, Fail> {
    let track = svc.track(p.required("id")?).await?;
    Ok(Reply::of("song", shape::song(&track, &svc.starred().await?)))
}

pub async fn album_list(svc: &Service, p: &Params) -> Result<Reply, Fail> {
    let (size, offset) = p.page("size", "offset", 10, 500)?;
    let mut albums = svc.library.albums(None).await?;
    match p.required("type")? {
        "random" => shuffle(&mut albums)?,
        // lull doesn't know when an album was added; the newest release stands in.
        "newest" => albums.sort_by(|a, b| released(b).cmp(&released(a))),
        "alphabeticalByName" => albums.sort_by_cached_key(|a| normalize(&a.title)),
        // `albums(None)` is already by artist, then year.
        "alphabeticalByArtist" => {}
        "byYear" => {
            let (from, to) = (p.number("fromYear", 0u16)?, p.number("toYear", u16::MAX)?);
            let (low, high) = (from.min(to), from.max(to));
            albums.retain(|a| a.year.is_some_and(|y| (low..=high).contains(&y)));
            albums.sort_by_key(|a| a.year);
            if from > to {
                albums.reverse();
            }
        }
        "byGenre" => {
            let genre = p.required("genre")?;
            albums.retain(|a| has_genre(a, genre));
        }
        "frequent" => by_plays(svc, &mut albums, |s| s.count).await?,
        "recent" => by_plays(svc, &mut albums, |s| s.last).await?,
        // lull stars songs only and has no ratings.
        "starred" | "highest" => albums.clear(),
        other => return Err(Fail::generic(format!("unknown album list type {other:?}"))),
    }
    let page: Vec<Value> = albums.iter().skip(offset).take(size).map(shape::album).collect();
    Ok(Reply::of("albumList2", json!({ "album": page })))
}

fn released(a: &Album) -> (Option<&str>, Option<u16>) {
    (a.released.as_deref(), a.year)
}

fn has_genre(a: &Album, genre: &str) -> bool {
    a.tags.iter().any(|t| t.eq_ignore_ascii_case(genre))
}

struct PlayStats {
    count: i64,
    last: i64,
}

/// Keeps the albums that have been played, most first by `key`.
async fn by_plays(svc: &Service, albums: &mut Vec<Album>, key: fn(&PlayStats) -> i64) -> Result<(), Fail> {
    let stats = play_stats(svc).await?;
    albums.retain(|a| stats.contains_key(&a.id));
    albums.sort_by_key(|a| std::cmp::Reverse(stats.get(&a.id).map_or(0, key)));
    Ok(())
}

/// Plays per album, from core's `plays` table joined to the catalog that knows each track's album.
async fn play_stats(svc: &Service) -> Result<HashMap<AlbumId, PlayStats>, Fail> {
    Ok(svc
        .db
        .call(|conn| {
            let mut stmt = conn.prepare(
                "SELECT c.album_id, COUNT(*), MAX(p.started_at) FROM plays p
                 JOIN catalog_tracks c ON c.id = p.track_id
                 WHERE c.album_id IS NOT NULL GROUP BY c.album_id",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    AlbumId(r.get(0)?),
                    PlayStats {
                        count: r.get(1)?,
                        last: r.get(2)?,
                    },
                ))
            })?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        })
        .await?)
}

/// Random saved songs; `genre`, `fromYear` and `toYear` filter by the song's album.
pub async fn random_songs(svc: &Service, p: &Params) -> Result<Reply, Fail> {
    let size = p.number("size", 10usize)?.min(500);
    let (from, to) = (
        p.optional_number::<u16>("fromYear")?,
        p.optional_number::<u16>("toYear")?,
    );
    let genre = p.get("genre");
    let mut ids: Vec<_> = svc
        .library
        .albums(None)
        .await?
        .into_iter()
        .filter(|a| genre.is_none_or(|g| has_genre(a, g)))
        .filter(|a| from.is_none_or(|f| a.year.is_some_and(|y| y >= f)))
        .filter(|a| to.is_none_or(|t| a.year.is_some_and(|y| y <= t)))
        .flat_map(|a| a.track_ids)
        .collect();
    shuffle(&mut ids)?;
    ids.truncate(size);
    let starred = svc.starred().await?;
    let songs: Vec<Value> = svc
        .tracks(&ids)
        .await?
        .iter()
        .map(|t| shape::song(t, &starred))
        .collect();
    Ok(Reply::of("randomSongs", json!({ "song": songs })))
}

/// Album tags are lull's genres.
pub async fn genres(svc: &Service) -> Result<Reply, Fail> {
    let mut counts: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    let albums = svc.library.albums(None).await?;
    for album in &albums {
        for tag in &album.tags {
            let (songs, albums) = counts.entry(tag).or_default();
            *songs += album.track_ids.len();
            *albums += 1;
        }
    }
    let genre: Vec<Value> = counts
        .into_iter()
        .map(|(name, (songs, albums))| json!({ "value": name, "songCount": songs, "albumCount": albums }))
        .collect();
    Ok(Reply::of("genres", json!({ "genre": genre })))
}
