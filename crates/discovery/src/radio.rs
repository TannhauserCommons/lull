//! `GET /api/radio`: the next tracks for a radio seeded by a track, an artist or a tag, steered
//! from familiar (0) to adventurous (1); the taste model orders them.
//!
//! - **ListenBrainz first** (5 s at most): artist radio for similar artists, tag radio within a
//!   popularity window. Who the artist is comes from its exact ids (`artist::identity`), never
//!   its name; a credit ("A feat. B & C") is tried whole, then artist by artist. An artist radio
//!   needs its MBID; with only a Deezer id, Deezer's radio of that id stands in. Without either,
//!   or when ListenBrainz has no artist radio for it, the seed's own tags lead into tag radio
//!   (a track's, or those of the listener's songs by the artist).
//! - **Then Last.fm** when ListenBrainz gave nothing (down, slow or empty): similar tracks, the
//!   top tracks of the similar artists of an artist with an MBID, the similar tracks of the
//!   listener's songs by one without, or the tag's top tracks.
//! - **Then the artist's own songs** in the catalogs, by its exact ids there (Apple's and
//!   Deezer's): a small artist no recommender knows yet still has a radio.
//! - **Then the saved library:** songs matching the artist or tag.
//! - **Last, the listener's own strongest tags:** tag radio, so a radio practically never ends.
//! - A source counts only for the songs it gives that the radio can still play (not queued, not
//!   by a hidden or held-back artist): one whose songs are all queued already leaves the next.
//! - **About 8 s at most** in all. The answer is an error only when no source answered: 503 with
//!   the reason, or 404 when they answered with nothing.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use lull_core::ids::{ArtistId, TrackId};
use lull_core::model::{Entities, Track};
use lull_core::{Error, Result};
use serde::Deserialize;
use tokio::time::Instant;

use crate::artist::discography;
use crate::artist::identity::{self, IdRow, Ids, Source};
use crate::artist::popular::song_key;
use crate::search::catalog::Choice;
use crate::search::merge::SongHit;
use crate::sources::lastfm::LastFm;
use crate::sources::listenbrainz::{Mode, Popularity};
use crate::sources::{ArtistSource, Song};
use crate::store::entities;
use crate::taste::Taste;
use crate::{Service, mix, taste};

/// Tracks returned per call; the UI asks again as the queue drains.
const BATCH: usize = 12;
/// Places a remix or live take behind every original in a batch.
const VERSION_PENALTY: f64 = 10_000.0;
/// Candidates looked at per call (one metadata request).
const POOL: usize = 50;
/// Per artist in one batch, except the seed artist of an artist radio.
const PER_ARTIST: usize = 2;
/// How long ListenBrainz gets, and the whole call.
const BUDGET: Budget = Budget {
    listenbrainz: Duration::from_secs(5),
    total: Duration::from_secs(8),
};
/// Similar artists whose top tracks fill a radio without ListenBrainz.
const LASTFM_SIMILAR: usize = 3;
/// The listener's strongest tags a radio falls back on, when nothing knows its seed.
const TASTE_TAGS: usize = 2;
/// The steps' names, as `gather` reports which one gave the songs.
const OWN_SONGS: &str = "the artist's own songs";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SeedKind {
    Track,
    Artist,
    Tag,
}

pub struct Seed {
    pub kind: SeedKind,
    pub id: String,
}

/// What a seed id stands for.
struct Known {
    kind: SeedKind,
    /// The artist credit (track and artist seeds) or the tag.
    name: String,
    /// The track's title (track seeds).
    title: Option<String>,
    tags: Vec<String>,
    /// The artist whose radio this is: more than `PER_ARTIST` of theirs are fine.
    artist: Option<ArtistId>,
    /// The credited artist's exact ids (track and artist seeds; `artist::identity`).
    ids: Ids,
    /// The name of the artist those ids are (a credit's first artist lull holds ids for).
    anchor: Option<String>,
    /// The titles of the listener's songs by an artist without an MBID (artist seeds): Last.fm's
    /// radio starts from them, as no id of its names this artist there.
    songs: Vec<String>,
}

