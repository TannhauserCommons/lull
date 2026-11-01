//! Followed artists and tags (core `follows` table), read by discovery for new releases and
//! followed-tag shelves. A `follow` event tells discovery and the listener's other devices.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, put};
use axum::{Json, Router};
use lull_core::ids::ArtistId;
use lull_core::model::{Artist, Entities};
use lull_core::{Error, Result};
use rusqlite::{Connection, params};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{Events, Service};

pub(crate) fn routes() -> Router<Arc<Service>> {
    Router::new()
        .route("/api/follows", get(list))
        .route("/api/follows/{kind}/{id}", put(follow).delete(unfollow))
}

/// `{ artists: [id], tags: [id], entities }`, newest first. Artist entities carry only the name
/// collection knows; discovery's artist page has the rest.
async fn list(State(svc): State<Arc<Service>>) -> Result<Json<Value>> {
    svc.ctx
        .db
        .call(|conn| {
            let rows = conn
                .prepare("SELECT kind, id, name FROM follows ORDER BY followed_at DESC, rowid DESC")?
                .query_map([], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let (mut artists, mut tags, mut entities) = (Vec::new(), Vec::new(), Entities::default());
            for (kind, id, name) in rows {
                if kind == "tag" {
                    tags.push(id);
                    continue;
                }
                let id = ArtistId(id);
                entities.artists.insert(id.clone(), named_artist(id.clone(), name));
                artists.push(id);
            }
            Ok(Json(json!({ "artists": artists, "tags": tags, "entities": entities })))
        })
        .await
}

pub(crate) fn named_artist(id: ArtistId, name: String) -> Artist {
    Artist {
        id,
        name,
        tags: Vec::new(),
        listeners: None,
        bio: None,
        musicbrainz_id: None,
    }
}

/// `PUT /api/follows/artist/{id} { name }` or `PUT /api/follows/tag/{name}`.
async fn follow(
    State(svc): State<Arc<Service>>,
    Path((kind, id)): Path<(String, String)>,
    body: Bytes,
) -> Result<StatusCode> {
    let kind = Kind::parse(&kind)?;
    let name = match kind {
        Kind::Tag => None,
        Kind::Artist => Some(artist_body(&body)?),
    };
    svc.write(move |tx, events| set(tx, events, kind, &id, name.as_deref(), true, lull_core::now()))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn unfollow(State(svc): State<Arc<Service>>, Path((kind, id)): Path<(String, String)>) -> Result<StatusCode> {
    let kind = Kind::parse(&kind)?;
    svc.write(move |tx, events| set(tx, events, kind, &id, None, false, lull_core::now()))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Follows (dated `at`) or unfollows an artist or tag, idempotently. Following an artist needs
/// its name, which is checked against the id rather than trusted: an artist id is derived from
/// its name.
pub(crate) fn set(
    conn: &Connection,
    events: &mut Events,
    kind: Kind,
    id: &str,
    name: Option<&str>,
    followed: bool,
    at: i64,
) -> Result<()> {
    if followed {
        let name = match kind {
            Kind::Tag => tag_name(id)?,
            Kind::Artist => artist_name(id, name)?,
        };
        conn.execute(
            "INSERT INTO follows (kind, id, name, followed_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(kind, id) DO NOTHING",
            params![kind.as_str(), id, name, at],
        )?;
    } else {
        conn.execute(
            "DELETE FROM follows WHERE kind = ?1 AND id = ?2",
            params![kind.as_str(), id],
        )?;
    }
    events.push(
        "follow",
        json!({ "kind": kind.as_str(), "id": id, "followed": followed }),
    );
    Ok(())
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Kind {
    Artist,
    Tag,
}

impl Kind {
    pub(crate) fn parse(s: &str) -> Result<Self> {
        match s {
            "artist" => Ok(Kind::Artist),
            "tag" => Ok(Kind::Tag),
            other => Err(Error::NotFound(format!(
                "Can't follow a {other:?}: only artists and tags."
            ))),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Kind::Artist => "artist",
            Kind::Tag => "tag",
        }
    }
}

/// A tag's id is its name (api.md#conventions).
fn tag_name(id: &str) -> Result<String> {
    let name = id.trim();
    if name.is_empty() || name.len() > 100 || name != id {
        return Err(Error::BadRequest(format!("{id:?} is not a tag name.")));
    }
    Ok(name.to_owned())
}

/// `{ name }` from a follow request's body.
fn artist_body(body: &[u8]) -> Result<String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Body {
        name: String,
    }
    let Body { name } = serde_json::from_slice(body)
        .map_err(|e| Error::BadRequest(format!("Following an artist needs {{ \"name\" }}: {e}.")))?;
    Ok(name)
}

fn artist_name(id: &str, name: Option<&str>) -> Result<String> {
    let name = name
        .map(str::trim)
        .ok_or_else(|| Error::BadRequest(format!("Following the artist {id} needs its name.")))?;
    if ArtistId::of(name).as_str() != id {
        return Err(Error::BadRequest(format!("{name:?} is not the artist {id}.")));
    }
    Ok(name.to_owned())
}
