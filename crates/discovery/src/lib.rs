//! The home feed (new releases, picks with reasons, radio starters), radio, album / artist / tag
//! pages, catalog search with its pictures, and the recency-weighted taste model
//! (docs/context/discovery.md, docs/context/taste.md).

mod art;
mod artist;
mod feed;
mod mix;
mod namesakes;
mod open;
mod pages;
mod picking;
mod radio;
mod search;
mod share;
mod share_host;
mod sources;
mod store;
mod taste;

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use lull_core::db::Migrations;
use lull_core::events::Event;
use lull_core::ids::{ArtistId, TrackId};
use lull_core::model::Entities;
use lull_core::seams::{Library, Transfers, Youtube};
use lull_core::{Ctx, Error, Result};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use sources::cache::Cache;
use sources::deezer::Deezer;
use sources::itunes::Itunes;
use sources::lastfm::LastFm;
use sources::listenbrainz::ListenBrainz;
use sources::musicbrainz::MusicBrainz;
use sources::wikidata::Wikidata;
use store::Store;
use taste::Taste;

pub const MIGRATIONS: Migrations = Migrations {
    owner: "discovery",
    steps: &[
        r#"
        -- Answers from outside sources, kept for a source-specific time (sources/cache.rs).
        CREATE TABLE discovery_cache (
            key         TEXT PRIMARY KEY,
            json        TEXT NOT NULL,
            fetched_at  INTEGER NOT NULL
        );
        -- Albums and artists discovery has shown, so their hashed ids resolve later.
        CREATE TABLE discovery_albums (
            id          TEXT PRIMARY KEY,
            json        TEXT NOT NULL,
            updated_at  INTEGER NOT NULL
        );
        CREATE TABLE discovery_artists (
            id          TEXT PRIMARY KEY,
            json        TEXT NOT NULL,
            updated_at  INTEGER NOT NULL
        );
        -- Taste signals discovery owns (plays, likes and follows are core tables).
        CREATE TABLE discovery_feedback (
            id        INTEGER PRIMARY KEY,
            positive  INTEGER NOT NULL,  -- 1 more like this, 0 less like this
            track_id  TEXT,              -- NULL when the feedback named an artist
            artist    TEXT NOT NULL,
            at        INTEGER NOT NULL
        );
        CREATE TABLE discovery_hidden (
            artist_id  TEXT PRIMARY KEY,
            artist     TEXT NOT NULL,
            at         INTEGER NOT NULL
        );
        CREATE TABLE discovery_saves (
            track_id  TEXT PRIMARY KEY,
            artist    TEXT NOT NULL,
            at        INTEGER NOT NULL
        );
        CREATE TABLE discovery_history (  -- the listener's Last.fm top artists
            artist  TEXT PRIMARY KEY,
            plays   INTEGER NOT NULL
        );
        -- taste_reset_at, feed (the last built feed, served after a restart).
        CREATE TABLE discovery_state (
            key    TEXT PRIMARY KEY,
            value  TEXT NOT NULL
        );
    "#,
        r#"
        -- The catalog album (iTunes, Deezer) behind an album id, so its page asks that catalog
        -- for the tracklist (sources::AlbumSource as JSON).
        CREATE TABLE discovery_album_sources (
            album_id    TEXT PRIMARY KEY,
            json        TEXT NOT NULL,
            updated_at  INTEGER NOT NULL
        );
    "#,
        r#"
        -- An artist's photo once vetted (artist/photo.rs): a picture URL with `{size}`, or NULL
        -- when none is surely a photo of this artist. Wins over the catalog's `image`.
        CREATE TABLE discovery_artist_photos (
            artist_id   TEXT PRIMARY KEY,
            image       TEXT,
            checked_at  INTEGER NOT NULL
        );
    "#,
        r#"
        -- Who an artist is (artist/identity.rs): the exact ids lull holds for it, one per
        -- source (itunes, deezer, musicbrainz), each with the evidence it came from
        -- (artist::identity::Evidence as JSON). The only place an artist's ids are kept:
        -- discovery_artists' JSON no longer holds an MBID.
        CREATE TABLE discovery_artist_ids (
            artist_id  TEXT NOT NULL,
            source     TEXT NOT NULL,
            id         TEXT NOT NULL,
            evidence   TEXT NOT NULL,
            PRIMARY KEY (artist_id, source)
        );
        -- When the listener's own evidence for an artist was last weighed.
        CREATE TABLE discovery_artist_checked (
            artist_id   TEXT PRIMARY KEY,
            checked_at  INTEGER NOT NULL
        );
        -- Every artist is identified again (artist::identity::recheck): until then none has an
        -- MBID or artist tags, which were taken from names.
        UPDATE discovery_artists SET json = json_set(json, '$.musicbrainzId', NULL, '$.tags', json('[]'));
        -- Photos were vetted with those MBIDs (Wikidata's photo of a namesake): vetted again.
        DELETE FROM discovery_artist_photos;
    "#,
        r#"
        -- What suggested each pick (picking.rs): its seed artist or tag, so how the pick fared
        -- (heard, heard again on a later day, liked, saved, skipped) counts for or against that
        -- seed (store::seed_outcomes).
        CREATE TABLE discovery_pick_seeds (
            track_id  TEXT PRIMARY KEY,
            kind      TEXT NOT NULL,
            seed      TEXT NOT NULL,
            shown_at  INTEGER NOT NULL
        );
    "#,
    ],
};

/// Radio's steer until the listener's settings arrive.
const DEFAULT_STEER: f64 = 0.4;

pub struct Service {
    ctx: Ctx,
    store: Store,
    lb: ListenBrainz,
    /// Shared, so a background look-up can outlive the request that started it.
    mb: Arc<MusicBrainz>,
    deezer: Deezer,
    itunes: Itunes,
    /// Artist photos on Wikimedia Commons.
    wikidata: Wikidata,
    /// YouTube Music's search: what the catalog lacks.
    youtube_music: search::youtube_music::YoutubeMusic,
    /// Deezer, or iTunes plus Deezer's artists where Deezer is closed.
    catalog: search::catalog::Catalog,
    searches: search::Searches,
    /// Artist photos waiting to be vetted.
    photos: artist::photo::Queue,
    /// Only with Last.fm API credentials configured; read through `lastfm()`.
    lastfm: Option<LastFm>,
    feed: feed::Feeds,
    /// Answers kept in `discovery_cache`, for share links (the sources hold their own).
    cache: Cache,
    shares: share::Shares,
    /// Soulseek, for new releases peers share.
    transfers: Arc<dyn Transfers>,
    /// yt-dlp, for YouTube's playlists in search.
    youtube: Arc<dyn Youtube>,
    /// Bare songs the UI showed, waiting for a catalog lookup.
    resolver: search::resolve::Queue,
    /// Settings > Discovery "adventurousness": radio's steer when the UI sends none.
    adventurousness: RwLock<f64>,
    /// How page sections that failed went, so a page read again says so (`pages::Unfinished`).
    unfinished: pages::Unfinished,
}

pub async fn start(
    ctx: &Ctx,
    library: Arc<dyn Library>,
    transfers: Arc<dyn Transfers>,
    youtube: Arc<dyn Youtube>,
) -> Result<Arc<Service>> {
    let cache = Cache::new(ctx.db.clone());
    cache.prune().await?;
    let svc = Arc::new(Service {
        store: Store::new(
            ctx.db.clone(),
            library,
            ctx.config.flags.on(&lull_core::flags::NAMESAKES),
        ),
        lb: ListenBrainz::new(
            ctx.http.clone(),
            cache.clone(),
            ctx.config.discovery.listenbrainz_token.clone(),
        ),
        mb: Arc::new(MusicBrainz::new(ctx.http.clone(), cache.clone())),
        deezer: Deezer::new(ctx.http.clone(), cache.clone()),
        itunes: Itunes::new(
            ctx.http.clone(),
            cache.clone(),
            ctx.config.discovery.catalog_country.clone(),
        ),
        wikidata: Wikidata::new(ctx.http.clone(), cache.clone()),
        youtube_music: search::youtube_music::YoutubeMusic::new(ctx.http.clone(), cache.clone()),
        catalog: search::catalog::Catalog::default(),
        searches: search::Searches::default(),
        photos: artist::photo::Queue::default(),
        lastfm: ctx
            .config
            .lastfm
            .as_ref()
            .map(|l| LastFm::new(ctx.http.clone(), cache.clone(), l.api_key.clone())),
        feed: feed::Feeds::default(),
        cache,
        shares: share::Shares::default(),
        transfers,
        youtube,
        resolver: search::resolve::Queue::default(),
        adventurousness: RwLock::new(DEFAULT_STEER),
        unfinished: pages::Unfinished::default(),
        ctx: ctx.clone(),
    });
    ctx.pictures.set_finder(Arc::new(art::Covers(Arc::downgrade(&svc))));
    ctx.spawn(listen(svc.clone(), ctx.bus.subscribe()));
    ctx.spawn(artist::photo::work(svc.clone()));
    ctx.spawn(artist::photo::follow_screens(svc.clone()));
    ctx.spawn(artist::identity::recheck(svc.clone()));
    if namesakes::on(&svc) {
        ctx.spawn(namesakes::anchor_known(svc.clone()));
    }
    ctx.spawn(feed::keep_fresh(svc.clone()));
    ctx.spawn(search::catalog::keep_choosing(svc.clone()));
    ctx.spawn(search::resolve::work(svc.clone()));
    Ok(svc)
}

/// `/api/search`, `/api/catalog/art/{id}`, `/api/feed`, `/api/radio`,
/// `/api/albums|artists|tags/{id}`, `/api/feedback`, `/api/taste`.
pub fn router(svc: Arc<Service>) -> Router {
    Router::new()
        .route("/api/search", get(search::search))
        .route("/api/catalog/art/{id}", get(art::get))
        .route("/api/catalog/resolve", axum::routing::post(search::resolve::post))
        .route("/api/feed", get(feed::get))
        .route("/api/feed/refresh", post(feed::refresh))
        .route("/api/share/{kind}/{id}", get(share::get))
        .route("/api/open", get(open::open))
        .route("/api/radio", get(radio_tracks))
        .route("/api/albums/{id}", get(pages::album))
        .route("/api/artists/named", get(namesakes::named))
        .route("/api/artists/named/split", post(namesakes::split))
        .route("/api/artists/named/choose", post(namesakes::choose))
        .route("/api/artists/place", post(namesakes::place))
        .route("/api/artists/{id}", get(artist::page))
        .route("/api/tags/{id}", get(pages::tag))
        .route("/api/feedback", post(feedback))
        .route("/api/taste", get(taste_summary))
        .route("/api/taste/reset", post(reset))
        .with_state(svc)
}

impl Service {
    /// Last.fm reads, when configured and switched on.
    fn lastfm(&self) -> Option<&LastFm> {
        self.lastfm.as_ref().filter(|l| l.switch.is_on())
    }

    async fn taste(&self) -> Result<Taste> {
        let mut signals = self.ctx.db.call(|c| store::signals(c, lull_core::now())).await?;
        // A library that can't be read now (Jellyfin away) only leaves its seeds out this once.
        signals.library = match self.store.library_albums().await {
            Ok(library) => library,
            Err(e) => {
                tracing::info!(error = %e, "taste without the library this time");
                Vec::new()
            }
        };
        // ADR-0035: each signal counts for the artist it is placed on, and for nobody when its name is
        // shared and nothing says which.
        if namesakes::on(self) {
            let read = signals.clone();
            let (placed, hidden) = self.ctx.db.call(move |c| namesakes::placed(c, &read)).await?;
            signals.namesakes = Some(placed);
            signals.hidden = hidden;
        }
        Ok(Taste::build(&signals, lull_core::now()))
    }
}

/// Learns from saves (`save` done) and the Last.fm history (`lastfm.history`); follows the
/// listener's discovery settings (`settings`); fetches the covers of what devices play and play
/// next ahead (`device.state`).
async fn listen(svc: Arc<Service>, mut rx: broadcast::Receiver<Event>) {
    loop {
        let event = match rx.recv().await {
            Ok(e) => e,
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => return,
        };
        let outcome = match event.topic.as_str() {
            "save" => saved(&svc, event.data).await,
            "lastfm.history" => history(&svc, event.data).await,
            "device.state" => art::playing(&svc, event.data).await,
            "settings" => settings(&svc, event.data),
            _ => Ok(()),
        };
        if let Err(e) = outcome {
            tracing::warn!(error = %e, topic = %event.topic, "discovery could not learn from an event");
        }
    }
}

async fn saved(svc: &Service, data: serde_json::Value) -> Result<()> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Save {
        track_id: TrackId,
        step: u8,
        status: String,
    }
    let save: Save = serde_json::from_value(data).map_err(|e| Error::internal(format!("save payload: {e}")))?;
    // Step 5 is the rescan: `done`, or `pending` when the file is written but Jellyfin lagged.
    if save.step != 5 || !matches!(save.status.as_str(), "done" | "pending") {
        return Ok(());
    }
    let Some(track) = svc.store.track(save.track_id).await? else {
        return Ok(());
    };
    svc.ctx
        .db
        .call(move |c| {
            c.execute(
                "INSERT OR REPLACE INTO discovery_saves (track_id, artist, at) VALUES (?1, ?2, ?3)",
                params![track.id.as_str(), track.artist, lull_core::now()],
            )?;
            Ok(())
        })
        .await
}