pub async fn next(
    svc: &Service,
    seed: Seed,
    steer: f64,
    exclude: HashSet<TrackId>,
) -> Result<(Vec<TrackId>, Entities)> {
    let taste = svc.taste().await?;
    let started = Instant::now();
    let seed = known(svc, seed).await?;
    // Finding who the seed's artist is counts against the radio's time.
    let spent = started.elapsed();
    let budget = Budget {
        listenbrainz: BUDGET.listenbrainz.saturating_sub(spent),
        total: BUDGET.total.saturating_sub(spent),
    };
    let mut steps = vec![Step::within(
        "ListenBrainz",
        budget.listenbrainz,
        from_listenbrainz(svc, &seed, steer),
    )];
    if let Some(lastfm) = svc.lastfm() {
        steps.push(Step::new("Last.fm", from_lastfm(svc, lastfm, &seed, steer)));
    }
    if seed.kind != SeedKind::Tag && (seed.ids.itunes().is_some() || seed.ids.deezer().is_some()) {
        steps.push(Step::new(OWN_SONGS, own_songs(svc, &seed)));
    }
    steps.push(Step::local("the library", async { Ok(from_library(svc, &seed).await) }));
    let tags = taste_tags(&taste);
    if !tags.is_empty() {
        steps.push(Step::new("the listener's tags", from_taste(svc, tags, steer)));
    }
    // Only songs the radio can still play count: what is queued, hidden or held back doesn't.
    let playable =
        |t: &Track| !exclude.contains(&t.id) && !taste.hidden.contains(&t.artist_id) && !taste.held_back(&t.artist_id);
    let gathered = gather(budget.total, playable, steps).await?;
    // The artist's own songs are all one artist's: the cap per artist would starve them.
    let one_artist = gathered.by == OWN_SONGS;

    // Originals first: remixes and live takes only fill up, unless the seed is one.
    let versions_too = seed.title.as_deref().is_some_and(is_version);
    let mut ranked: Vec<(f64, Track)> = gathered
        .tracks
        .into_iter()
        .enumerate()
        .map(|(i, t)| {
            // Shuffled order, nudged towards known artists when familiar and away when adventurous.
            let known = if taste.artist_weight(&t.artist_id) > 0.0 {
                1.0
            } else {
                0.0
            };
            let nudge = (1.0 - 2.0 * steer) * known * 10.0;
            let version = if !versions_too && is_version(&t.title) {
                VERSION_PENALTY
            } else {
                0.0
            };
            (i as f64 - nudge + version, t)
        })
        .collect();
    ranked.sort_by(|a, b| a.0.total_cmp(&b.0));

    let mut per_artist: HashMap<ArtistId, usize> = HashMap::new();
    let picked: Vec<Track> = ranked
        .into_iter()
        .map(|(_, t)| t)
        .filter(|t| {
            let n = per_artist.entry(t.artist_id.clone()).or_default();
            *n += 1;
            *n <= PER_ARTIST || one_artist || seed.artist.as_ref() == Some(&t.artist_id)
        })
        .take(BATCH)
        .collect();
    Ok((
        picked.iter().map(|t| t.id.clone()).collect(),
        entities(picked, Vec::new(), Vec::new()),
    ))
}

async fn known(svc: &Service, seed: Seed) -> Result<Known> {
    match seed.kind {
        SeedKind::Tag => Ok(Known {
            kind: seed.kind,
            name: seed.id.to_lowercase(),
            title: None,
            tags: Vec::new(),
            artist: None,
            ids: Ids::default(),
            anchor: None,
            songs: Vec::new(),
        }),
        SeedKind::Artist => {
            let artist = svc
                .store
                .artist_ref(ArtistId(seed.id.clone()))
                .await?
                .ok_or_else(|| Error::NotFound(format!("Artist {} is unknown to lull.", seed.id)))?;
            // ADR-0035: an artist's own ids, never those of others of its name.
            let (ids, anchor) = match crate::namesakes::on(svc) {
                true => (
                    crate::namesakes::artist_ids(svc, &artist.id).await?,
                    Some(artist.name.clone()),
                ),
                false => credited_ids(svc, &artist.name).await,
            };
            let (songs, mut tags) = match ids.mbid() {
                Some(_) => (Vec::new(), Vec::new()),
                None => listener_songs(svc, &artist.id).await?,
            };
            tags.extend(artist.tags);
            Ok(Known {
                kind: seed.kind,
                name: artist.name,
                title: None,
                tags,
                artist: Some(artist.id),
                ids,
                anchor,
                songs,
            })
        }
        SeedKind::Track => {
            let track = svc
                .store
                .track(TrackId(seed.id.clone()))
                .await?
                .ok_or_else(|| Error::NotFound(format!("Track {} is unknown to lull.", seed.id)))?;
            let placed = match crate::namesakes::on(svc) {
                true => crate::namesakes::song_artist(svc, &track).await?,
                false => None,
            };
            let (ids, anchor) = match placed {
                Some(artist) => {
                    let name = svc.store.artist_ref(artist.clone()).await?.map(|a| a.name);
                    (crate::namesakes::artist_ids(svc, &artist).await?, name)
                }
                None => credited_ids(svc, &track.artist).await,
            };
            Ok(Known {
                kind: seed.kind,
                ids,
                anchor,
                name: track.artist,
                title: Some(track.title),
                tags: track.tags,
                artist: None,
                songs: Vec::new(),
            })
        }
    }
}

