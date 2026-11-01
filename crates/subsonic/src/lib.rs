//! The OpenSubsonic API at `/rest/*`, over the `Library`, `Collection` and `Lyrics` seams, so existing
//! Subsonic apps work with lull (docs/context/subsonic.md, ADR-0002 for app passwords).
//! Every call is `/rest/<endpoint>` (`.view` optional), by GET or form POST; the hub's gate
//! leaves these paths to `auth`.

mod auth;
mod browse;
mod likes;
mod lyrics;
mod media;
mod params;
mod playlists;
mod response;
mod search;
mod shape;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, RawQuery, State};
use axum::http::{HeaderMap, header};
use axum::response::Response;
use axum::routing::get;
use lull_core::db::{Db, Migrations};
use lull_core::ids::TrackId;
use lull_core::model::Track;
use lull_core::seams::{Collection, Library, Lyrics};
use lull_core::{Ctx, Result};
use serde_json::{Map, Value, json};

use crate::params::Params;
use crate::response::{Fail, Format};

pub const MIGRATIONS: Migrations = Migrations {
    owner: "subsonic",
    steps: &[],
};

pub struct Service {
    db: Db,
    library: Arc<dyn Library>,
    collection: Arc<dyn Collection>,
    lyrics: Arc<dyn Lyrics>,
}

pub async fn start(
    ctx: &Ctx,
    library: Arc<dyn Library>,
    collection: Arc<dyn Collection>,
    lyrics: Arc<dyn Lyrics>,
) -> Result<Arc<Service>> {
    Ok(Arc::new(Service {
        db: ctx.db.clone(),
        library,
        collection,
        lyrics,
    }))
}

pub fn router(svc: Arc<Service>) -> Router {
    Router::new()
        .route("/rest/{endpoint}", get(handle).post(handle))
        .with_state(svc)
}

/// What an endpoint answers: a body for the envelope, or a response of its own (audio, images).
enum Reply {
    Body(Map<String, Value>),
    Raw(Response),
}

impl Reply {
    fn empty() -> Self {
        Reply::Body(Map::new())
    }

    fn of(key: &str, value: Value) -> Self {
        Reply::Body(Map::from_iter([(key.to_owned(), value)]))
    }
}

async fn handle(
    State(svc): State<Arc<Service>>,
    Path(endpoint): Path<String>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let params = Params::parse(query.as_deref(), form(&headers, &body));
    let format = Format::of(params.get("f"));
    let endpoint = endpoint.strip_suffix(".view").unwrap_or(&endpoint);
    match svc.answer(endpoint, &params, &headers).await {
        Ok(Reply::Body(body)) => response::ok(format, body),
        Ok(Reply::Raw(response)) => response,
        Err(fail) => response::fail(format, fail),
    }
}

/// The body's parameters when it is a form POST; any other body is not Subsonic's.
fn form<'a>(headers: &HeaderMap, body: &'a [u8]) -> &'a [u8] {
    let is_form = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("application/x-www-form-urlencoded"));
    if is_form { body } else { &[] }
}

impl Service {
    async fn answer(&self, endpoint: &str, p: &Params, headers: &HeaderMap) -> Result<Reply, Fail> {
        // OpenSubsonic: clients ask this before they know how to sign in.
        if endpoint == "getOpenSubsonicExtensions" {
            return Ok(extensions());
        }
        auth::check(&self.db, p).await?;
        match endpoint {
            "ping" => Ok(Reply::empty()),
            "getLicense" => Ok(Reply::of("license", json!({ "valid": true }))),
            "getUser" => auth::user(p),
            "getMusicFolders" => Ok(browse::music_folders()),
            "getIndexes" => browse::indexes(self).await,
            "getArtists" => browse::artists(self).await,
            "getArtist" => browse::artist(self, p).await,
            "getAlbum" => browse::album(self, p).await,
            "getSong" => browse::song(self, p).await,
            "getAlbumList2" => browse::album_list(self, p).await,
            "getRandomSongs" => browse::random_songs(self, p).await,
            "getGenres" => browse::genres(self).await,
            "getSongsByGenre" => browse::songs_by_genre(self, p).await,
            "search3" => search::search(self, p).await,
            "stream" | "download" => media::stream(self, p, headers).await,
            "getCoverArt" => media::cover(self, p).await,
            "star" => likes::star(self, p, true).await,
            "unstar" => likes::star(self, p, false).await,
            "getStarred2" => likes::starred(self, "starred2").await,
            "getStarred" => likes::starred(self, "starred").await,
            "scrobble" => likes::scrobble(self, p).await,
            "getPlaylists" => playlists::list(self).await,
            "getPlaylist" => playlists::get(self, p).await,
            "createPlaylist" => playlists::create(self, p).await,
            "updatePlaylist" => playlists::update(self, p).await,
            "deletePlaylist" => playlists::delete(self, p).await,
            "getLyricsBySongId" => lyrics::by_song(self, p).await,
            other => Err(Fail::not_found(format!("lull has no {other} endpoint"))),
        }
    }

    /// Saved tracks for `ids`, in the order given (repeats kept); unsaved ids are left out.
    async fn tracks(&self, ids: &[TrackId]) -> Result<Vec<Track>, Fail> {
        let saved = self.saved(ids).await?;
        Ok(ids.iter().filter_map(|id| saved.get(id).cloned()).collect())
    }

    async fn saved(&self, ids: &[TrackId]) -> Result<HashMap<TrackId, Track>, Fail> {
        let unique: Vec<TrackId> = ids.iter().collect::<HashSet<_>>().into_iter().cloned().collect();
        if unique.is_empty() {
            return Ok(HashMap::new());
        }
        let found = self.library.saved(&unique).await?;
        Ok(found.into_iter().map(|t| (t.id.clone(), t)).collect())
    }

    async fn track(&self, id: &str) -> Result<Track, Fail> {
        let id = TrackId(id.to_owned());
        self.tracks(std::slice::from_ref(&id))
            .await?
            .pop()
            .ok_or_else(|| Fail::not_found(format!("no saved song {id}")))
    }

    /// Liked songs, newest first, from core's `likes` table (every crate may read it).
    async fn liked(&self) -> Result<Vec<(TrackId, i64)>, Fail> {
        Ok(self
            .db
            .call(|conn| {
                let mut stmt = conn.prepare("SELECT track_id, liked_at FROM likes ORDER BY liked_at DESC")?;
                let rows = stmt.query_map([], |r| Ok((TrackId(r.get(0)?), r.get(1)?)))?;
                Ok(rows.collect::<rusqlite::Result<_>>()?)
            })
            .await?)
    }

    async fn starred(&self) -> Result<shape::Starred, Fail> {
        Ok(self.liked().await?.into_iter().collect())
    }
}

fn extensions() -> Reply {
    Reply::of(
        "openSubsonicExtensions",
        json!([
            { "name": "apiKeyAuthentication", "versions": [1] },
            { "name": "formPost", "versions": [1] },
            { "name": "songLyrics", "versions": [1] },
        ]),
    )
}

/// A uniformly shuffled order, from the OS's randomness.
fn shuffle<T>(items: &mut [T]) -> Result<(), Fail> {
    let mut draws = vec![0u8; items.len() * 8];
    getrandom::fill(&mut draws).map_err(|e| Fail::generic(format!("random: {e}")))?;
    for i in (1..items.len()).rev() {
        let mut draw = [0u8; 8];
        draw.copy_from_slice(&draws[i * 8..i * 8 + 8]);
        let j = (u64::from_le_bytes(draw) % (i as u64 + 1)) as usize;
        items.swap(i, j);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