async fn history(svc: &Service, data: serde_json::Value) -> Result<()> {
    #[derive(Deserialize)]
    struct History {
        artists: Vec<Played>,
    }
    #[derive(Deserialize)]
    struct Played {
        name: String,
        plays: u64,
    }
    let h: History = serde_json::from_value(data).map_err(|e| Error::internal(format!("history payload: {e}")))?;
    let artists: BTreeMap<String, u64> = h.artists.into_iter().map(|a| (a.name, a.plays)).collect();
    svc.ctx.db.call(move |c| store::put_history(c, &artists)).await
}

/// `settings.discovery` from collection: which sources to use, and how adventurous radio is.
fn settings(svc: &Arc<Service>, data: serde_json::Value) -> Result<()> {
    #[derive(Deserialize)]
    struct Settings {
        discovery: Discovery,
    }
    #[derive(Deserialize)]
    struct Discovery {
        adventurousness: f64,
        on: On,
    }
    #[derive(Deserialize)]
    struct On {
        listenbrainz: bool,
        musicbrainz: bool,
        lastfm: bool,
    }
    let s: Settings = serde_json::from_value(data).map_err(|e| Error::internal(format!("settings payload: {e}")))?;
    if let Ok(mut a) = svc.adventurousness.write() {
        *a = s.discovery.adventurousness.clamp(0.0, 1.0);
    }
    let mut changed = svc.lb.switch.set(s.discovery.on.listenbrainz);
    changed |= svc.mb.switch.set(s.discovery.on.musicbrainz);
    if let Some(lastfm) = &svc.lastfm {
        changed |= lastfm.switch.set(s.discovery.on.lastfm);
    }
    if changed {
        feed::refresh_soon(svc);
    }
    Ok(())
}