/// The exact ids of the credit's first artist lull holds any for, and that artist's name, within
/// ListenBrainz's time. Ids not decided by then are none for this batch (their lookups are cached
/// for the next).
async fn credited_ids(svc: &Service, credit: &str) -> (Ids, Option<String>) {
    let names = credited_names(credit);
    let found = resolve(&names, |name| async move {
        let ids = identity::ids(svc, &ArtistId::of(&name)).await?;
        Ok((!ids.is_empty()).then_some((ids, name)))
    });
    match tokio::time::timeout(BUDGET.listenbrainz, found).await {
        Ok((found, failure)) => {
            if let Some(e) = failure {
                tracing::info!(credit, error = %e, "radio without one artist's ids");
            }
            match found {
                Some((ids, name)) => (ids, Some(name)),
                None => (Ids::default(), None),
            }
        }
        Err(_) => (Ids::default(), None),
    }
}

/// The titles of the listener's songs credited to the artist, and their tags.
async fn listener_songs(svc: &Service, id: &ArtistId) -> Result<(Vec<String>, Vec<String>)> {
    let (mut titles, mut tags) = (Vec::new(), Vec::new());
    let namesakes = crate::namesakes::on(svc);
    // Only songs credited by a name the artist is called by can be its (ADR-0035).
    let names = match namesakes {
        true => {
            let at = id.clone();
            let mut names = svc.ctx.db.call(move |c| lull_core::identity::names_of(c, &at)).await?;
            names.push(id.clone());
            names
        }
        false => vec![id.clone()],
    };
    for (track, artist) in svc.store.listener_songs().await? {
        if !names.contains(&ArtistId::of(&artist)) {
            continue;
        }
        if let Some(t) = svc.store.track(track).await? {
            // ADR-0035: a song counts for the artist it is placed on, never for a namesake.
            if namesakes && crate::namesakes::song_artist(svc, &t).await?.as_ref() != Some(id) {
                continue;
            }
            titles.push(t.title);
            tags.extend(t.tags);
        }
    }
    Ok((titles, tags))
}

/// How long the radio's sources get.
#[derive(Debug, Clone, Copy)]
pub struct Budget {
    pub listenbrainz: Duration,
    pub total: Duration,
}

/// A step of the radio's search: a source and how long it may take.
pub struct Step<'a> {
    pub service: &'static str,
    limit: Limit,
    songs: Pin<Box<dyn Future<Output = Result<Vec<Track>>> + Send + 'a>>,
}

#[derive(Clone, Copy)]
enum Limit {
    /// Its own limit, within the whole call's.
    Within(Duration),
    /// What is left of the whole call.
    Rest,
    /// Lull's own data: never cut short.
    Local,
}

impl<'a> Step<'a> {
    pub fn new(service: &'static str, songs: impl Future<Output = Result<Vec<Track>>> + Send + 'a) -> Self {
        Step {
            service,
            limit: Limit::Rest,
            songs: Box::pin(songs),
        }
    }

    pub fn within(
        service: &'static str,
        limit: Duration,
        songs: impl Future<Output = Result<Vec<Track>>> + Send + 'a,
    ) -> Self {
        Step {
            limit: Limit::Within(limit),
            ..Step::new(service, songs)
        }
    }

    pub fn local(service: &'static str, songs: impl Future<Output = Result<Vec<Track>>> + Send + 'a) -> Self {
        Step {
            limit: Limit::Local,
            ..Step::new(service, songs)
        }
    }
}

/// The songs a step gave, and which step.
#[derive(Debug)]
pub struct Gathered {
    pub tracks: Vec<Track>,
    pub by: &'static str,
}

/// The first step, in order, to give songs `playable` keeps, within `total`. A step whose songs
/// are all queued already (or hidden) gave nothing: the next one is asked. Fails only when none
/// gave any: with a source's error when no outside source answered at all, else "nothing found".
pub async fn gather(total: Duration, playable: impl Fn(&Track) -> bool, steps: Vec<Step<'_>>) -> Result<Gathered> {
    let start = Instant::now();
    let mut failure = None;
    let mut answered = false;
    for step in steps {
        let left = total.saturating_sub(start.elapsed());
        let limit = match step.limit {
            Limit::Within(own) => Some(own.min(left)),
            Limit::Rest => Some(left),
            Limit::Local => None,
        };
        let outcome = match limit {
            Some(limit) => tokio::time::timeout(limit, step.songs).await.map_err(|_| limit),
            None => Ok(step.songs.await),
        };
        match outcome {
            Ok(Ok(mut tracks)) => {
                // Lull's own data having nothing says nothing of the sources.
                answered |= !matches!(step.limit, Limit::Local);
                tracks.retain(|t| playable(t));
                if !tracks.is_empty() {
                    return Ok(Gathered {
                        tracks,
                        by: step.service,
                    });
                }
            }
            Ok(Err(Error::NotFound(_))) => answered |= !matches!(step.limit, Limit::Local),
            Ok(Err(e)) => {
                tracing::info!(error = %e, "radio goes on without {}", step.service);
                failure = Some(e);
            }
            Err(limit) => {
                tracing::info!(
                    "radio goes on without {}: no answer within {} s",
                    step.service,
                    limit.as_secs()
                );
                failure = Some(Error::unavailable(
                    step.service,
                    format!("didn't answer within {} s", limit.as_secs()),
                ));
            }
        }
    }
    match failure {
        Some(e) if !answered => Err(e),
        _ => Err(Error::NotFound(
            "No music source has anything for this radio yet.".into(),
        )),
    }
}

