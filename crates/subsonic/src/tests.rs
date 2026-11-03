//! Through the router, against small fakes of the `Library` and `Collection` seams.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, Request, StatusCode, header};
use axum::response::{IntoResponse, Response};
use lull_core::db::{self, Db};
use lull_core::devices::{self, Kind};
use lull_core::ids::{AlbumId, ArtistId, TrackId};
use lull_core::model::{
    Album, Artist, LyricLine, Lyrics as Words, LyricsKind, PlayReport, Playlist, PlaylistEdit, Saved, Track,
};
use lull_core::seams::{Collection, Library, Lyrics};
use lull_core::{Error, Result};
use serde_json::Value;
use tower::ServiceExt;

use super::*;

const ARTIST: &str = "The Velvet Orchard";
const ALBUM: &str = "Satellite Porch";

fn track(title: &str, no: u32) -> Track {
    Track {
        id: TrackId::of(ARTIST, ALBUM, title),
        title: title.into(),
        artist_id: ArtistId::of(ARTIST),
        artist: ARTIST.into(),
        album_id: AlbumId::of(ARTIST, ALBUM),
        album: ALBUM.into(),
        year: None,
        no: Some(no),
        disc: None,
        duration: Some(200),
        tags: vec!["dream pop".into()],
        saved: true,
        file: None,
    }
}

struct FakeLibrary {
    tracks: Vec<Track>,
    last_range: Mutex<Option<HeaderValue>>,
}

#[async_trait]
impl Library for FakeLibrary {
    async fn cover(&self, id: &AlbumId, _size: Option<u32>) -> Result<Response> {
        Ok(format!("cover of {id}").into_response())
    }
    async fn search(&self, query: &str, limit: usize) -> Result<Vec<Track>> {
        Ok(self
            .tracks
            .iter()
            .filter(|t| t.title.contains(query))
            .take(limit)
            .cloned()
            .collect())
    }
    async fn saved(&self, ids: &[TrackId]) -> Result<Vec<Track>> {
        Ok(self.tracks.iter().filter(|t| ids.contains(&t.id)).cloned().collect())
    }
    async fn artists(&self) -> Result<Vec<Artist>> {
        Ok(vec![Artist {
            id: ArtistId::of(ARTIST),
            name: ARTIST.into(),
            tags: vec![],
            listeners: None,
            bio: None,
            musicbrainz_id: None,
        }])
    }
    async fn albums(&self, _artist: Option<&ArtistId>) -> Result<Vec<Album>> {
        Ok(vec![Album {
            id: AlbumId::of(ARTIST, ALBUM),
            title: ALBUM.into(),
            artist_id: ArtistId::of(ARTIST),
            artist: ARTIST.into(),
            year: Some(2026),
            kind: None,
            released: None,
            label: None,
            tags: vec!["dream pop".into()],
            saved: Saved::All,
            track_ids: self.tracks.iter().map(|t| t.id.clone()).collect(),
            duration: Some(400),
            musicbrainz_id: None,
        }])
    }
    async fn album(&self, id: &AlbumId) -> Result<Option<(Album, Vec<Track>)>> {
        let album = self.albums(None).await?.into_iter().find(|a| &a.id == id);
        Ok(album.map(|a| (a, self.tracks.clone())))
    }
    /// Ten bytes of "audio"; a range of the first four answers 206, as the real library does.
    async fn stream(&self, _id: &TrackId, range: Option<HeaderValue>) -> Result<Response> {
        let partial = range.is_some();
        *self.last_range.lock().map_err(|_| Error::internal("poisoned"))? = range;
        Ok(if partial {
            (
                StatusCode::PARTIAL_CONTENT,
                [(header::CONTENT_RANGE, "bytes 0-3/10")],
                "0123",
            )
                .into_response()
        } else {
            "0123456789".into_response()
        })
    }
}

#[derive(Default)]
struct FakeCollection {
    playlist: Mutex<Option<Playlist>>,
    /// The playlist's own cover, as its body.
    cover: Mutex<Option<String>>,
    edits: Mutex<Vec<PlaylistEdit>>,
    plays: Mutex<Vec<PlayReport>>,
}

fn poisoned<T>(_: T) -> Error {
    Error::internal("poisoned")
}

