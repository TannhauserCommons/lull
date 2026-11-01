//! `GET /api/feed`: the home feed. Built in the background (every source is slow and paced),
//! kept in memory and in the database, served at once and rebuilt when it is six hours old,
//! when follows change, or after feedback. A shelf whose source failed is empty and says why in
//! `errors`; the other shelves still come.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::Json;
use axum::extract::State;
use lull_core::ids::{AlbumId, ArtistId, TrackId};
use lull_core::model::{Album, Artist, Entities, Track};
use lull_core::{Error, Result};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};

use crate::sources::listenbrainz::Mode;
use crate::sources::{ArtistInfo, Release, Song};
use crate::store::{self, entities};
use crate::taste::{self, Taste};
use crate::{Service, art, mix};

const STALE_AFTER_S: i64 = 6 * 3600;
const NEW_RELEASES: usize = 12;
const PICKS: usize = 12;
/// Niche and micro-genre tags offered on first run, across the map rather than the charts.
const COLD_START_TAGS: &[&str] = &[
    "dream pop",
    "slowcore",
    "shoegaze",
    "city pop",
    "shibuya-kei",
    "ambient",
    "jazz fusion",
    "math rock",
    "post-rock",
    "bossa nova",
    "krautrock",
    "chillwave",
    "neo-soul",
    "afrobeat",
    "dub",
    "folktronica",
    "midwest emo",
    "baroque pop",
    "minimal techno",
    "trip hop",
    "psychedelic folk",
    "hyperpop",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Feed {
    pub updated: i64,
    pub summary: String,
    pub new_releases: Vec<AlbumId>,
    pub picks: Vec<Pick>,
    pub radio_starters: Vec<Starter>,
    pub jump_back_in: Vec<Jump>,
    pub followed_tags: Vec<String>,
    pub cold_start_tags: Vec<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub errors: BTreeMap<String, String>,
    pub entities: Entities,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pick {
    pub track_id: TrackId,
    pub reasons: Vec<Reason>,
    /// The source, shown in the reason popover.
    pub via: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reason {
    pub kind: ReasonKind,
    pub name: String,
    /// The artist id or tag the reason names; absent when it names neither.
    #[serde(rename = "ref", skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasonKind {
    /// Similar to an artist the listener likes.
    Like,
    /// Carries a tag the listener likes or follows.
    Tag,
    /// Played by listeners like this one.
    Listeners,
    /// New from an artist the listener follows or likes.
    Fresh,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Starter {
    pub id: String,
    pub title: String,
    pub sub: String,
    pub seed: Seed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Seed {
    pub kind: String,
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Jump {
    pub kind: String,
    pub id: String,
}

/// A feed and the follows it was built from (a change means rebuild).
#[derive(Clone, Serialize, Deserialize)]
struct Built {
    feed: Feed,
    follows: String,
}

#[derive(Default)]
pub struct Feeds {
    current: RwLock<Option<Built>>,
    building: Mutex<()>,
    queued: AtomicBool,
}

/// The feed, its albums and artists with their pictures (`art::pictured`).
pub async fn get(State(svc): State<Arc<Service>>) -> Result<Json<serde_json::Value>> {
    let built = match current(&svc).await? {
        Some(built) => built,
        None => build_now(&svc).await?,
    };
    let (follows, followed_tags, hidden) = svc
        .ctx
        .db
        .call(|c| Ok((follows_key(c)?, store::followed_tags(c)?, store::hidden(c)?)))
        .await?;
    if lull_core::now() - built.feed.updated > STALE_AFTER_S || follows != built.follows {
        refresh_soon(&svc);
    }
    let mut feed = built.feed;
    feed.followed_tags = followed_tags;
    // Hiding takes effect at once; the rebuild catches up with the rest.
    feed.picks.retain(|p| {
        feed.entities
            .tracks
            .get(&p.track_id)
            .is_none_or(|t| !hidden.contains(&t.artist_id))
    });
    feed.new_releases.retain(|id| {
        feed.entities
            .albums
            .get(id)
            .is_none_or(|a| !hidden.contains(&a.artist_id))
    });
    let entities = art::pictured(&svc, std::mem::take(&mut feed.entities)).await?;
    let mut answer = serde_json::to_value(feed).map_err(|e| Error::internal(format!("feed json: {e}")))?;
    answer["entities"] = serde_json::to_value(entities).map_err(|e| Error::internal(format!("feed json: {e}")))?;
    Ok(Json(answer))
}

/// Rebuilds in the background (once, however often it is asked while one is queued).
pub fn refresh_soon(svc: &Arc<Service>) {
    if svc.feed.queued.swap(true, Ordering::SeqCst) {
        return;
    }
    let svc = svc.clone();
    // A rebuild makes paced outside calls for a while; it stops with the crates on a restart.
    svc.ctx.clone().spawn(async move {
        let _building = svc.feed.building.lock().await;
        svc.feed.queued.store(false, Ordering::SeqCst);
        if let Err(e) = rebuild(&svc).await {
            tracing::warn!(error = %e, "feed rebuild failed");
        }
    });
}

/// Keeps the feed from going stale while nobody looks at it.
pub async fn keep_fresh(svc: Arc<Service>) {
    // Let start-up finish before the first round of outside calls.
    tokio::time::sleep(Duration::from_secs(20)).await;
    loop {
        match current(&svc).await {
            Ok(Some(b)) if lull_core::now() - b.feed.updated < STALE_AFTER_S => {}
            Ok(_) => refresh_soon(&svc),
            Err(e) => tracing::warn!(error = %e, "feed state"),
        }
        tokio::time::sleep(Duration::from_secs(30 * 60)).await;
    }
}

async fn current(svc: &Service) -> Result<Option<Built>> {
    if let Some(b) = svc.feed.current.read().await.clone() {
        return Ok(Some(b));
    }
    let stored: Option<String> = svc
        .ctx
        .db
        .call(|c| {
            Ok(
                c.query_row("SELECT value FROM discovery_state WHERE key = 'feed'", [], |r| r.get(0))
                    .optional()?,
            )
        })
        .await?;
    let built: Option<Built> = stored.and_then(|s| serde_json::from_str(&s).ok());
    if let Some(b) = &built {
        *svc.feed.current.write().await = Some(b.clone());
    }
    Ok(built)
}

/// The first feed ever: built while the caller waits (the UI shows its loading state).
async fn build_now(svc: &Service) -> Result<Built> {
    let _building = svc.feed.building.lock().await;
    if let Some(b) = svc.feed.current.read().await.clone() {
        return Ok(b);
    }
    rebuild(svc).await
}

async fn rebuild(svc: &Service) -> Result<Built> {
    let built = build(svc).await?;
    let json = serde_json::to_string(&built).map_err(|e| Error::internal(format!("feed json: {e}")))?;
    svc.ctx
        .db
        .call(move |c| {
            c.execute(
                "INSERT INTO discovery_state (key, value) VALUES ('feed', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [json],
            )?;
            Ok(())
        })
        .await?;
    *svc.feed.current.write().await = Some(built.clone());
    Ok(built)
}

/// A pick before it is a track: the song, why, and from where.
struct Candidate {
    song: Song,
    reason: Reason,
    via: &'static str,
}

async fn build(svc: &Service) -> Result<Built> {
    let now = lull_core::now();
    let taste = svc.taste().await?;
    let (followed_artists, followed_tags, recent, follows) = svc
        .ctx
        .db
        .call(move |c| {
            Ok((
                store::followed_artists(c)?,
                store::followed_tags(c)?,
                store::recent_plays(c, now, 30)?,
                follows_key(c)?,
            ))
        })
        .await?;
    let cold = taste.is_empty();
    let mut errors = BTreeMap::new();

    let releases = new_releases(svc, &taste, &followed_artists, &followed_tags)
        .await
        .unwrap_or_else(|e| {
            errors.insert("newReleases".into(), e.to_string());
            Vec::new()
        });
    let candidates = picks(svc, &taste, &followed_tags, &releases, &mut errors).await;

    // Entities: picks as catalog tracks, releases as albums, seeds as artists.
    let pick_songs: Vec<Song> = candidates.iter().map(|c| c.song.clone()).collect();
    let tracks = svc.store.tracks(pick_songs).await?;
    let picks: Vec<Pick> = candidates
        .into_iter()
        .zip(&tracks)
        .map(|(c, t)| Pick {
            track_id: t.id.clone(),
            reasons: vec![c.reason],
            via: c.via.to_owned(),
        })
        .collect();
    let from_follows = releases.iter().filter(|r| r.1.is_some()).count();
    let albums = svc.store.albums(releases.into_iter().map(|r| r.0).collect()).await?;
    let (starters, starter_artists, starter_tracks) = starters(svc, &taste, &followed_tags, &recent).await?;
    let (jumps, jump_albums, jump_artists) = jump_back_in(svc, &recent).await?;

    let summary = summary(cold && followed_tags.is_empty(), from_follows, &picks);
    let mut artists = starter_artists;
    artists.extend(jump_artists);
    let mut all_albums = albums.clone();
    all_albums.extend(jump_albums);
    let mut all_tracks = tracks;
    all_tracks.extend(starter_tracks);
    Ok(Built {
        feed: Feed {
            updated: now,
            summary,
            new_releases: albums.into_iter().map(|a| a.id).collect(),
            picks,
            radio_starters: starters,
            jump_back_in: jumps,
            followed_tags,
            cold_start_tags: COLD_START_TAGS.iter().map(|t| (*t).to_owned()).collect(),
            errors,
            entities: entities(all_tracks, all_albums, artists),
        },
        follows,
    })
}

/// Fresh releases (last 30 days, every size of artist) by followed and liked artists, then
/// those tagged with followed and liked tags. The second half is a release's artist name when
/// it is "fresh from an artist you like".
async fn new_releases(
    svc: &Service,
    taste: &Taste,
    followed: &[(ArtistId, String)],
    followed_tags: &[String],
) -> Result<Vec<(Release, Option<String>)>> {
    let fresh = svc.lb.fresh_releases().await?;
    let followed: HashSet<&ArtistId> = followed.iter().map(|(id, _)| id).collect();
    let liked: HashMap<&ArtistId, f64> = taste.top_artists(60).map(|a| (&a.id, a.weight)).collect();
    let mut tags: HashMap<String, f64> = taste.top_tags(8).cloned().collect();
    for t in followed_tags {
        *tags.entry(t.to_lowercase()).or_default() += 1.0;
    }

    let mut scored: Vec<(f64, Release, Option<String>)> = fresh
        .into_iter()
        .filter_map(|r| {
            let id = ArtistId::of(&r.artist);
            if taste.hidden.contains(&id) {
                return None;
            }
            let by_artist = if followed.contains(&id) {
                Some(100.0)
            } else {
                liked.get(&id).map(|w| 50.0 + w)
            };
            let by_tag: f64 = r.tags.iter().filter_map(|t| tags.get(t)).sum();
            let fuller = matches!(r.kind.as_deref(), Some("Album" | "EP"));
            let score = match (by_artist, by_tag > 0.0) {
                (Some(s), _) => s,
                (None, true) => 10.0 + by_tag + if fuller { 5.0 } else { 0.0 },
                (None, false) => return None,
            };
            let why = by_artist.map(|_| r.artist.clone());
            Some((score, r, why))
        })
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut seen = HashSet::new();
    Ok(scored
        .into_iter()
        .filter(|(_, r, _)| seen.insert(AlbumId::of(&r.artist, &r.title)))
        .take(NEW_RELEASES)
        .map(|(_, r, why)| (r, why))
        .collect())
}

/// Picks from four sources, interleaved so no one source or artist dominates: similar artists
/// of liked ones, deep cuts from liked tags (low popularity on purpose), ListenBrainz's
/// recommendations, and the first track of fresh releases by liked artists.
async fn picks(
    svc: &Service,
    taste: &Taste,
    followed_tags: &[String],
    releases: &[(Release, Option<String>)],
    errors: &mut BTreeMap<String, String>,
) -> Vec<Candidate> {
    let mut failures = Vec::new();
    let mut pools: Vec<Vec<Candidate>> = Vec::new();
    let mut pending: Vec<(String, Reason, &'static str)> = Vec::new();

    for seed in taste.top_artists(4) {
        match similar_to(svc, &seed.name).await {
            Ok(found) => pending.extend(found.into_iter().map(|mbid| {
                let reason = Reason {
                    kind: ReasonKind::Like,
                    name: seed.name.clone(),
                    reference: Some(seed.id.0.clone()),
                };
                (mbid, reason, "ListenBrainz similar artists")
            })),
            Err(e) => failures.push(e),
        }
    }
    for tag in pick_tags(taste, followed_tags) {
        match svc.lb.tag_radio(&tag, (0, 50), 30).await {
            Ok(mut found) => {
                mix::shuffle(&mut found, mix::seed());
                pending.extend(found.into_iter().take(6).map(|mbid| {
                    let reason = Reason {
                        kind: ReasonKind::Tag,
                        name: tag.clone(),
                        reference: Some(tag.clone()),
                    };
                    (mbid, reason, "ListenBrainz tags")
                }));
            }
            Err(e) => failures.push(e),
        }
    }
    if let Some(user) = &svc.ctx.config.discovery.listenbrainz_user {
        match svc.lb.recommended(user).await {
            Ok(found) => pending.extend(found.into_iter().take(30).map(|mbid| {
                let reason = Reason {
                    kind: ReasonKind::Listeners,
                    name: "listeners like you".into(),
                    reference: None,
                };
                (mbid, reason, "ListenBrainz recommendations")
            })),
            Err(e) => failures.push(e),
        }
    }

    match svc
        .lb
        .songs(&pending.iter().map(|p| p.0.clone()).collect::<Vec<_>>())
        .await
    {
        Ok(songs) => {
            let by_mbid: HashMap<String, Song> = songs.into_iter().filter_map(|s| Some((s.mbid.clone()?, s))).collect();
            // One pool per seed (artist, tag, recommendations), so the round-robin mixes them.
            let mut by_seed: BTreeMap<(&'static str, String), Vec<Candidate>> = BTreeMap::new();
            for (mbid, reason, via) in pending {
                if let Some(song) = by_mbid.get(&mbid) {
                    by_seed.entry((via, reason.name.clone())).or_default().push(Candidate {
                        song: song.clone(),
                        reason,
                        via,
                    });
                }
            }
            pools.extend(by_seed.into_values());
        }
        Err(e) => failures.push(e),
    }

    let mut fresh = Vec::new();
    for (release, artist) in releases.iter().filter(|r| r.1.is_some()).take(3) {
        match svc.mb.tracklist(release).await {
            Ok(tracks) => fresh.extend(tracks.into_iter().next().map(|song| Candidate {
                song,
                reason: Reason {
                    kind: ReasonKind::Fresh,
                    name: artist.clone().unwrap_or_default(),
                    reference: Some(ArtistId::of(&release.artist).0),
                },
                via: "MusicBrainz releases",
            })),
            Err(e) => failures.push(e),
        }
    }
    pools.push(fresh);

    let chosen = interleave(pools, taste);
    if chosen.is_empty()
        && let Some(e) = failures.first()
    {
        errors.insert("picks".into(), e.to_string());
    } else if !failures.is_empty() {
        tracing::info!(failed = failures.len(), first = %failures[0], "some pick sources failed");
    }
    chosen
}

/// Recordings by artists similar to `name` (not by `name`), via ListenBrainz artist radio.
async fn similar_to(svc: &Service, name: &str) -> Result<Vec<String>> {
    let Some(mbid) = artist_mbid(svc, name).await? else {
        return Ok(Vec::new());
    };
    let found = svc.lb.artist_radio(&mbid, Mode::Medium, 6, 1, (0, 60)).await?;
    Ok(found
        .into_iter()
        .filter(|p| p.artist_mbid != mbid)
        .map(|p| p.recording_mbid)
        .collect())
}

/// An artist's MBID: remembered from an earlier page, else looked up on MusicBrainz.
pub async fn artist_mbid(svc: &Service, name: &str) -> Result<Option<String>> {
    if let Some(mbid) = svc
        .store
        .artist_ref(ArtistId::of(name))
        .await?
        .and_then(|a| a.musicbrainz_id)
    {
        return Ok(Some(mbid));
    }
    Ok(svc.mb.find_artist(name).await?.and_then(|a| a.mbid))
}

/// Tags to dig in: followed ones and the strongest specific liked ones; on a cold start, three
/// of the cold-start tags, a different three each day.
fn pick_tags(taste: &Taste, followed: &[String]) -> Vec<String> {
    let mut tags: Vec<String> = followed.iter().take(3).cloned().collect();
    for (t, _) in taste.top_tags(20).filter(|(t, _)| taste::is_specific(t)) {
        if tags.len() >= 4 {
            break;
        }
        if !tags.contains(t) {
            tags.push(t.clone());
        }
    }
    if tags.is_empty() {
        let day = (lull_core::now() / 86_400) as usize;
        tags = (0..3)
            .map(|i| COLD_START_TAGS[(day * 3 + i) % COLD_START_TAGS.len()].to_owned())
            .collect();
    }
    tags
}

/// Round-robin over the sources, one track per artist, nothing the listener already knows,
/// and (except fresh releases) no artist they already like: picks are for finding new ones.
fn interleave(mut pools: Vec<Vec<Candidate>>, taste: &Taste) -> Vec<Candidate> {
    for pool in &mut pools {
        pool.reverse(); // pop() takes from the front
    }
    let mut artists = HashSet::new();
    let mut out = Vec::new();
    while out.len() < PICKS && pools.iter().any(|p| !p.is_empty()) {
        for pool in &mut pools {
            while let Some(c) = pool.pop() {
                let t = store::track(c.song.clone());
                let weight = taste.artist_weight(&t.artist_id);
                let familiar = weight > 0.0 && c.reason.kind != ReasonKind::Fresh;
                if taste.known.contains(&t.id)
                    || taste.hidden.contains(&t.artist_id)
                    || weight < 0.0
                    || familiar
                    || !artists.insert(t.artist_id)
                {
                    continue;
                }
                out.push(c);
                break;
            }
            if out.len() >= PICKS {
                break;
            }
        }
    }
    out
}

/// Radio starters: the top artist, followed and liked tags, the song on repeat this week.
async fn starters(
    svc: &Service,
    taste: &Taste,
    followed_tags: &[String],
    recent: &[(TrackId, String, u32, i64)],
) -> Result<(Vec<Starter>, Vec<Artist>, Vec<Track>)> {
    let mut out = Vec::new();
    let mut artists = Vec::new();
    let mut tracks = Vec::new();
    if let Some(top) = taste.top_artists(1).next() {
        out.push(starter(
            "artist",
            &top.id.0,
            format!("{} radio", top.name),
            "your top artist",
        ));
        artists.push(store::artist(ArtistInfo {
            mbid: None,
            name: top.name.clone(),
        }));
    }
    for t in followed_tags.iter().take(2) {
        out.push(starter("tag", t, format!("{t} radio"), "a tag you follow"));
    }
    if let Some((t, _)) = taste
        .top_tags(20)
        .find(|(t, _)| taste::is_specific(t) && !followed_tags.contains(t))
    {
        out.push(starter("tag", t, format!("{t} radio"), "a tag you play a lot"));
    }
    let week_ago = lull_core::now() - 7 * 86_400;
    if let Some((id, _, n, _)) = recent.iter().find(|(_, _, n, at)| *n >= 3 && *at >= week_ago)
        && let Some(t) = svc.store.track(id.clone()).await?
    {
        out.push(starter(
            "track",
            &t.id.0,
            format!("More like {}", t.title),
            &format!("played {n} times lately"),
        ));
        tracks.push(t);
    }
    if out.is_empty() {
        for t in COLD_START_TAGS.iter().take(3) {
            out.push(starter("tag", t, format!("{t} radio"), "a place to start"));
        }
    }
    let artists = svc.store.artists(artists).await?;
    Ok((out, artists, tracks))
}

fn starter(kind: &str, id: &str, title: String, sub: &str) -> Starter {
    Starter {
        id: format!("rs-{kind}-{id}"),
        title,
        sub: sub.to_owned(),
        seed: Seed {
            kind: kind.to_owned(),
            id: id.to_owned(),
        },
    }
}

/// Albums played lately, most recent first, then the most played artist of the month.
async fn jump_back_in(
    svc: &Service,
    recent: &[(TrackId, String, u32, i64)],
) -> Result<(Vec<Jump>, Vec<Album>, Vec<Artist>)> {
    let mut by_recency: Vec<&(TrackId, String, u32, i64)> = recent.iter().collect();
    by_recency.sort_by_key(|r| std::cmp::Reverse(r.3));
    let mut jumps = Vec::new();
    let mut albums = Vec::new();
    let mut seen = HashSet::new();
    for (id, _, _, _) in by_recency {
        if albums.len() >= 5 {
            break;
        }
        let Some(t) = svc.store.track(id.clone()).await? else {
            continue;
        };
        if t.album.is_empty() || !seen.insert(t.album_id.clone()) {
            continue;
        }
        jumps.push(Jump {
            kind: "album".into(),
            id: t.album_id.0.clone(),
        });
        let mut album = store::album(Release {
            mbid: None,
            title: t.album,
            artist: t.artist,
            artist_mbid: None,
            kind: None,
            date: t.year.map(|y| y.to_string()),
            tags: t.tags,
        });
        album.id = t.album_id;
        albums.push(album);
    }
    let mut artists = Vec::new();
    let mut plays: HashMap<&str, u32> = HashMap::new();
    for (_, artist, n, _) in recent {
        *plays.entry(artist).or_default() += n;
    }
    if let Some((artist, _)) = plays.into_iter().max_by_key(|(_, n)| *n) {
        let a = store::artist(ArtistInfo {
            mbid: None,
            name: artist.to_owned(),
        });
        jumps.push(Jump {
            kind: "artist".into(),
            id: a.id.0.clone(),
        });
        artists.push(a);
    }
    let albums = svc.store.remember_albums(albums).await?;
    let artists = svc.store.artists(artists).await?;
    Ok((jumps, albums, artists))
}

fn summary(cold: bool, from_follows: usize, picks: &[Pick]) -> String {
    if cold {
        return "Pick a few tags you like and the feed will follow.".into();
    }
    let tag = picks
        .iter()
        .flat_map(|p| &p.reasons)
        .find(|r| r.kind == ReasonKind::Tag)
        .map(|r| r.name.clone());
    match (from_follows, tag) {
        (0, Some(tag)) => format!("Fresh picks from {tag}."),
        (0, None) => "Fresh picks from what you play.".into(),
        (n, tag) => format!(
            "{n} new release{} from artists you like{}.",
            if n == 1 { "" } else { "s" },
            tag.map(|t| format!(", and fresh picks from {t}")).unwrap_or_default()
        ),
    }
}

/// What the feed depends on from follows, compared to spot a change.
fn follows_key(c: &rusqlite::Connection) -> Result<String> {
    Ok(c.query_row(
        "SELECT COALESCE(group_concat(kind || ':' || id, ','), '') FROM (SELECT kind, id FROM follows ORDER BY kind, id)",
        [],
        |r| r.get(0),
    )?)
}