/// ListenBrainz's radio for the seed, as catalog tracks.
async fn from_listenbrainz(svc: &Service, seed: &Known, steer: f64) -> Result<Vec<Track>> {
    if seed.kind != SeedKind::Tag
        && seed.ids.mbid().is_none()
        && let Some(id) = seed.ids.deezer()
    {
        // No MBID for ListenBrainz: Deezer's radio of the artist's own Deezer id.
        let mut songs = svc.deezer.radio(id).await?;
        offer(svc, &songs, "Deezer radio").await;
        mix::shuffle(&mut songs, mix::seed());
        songs.truncate(POOL);
        return svc.store.tracks(songs).await;
    }
    let pop = popularity(steer);
    let mut mbids = match seed.kind {
        SeedKind::Tag => svc.lb.tag_radio(&seed.name, pop, 100).await?,
        SeedKind::Artist => by_artist(svc, seed, steer, pop).await?,
        SeedKind::Track => {
            let mut found = by_artist(svc, seed, steer, pop).await?;
            if let Some(tag) = seed.tags.iter().find(|t| taste::is_specific(t)) {
                // A bonus on top of the artist's radio: its failure doesn't stop the radio.
                match svc.lb.tag_radio(tag, pop, 50).await {
                    Ok(more) => found.extend(more),
                    Err(e) => tracing::info!(tag, error = %e, "track radio without its tag"),
                }
            }
            found
        }
    };
    mix::shuffle(&mut mbids, mix::seed());
    mbids.truncate(POOL);
    let songs = svc.lb.songs(&mbids).await?;
    offer(svc, &songs, "ListenBrainz radio").await;
    svc.store.tracks(songs).await
}

/// The ids a source named its songs' artists by, offered as who those artists are
/// (`identity::offer`): ListenBrainz's MBIDs, Deezer's artist ids. ListenBrainz names a
/// credit's first artist, so its MBID is the credit's only when the credit is one artist.
pub async fn offer(svc: &Service, songs: &[Song], by: &str) {
    let offers = songs
        .iter()
        .filter_map(|s| {
            let alone = credited_names(&s.artist).len() == 1;
            let row = match (&s.artist_mbid, s.artist_source) {
                (Some(mbid), _) if alone => IdRow::recommended(Source::Musicbrainz, mbid, by),
                (Some(_), _) => return None,
                (None, Some(ArtistSource::Deezer(id))) => IdRow::recommended(Source::Deezer, id, by),
                (None, Some(ArtistSource::Itunes(id))) => IdRow::recommended(Source::Itunes, id, by),
                (None, None) => return None,
            };
            Some((s.artist.clone(), row))
        })
        .collect();
    if let Err(e) = identity::offer(svc, offers).await {
        tracing::info!(error = %e, "a source's artist ids not kept");
    }
}

/// Last.fm's radio for the seed: similar tracks, the top tracks of the similar artists of an
/// artist with an MBID (asked by it), the similar tracks of the listener's songs by one without,
/// or the tag's top tracks. A name alone never leads to an artist's similar artists.
async fn from_lastfm(svc: &Service, lastfm: &LastFm, seed: &Known, steer: f64) -> Result<Vec<Track>> {
    let artist = main_and_featured(&seed.name).0;
    let mut songs = match (seed.kind, &seed.title, seed.ids.mbid()) {
        (SeedKind::Tag, _, _) => lastfm.tag_top_tracks(&seed.name).await?,
        (SeedKind::Track, Some(title), mbid) => {
            let similar = lastfm.similar_tracks(artist, title).await?;
            match mbid {
                Some(mbid) if similar.is_empty() => around_artist(lastfm, artist, mbid, steer).await?,
                _ => similar,
            }
        }
        (_, _, Some(mbid)) => around_artist(lastfm, artist, mbid, steer).await?,
        (_, _, None) => {
            let mut similar = Vec::new();
            for title in seed.songs.iter().take(LASTFM_SIMILAR) {
                similar.extend(lastfm.similar_tracks(artist, title).await?);
            }
            similar
        }
    };
    mix::shuffle(&mut songs, mix::seed());
    songs.truncate(POOL);
    svc.store.tracks(songs).await
}