#[async_trait]
impl Collection for FakeCollection {
    async fn liked(&self) -> Result<Vec<TrackId>> {
        Ok(vec![])
    }
    async fn set_liked(&self, _track: &TrackId, _liked: bool) -> Result<()> {
        Ok(())
    }
    async fn playlists(&self) -> Result<Vec<Playlist>> {
        Ok(self.playlist.lock().map_err(poisoned)?.clone().into_iter().collect())
    }
    async fn playlist(&self, id: &str) -> Result<Option<Playlist>> {
        Ok(self.playlist.lock().map_err(poisoned)?.clone().filter(|p| p.id == id))
    }
    async fn create_playlist(&self, _name: &str, _tracks: &[TrackId]) -> Result<Playlist> {
        Err(Error::internal("not in this fake"))
    }
    async fn edit_playlist(&self, id: &str, edit: PlaylistEdit) -> Result<Playlist> {
        self.edits.lock().map_err(poisoned)?.push(edit);
        self.playlist(id).await?.ok_or_else(|| Error::NotFound(id.into()))
    }
    async fn delete_playlist(&self, _id: &str) -> Result<()> {
        Ok(())
    }
    async fn record_play(&self, play: PlayReport) -> Result<()> {
        self.plays.lock().map_err(poisoned)?.push(play);
        Ok(())
    }
    async fn playlist_cover(&self, _id: &str, _size: Option<u32>) -> Result<Option<Response>> {
        Ok(self
            .cover
            .lock()
            .map_err(poisoned)?
            .clone()
            .map(IntoResponse::into_response))
    }
}

/// Every song has the same two synced lines.
struct FakeLyrics;

#[async_trait]
impl Lyrics for FakeLyrics {
    async fn lyrics(&self, _track: &TrackId) -> Result<Words> {
        let line = |t, text: &str| LyricLine { t, text: text.into() };
        Ok(Words {
            kind: LyricsKind::Synced,
            lines: vec![line(12.4, "Paper moon"), line(15.0, "Static & hum")],
            plain: None,
            source: "LRCLIB".into(),
        })
    }
}

struct Hub {
    app: Router,
    library: Arc<FakeLibrary>,
    collection: Arc<FakeCollection>,
    password: String,
    device: String,
}

async fn hub() -> Result<Hub> {
    let db = Db::memory()?;
    db.migrate(&db::CORE)?;
    let (device, password) = devices::create(&db, "Feishin", Kind::Subsonic).await?;
    let library = Arc::new(FakeLibrary {
        tracks: vec![track("Paper Moon Static", 1), track("Rock & Roll", 2)],
        last_range: Mutex::new(None),
    });
    let collection = Arc::new(FakeCollection::default());
    let svc = Arc::new(Service {
        db,
        library: library.clone(),
        collection: collection.clone(),
        lyrics: Arc::new(FakeLyrics),
    });
    Ok(Hub {
        app: router(svc),
        library,
        collection,
        password: password.expose().to_owned(),
        device: device.id,
    })
}

impl Hub {
    /// Signs in the way most clients do: a fresh salt and `md5(password + salt)`.
    fn auth(&self) -> String {
        let salt = "c19b2d";
        format!(
            "u=lull&t={}&s={salt}&v=1.16.1&c=test",
            auth::token_of(&self.password, salt)
        )
    }

    async fn get(
        &self,
        path_and_query: &str,
        headers: &[(header::HeaderName, &str)],
    ) -> (StatusCode, HeaderMap, String) {
        let mut req = Request::get(path_and_query);
        for (name, value) in headers {
            req = req.header(name, *value);
        }
        let Ok(req) = req.body(Body::empty()) else {
            return (StatusCode::BAD_REQUEST, HeaderMap::new(), String::new());
        };
        let Ok(res) = self.app.clone().oneshot(req).await;
        let (parts, body) = res.into_parts();
        let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap_or_default();
        (
            parts.status,
            parts.headers,
            String::from_utf8_lossy(&bytes).into_owned(),
        )
    }

    async fn json(&self, path_and_query: &str) -> Value {
        let (_, _, body) = self.get(path_and_query, &[]).await;
        serde_json::from_str::<Value>(&body).map_or(Value::Null, |v| v["subsonic-response"].clone())
    }
}

