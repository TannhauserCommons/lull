//! `GET /api/feed`: the home feed. Built in the background (every source is slow and paced),
//! kept in memory and in the database, served at once and rebuilt when it is six hours old,
//! when follows change, or after feedback. A shelf whose source failed is empty and says why in
//! `errors`; the other shelves still come.
//!
//! New releases show only what can play: saved albums and those Soulseek peers share
//! (`check_shelf`). Remembered answers fill the shelf at once; the rest are searched one at a
//! time, best first, only while they could still make the shelf. The shelf is served with the
//! answers in after 3 s, and re-served (`feed.updated`) as later ones arrive.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use axum::Json;
use axum::extract::State;
use lull_core::ids::{AlbumId, ArtistId, TrackId};
use lull_core::model::{Album, Artist, Entities, Saved, Track};
use lull_core::{Error, Result};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock, mpsc};
use tokio::time::Instant;

use crate::artist::identity;
use crate::sources::listenbrainz::Mode;
use crate::sources::{ArtistInfo, ArtistSource, Release, Song};
use crate::store::{self, entities};
use crate::taste::{Evidence, Taste};
use crate::{Service, art, mix, pages, radio};

const STALE_AFTER_S: i64 = 6 * 3600;
const NEW_RELEASES: usize = 12;
/// The best fresh releases asked about on Soulseek, to fill `NEW_RELEASES`.
const RELEASE_CANDIDATES: usize = 30;
/// How long a build waits for Soulseek's answers before serving the shelf.
const AVAILABILITY_WAIT: Duration = Duration::from_secs(3);
/// A check still unanswered after this counts as unchecked.
const AVAILABILITY_LIMIT: Duration = Duration::from_secs(600);
/// How long releases Soulseek couldn't answer for (offline, searches unanswered) wait before
/// they are asked again. Asking a paused or offline Soulseek costs no search.
const RECHECK: Duration = Duration::from_secs(5 * 60);
const PICKS: usize = 12;
/// Artists offered to follow on a first run.
const COLD_START_ARTISTS: usize = 6;
/// Most played artists looked at for Jump back in's one artist.
const JUMP_ARTIST_TRIES: usize = 5;
/// Tags shown beside a followed one ("you might like").
const RELATED_TAGS: usize = 4;
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
    /// Built with nothing known of the listener: Home shows its first-run screen.
    #[serde(default)]
    pub cold: bool,
    /// On a first run, artists of the starter tags' picks, to follow from the first-run screen.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cold_start_artists: Vec<ArtistId>,
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
    /// What the listener did that the reason rests on (`Taste::evidence`): an artist's plays,
    /// likes, saves, follow, saved albums and Last.fm plays; a tag's liked artists and follow.
    #[serde(default, skip_serializing_if = "Evidence::is_empty")]
    pub evidence: Evidence,
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
    /// The new releases asked about, best first; `newReleases` is chosen from them.
    #[serde(default)]
    shelf: Vec<Shelved>,
    /// Built with nothing known of the listener (the summary says so).
    #[serde(default)]
    cold: bool,
    /// Tags that come with each followed tag on this month's releases (Home's "you might like").
    #[serde(default)]
    related: BTreeMap<String, Vec<String>>,
}

/// A fresh release that may go on the shelf.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Shelved {
    pub id: AlbumId,
    pub artist: String,
    pub title: String,
    /// By an artist the listener follows or likes.
    pub liked: bool,
    /// Saved in the library: shown without asking Soulseek.
    pub saved: bool,
    /// Soulseek's answer: `None` until it gives one.
    pub available: Option<bool>,
}

impl Shelved {
    /// Can go on the shelf: saved, or peers share it. A release without an answer can't.
    fn playable(&self) -> bool {
        self.saved || self.available == Some(true)
    }
}

#[derive(Default)]
pub struct Feeds {
    current: RwLock<Option<Built>>,
    building: Mutex<()>,
    queued: AtomicBool,
    /// Which shelf `check_shelf` asks about: a newer one stops the older check.
    shelf_checks: AtomicU64,
}

/// Soulseek's answers as they arrive.
type Answers = mpsc::UnboundedReceiver<(AlbumId, bool)>;