/// The artist's top tracks and those of a few similar artists (by its confirmed MBID): the
/// closest when familiar, further down Last.fm's list when adventurous.
async fn around_artist(lastfm: &LastFm, artist: &str, mbid: &str, steer: f64) -> Result<Vec<Song>> {
    let similar = lastfm.similar_artists(artist, Some(mbid)).await?;
    let skip = (similar.len().saturating_sub(LASTFM_SIMILAR) as f64 * quantized(steer) * 0.5) as usize;
    let mut songs: Vec<Song> = lastfm.artist_top_tracks(artist).await?.into_iter().take(4).collect();
    for other in similar.iter().skip(skip).take(LASTFM_SIMILAR) {
        match lastfm.artist_top_tracks(&other.name).await {
            Ok(top) => songs.extend(top.into_iter().take(5)),
            Err(e) => tracing::info!(artist = %other.name, error = %e, "radio without one similar artist"),
        }
    }
    Ok(songs)
}

/// Saved songs by the artist, or matching the tag: the radio's last resort.
async fn from_library(svc: &Service, seed: &Known) -> Vec<Track> {
    let words = match seed.kind {
        SeedKind::Tag => seed.name.as_str(),
        SeedKind::Artist | SeedKind::Track => main_and_featured(&seed.name).0,
    };
    let mut saved = svc.store.search_saved(words, POOL).await.unwrap_or_else(|e| {
        tracing::info!(error = %e, "radio without the library");
        Vec::new()
    });
    mix::shuffle(&mut saved, mix::seed());
    saved
}

/// The artist's own songs, by its exact ids in the catalogs (Apple's, and Deezer's top where its
/// catalog is open), as its artist page lists them: what keeps the radio of an artist no
/// recommender knows yet going. Never by a name (ADR-0035). Each song once, the seed's own aside.
async fn own_songs(svc: &Service, seed: &Known) -> Result<Vec<Track>> {
    let Some(artist) = seed.anchor.as_deref() else {
        return Ok(Vec::new());
    };
    let open = svc.catalog.choice() == Choice::Deezer;
    let (from_itunes, from_deezer) = tokio::join!(
        async {
            match seed.ids.itunes() {
                Some(id) => svc.itunes.artist_catalog(id).await.map(|c| c.songs),
                None => Ok(Vec::new()),
            }
        },
        async {
            match seed.ids.deezer() {
                Some(id) if open => svc.deezer.artist_top(id).await,
                _ => Ok(Vec::new()),
            }
        },
    );
    let hits = match (from_deezer, from_itunes) {
        (Err(e), Err(_)) => return Err(e),
        (deezer, itunes) => {
            let mut hits = deezer.unwrap_or_default();
            hits.extend(itunes.unwrap_or_default());
            hits
        }
    };
    let mut hits = one_take_each(artist, seed.title.as_deref(), hits);
    mix::shuffle(&mut hits, mix::seed());
    hits.truncate(POOL);
    // Their releases remembered, so their covers show.
    crate::artist::remember(svc, hits.iter().filter_map(SongHit::album_hit).collect()).await?;
    svc.store.tracks(hits.iter().map(SongHit::song).collect()).await
}

/// The catalog's songs credited to `artist`, each song once (its first take in the catalog's
/// order, so every batch picks the same one) and never the seed's own song again.
fn one_take_each(artist: &str, seed_title: Option<&str>, hits: Vec<SongHit>) -> Vec<SongHit> {
    let mut seen: HashSet<String> = seed_title.map(song_key).into_iter().collect();
    hits.into_iter()
        .filter(|s| discography::credits(&s.artist, artist))
        .filter(|s| seen.insert(song_key(&s.title)))
        .collect()
}

/// The listener's strongest specific tags: where a radio nothing else knows anything around goes.
fn taste_tags(taste: &Taste) -> Vec<String> {
    taste
        .top_tags(usize::MAX)
        .map(|(tag, _)| tag)
        .filter(|tag| taste::is_specific(tag))
        .take(TASTE_TAGS)
        .cloned()
        .collect()
}

/// Tag radio of the listener's strongest tags (ListenBrainz, else Last.fm's top tracks of the
/// first): the radio's last resort, so it practically never ends.
async fn from_taste(svc: &Service, tags: Vec<String>, steer: f64) -> Result<Vec<Track>> {
    let mut failure = None;
    for tag in &tags {
        match svc.lb.tag_radio(tag, popularity(steer), 50).await {
            Ok(mut mbids) if !mbids.is_empty() => {
                mix::shuffle(&mut mbids, mix::seed());
                mbids.truncate(POOL);
                let songs = svc.lb.songs(&mbids).await?;
                offer(svc, &songs, "ListenBrainz radio").await;
                return svc.store.tracks(songs).await;
            }
            Ok(_) => {}
            Err(e) => failure = Some(e),
        }
    }
    if let (Some(lastfm), Some(tag)) = (svc.lastfm(), tags.first()) {
        let mut songs = lastfm.tag_top_tracks(tag).await?;
        mix::shuffle(&mut songs, mix::seed());
        songs.truncate(POOL);
        return svc.store.tracks(songs).await;
    }
    Err(failure.unwrap_or_else(|| Error::NotFound("No radio for the listener's tags yet.".into())))
}