#[tokio::test]
async fn token_and_salt_auth_accepts_only_a_paired_app_password() -> Result<()> {
    let hub = hub().await?;
    let ok = hub.json(&format!("/rest/ping?{}&f=json", hub.auth())).await;
    assert_eq!(ok["status"], "ok");

    let wrong_salt = hub.auth().replace("s=c19b2d", "s=other");
    assert_eq!(
        hub.json(&format!("/rest/ping?{wrong_salt}&f=json")).await["error"]["code"],
        40
    );
    let wrong_user = hub.auth().replace("u=lull", "u=admin");
    assert_eq!(
        hub.json(&format!("/rest/ping?{wrong_user}&f=json")).await["error"]["code"],
        40
    );

    let hex: String = hub.password.bytes().map(|b| format!("{b:02x}")).collect();
    assert_eq!(
        hub.json(&format!("/rest/ping?u=lull&p=enc:{hex}&f=json")).await["status"],
        "ok"
    );

    let key = &hub.password;
    assert_eq!(
        hub.json(&format!("/rest/ping?apiKey={key}&f=json")).await["status"],
        "ok"
    );
    assert_eq!(
        hub.json(&format!("/rest/ping?apiKey={key}&u=lull&f=json")).await["error"]["code"],
        43
    );
    assert_eq!(
        hub.json("/rest/ping?apiKey=not-a-key&f=json").await["error"]["code"],
        44
    );
    assert_eq!(hub.json("/rest/ping?f=json").await["error"]["code"], 10);
    Ok(())
}

/// Feishin signs in with `getUser` for the name it was given and refuses the server on any error.
#[tokio::test]
async fn get_user_answers_for_the_one_user_clients_sign_in_as() -> Result<()> {
    let hub = hub().await?;
    let me = hub
        .json(&format!("/rest/getUser?{}&username=lull&f=json", hub.auth()))
        .await;
    assert_eq!(me["user"]["username"], "lull");
    assert_eq!(me["user"]["streamRole"], true);
    assert_eq!(me["user"]["folder"], json!([1]));
    let other = hub
        .json(&format!("/rest/getUser?{}&username=admin&f=json", hub.auth()))
        .await;
    assert_eq!(other["error"]["code"], 70);
    Ok(())
}

