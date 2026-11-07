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
use lull_core::{Error, Result, identity};
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
    let namesakes = svc.ctx.config.flags.on(&lull_core::flags::NAMESAKES);
    svc.ctx
        .db
        .call(move |conn| {
            let rows = conn
                .prepare("SELECT kind, id, name FROM follows ORDER BY followed_at DESC, rowid DESC")?
                .query_map([], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let (mut artists, mut tags, mut entities) = (Vec::new(), Vec::new(), Entities::default());
            // ADR-0035: a follow is of the artist it is now; one kept under a name several artists
            // share, that nothing says which of, is listed as it is and named in `unsure`.
            let mut unsure = Vec::new();
            for (kind, id, name) in rows {
                if kind == "tag" {
                    tags.push(id);
                    continue;
                }
                let mut id = ArtistId(id);
                if namesakes {
                    match identity::now_is(conn, &id)? {
                        Some(now) => id = now,
                        None => unsure.push(id.clone()),
                    }
                }
                if !artists.contains(&id) {
                    entities.artists.insert(id.clone(), named_artist(id.clone(), name));
                    artists.push(id);
                }
            }
            let mut out = json!({ "artists": artists, "tags": tags, "entities": entities });
            if !unsure.is_empty() {
                out["unsure"] = json!(unsure);
            }
            Ok(Json(out))
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
    let namesakes = svc.ctx.config.flags.on(&lull_core::flags::NAMESAKES);
    svc.write(move |tx, events| {
        set(
            tx,
            events,
            kind,
            &id,
            name.as_deref(),
            Change {
                followed: true,
                at: lull_core::now(),
                namesakes,
            },
        )
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn unfollow(State(svc): State<Arc<Service>>, Path((kind, id)): Path<(String, String)>) -> Result<StatusCode> {
    let kind = Kind::parse(&kind)?;
    let namesakes = svc.ctx.config.flags.on(&lull_core::flags::NAMESAKES);
    svc.write(move |tx, events| {
        set(
            tx,
            events,
            kind,
            &id,
            None,
            Change {
                followed: false,
                at: lull_core::now(),
                namesakes,
            },
        )
    })
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
    change: Change,
) -> Result<()> {
    let Change {
        followed,
        at,
        namesakes,
    } = change;
    // ADR-0035: an artist's follow kept from before under its name's id is that artist's follow.
    let same = |conn: &Connection| -> Result<Vec<String>> {
        let ids: Vec<String> = conn
            .prepare("SELECT id FROM follows WHERE kind = 'artist'")?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let mut same = Vec::new();
        for kept in ids {
            if kept == id || identity::now_is(conn, &ArtistId(kept.clone()))?.is_some_and(|a| a.as_str() == id) {
                same.push(kept);
            }
        }
        Ok(same)
    };
    let artist_by_name = matches!(kind, Kind::Artist) && namesakes;
    if followed {
        let name = match kind {
            Kind::Tag => tag_name(id)?,
            Kind::Artist if namesakes && own_id_called(conn, id, name)? => name.map(str::to_owned).unwrap_or_default(),
            Kind::Artist => artist_name(id, name)?,
        };
        if artist_by_name && !same(conn)?.is_empty() {
            events.push("follow", json!({ "kind": kind.as_str(), "id": id, "followed": true }));
            return Ok(());
        }
        conn.execute(
            "INSERT INTO follows (kind, id, name, followed_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(kind, id) DO NOTHING",
            params![kind.as_str(), id, name, at],
        )?;
    } else if artist_by_name {
        for kept in same(conn)? {
            conn.execute("DELETE FROM follows WHERE kind = 'artist' AND id = ?1", [kept])?;
        }
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

/// ADR-0035: an artist's own id comes with a name it is called by (`identity`), not one it hashes from.
fn own_id_called(conn: &Connection, id: &str, name: Option<&str>) -> Result<bool> {
    match name {
        Some(n) => identity::is_called(conn, &ArtistId(id.to_owned()), n.trim()),
        None => Ok(false),
    }
}

/// What a follow change does: follow or unfollow, when, and whether ADR-0035's switch is on.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Change {
    pub followed: bool,
    pub at: i64,
    pub namesakes: bool,
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

#[cfg(test)]
mod tests {
    use lull_core::identity::{self, Anchor, Came};
    use lull_core::ids::AlbumId;

    use super::*;

    /// A hub with two artists called Nirvana, the listener's songs on the first, and a follow of
    /// "Nirvana" from before.
    fn hub() -> Result<Connection> {
        let conn = Connection::open_in_memory()?;
        for m in [
            &lull_core::db::CORE,
            &lull_core::catalog::MIGRATIONS,
            &crate::MIGRATIONS,
            &identity::MIGRATIONS,
        ] {
            for step in m.steps {
                conn.execute_batch(step)?;
            }
        }
        conn.execute(
            "INSERT INTO plays (track_id, artist, title, album, duration_s, started_at) VALUES ('t-0000000000000001', 'Nirvana', 'Lithium', 'Nevermind', 257, 1)",
            [],
        )?;
        let song = lull_core::model::Track {
            id: lull_core::ids::TrackId("t-0000000000000001".into()),
            title: "Lithium".into(),
            artist_id: ArtistId::of("Nirvana"),
            artist: "Nirvana".into(),
            album_id: AlbumId("al-0000000000000001".into()),
            album: "Nevermind".into(),
            year: None,
            no: None,
            disc: None,
            duration: Some(257),
            tags: Vec::new(),
            saved: false,
            file: None,
        };
        lull_core::catalog::remember(&conn, &[song])?;
        let mut events = Events(Vec::new());
        let change = |followed| Change {
            followed,
            at: 1,
            namesakes: false,
        };
        set(
            &conn,
            &mut events,
            Kind::Artist,
            ArtistId::of("Nirvana").as_str(),
            Some("Nirvana"),
            change(true),
        )?;
        let nevermind = Anchor::Album(AlbumId("al-0000000000000001".into()));
        identity::place(&conn, "Nirvana", &[Anchor::Deezer(415), nevermind], Came::Credit)?;
        identity::place(&conn, "Nirvana", &[Anchor::Deezer(278_793_911)], Came::Credit)?;
        Ok(conn)
    }

    fn kept(conn: &Connection) -> Result<Vec<String>> {
        Ok(conn
            .prepare("SELECT id FROM follows WHERE kind = 'artist'")?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// The owner's rule: the follow is the listener's Nirvana's, so following that Nirvana again
    /// adds nothing and unfollowing it removes the follow from before; the namesake never had it.
    #[test]
    fn a_follow_from_before_is_the_listeners_nirvanas_to_follow_and_unfollow() -> Result<()> {
        let conn = hub()?;
        let mut events = Events(Vec::new());
        let on = |followed| Change {
            followed,
            at: 2,
            namesakes: true,
        };
        let (mine, other) = (
            ArtistId::own("Nirvana", "dz:415"),
            ArtistId::own("Nirvana", "dz:278793911"),
        );
        assert_eq!(identity::now_is(&conn, &ArtistId::of("Nirvana"))?, Some(mine.clone()));

        set(
            &conn,
            &mut events,
            Kind::Artist,
            mine.as_str(),
            Some("Nirvana"),
            on(true),
        )?;
        assert_eq!(
            kept(&conn)?,
            [ArtistId::of("Nirvana").0.clone()],
            "no second follow of the same artist"
        );
        set(&conn, &mut events, Kind::Artist, other.as_str(), None, on(false))?;
        assert_eq!(
            kept(&conn)?.len(),
            1,
            "unfollowing the namesake touches nothing of the listener's"
        );
        set(&conn, &mut events, Kind::Artist, mine.as_str(), None, on(false))?;
        assert!(
            kept(&conn)?.is_empty(),
            "unfollowing their Nirvana removes the follow from before"
        );

        // An artist's own id is followed by the name it is called, which is checked.
        set(
            &conn,
            &mut events,
            Kind::Artist,
            other.as_str(),
            Some("Nirvana"),
            on(true),
        )?;
        assert_eq!(kept(&conn)?, vec![other.0.clone()]);
        assert!(
            set(
                &conn,
                &mut events,
                Kind::Artist,
                mine.as_str(),
                Some("Pearl Jam"),
                on(true)
            )
            .is_err()
        );
        Ok(())
    }
}