/// Recordings for an artist radio on the seed's confirmed artist, else tag radio on its own
/// tags.
async fn by_artist(svc: &Service, seed: &Known, steer: f64, pop: Popularity) -> Result<Vec<String>> {
    let mut failure = None;
    if let Some(mbid) = seed.ids.mbid() {
        match svc.lb.artist_radio(mbid, mode(steer), 15, 4, pop).await {
            Ok(picks) if !picks.is_empty() => return Ok(picks.into_iter().map(|p| p.recording_mbid).collect()),
            Ok(_) => tracing::info!(credit = %seed.name, "ListenBrainz has no artist radio; using the seed's tags"),
            Err(e) => failure = Some(e),
        }
    }
    if let Some(tag) = seed.tags.iter().find(|t| taste::is_specific(t)) {
        match svc.lb.tag_radio(tag, pop, 50).await {
            Ok(found) if !found.is_empty() => return Ok(found),
            Ok(_) => {}
            Err(e) => failure = Some(e),
        }
    }
    Err(failure.unwrap_or_else(|| {
        Error::NotFound(format!(
            "No music source knows {} well enough for a radio yet.",
            seed.name
        ))
    }))
}

/// Whether a title names another take of a song: a remix, live, sped-up, slowed, acoustic or
/// instrumental take, edit, mix, mashup, demo, version or dub in its brackets or " - " suffix
/// ("Talamak (Tom Lown Remix)", "So Many Details - Live", "Drift (sped up)").
/// Search's list of version words is the one list (`merge::names_a_version`).
pub fn is_version(title: &str) -> bool {
    crate::search::merge::names_a_version(title)
}

/// The artists in a credit, best first: the whole credit, the part before "feat.", "ft.",
/// "featuring" or "with", then each artist of it split on "&", "," and "x", then the featured.
pub fn credited_names(credit: &str) -> Vec<String> {
    let (main, featured) = main_and_featured(credit);
    let split = |s: &str| -> Vec<String> {
        s.replace(" & ", ",")
            .replace(" x ", ",")
            .replace(" X ", ",")
            .split(',')
            .map(|p| p.trim().trim_end_matches(')').trim().to_owned())
            .collect()
    };
    let mut names = vec![credit.trim().to_owned(), main.to_owned()];
    names.extend(split(main));
    names.extend(split(featured));
    let mut seen = HashSet::new();
    names.retain(|n| !n.is_empty() && seen.insert(n.to_lowercase()));
    names
}

/// A credit's main artists and its featured ones ("A & B feat. C" → "A & B", "C").
pub(crate) fn main_and_featured(credit: &str) -> (&str, &str) {
    let lower = credit.to_lowercase();
    let markers = [
        " feat. ",
        " feat ",
        " ft. ",
        " ft ",
        " featuring ",
        " with ",
        " (feat. ",
        " (ft. ",
        " (with ",
    ];
    match markers.iter().filter_map(|m| lower.find(m)).min() {
        Some(i) => (
            credit[..i].trim(),
            credit[i..].splitn(3, ' ').nth(2).unwrap_or_default(),
        ),
        None => (credit.trim(), ""),
    }
}

/// The first name's answer among `names`, in order, and the last lookup failure, if any. A name
/// whose identity can't be decided now doesn't stop the others.
pub async fn resolve<T, I, IF>(names: &[String], identify: I) -> (Option<T>, Option<Error>)
where
    I: Fn(String) -> IF,
    IF: Future<Output = Result<Option<T>>>,
{
    let mut failure = None;
    for name in names {
        match identify(name.clone()).await {
            Ok(Some(found)) => return (Some(found), failure),
            Ok(None) => {}
            Err(e) => failure = Some(e),
        }
    }
    (None, failure)
}

/// Familiar radio stays close to the seed; adventurous radio goes further out.
fn mode(steer: f64) -> Mode {
    match quantized(steer) {
        s if s < 0.34 => Mode::Easy,
        s if s < 0.67 => Mode::Medium,
        _ => Mode::Hard,
    }
}

/// Familiar: the well-played 40-100% of recordings; adventurous: the obscure 0-50%.
fn popularity(steer: f64) -> Popularity {
    let steer = quantized(steer);
    let begin = (40.0 * (1.0 - steer)).round() as u8;
    let end = (100.0 - 50.0 * steer).round() as u8;
    (begin, end)
}