#[tokio::test]
async fn envelopes_carry_the_same_body_in_json_and_xml() -> Result<()> {
    let hub = hub().await?;
    let album = AlbumId::of(ARTIST, ALBUM);

    let json = hub
        .json(&format!("/rest/getAlbum?{}&id={album}&f=json", hub.auth()))
        .await;
    assert_eq!(json["status"], "ok");
    assert_eq!(json["version"], response::VERSION);
    assert_eq!(json["openSubsonic"], true);
    let song = &json["album"]["song"][1];
    assert_eq!(song["title"], "Rock & Roll");
    assert_eq!(song["coverArt"], album.as_str());
    assert!(song.get("year").is_none(), "unknown facts are left out, not null");

    let (status, headers, xml) = hub
        .get(&format!("/rest/getAlbum.view?{}&id={album}", hub.auth()), &[])
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        headers[header::CONTENT_TYPE]
            .to_str()
            .is_ok_and(|t| t.starts_with("application/xml"))
    );
    assert!(xml.contains(r#"status="ok""#) && xml.contains(r#"xmlns="http://subsonic.org/restapi""#));
    assert!(xml.contains("<album ") && xml.contains(&format!(r#" artist="{ARTIST}""#)));
    assert_eq!(xml.matches("<song ").count(), 2);
    assert!(xml.contains(r#"title="Rock &amp; Roll""#));
    assert!(xml.contains(r#"<genres name="dream pop"/>"#));

    let (_, _, failed) = hub.get(&format!("/rest/getAlbum?{}", hub.auth()), &[]).await;
    assert!(failed.contains(r#"status="failed""#) && failed.contains(r#"<error code="10""#));
    Ok(())
}

#[tokio::test]
async fn stream_hands_the_range_to_the_library_and_its_206_back() -> Result<()> {
    let hub = hub().await?;
    let id = TrackId::of(ARTIST, ALBUM, "Paper Moon Static");
    let uri = format!("/rest/stream.view?{}&id={id}", hub.auth());

    let (status, headers, body) = hub.get(&uri, &[(header::RANGE, "bytes=0-3")]).await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(headers[header::CONTENT_RANGE], "bytes 0-3/10");
    assert_eq!(body, "0123");
    assert_eq!(
        *hub.library.last_range.lock().map_err(poisoned)?,
        Some(HeaderValue::from_static("bytes=0-3"))
    );

    let (status, _, body) = hub.get(&uri, &[]).await;
    assert_eq!((status, body.as_str()), (StatusCode::OK, "0123456789"));
    Ok(())
}

#[tokio::test]
async fn playlist_indexes_count_only_the_saved_songs_a_client_sees() -> Result<()> {
    let hub = hub().await?;
    let saved = |title| TrackId::of(ARTIST, ALBUM, title);
    *hub.collection.playlist.lock().map_err(poisoned)? = Some(Playlist {
        id: "pl-1".into(),
        name: "Porch".into(),
        description: String::new(),
        track_ids: vec![
            saved("Paper Moon Static"),
            TrackId::of("Someone", "Elsewhere", "Not Saved"),
            saved("Rock & Roll"),
        ],
        created_at: 0,
        updated_at: 0,
    });

    let shown = hub
        .json(&format!("/rest/getPlaylist?{}&id=pl-1&f=json", hub.auth()))
        .await;
    assert_eq!(shown["playlist"]["songCount"], 2);
    assert_eq!(shown["playlist"]["entry"][1]["title"], "Rock & Roll");

    let removed = hub
        .json(&format!(
            "/rest/updatePlaylist?{}&playlistId=pl-1&songIndexToRemove=1&f=json",
            hub.auth()
        ))
        .await;
    assert_eq!(removed["status"], "ok");
    let edits = hub.collection.edits.lock().map_err(poisoned)?;
    assert_eq!(edits.first().map(|e| e.remove_positions.clone()), Some(vec![2]));
    Ok(())
}

#[tokio::test]
async fn a_playlist_cover_is_its_own_else_its_first_saved_songs_album() -> Result<()> {
    let hub = hub().await?;
    *hub.collection.playlist.lock().map_err(poisoned)? = Some(Playlist {
        id: "pl-1".into(),
        name: "Porch".into(),
        description: String::new(),
        track_ids: vec![
            TrackId::of("Someone", "Elsewhere", "Not Saved"),
            TrackId::of(ARTIST, ALBUM, "Paper Moon Static"),
        ],
        created_at: 0,
        updated_at: 0,
    });
    let shown = hub
        .json(&format!("/rest/getPlaylist?{}&id=pl-1&f=json", hub.auth()))
        .await;
    assert_eq!(shown["playlist"]["coverArt"], "pl-1");

    let cover = format!("/rest/getCoverArt?{}&id=pl-1", hub.auth());
    let (status, _, body) = hub.get(&cover, &[]).await;
    assert_eq!(
        (status, body),
        (StatusCode::OK, format!("cover of {}", AlbumId::of(ARTIST, ALBUM)))
    );
    *hub.collection.cover.lock().map_err(poisoned)? = Some("own cover".into());
    let (_, _, body) = hub.get(&cover, &[]).await;
    assert_eq!(body, "own cover");
    Ok(())
}

#[tokio::test]
async fn a_scrobble_submission_is_a_completed_play_from_its_time() -> Result<()> {
    let hub = hub().await?;
    let id = TrackId::of(ARTIST, ALBUM, "Paper Moon Static");
    let now_playing = hub
        .json(&format!(
            "/rest/scrobble?{}&id={id}&submission=false&f=json",
            hub.auth()
        ))
        .await;
    assert_eq!(now_playing["status"], "ok");
    assert!(hub.collection.plays.lock().map_err(poisoned)?.is_empty());

    let done = hub
        .json(&format!(
            "/rest/scrobble?{}&id={id}&time=1793534400000&f=json",
            hub.auth()
        ))
        .await;
    assert_eq!(done["status"], "ok");
    assert_eq!(
        hub.collection.plays.lock().map_err(poisoned)?.as_slice(),
        [PlayReport {
            device: hub.device.clone(),
            track_id: id,
            started_at: 1_793_534_400,
            played_s: 200,
            completed: true,
        }]
    );
    Ok(())
}

#[tokio::test]
async fn synced_lyrics_are_timed_lines_in_milliseconds() -> Result<()> {
    let hub = hub().await?;
    let id = TrackId::of(ARTIST, ALBUM, "Paper Moon Static");

    let json = hub
        .json(&format!("/rest/getLyricsBySongId?{}&id={id}&f=json", hub.auth()))
        .await;
    let lyrics = &json["lyricsList"]["structuredLyrics"][0];
    assert_eq!(lyrics["synced"], true);
    assert_eq!(lyrics["displayTitle"], "Paper Moon Static");
    assert_eq!(lyrics["line"][0]["start"], 12_400);
    assert_eq!(lyrics["line"][1]["value"], "Static & hum");

    let (_, _, xml) = hub
        .get(&format!("/rest/getLyricsBySongId?{}&id={id}", hub.auth()), &[])
        .await;
    assert!(xml.contains(r#"<line start="15000">Static &amp; hum</line>"#));
    Ok(())
}