#[derive(Deserialize)]
struct RadioQuery {
    kind: radio::SeedKind,
    id: String,
    steer: Option<f64>,
    /// Comma-separated track ids already queued.
    exclude: Option<String>,
}

#[derive(Serialize)]
struct Items {
    items: Vec<TrackId>,
    entities: Entities,
}

async fn radio_tracks(State(svc): State<Arc<Service>>, Query(q): Query<RadioQuery>) -> Result<Json<Items>> {
    let steer = match q.steer {
        Some(s) => s,
        None => svc.adventurousness.read().map(|a| *a).unwrap_or(DEFAULT_STEER),
    };
    if !(0.0..=1.0).contains(&steer) {
        return Err(Error::BadRequest(
            "steer goes from 0 (familiar) to 1 (adventurous)".into(),
        ));
    }
    let exclude = q
        .exclude
        .unwrap_or_default()
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|s| TrackId(s.to_owned()))
        .collect();
    let (items, entities) = radio::next(&svc, radio::Seed { kind: q.kind, id: q.id }, steer, exclude).await?;
    Ok(Json(Items { items, entities }))
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
enum FeedbackKind {
    MoreLike,
    LessLike,
    HideArtist,
}

#[derive(Deserialize)]
struct FeedbackBody {
    kind: FeedbackKind,
    /// A track or an artist id.
    id: String,
}

