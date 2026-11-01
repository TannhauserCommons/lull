//! `search3`: artists and albums by name, songs through the library's own search. An empty query
//! lists everything, which is how clients such as Symfonium sync a whole library.

use lull_core::ids::normalize;
use serde_json::{Value, json};

use crate::browse::group_by_artist;
use crate::params::Params;
use crate::response::Fail;
use crate::{Reply, Service, shape};

pub async fn search(svc: &Service, p: &Params) -> Result<Reply, Fail> {
    let raw = p.required("query")?;
    // Some clients send the empty query as `""`.
    let query = normalize(raw.trim_matches('"'));
    let (artist_count, artist_offset) = p.page("artistCount", "artistOffset", 20, 500)?;
    let (album_count, album_offset) = p.page("albumCount", "albumOffset", 20, 500)?;
    let (song_count, song_offset) = p.page("songCount", "songOffset", 20, 500)?;
    let matches = |text: &str| query.is_empty() || normalize(text).contains(&query);

    let albums = svc.library.albums(None).await?;
    let by_artist = group_by_artist(&albums);
    let artist: Vec<Value> = svc
        .library
        .artists()
        .await?
        .iter()
        .filter(|a| matches(&a.name))
        .skip(artist_offset)
        .take(artist_count)
        .map(|a| {
            let theirs = by_artist.get(&a.id).map_or(&[][..], Vec::as_slice);
            shape::artist(a, theirs.len(), theirs.first().copied())
        })
        .collect();
    let album: Vec<Value> = albums
        .iter()
        .filter(|a| matches(&a.title) || matches(&a.artist))
        .skip(album_offset)
        .take(album_count)
        .map(shape::album)
        .collect();

    let songs = if query.is_empty() {
        let ids: Vec<_> = albums
            .iter()
            .flat_map(|a| &a.track_ids)
            .skip(song_offset)
            .take(song_count)
            .cloned()
            .collect();
        svc.tracks(&ids).await?
    } else {
        let found = svc.library.search(raw, song_offset + song_count).await?;
        found.into_iter().skip(song_offset).collect()
    };
    let starred = svc.starred().await?;
    let song: Vec<Value> = songs.iter().map(|t| shape::song(t, &starred)).collect();
    Ok(Reply::of(
        "searchResult3",
        json!({ "artist": artist, "album": album, "song": song }),
    ))
}
