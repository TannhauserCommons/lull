//! Audio and cover art, handed straight through from the library. lull doesn't transcode:
//! `stream` and `download` both send the saved file, and `maxBitRate` / `format` are ignored.

use axum::http::{HeaderMap, header};
use lull_core::ids::{AlbumId, TrackId};

use crate::params::Params;
use crate::response::Fail;
use crate::{Reply, Service};

/// The request's `Range` goes to the library, which answers 200 or 206.
pub async fn stream(svc: &Service, p: &Params, headers: &HeaderMap) -> Result<Reply, Fail> {
    let id = TrackId(p.required("id")?.to_owned());
    let range = headers.get(header::RANGE).cloned();
    Ok(Reply::Raw(svc.library.stream(&id, range).await?))
}

/// Cover art ids are album ids; a song's id gives its album's cover, a playlist's its own cover
/// or else its first song's album's.
pub async fn cover(svc: &Service, p: &Params) -> Result<Reply, Fail> {
    let id = p.required("id")?;
    let size = p.optional_number("size")?;
    let album = if id.starts_with("pl-") {
        if let Some(own) = svc.collection.playlist_cover(id, size).await? {
            return Ok(Reply::Raw(own));
        }
        crate::playlists::first_album(svc, id)
            .await?
            .ok_or_else(|| Fail::not_found(format!("no cover art {id}")))?
    } else if id.starts_with(AlbumId::PREFIX) {
        AlbumId(id.to_owned())
    } else if id.starts_with(TrackId::PREFIX) {
        svc.track(id).await?.album_id
    } else {
        return Err(Fail::not_found(format!("no cover art {id}")));
    };
    Ok(Reply::Raw(svc.library.cover(&album, size).await?))
}