/// Steps of a quarter, so nearby steers share cached ListenBrainz answers.
fn quantized(steer: f64) -> f64 {
    (steer.clamp(0.0, 1.0) * 4.0).round() / 4.0
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use lull_core::Error;

    use std::time::Duration;

    use lull_core::ids::{AlbumId, ArtistId, TrackId};
    use lull_core::model::Track;

    use super::{OWN_SONGS, Step, credited_names, gather, is_version, one_take_each, resolve};
    use crate::search::merge::SongHit;

    fn track(title: &str) -> Track {
        Track {
            id: TrackId(format!("t-{title}")),
            title: title.into(),
            artist_id: ArtistId::of("Velvet Orchard"),
            artist: "Velvet Orchard".into(),
            album_id: AlbumId::of("Velvet Orchard", ""),
            album: String::new(),
            year: None,
            no: None,
            disc: None,
            duration: None,
            tags: vec![],
            saved: false,
            file: None,
        }
    }

    const LB: Duration = Duration::from_millis(50);
    const TOTAL: Duration = Duration::from_millis(150);
    fn any(_: &Track) -> bool {
        true
    }
    fn titles(found: &lull_core::Result<super::Gathered>) -> Option<Vec<String>> {
        found
            .as_ref()
            .ok()
            .map(|g| g.tracks.iter().map(|t| t.title.clone()).collect())
    }

    #[tokio::test]
    async fn with_listenbrainz_hanging_last_fm_fills_the_radio_in_time() {
        // Live, tag radio hung 61 s on ListenBrainz and then answered 503.
        let started = std::time::Instant::now();
        let found = gather(
            TOTAL,
            any,
            vec![
                Step::within("ListenBrainz", LB, std::future::pending()),
                Step::new("Last.fm", async { Ok(vec![track("Paper Moon Static")]) }),
            ],
        )
        .await;
        assert_eq!(titles(&found), Some(vec!["Paper Moon Static".into()]));
        assert!(started.elapsed() < Duration::from_millis(500));
    }

    #[tokio::test]
    async fn with_listenbrainz_failing_and_no_last_fm_saved_songs_play() {
        let found = gather(
            TOTAL,
            any,
            vec![
                Step::within("ListenBrainz", LB, async {
                    Err(Error::unavailable("ListenBrainz", "error sending request"))
                }),
                Step::local("the library", async { Ok(vec![track("Satellite Porch")]) }),
            ],
        )
        .await;
        assert_eq!(titles(&found), Some(vec!["Satellite Porch".into()]));
    }

    #[tokio::test]
    async fn it_is_503_only_when_no_source_answered() {
        let down = gather(
            TOTAL,
            any,
            vec![
                Step::within("ListenBrainz", LB, async {
                    Err(Error::unavailable("ListenBrainz", "error sending request"))
                }),
                Step::new("Last.fm", std::future::pending()),
                Step::local("the library", async { Ok(Vec::new()) }),
            ],
        )
        .await;
        assert!(matches!(down, Err(Error::Unavailable { .. })), "{down:?}");
        let empty = gather(
            TOTAL,
            any,
            vec![
                Step::within("ListenBrainz", LB, async {
                    Err(Error::unavailable("ListenBrainz", "error sending request"))
                }),
                Step::new("Last.fm", async { Ok(Vec::new()) }),
                Step::local("the library", async { Ok(Vec::new()) }),
            ],
        )
        .await;
        assert!(
            matches!(empty, Err(Error::NotFound(_))),
            "Last.fm answered, with nothing: {empty:?}"
        );
    }

    #[tokio::test]
    async fn a_source_whose_songs_are_all_queued_leaves_the_next_one() {
        // A radio whose first source gives only the song playing (or songs queued already) asked
        // nobody else: the answer was empty and the radio stalled with nothing after this song.
        let queued = |t: &Track| t.title != "Metaphor";
        let found = gather(
            TOTAL,
            queued,
            vec![
                Step::within("ListenBrainz", LB, async { Ok(vec![track("Metaphor")]) }),
                Step::new("Last.fm", async { Ok(vec![track("Red Hearts")]) }),
            ],
        )
        .await;
        assert_eq!(titles(&found), Some(vec!["Red Hearts".into()]));
    }

    #[tokio::test]
    async fn a_small_artist_no_recommender_knows_plays_its_own_songs() {
        // Live: "Metaphor" by Marlon Funaki, an artist lull knows only by its Apple id. ListenBrainz
        // had nothing to go on (no MBID, no tags), Last.fm no similar tracks, the library nothing:
        // "No music source has anything for this radio yet." Apple lists more of the artist's songs.
        let playing = |t: &Track| t.title != "Metaphor";
        let found = gather(
            TOTAL,
            playing,
            vec![
                Step::within("ListenBrainz", LB, async {
                    Err(Error::NotFound(
                        "No music source knows Marlon Funaki well enough".into(),
                    ))
                }),
                Step::new("Last.fm", async { Ok(Vec::new()) }),
                Step::new(OWN_SONGS, async {
                    Ok(vec![track("Red Hearts"), track("Murphy's Law")])
                }),
                Step::local("the library", async { Ok(Vec::new()) }),
            ],
        )
        .await;
        assert_eq!(found.as_ref().map(|g| g.by).ok(), Some(OWN_SONGS), "{found:?}");
        assert_eq!(titles(&found).map(|t| t.len()), Some(2));
    }

    #[tokio::test]
    async fn with_nothing_around_the_seed_the_listeners_own_tags_carry_the_radio() {
        let found = gather(
            TOTAL,
            any,
            vec![
                Step::within("ListenBrainz", LB, async { Err(Error::NotFound("no tags".into())) }),
                Step::new("Last.fm", async { Ok(Vec::new()) }),
                Step::local("the library", async { Ok(Vec::new()) }),
                Step::new("the listener's tags", async { Ok(vec![track("Lanterns")]) }),
            ],
        )
        .await;
        assert_eq!(titles(&found), Some(vec!["Lanterns".into()]));
    }

    #[test]
    fn an_artists_own_songs_are_its_credits_each_once_and_never_the_seed_again() {
        let hit = |title: &str, artist: &str, album: &str| SongHit {
            title: title.into(),
            artist: artist.into(),
            album: album.into(),
            ..SongHit::default()
        };
        let hits = vec![
            hit("Metaphor", "Marlon Funaki", "Metaphor"),
            hit("Red Hearts", "Marlon Funaki", "Red Hearts"),
            hit("Red Hearts", "Marlon Funaki", "Monterey Village"),
            hit("Sundown", "Someone Else feat. Marlon Funaki", "Sundown"),
            hit("Unrelated", "Someone Else", "Unrelated"),
        ];
        let kept: Vec<(String, String)> = one_take_each("Marlon Funaki", Some("Metaphor"), hits)
            .into_iter()
            .map(|s| (s.title, s.album))
            .collect();
        assert_eq!(
            kept,
            [
                ("Red Hearts".to_owned(), "Red Hearts".to_owned()),
                ("Sundown".to_owned(), "Sundown".to_owned()),
            ]
        );
    }

    #[test]
    fn a_credit_is_tried_whole_then_artist_by_artist() {
        assert_eq!(
            credited_names("Shawn Chrystopher feat. Cults & Ashly"),
            [
                "Shawn Chrystopher feat. Cults & Ashly",
                "Shawn Chrystopher",
                "Cults",
                "Ashly"
            ]
        );
        assert_eq!(
            credited_names("Simon & Garfunkel"),
            ["Simon & Garfunkel", "Simon", "Garfunkel"]
        );
        assert_eq!(
            credited_names("Run with the Kittens")[0],
            "Run with the Kittens",
            "a band called that comes first"
        );
        assert_eq!(
            credited_names("Sia x Diplo, Labrinth"),
            ["Sia x Diplo, Labrinth", "Sia", "Diplo", "Labrinth"]
        );
        assert_eq!(credited_names("Radiohead"), ["Radiohead"]);
    }

    #[tokio::test]
    async fn a_feat_credit_finds_its_main_artist() {
        let names = credited_names("Shawn Chrystopher feat. Cults & Ashly");
        let (mbid, failure) = resolve(&names, |name| async move {
            Ok((name == "Shawn Chrystopher").then(|| "mbid-shawn".to_owned()))
        })
        .await;
        assert_eq!(mbid.as_deref(), Some("mbid-shawn"));
        assert!(failure.is_none());
    }

    #[tokio::test]
    async fn an_identity_that_cant_be_decided_now_leaves_the_next_name_with_the_reason_kept() {
        let asked = Cell::new(0);
        let (mbid, failure) = resolve(&credited_names("Nobody feat. Anybody"), |name| {
            asked.set(asked.get() + 1);
            async move {
                match name.as_str() {
                    "Anybody" => Ok(Some("mbid-anybody".to_owned())),
                    _ => Err(Error::unavailable("MusicBrainz", "error sending request")),
                }
            }
        })
        .await;
        assert_eq!(mbid.as_deref(), Some("mbid-anybody"));
        assert!(failure.is_some_and(|e| e.to_string().contains("MusicBrainz")));
        assert_eq!(
            asked.get(),
            3,
            "the whole credit, its main part, then the featured artist"
        );
    }

    #[test]
    fn remixes_and_live_takes_are_versions_but_titles_with_the_words_are_not() {
        // Live, "Toro y Moi radio" opened with a Tom Lown remix, then two more and a live take.
        assert!(is_version("Talamak (Tom Lown Remix)"));
        assert!(is_version("So Many Details - Live at KEXP"));
        assert!(is_version("Rose Quartz [Midland Version]"));
        assert!(is_version("Drift (sped up)"));
        assert!(is_version("Paper Sun (Night Arcade Mashup)"));
        assert!(is_version("Lanterns (instrumental)"));
        assert!(is_version("Haze (demo)"));
        assert!(!is_version("Live and Let Die"), "the word in the title itself");
        assert!(!is_version("Dub Be Good to Me"));
        assert!(!is_version("Mix Tape (feat. Neon Wren)"));
    }
}