#[derive(Serialize)]
struct Updated {
    updated: i64,
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
    keep_named_albums(&mut feed);
    let tags: Vec<serde_json::Value> = feed
        .followed_tags
        .iter()
        .filter_map(|t| Some(serde_json::json!({ "id": t, "name": t, "related": built.related.get(t)? })))
        .collect();
    let entities = art::pictured(&svc, std::mem::take(&mut feed.entities)).await?;
    let mut answer = serde_json::to_value(feed).map_err(|e| Error::internal(format!("feed json: {e}")))?;
    answer["entities"] = serde_json::to_value(entities).map_err(|e| Error::internal(format!("feed json: {e}")))?;
    if !tags.is_empty() {
        answer["entities"]["tags"] = serde_json::Value::Array(tags);
    }
    Ok(Json(answer))
}

/// Drops the albums the feed doesn't name: the shelf's other candidates (more than half the
/// feed's bytes) stay in the stored feed, for when Soulseek's answers put them on the shelf.
fn keep_named_albums(feed: &mut Feed) {
    let jumps = feed.jump_back_in.iter().filter(|j| j.kind == "album");
    let named: HashSet<AlbumId> = feed
        .new_releases
        .iter()
        .cloned()
        .chain(jumps.map(|j| AlbumId(j.id.clone())))
        .chain(feed.entities.tracks.values().map(|t| t.album_id.clone()))
        .collect();
    feed.entities.albums.retain(|id, _| named.contains(id));
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

/// The feed in memory, else the stored one. A stored feed's shelf is checked again: the hub
/// restarted, so whatever was asking about it stopped.
async fn current(svc: &Arc<Service>) -> Result<Option<Built>> {
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
    let Some(mut built) = stored.and_then(|s| serde_json::from_str::<Built>(&s).ok()) else {
        return Ok(None);
    };
    let mut current = svc.feed.current.write().await;
    if let Some(b) = current.as_ref() {
        return Ok(Some(b.clone()));
    }
    // Stored by an earlier build of lull, the shelf may hold releases nobody answered for.
    built.feed.new_releases = shown(&built.shelf);
    *current = Some(built.clone());
    svc.ctx
        .spawn(keep_checking(svc.clone(), ask_soulseek(svc, &built.shelf)));
    Ok(Some(built))
}

/// The first feed ever: built while the caller waits (the UI shows its loading state).
async fn build_now(svc: &Arc<Service>) -> Result<Built> {
    let _building = svc.feed.building.lock().await;
    if let Some(b) = svc.feed.current.read().await.clone() {
        return Ok(b);
    }
    rebuild(svc).await
}

async fn rebuild(svc: &Arc<Service>) -> Result<Built> {
    let (built, answers) = build(svc).await?;
    {
        let mut current = svc.feed.current.write().await;
        persist(svc, &built).await?;
        *current = Some(built.clone());
    }
    svc.ctx.bus.publish(
        "feed.updated",
        Updated {
            updated: built.feed.updated,
        },
    );
    svc.ctx.spawn(keep_checking(svc.clone(), answers));
    Ok(built)
}

/// Stores the feed, served after a restart. Callers hold `current`'s write lock, so the stored
/// feed is the current one.
async fn persist(svc: &Service, built: &Built) -> Result<()> {
    let json = serde_json::to_string(built).map_err(|e| Error::internal(format!("feed json: {e}")))?;
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
        .await
}

/// Starts asking Soulseek about this shelf, which stops any check of an older one.
fn ask_soulseek(svc: &Arc<Service>, shelf: &[Shelved]) -> Answers {
    let (tx, rx) = mpsc::unbounded_channel();
    let check = svc.feed.shelf_checks.fetch_add(1, Ordering::SeqCst) + 1;
    svc.ctx.spawn(check_shelf(svc.clone(), check, shelf.to_vec(), tx));
    rx
}

/// Asks Soulseek which of the shelf's releases peers share, until it holds `NEW_RELEASES`
/// playable ones or nothing left could make it. Remembered answers come first, all of them,
/// so the shelf fills at once; then one search per release, best first, never for a release
/// already ranked below a full shelf: the 12 best that peers share, for as few searches as
/// that takes. Releases Soulseek couldn't answer for are asked again every `RECHECK`. Stops
/// once a newer shelf is being checked, or the answers' receiver is gone.
async fn check_shelf(
    svc: Arc<Service>,
    check: u64,
    mut shelf: Vec<Shelved>,
    tx: mpsc::UnboundedSender<(AlbumId, bool)>,
) {
    let current = || svc.feed.shelf_checks.load(Ordering::SeqCst) == check && !tx.is_closed();
    for c in shelf.iter_mut().filter(|c| !c.saved && c.available.is_none()) {
        match svc.transfers.album_known(&c.artist, &c.title).await {
            Ok(Some(shared)) => {
                c.available = Some(shared);
                let _ = tx.send((c.id.clone(), shared));
            }
            Ok(None) => {}
            Err(e) => tracing::info!(error = %e, album = %c.id, "reading Soulseek's remembered answer"),
        }
    }
    loop {
        let mut unanswered = HashSet::new();
        while let Some(i) = next_to_ask(&shelf, &unanswered) {
            if !current() {
                return;
            }
            let c = &mut shelf[i];
            match ask(&svc, c).await {
                Some(shared) => {
                    c.available = Some(shared);
                    let _ = tx.send((c.id.clone(), shared));
                }
                None => {
                    unanswered.insert(c.id.clone());
                }
            }
        }
        if unanswered.is_empty() {
            return;
        }
        tokio::time::sleep(RECHECK).await;
        if !current() {
            return;
        }
    }
}

/// Whether peers share the release; `None` when Soulseek can't say now.
async fn ask(svc: &Service, c: &Shelved) -> Option<bool> {
    let asked = svc.transfers.album_available(&c.artist, &c.title);
    match tokio::time::timeout(AVAILABILITY_LIMIT, asked).await {
        Ok(Ok(answer)) => answer,
        Ok(Err(e)) => {
            tracing::info!(error = %e, album = %c.id, "Soulseek couldn't say whether peers share it");
            None
        }
        Err(_) => None,
    }
}

/// The best release still to ask about that could make the shelf: one without an answer
/// (and not in `skip`) with fewer than `NEW_RELEASES` playable releases ranked above it.
fn next_to_ask(shelf: &[Shelved], skip: &HashSet<AlbumId>) -> Option<usize> {
    let mut playable = 0;
    for (i, c) in shelf.iter().enumerate() {
        if playable >= NEW_RELEASES {
            return None;
        }
        if c.playable() {
            playable += 1;
        } else if c.available.is_none() && !skip.contains(&c.id) {
            return Some(i);
        }
    }
    None
}

/// Applies the answers that come in by `until`.
async fn first_answers(answers: &mut Answers, shelf: &mut [Shelved], until: Instant) {
    while let Ok(Some((id, answer))) = tokio::time::timeout_at(until, answers.recv()).await {
        answered(shelf, &id, answer);
    }
}

/// Re-serves the new releases as Soulseek's later answers arrive, to whichever feed is current.
async fn keep_checking(svc: Arc<Service>, mut answers: Answers) {
    while let Some(first) = answers.recv().await {
        let mut batch = vec![first];
        while let Ok(more) = answers.try_recv() {
            batch.push(more);
        }
        if let Err(e) = reshelve(&svc, batch).await {
            tracing::warn!(error = %e, "new releases not re-served");
        }
    }
}

async fn reshelve(svc: &Service, batch: Vec<(AlbumId, bool)>) -> Result<()> {
    let mut current = svc.feed.current.write().await;
    let Some(built) = current.as_mut() else { return Ok(()) };
    let mut changed = false;
    for (id, answer) in &batch {
        changed |= answered(&mut built.shelf, id, *answer);
    }
    let shown = shown(&built.shelf);
    if !changed || shown == built.feed.new_releases {
        return Ok(());
    }
    built.feed.summary = summary(built.cold, from_follows(&built.shelf, &shown), &built.feed.picks);
    built.feed.new_releases = shown;
    persist(svc, built).await?;
    svc.ctx.bus.publish(
        "feed.updated",
        Updated {
            updated: built.feed.updated,
        },
    );
    Ok(())
}

/// Records Soulseek's answer for a release on the shelf; returns whether it changed anything.
fn answered(shelf: &mut [Shelved], id: &AlbumId, shared: bool) -> bool {
    shelf.iter_mut().filter(|c| c.id == *id).fold(false, |changed, c| {
        let differs = c.available != Some(shared);
        c.available = Some(shared);
        changed || differs
    })
}

/// The new releases shown: saved ones and those Soulseek peers share, best first. A release
/// without an answer isn't shown: the shelf is empty rather than wrong while Soulseek can't say.
pub fn shown(shelf: &[Shelved]) -> Vec<AlbumId> {
    shelf
        .iter()
        .filter(|c| c.playable())
        .take(NEW_RELEASES)
        .map(|c| c.id.clone())
        .collect()
}

/// How many shown releases are by artists the listener follows or likes.
fn from_follows(shelf: &[Shelved], shown: &[AlbumId]) -> usize {
    shelf.iter().filter(|c| c.liked && shown.contains(&c.id)).count()
}

/// A pick before it is a track: the song, why, and from where.
struct Candidate {
    song: Song,
    reason: Reason,
    via: &'static str,
}

async fn build(svc: &Arc<Service>) -> Result<(Built, Answers)> {
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

    let (releases, related) = new_releases(svc, &taste, &followed_artists, &followed_tags)
        .await
        .unwrap_or_else(|e| {
            errors.insert("newReleases".into(), e.to_string());
            (Vec::new(), BTreeMap::new())
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
    let albums = svc.store.albums(releases.iter().map(|r| r.0.clone()).collect()).await?;
    let mut shelf: Vec<Shelved> = albums
        .iter()
        .zip(&releases)
        .map(|(a, (_, why))| Shelved {
            id: a.id.clone(),
            artist: a.artist.clone(),
            title: a.title.clone(),
            liked: why.is_some(),
            saved: matches!(a.saved, Saved::All | Saved::Partial),
            available: None,
        })
        .collect();
    let mut answers = ask_soulseek(svc, &shelf);
    let (starters, starter_artists, starter_tracks) = starters(svc, &taste, &followed_tags, &recent).await?;
    let (jumps, jump_albums, jump_artists) = jump_back_in(svc, &recent).await?;
    // Remembered answers are in at once; the rest re-serve the shelf as they come.
    first_answers(&mut answers, &mut shelf, Instant::now() + AVAILABILITY_WAIT).await;
    let new_releases = shown(&shelf);

    let cold = cold && followed_tags.is_empty();
    let summary = summary(cold, from_follows(&shelf, &new_releases), &picks);
    let mut artists = starter_artists;
    artists.extend(jump_artists);
    // On a first run the picks come from the starter tags: their artists are offered to follow.
    let cold_start_artists: Vec<ArtistId> = if cold {
        let mut seen = HashSet::new();
        let named: Vec<Artist> = tracks_of(&picks, &tracks)
            .filter(|t| seen.insert(t.artist_id.clone()))
            .take(COLD_START_ARTISTS)
            .map(|t| {
                store::artist(ArtistInfo {
                    mbid: None,
                    name: t.artist.clone(),
                })
            })
            .collect();
        let named = svc.store.artists(named).await?;
        let ids = named.iter().map(|a| a.id.clone()).collect();
        artists.extend(named);
        ids
    } else {
        Vec::new()
    };
    let mut all_albums = albums.clone();
    all_albums.extend(jump_albums);
    let mut all_tracks = tracks;
    all_tracks.extend(starter_tracks);
    let built = Built {
        feed: Feed {
            updated: now,
            summary,
            new_releases,
            picks,
            radio_starters: starters,
            jump_back_in: jumps,
            followed_tags,
            cold_start_tags: COLD_START_TAGS.iter().map(|t| (*t).to_owned()).collect(),
            cold,
            cold_start_artists,
            errors,
            entities: entities(all_tracks, all_albums, artists),
        },
        follows,
        shelf,
        cold,
        related,
    };
    Ok((built, answers))
}

/// The catalog tracks the picks name, in their order.
fn tracks_of<'a>(picks: &'a [Pick], tracks: &'a [Track]) -> impl Iterator<Item = &'a Track> {
    picks
        .iter()
        .filter_map(move |p| tracks.iter().find(|t| t.id == p.track_id))
}

/// Fresh releases (last 30 days, every size of artist) by followed and liked artists, then
/// those tagged with followed tags or tags that run through liked artists (`Taste::dig_tags`:
/// never an umbrella genre, which matches the charts), the best `RELEASE_CANDIDATES`. Nothing the
/// listener has met already, and no reissue: neither is news. The second half is a release's
/// artist name when it is "fresh from an artist you like". With them, the tags that come with
/// each followed tag on these releases (`related_tags`).
async fn new_releases(
    svc: &Service,
    taste: &Taste,
    followed: &[(ArtistId, String)],
    followed_tags: &[String],
) -> Result<Releases> {
    let fresh = svc.lb.fresh_releases().await?;
    offer_release_artists(svc, &fresh).await;
    let related = related_tags(&fresh, followed_tags);
    Ok((rank_releases(fresh, taste, followed, followed_tags), related))
}

type Releases = (Vec<(Release, Option<String>)>, BTreeMap<String, Vec<String>>);

/// For each followed tag, the specific tags most often on the same fresh releases: from the
/// releases the feed reads anyway, so no source is asked for them.
fn related_tags(fresh: &[Release], followed: &[String]) -> BTreeMap<String, Vec<String>> {
    followed
        .iter()
        .filter_map(|tag| {
            let tagged: Vec<Release> = fresh.iter().filter(|r| r.tags.contains(tag)).cloned().collect();
            let related: Vec<String> = pages::co_tags(tag, &tagged)
                .into_iter()
                .filter(|t| !followed.contains(t))
                .take(RELATED_TAGS)
                .collect();
            (!related.is_empty()).then(|| (tag.clone(), related))
        })
        .collect()
}

/// ListenBrainz names a release's artist by its MBID: offered as who that artist is
/// (`artist::identity::offer`), for a credit of one artist.
async fn offer_release_artists(svc: &Service, releases: &[Release]) {
    let offers = releases
        .iter()
        .filter(|r| radio::credited_names(&r.artist).len() == 1)
        .filter_map(|r| {
            let mbid = r.artist_mbid.as_ref()?;
            let row = identity::IdRow::recommended(identity::Source::Musicbrainz, mbid, "ListenBrainz fresh releases");
            Some((r.artist.clone(), row))
        })
        .collect();
    if let Err(e) = identity::offer(svc, offers).await {
        tracing::info!(error = %e, "fresh releases' artist ids not kept");
    }
}

/// `new_releases`' choice among the fresh releases, best first.
fn rank_releases(
    fresh: Vec<Release>,
    taste: &Taste,
    followed: &[(ArtistId, String)],
    followed_tags: &[String],
) -> Vec<(Release, Option<String>)> {
    let followed: HashSet<&ArtistId> = followed.iter().map(|(id, _)| id).collect();
    let liked: HashMap<&ArtistId, f64> = taste.top_artists(60).map(|a| (&a.id, a.weight)).collect();
    let mut tags: HashMap<String, f64> = taste.dig_tags().take(8).cloned().collect();
    for t in followed_tags {
        *tags.entry(t.to_lowercase()).or_default() += 1.0;
    }

    let mut scored: Vec<(f64, Release, Option<String>)> = fresh
        .into_iter()
        .filter_map(|r| {
            let id = ArtistId::of(&r.artist);
            if taste.hidden.contains(&id) || taste.knows_release(&r.artist, &r.title) || is_reissue(&r.title) {
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
    scored
        .into_iter()
        .filter(|(_, r, _)| seen.insert(AlbumId::of(&r.artist, &r.title)))
        .take(RELEASE_CANDIDATES)
        .map(|(_, r, why)| (r, why))
        .collect()
}

/// A release of music already out: "Paper Sun (remastered 2026)", "… (2026 Reissue)".
fn is_reissue(title: &str) -> bool {
    lull_core::ids::normalize(title)
        .split(' ')
        .any(|w| matches!(w, "remaster" | "remastered" | "reissue" | "reissued"))
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
        let reason = Reason {
            kind: ReasonKind::Like,
            name: seed.name.clone(),
            reference: Some(seed.id.0.clone()),
            evidence: taste.evidence(&seed.id),
        };
        match similar_to(svc, &seed.id).await {
            Ok(Similar::Recordings(found)) => pending.extend(
                found
                    .into_iter()
                    .map(|mbid| (mbid, reason.clone(), "ListenBrainz similar artists")),
            ),
            Ok(Similar::Songs(songs)) => {
                radio::offer(svc, &songs, "Deezer radio").await;
                pools.push(
                    songs
                        .into_iter()
                        .map(|song| Candidate {
                            song,
                            reason: reason.clone(),
                            via: "Deezer radio",
                        })
                        .collect(),
                );
            }
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
                        evidence: taste.tag_evidence(&tag, followed_tags.contains(&tag)),
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
                    evidence: Evidence::default(),
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
            radio::offer(svc, &songs, "ListenBrainz").await;
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
            // The first song of it the listener hasn't met, an original when there is one.
            Ok(tracks) => fresh.extend(
                originals_first(
                    tracks
                        .into_iter()
                        .filter(|s| !taste.knows_song(&s.artist, &s.title))
                        .collect(),
                )
                .into_iter()
                .next()
                .map(|song| Candidate {
                    song,
                    reason: Reason {
                        kind: ReasonKind::Fresh,
                        name: artist.clone().unwrap_or_default(),
                        reference: Some(ArtistId::of(&release.artist).0),
                        evidence: taste.evidence(&ArtistId::of(&release.artist)),
                    },
                    via: "MusicBrainz releases",
                }),
            ),
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

/// Originals before remixes, sped-up and live takes, each in its order (`radio::is_version`).
fn originals_first(mut songs: Vec<Song>) -> Vec<Song> {
    songs.sort_by_key(|s| radio::is_version(&s.title));
    songs
}

/// Songs by artists similar to a liked one (not by it), by its exact ids (`artist::identity`):
/// ListenBrainz's artist radio by its MBID (recordings, for `ListenBrainz::songs`), else Deezer's
/// radio by its Deezer id. Without either, none.
enum Similar {
    Recordings(Vec<String>),
    Songs(Vec<Song>),
}

async fn similar_to(svc: &Service, id: &ArtistId) -> Result<Similar> {
    let ids = identity::ids(svc, id).await?;
    if let Some(mbid) = ids.mbid() {
        let found = svc.lb.artist_radio(mbid, Mode::Medium, 6, 1, (0, 60)).await?;
        return Ok(Similar::Recordings(
            found
                .into_iter()
                .filter(|p| p.artist_mbid != mbid)
                .map(|p| p.recording_mbid)
                .collect(),
        ));
    }
    let Some(deezer) = ids.deezer() else {
        return Ok(Similar::Songs(Vec::new()));
    };
    let mut songs = svc.deezer.radio(deezer).await?;
    songs.retain(|s| s.artist_source != Some(ArtistSource::Deezer(deezer)));
    mix::shuffle(&mut songs, mix::seed());
    songs.truncate(6);
    Ok(Similar::Songs(songs))
}

/// Tags to dig in: followed ones and the strongest that run through liked artists; on a cold
/// start, three of the cold-start tags, a different three each day.
fn pick_tags(taste: &Taste, followed: &[String]) -> Vec<String> {
    let mut tags: Vec<String> = followed.iter().take(3).cloned().collect();
    for (t, _) in taste.dig_tags() {
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

/// Round-robin over the sources, one track per artist, originals first, nothing the listener
/// already knows (on any release), and (except fresh releases) no artist they already like:
/// picks are for finding new ones.
fn interleave(mut pools: Vec<Vec<Candidate>>, taste: &Taste) -> Vec<Candidate> {
    for pool in &mut pools {
        pool.sort_by_key(|c| radio::is_version(&c.song.title));
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
                    || taste.knows_song(&c.song.artist, &c.song.title)
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
    if let Some((t, _)) = taste.dig_tags().find(|(t, _)| !followed_tags.contains(t)) {
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

/// Albums played lately, most recent first, then the most played artist of the month that lull
/// knows by its exact ids.
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
    let mut by_plays: Vec<(&str, u32)> = plays.into_iter().collect();
    by_plays.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    // The most played artist lull knows by its exact ids, so its page and photo are that
    // artist's (a credit like "A feat. B" holds none of its own).
    for (artist, _) in by_plays.into_iter().take(JUMP_ARTIST_TRIES) {
        let a = store::artist(ArtistInfo {
            mbid: None,
            name: artist.to_owned(),
        });
        match identity::ids(svc, &a.id).await {
            Ok(ids) if !ids.is_empty() => {}
            Ok(_) => continue,
            Err(e) => {
                tracing::info!(error = %e, artist, "jump back in: who the artist is isn't known now");
                continue;
            }
        }
        jumps.push(Jump {
            kind: "artist".into(),
            id: a.id.0.clone(),
        });
        artists.push(a);
        break;
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

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use lull_core::ids::AlbumId;

    use lull_core::ids::{ArtistId, TrackId};

    use super::{
        Candidate, NEW_RELEASES, Reason, ReasonKind, Shelved, answered, interleave, next_to_ask, rank_releases,
        related_tags, shown,
    };
    use crate::sources::{Release, Song};
    use crate::taste::{Mark, Named, Signals, Taste};

    const NOW: i64 = 1_800_000_000;

    /// A listener who liked three chillwave artists and one artist whose only tags are a
    /// namesake's, and one song of an artist new releases may come from.
    fn listener() -> Taste {
        let like = |artist: &str, title: &str| Mark {
            track: TrackId::of(artist, "", title),
            artist: artist.into(),
            at: NOW,
        };
        let mut signals = Signals {
            likes: vec![
                like("Glow Tape", "a"),
                like("Tidal Bloom", "b"),
                like("Night Arcade", "c"),
                like("Namesake", "d"),
            ],
            songs: vec![Named {
                artist: "Glow Tape & Mira Vale".into(),
                title: "Paper Sun".into(),
                album: String::new(),
            }],
            ..Default::default()
        };
        for artist in ["Glow Tape", "Tidal Bloom", "Night Arcade"] {
            signals
                .artist_tags
                .insert(ArtistId::of(artist), vec!["chillwave".into(), "hip hop".into()]);
        }
        signals
            .artist_tags
            .insert(ArtistId::of("Namesake"), vec!["kurdish".into()]);
        Taste::build(&signals, NOW)
    }

    fn fresh(artist: &str, title: &str, tags: &[&str]) -> Release {
        Release {
            mbid: None,
            title: title.into(),
            artist: artist.into(),
            artist_mbid: None,
            kind: Some("Album".into()),
            date: None,
            tags: tags.iter().map(|t| (*t).to_owned()).collect(),
        }
    }

    #[test]
    fn new_releases_are_news_in_the_tags_the_listener_likes() {
        // Live, the shelf held chart releases matched on "hip hop" and "pop", two songs the
        // listener had already liked, and two 2026 remasters of 20-year-old songs.
        let ranked = rank_releases(
            vec![
                fresh("Chart Star", "Platinum", &["hip hop"]),
                fresh("Oud Ensemble", "Mountains", &["kurdish"]),
                fresh("Glow Tape", "Paper Sun", &[]),
                fresh("Glow Tape", "Old Hit (remastered 2026)", &[]),
                fresh("Glow Tape", "Second Light", &[]),
                fresh("Unknown Coast", "Haze", &["chillwave"]),
            ],
            &listener(),
            &[],
            &[],
        );
        let titles: Vec<&str> = ranked.iter().map(|(r, _)| r.title.as_str()).collect();
        assert_eq!(
            titles,
            ["Second Light", "Haze"],
            "a liked artist's new album, then a tag running through liked artists; never an umbrella \
             genre, one artist's label, a known song or a reissue"
        );
        assert_eq!(
            ranked[0].1.as_deref(),
            Some("Glow Tape"),
            "fresh from an artist you like"
        );
    }

    #[test]
    fn a_followed_tag_brings_the_tags_its_releases_share_never_a_followed_or_broad_one() {
        // Home's "you might like" was empty unless the tag's page had been opened that visit.
        let fresh = vec![
            fresh("Velvet Orchard", "One", &["shoegaze", "dream pop", "rock"]),
            fresh("Glow Tape", "Two", &["shoegaze", "dream pop", "slowcore"]),
            fresh("Pale Harbor", "Three", &["shoegaze", "slowcore", "city pop"]),
            fresh("Quiet Ferns", "Four", &["city pop"]),
        ];
        let followed = ["shoegaze".to_owned(), "city pop".to_owned()];
        let related = related_tags(&fresh, &followed);
        assert_eq!(
            related["shoegaze"],
            ["dream pop", "slowcore"],
            "on two releases or more, specific only"
        );
        assert!(!related.contains_key("city pop"), "nothing comes with it twice");
    }

    fn candidate(artist: &str, title: &str) -> Candidate {
        Candidate {
            song: Song {
                mbid: None,
                title: title.into(),
                artist: artist.into(),
                artist_mbid: None,
                album: "Somewhere".into(),
                album_artist: None,
                album_mbid: None,
                year: None,
                no: None,
                disc: None,
                duration_s: None,
                tags: Vec::new(),
                artist_source: None,
                isrc: None,
            },
            reason: Reason {
                kind: ReasonKind::Tag,
                name: "chillwave".into(),
                reference: Some("chillwave".into()),
                evidence: Default::default(),
            },
            via: "ListenBrainz tags",
        }
    }

    #[test]
    fn picks_are_originals_the_listener_hasnt_heard() {
        let pool = vec![
            candidate("Mira Vale", "Paper Sun"),
            candidate("Soft Static", "Drift (Night Arcade remix)"),
            candidate("Wave Cellar", "Lanterns at Noon (sped up)"),
            candidate("Pale Harbor", "Lanterns"),
        ];
        let picks: Vec<String> = interleave(vec![pool], &listener())
            .into_iter()
            .map(|c| c.song.title)
            .collect();
        assert_eq!(
            picks,
            ["Lanterns", "Drift (Night Arcade remix)", "Lanterns at Noon (sped up)"],
            "a song liked on another release isn't news; other takes come after the originals"
        );
    }

    fn release(n: usize, saved: bool) -> Shelved {
        Shelved {
            id: AlbumId::of("Velvet Orchard", &format!("Release {n}")),
            artist: "Velvet Orchard".into(),
            title: format!("Release {n}"),
            liked: false,
            saved,
            available: None,
        }
    }

    #[test]
    fn new_releases_show_only_what_can_play_best_first() {
        // Live, the shelf listed releases no Soulseek peer shared, because none had an answer.
        let mut shelf: Vec<Shelved> = (0..30).map(|n| release(n, n == 4)).collect();
        assert_eq!(
            shown(&shelf),
            [shelf[4].id.clone()],
            "no answers yet (Soulseek offline, or still asking): only what is saved"
        );
        let id = |n: usize| shelf[n].id.clone();
        let (one, two, three, twenty) = (id(1), id(2), id(3), id(20));
        assert!(answered(&mut shelf, &one, false));
        assert!(answered(&mut shelf, &two, true));
        assert!(answered(&mut shelf, &twenty, true));
        assert!(!answered(&mut shelf, &twenty, true), "the same answer changes nothing");
        assert_eq!(
            shown(&shelf),
            [two.clone(), shelf[4].id.clone(), twenty.clone()],
            "peers share it, or it is saved; unchecked ones wait"
        );
        answered(&mut shelf, &three, true);
        assert_eq!(shown(&shelf)[..2], [two, three]);
    }

    #[test]
    fn searches_go_best_first_and_stop_once_the_shelf_is_full() {
        let mut shelf: Vec<Shelved> = (0..30).map(|n| release(n, n == 0)).collect();
        let none = HashSet::new();
        assert_eq!(next_to_ask(&shelf, &none), Some(1), "saved ones need no search");
        let id = |n: usize| AlbumId::of("Velvet Orchard", &format!("Release {n}"));
        answered(&mut shelf, &id(1), false);
        let unanswerable = HashSet::from([id(2)]);
        assert_eq!(
            next_to_ask(&shelf, &unanswerable),
            Some(3),
            "one Soulseek couldn't answer for waits for the next round"
        );
        for n in 3..3 + NEW_RELEASES - 1 {
            answered(&mut shelf, &id(n), true);
        }
        assert_eq!(shown(&shelf).len(), NEW_RELEASES);
        assert_eq!(
            next_to_ask(&shelf, &unanswerable),
            None,
            "nothing below a full shelf could make it: no search spent on it"
        );
        assert_eq!(
            next_to_ask(&shelf, &none),
            Some(2),
            "an unanswered one above the twelfth could still take its place"
        );
    }
}