/// `POST /api/feedback { kind: more-like|less-like|hide-artist, id }`.
async fn feedback(State(svc): State<Arc<Service>>, Json(body): Json<FeedbackBody>) -> Result<StatusCode> {
    let (track, artist) = if body.id.starts_with(TrackId::PREFIX) {
        let t = svc.store.track(TrackId(body.id.clone())).await?;
        let t = t.ok_or_else(|| Error::NotFound(format!("Track {} is unknown to lull.", body.id)))?;
        (Some(t.id), t.artist)
    } else if body.id.starts_with(ArtistId::PREFIX) {
        let a = svc.store.artist_ref(ArtistId(body.id.clone())).await?;
        let a = a.ok_or_else(|| Error::NotFound(format!("Artist {} is unknown to lull.", body.id)))?;
        (None, a.name)
    } else {
        return Err(Error::BadRequest("feedback needs a track or artist id".into()));
    };
    let now = lull_core::now();
    svc.ctx
        .db
        .call(move |c| {
            match body.kind {
                FeedbackKind::HideArtist => c.execute(
                    "INSERT OR REPLACE INTO discovery_hidden (artist_id, artist, at) VALUES (?1, ?2, ?3)",
                    params![ArtistId::of(&artist).as_str(), artist, now],
                )?,
                kind => c.execute(
                    "INSERT INTO discovery_feedback (positive, track_id, artist, at) VALUES (?1, ?2, ?3, ?4)",
                    params![
                        matches!(kind, FeedbackKind::MoreLike),
                        track.as_ref().map(TrackId::as_str),
                        artist,
                        now
                    ],
                )?,
            };
            Ok(())
        })
        .await?;
    feed::refresh_soon(&svc);
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/taste/reset`: forget learned signals (plays, likes, saves and feedback before now).
/// Follows and hidden artists stay: they are standing choices, undone where they were made.
async fn reset(State(svc): State<Arc<Service>>) -> Result<StatusCode> {
    svc.ctx
        .db
        .call(|c| {
            let tx = c.transaction()?;
            tx.execute("DELETE FROM discovery_feedback", [])?;
            tx.execute("DELETE FROM discovery_saves", [])?;
            tx.execute(
                "INSERT INTO discovery_state (key, value) VALUES ('taste_reset_at', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [lull_core::now().to_string()],
            )?;
            tx.commit()?;
            Ok(())
        })
        .await?;
    feed::refresh_soon(&svc);
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
struct TasteSummary {
    artists: Vec<(ArtistId, f64)>,
    tags: Vec<(String, f64)>,
    entities: Entities,
}

/// `GET /api/taste`: the Stats taste block, strongest first, scaled to 0..1.
async fn taste_summary(State(svc): State<Arc<Service>>) -> Result<Json<TasteSummary>> {
    let taste = svc.taste().await?;
    let (artists, tags) = taste.summary(12);
    let named = taste
        .top_artists(12)
        .map(|a| {
            store::artist(sources::ArtistInfo {
                mbid: None,
                name: a.name.clone(),
            })
        })
        .collect();
    let named = svc.store.artists(named).await?;
    Ok(Json(TasteSummary {
        artists,
        tags,
        entities: store::entities(Vec::new(), Vec::new(), named),
    }))
}
