//! Soulseek search, behind discovery's catalog search (ADR-0008). One search runs in slskd per
//! query; slskd reveals its answers only once it ends, so lull publishes its counters live and
//! the merged results when it ends (≤ `DEADLINE`): one row per song across peers, and folders
//! grouped as albums. Songs no catalog listed for the query are marked `onlySoulseek`. Resolve
//! searches find copies of one song (docs/context/soulseek.md#search).

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use lull_core::events::Bus;
use lull_core::ids::{AlbumId, ArtistId, TrackId, normalize};
use lull_core::model::{Album, Entities, FileFacts, Saved, Track};
use lull_core::quality::Quality;
use lull_core::seams::Library;
use lull_core::{Error, Result, catalog, db::Db};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::watch;

use crate::files::{self, Attributes};
use crate::matching::Wanted;
use crate::slskd::{self, Slskd};
use crate::sources::{Ranked, Source, Sources};

/// A search ends once no peer has answered for this long...
const INACTIVITY: Duration = Duration::from_secs(6);
/// ...or after this long in all, so results never take longer than the design's 10-20 s.
const DEADLINE: Duration = Duration::from_secs(15);
/// Popular queries saturate quickly; past this many peers more answers add nothing.
const RESPONSE_LIMIT: u32 = 300;
const POLL: Duration = Duration::from_millis(700);
/// Searches start at least this far apart: the Soulseek server throttles clients that flood it.
const SPACING: Duration = Duration::from_secs(1);
/// The same query again within this window reuses the earlier search instead of asking the network.
const REUSE_FOR: Duration = Duration::from_secs(10 * 60);
/// Shorter queries match half the network; they search the library only.
const MIN_QUERY: usize = 3;
/// A folder with at least this many matching songs is shown as an album.
const ALBUM_MIN_TRACKS: usize = 3;
const MAX_SONGS: usize = 200;
const MAX_ALBUMS: usize = 40;

#[derive(Clone)]
pub struct Searches {
    slskd: Slskd,
    bus: Bus,
    db: Db,
    sources: Sources,
    /// Handed over by `router` (server wires the library after soulseek starts); duplicate notes.
    library: Arc<OnceLock<Arc<dyn Library>>>,
    inner: Arc<Mutex<Inner>>,
    last_start: Arc<tokio::sync::Mutex<Option<Instant>>>,
}

#[derive(Default)]
struct Inner {
    next: u64,
    /// The listener's searches, newest last; the newest one supersedes a still-running older one.
    recent: VecDeque<Arc<Session>>,
    /// Searches run to find sources for one song, by query.
    resolving: HashMap<String, watch::Receiver<bool>>,
    /// How the latest network searches ended, newest last.
    outcomes: VecDeque<Outcome>,
    /// The catalog's top tracks per query key (`search.catalog`), for `onlySoulseek`.
    catalog: HashMap<String, (Instant, Vec<Track>)>,
}

/// How one network search ended: what the status and diagnostics report.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    pub query: String,
    /// slskd's final state, e.g. `Completed, TimedOut`.
    pub state: String,
    pub peers: u32,
    pub files: u32,
    pub at: i64,
}

/// Outcomes kept for diagnostics.
const OUTCOMES: usize = 20;

struct Session {
    id: String,
    key: String,
    started: Instant,
    /// The latest `search.results` payload, for clients that ask again (`resync`, the same query).
    latest: Mutex<Option<Value>>,
    /// The songs of the final results, to tell which ones no catalog listed.
    songs: Mutex<Vec<Track>>,
    stop: watch::Sender<bool>,
}

/// `GET /api/soulseek/search` answer.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Started {
    pub search_id: String,
    /// The latest Soulseek results when this query was searched moments ago.
    pub soulseek: Option<Value>,
    pub entities: Entities,
}

impl Searches {
    pub fn new(slskd: Slskd, bus: Bus, db: Db, sources: Sources) -> Self {
        Searches {
            slskd,
            bus,
            db,
            sources,
            library: Arc::default(),
            inner: Arc::default(),
            last_start: Arc::default(),
        }
    }

    pub fn set_library(&self, library: Arc<dyn Library>) {
        let _ = self.library.set(library);
    }

    /// The listener submitted a query: Soulseek results follow as events.
    pub fn start(&self, query: &str, online: bool) -> Result<Started> {
        let query = query.trim();
        if query.is_empty() || query.chars().count() > 200 {
            return Err(Error::BadRequest("a search needs 1 to 200 characters".into()));
        }
        let (session, fresh) = self.session(query);
        if fresh {
            let this = self.clone();
            let (query, session) = (query.to_owned(), session.clone());
            tokio::spawn(async move { this.run(&session, &query, online).await });
        }
        let soulseek = session.latest.lock().ok().and_then(|l| l.clone());
        Ok(Started {
            search_id: session.id.clone(),
            soulseek,
            entities: Entities::default(),
        })
    }

    /// Discovery's top catalog tracks for a query (`search.catalog`): the search's final
    /// results get `onlySoulseek`, now if they are in, else when they arrive.
    pub fn catalog(&self, query: &str, tracks: Vec<Track>) {
        let key = normalize(query);
        let session = {
            let Ok(mut inner) = self.inner.lock() else { return };
            inner.catalog.retain(|_, (at, _)| at.elapsed() < REUSE_FOR);
            inner.catalog.insert(key.clone(), (Instant::now(), tracks));
            inner.recent.iter().rev().find(|s| s.key == key).cloned()
        };
        let Some(session) = session.filter(|s| s.latest_done() == Some(true)) else {
            return;
        };
        let Some(mut payload) = session.latest.lock().ok().and_then(|l| l.clone()) else {
            return;
        };
        if payload.get("results").is_none() {
            return;
        }
        payload["onlySoulseek"] = json!(self.only_soulseek(&session));
        if let Ok(mut l) = session.latest.lock() {
            *l = Some(payload.clone());
        }
        self.bus.publish("search.results", payload);
    }

    /// The result songs no catalog track for this query matches. `None` before the catalog
    /// has answered.
    fn only_soulseek(&self, session: &Session) -> Option<Vec<TrackId>> {
        let catalog = self.inner.lock().ok()?.catalog.get(&session.key)?.1.clone();
        let wanted: Vec<Wanted> = catalog.iter().map(Wanted::of).collect();
        let songs = session.songs.lock().ok()?;
        Some(
            songs
                .iter()
                .filter(|t| !t.saved && !wanted.iter().any(|w| w.matches(t, t.duration, "")))
                .map(|t| t.id.clone())
                .collect(),
        )
    }

    /// The session for a query: a recent one for the same words, or a new one that stops
    /// whatever older search is still running.
    fn session(&self, query: &str) -> (Arc<Session>, bool) {
        let key = normalize(query);
        let Ok(mut inner) = self.inner.lock() else {
            return (Arc::new(Session::new("s-0".into(), key)), true);
        };
        inner.recent.retain(|s| s.started.elapsed() < REUSE_FOR);
        if let Some(s) = inner.recent.iter().rev().find(|s| s.key == key) {
            return (s.clone(), false);
        }
        if let Some(previous) = inner.recent.back() {
            previous.stop.send_replace(true);
        }
        inner.next += 1;
        let session = Arc::new(Session::new(format!("s-{}", inner.next), key));
        inner.recent.push_back(session.clone());
        (session, true)
    }

    async fn run(&self, session: &Session, query: &str, online: bool) {
        let publish = |payload: Value| {
            if let Ok(mut l) = session.latest.lock() {
                *l = Some(payload.clone());
            }
            self.bus.publish("search.results", payload);
        };
        let base = json!({ "searchId": session.id, "query": query });
        let with = |extra: Value| {
            let mut v = base.clone();
            if let (Some(v), Some(extra)) = (v.as_object_mut(), extra.as_object()) {
                v.extend(extra.clone());
            }
            v
        };
        let error = if query.chars().count() < MIN_QUERY {
            Some(format!("Soulseek is searched from {MIN_QUERY} characters"))
        } else if !online {
            Some("Soulseek is not connected right now".to_owned())
        } else {
            None
        };
        if let Some(error) = error {
            publish(with(
                json!({ "results": [], "files": 0, "peers": 0, "done": true, "error": error }),
            ));
            return;
        }
        let progress = |s: &slskd::Search| {
            self.bus.publish(
                "search.results",
                with(json!({ "files": s.file_count, "peers": s.response_count, "done": false })),
            )
        };
        let mut stop = session.stop.subscribe();
        let responses = match self.network(query, &mut stop, progress).await {
            Ok(Some(r)) => r,
            Ok(None) => return, // superseded by a newer search
            Err(e) => {
                tracing::warn!(error = %e, "soulseek search failed");
                publish(with(
                    json!({ "results": [], "files": 0, "peers": 0, "done": true, "error": e.to_string() }),
                ));
                return;
            }
        };
        let merged = merge(&responses);
        let files = merged.files;
        let peers = responses.len();
        match self.results(merged).await {
            Ok((results, entities)) => {
                let songs = results
                    .iter()
                    .filter_map(Row::track_id)
                    .filter_map(|id| entities.tracks.get(id));
                if let Ok(mut s) = session.songs.lock() {
                    *s = songs.cloned().collect();
                }
                publish(with(json!({
                    "results": results, "files": files, "peers": peers, "done": true, "entities": entities,
                    "onlySoulseek": self.only_soulseek(session),
                })))
            }
            Err(e) => publish(with(
                json!({ "results": [], "files": files, "peers": peers, "done": true, "error": e.to_string() }),
            )),
        }
    }

    /// Finds sources for one song (a queue item, a discovery pick, a save) by searching its
    /// artist and title, then a looser query when that finds no copy. Files count as the song
    /// by `matching` (loose names, length). Concurrent asks for the same song share one search.
    /// Returns once it has ended.
    pub async fn resolve(&self, track: &Track) -> Result<()> {
        let text = Wanted::query(track);
        let existing = self.inner.lock().ok().and_then(|i| i.resolving.get(&text).cloned());
        if let Some(mut done) = existing {
            let _ = done.wait_for(|d| *d).await;
            return Ok(());
        }
        let (tx, rx) = watch::channel(false);
        if let Ok(mut inner) = self.inner.lock() {
            inner.resolving.insert(text.clone(), rx);
        }
        let wanted = Wanted::of(track);
        let mut outcome = self.search_for(track, &wanted, &text).await;
        if matches!(outcome, Ok(0))
            && let Some(looser) = wanted.looser_query(&text)
        {
            outcome = self.search_for(track, &wanted, &looser).await;
        }
        if let Ok(mut inner) = self.inner.lock() {
            inner.resolving.remove(&text);
        }
        tx.send_replace(true);
        outcome.map(drop)
    }

    /// One resolve search: everything it found is kept by song, and the files that are this
    /// song also under its id. Returns how many files matched.
    async fn search_for(&self, track: &Track, wanted: &Wanted, text: &str) -> Result<usize> {
        let (_never, mut stop) = watch::channel(false);
        let Some(responses) = self.network(text, &mut stop, |_| {}).await? else {
            return Ok(0);
        };
        let mut merged = merge(&responses);
        let matching: Vec<Source> = merged
            .songs
            .values()
            .flat_map(|s| {
                s.sources
                    .iter()
                    .filter(|src| wanted.matches(&s.track, src.length, &src.path))
            })
            .cloned()
            .collect();
        let n = matching.len();
        let mut found: HashMap<TrackId, Vec<Source>> = merged.songs.drain().map(|(id, s)| (id, s.sources)).collect();
        found.entry(track.id.clone()).or_default().extend(matching);
        self.sources.add(found);
        Ok(n)
    }

    /// Whether a search that may still add sources for this song is running.
    pub fn searching_for(&self, track: &Track) -> bool {
        let text = Wanted::query(track);
        self.inner.lock().is_ok_and(|i| {
            i.resolving.contains_key(&text) || i.recent.back().is_some_and(|s| s.latest_done() == Some(false))
        })
    }

    /// Runs one network search to its end. `None` when `stop` fired first (the search is
    /// stopped and dropped in slskd too).
    async fn network(
        &self,
        text: &str,
        stop: &mut watch::Receiver<bool>,
        progress: impl Fn(&slskd::Search),
    ) -> Result<Option<Vec<slskd::Response>>> {
        self.space_out().await;
        let search = self.slskd.start_search(text, INACTIVITY, RESPONSE_LIMIT).await?;
        let started = Instant::now();
        let mut stopping = false;
        let mut last = search;
        loop {
            tokio::select! {
                _ = tokio::time::sleep(POLL) => {}
                _ = crate::fetch::stopped(stop) => {
                    self.drop_search(&last.id, true).await;
                    return Ok(None);
                }
            }
            let state = self.slskd.search(&last.id).await?;
            progress(&state);
            last = state;
            if last.is_complete() {
                break;
            }
            if !stopping && started.elapsed() >= DEADLINE {
                stopping = true;
                self.slskd.stop_search(&last.id).await?;
            }
        }
        let responses = self.slskd.responses(&last.id).await;
        self.drop_search(&last.id, false).await;
        self.record(Outcome {
            query: text.to_owned(),
            state: last.state,
            peers: last.response_count,
            files: last.file_count,
            at: lull_core::now(),
        });
        responses.map(Some)
    }

    fn record(&self, outcome: Outcome) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.outcomes.push_back(outcome);
            while inner.outcomes.len() > OUTCOMES {
                inner.outcomes.pop_front();
            }
        }
    }

    /// The latest network searches, newest last.
    pub fn outcomes(&self) -> Vec<Outcome> {
        self.inner
            .lock()
            .map(|i| i.outcomes.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// How many of the latest searches in a row no peer answered, counting only those that
    /// ended after `since` (unix seconds): a fresh login starts the count again.
    pub fn unanswered_since(&self, since: i64) -> usize {
        self.inner
            .lock()
            .map(|i| {
                i.outcomes
                    .iter()
                    .rev()
                    .take_while(|o| o.peers == 0 && o.at > since)
                    .count()
            })
            .unwrap_or(0)
    }

    /// Whether a search has been answered since `since` (unix seconds).
    pub fn answered_since(&self, since: i64) -> bool {
        self.inner
            .lock()
            .is_ok_and(|i| i.outcomes.iter().any(|o| o.peers > 0 && o.at > since))
    }

    /// slskd stores every search; lull keeps what it needs and tidies up after itself.
    async fn drop_search(&self, id: &str, stop_first: bool) {
        if stop_first && let Err(e) = self.slskd.stop_search(id).await {
            tracing::debug!(error = %e, "stopping a superseded search");
        }
        if let Err(e) = self.slskd.delete_search(id).await {
            tracing::debug!(error = %e, "deleting a finished search from slskd");
        }
    }

    async fn space_out(&self) {
        let mut last = self.last_start.lock().await;
        if let Some(at) = *last {
            tokio::time::sleep(SPACING.saturating_sub(at.elapsed())).await;
        }
        *last = Some(Instant::now());
    }

    /// Ranks and caps the merged answers, notes duplicates of saved songs, records every track
    /// handed out in the catalog, and builds the result rows with their entities.
    async fn results(&self, merged: Merged) -> Result<(Vec<Row>, Entities)> {
        let Merged { songs, folders, .. } = merged;
        self.sources
            .add(songs.iter().map(|(id, s)| (id.clone(), s.sources.clone())).collect());

        let mut rows: Vec<(Song, Ranked)> = songs
            .into_values()
            .filter_map(|s| {
                let best = s
                    .sources
                    .iter()
                    .cloned()
                    .map(|src| self.sources.rank(src))
                    .max_by_key(|r| r.score)?;
                Some((s, best))
            })
            .collect();
        rows.sort_by(|(a, ra), (b, rb)| b.sources.len().cmp(&a.sources.len()).then(rb.score.cmp(&ra.score)));
        rows.truncate(MAX_SONGS);
        let albums = albums(folders, &self.sources);

        let mut wanted: Vec<TrackId> = rows.iter().map(|(s, _)| s.track.id.clone()).collect();
        wanted.extend(albums.iter().flat_map(|a| a.album.track_ids.iter().cloned()));
        // Without the library (down, or not wired yet) rows simply carry no duplicate notes.
        let saved: HashMap<TrackId, Track> = match self.library.get() {
            Some(l) => l.saved(&wanted).await.unwrap_or_default(),
            None => Vec::new(),
        }
        .into_iter()
        .map(|t| (t.id.clone(), t))
        .collect();

        let mut entities = Entities::default();
        let mut out = Vec::with_capacity(rows.len() + albums.len());
        for (mut song, best) in rows {
            song.track.file = Some(best.source.facts.clone());
            let dup = saved
                .get(&song.track.id)
                .map(|t| dup_kind(&best.source.facts, t.file.as_ref()));
            out.push(Row::Track {
                track_id: song.track.id.clone(),
                sources: song.sources.len(),
                best: Box::new(best),
                dup,
            });
            let entity = saved.get(&song.track.id).cloned().unwrap_or(song.track);
            entities.tracks.insert(entity.id.clone(), entity);
        }
        for mut a in albums {
            let n = a.album.track_ids.iter().filter(|id| saved.contains_key(*id)).count();
            a.album.saved = match n {
                0 => Saved::None,
                n if n == a.album.track_ids.len() => Saved::All,
                _ => Saved::Partial,
            };
            for t in a.tracks {
                let entity = saved.get(&t.id).cloned().unwrap_or(t);
                entities.tracks.entry(entity.id.clone()).or_insert(entity);
            }
            out.push(Row::Album {
                album_id: a.album.id.clone(),
                track_ids: a.album.track_ids.clone(),
                folders: a.folders,
            });
            entities.albums.insert(a.album.id.clone(), a.album);
        }
        let unsaved: Vec<Track> = entities.tracks.values().filter(|t| !t.saved).cloned().collect();
        self.db.call(move |c| catalog::remember(c, &unsaved)).await?;
        Ok((out, entities))
    }
}

impl Session {
    fn new(id: String, key: String) -> Self {
        Session {
            id,
            key,
            started: Instant::now(),
            latest: Mutex::new(None),
            songs: Mutex::default(),
            stop: watch::channel(false).0,
        }
    }

    fn latest_done(&self) -> Option<bool> {
        let latest = self.latest.lock().ok()?;
        Some(latest.as_ref()?.get("done")?.as_bool().unwrap_or(false))
    }
}

impl Row {
    fn track_id(&self) -> Option<&TrackId> {
        match self {
            Row::Track { track_id, .. } => Some(track_id),
            Row::Album { .. } => None,
        }
    }
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Row {
    #[serde(rename_all = "camelCase")]
    Track {
        track_id: TrackId,
        /// How many peers share it.
        sources: usize,
        best: Box<Ranked>,
        /// Set when the song is already saved: `note`, or `better` when this file beats the saved one.
        #[serde(skip_serializing_if = "Option::is_none")]
        dup: Option<&'static str>,
    },
    #[serde(rename_all = "camelCase")]
    Album {
        album_id: AlbumId,
        track_ids: Vec<TrackId>,
        /// The peers' folders for this album, best first.
        folders: Vec<FolderView>,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderView {
    pub peer: String,
    /// The peer's folder path.
    pub path: String,
    pub format: String,
    pub tracks: usize,
    pub size_bytes: u64,
    pub speed: f32,
    pub free_slot: bool,
    pub queue: u32,
    pub score: u8,
    /// The folder has a cover image.
    pub cover: bool,
}

/// A saved song met again: is this file an upgrade over the saved one (`Quality::beats`)?
fn dup_kind(found: &FileFacts, saved: Option<&FileFacts>) -> &'static str {
    let better = saved.is_some_and(|s| Quality::of(Some(found)).beats(&Quality::of(Some(s))));
    if better { "better" } else { "note" }
}

/// Everything one search turned up, merged.
pub struct Merged {
    pub songs: HashMap<TrackId, Song>,
    folders: HashMap<(String, String), Folder>,
    pub files: usize,
}

pub struct Song {
    pub track: Track,
    pub sources: Vec<Source>,
}

struct Folder {
    user: String,
    path: String,
    album: Album,
    tracks: BTreeMap<(u32, u32, String), Track>,
    best: Option<Source>,
    size: u64,
    formats: HashMap<String, usize>,
    cover: bool,
}

/// Merges peers' answers: one song per `TrackId` with all its sources, and each peer folder
/// with the songs it holds.
pub fn merge(responses: &[slskd::Response]) -> Merged {
    let mut songs: HashMap<TrackId, Song> = HashMap::new();
    let mut folders: HashMap<(String, String), Folder> = HashMap::new();
    let mut files = 0;
    let covers: HashSet<(&str, &str)> = responses
        .iter()
        .flat_map(|r| r.files.iter().map(move |f| (r, f)))
        .filter(|(_, f)| files::is_cover(&f.filename))
        .map(|(r, f)| (r.username.as_str(), files::folder(&f.filename)))
        .collect();
    for r in responses {
        let mut seen = HashSet::new();
        for f in &r.files {
            files += 1;
            let dir = files::folder(&f.filename).to_owned();
            let Some((track, source)) = song_of(r, f) else { continue };
            // A peer sharing the same song twice (two rips) counts once: keep the first.
            if !seen.insert(track.id.clone()) {
                continue;
            }
            let folder = folders
                .entry((r.username.clone(), dir.clone()))
                .or_insert_with(|| Folder {
                    user: r.username.clone(),
                    path: dir,
                    album: album_of(&track),
                    tracks: BTreeMap::new(),
                    best: None,
                    size: 0,
                    formats: HashMap::new(),
                    cover: covers.contains(&(r.username.as_str(), files::folder(&f.filename))),
                });
            folder.size = folder.size.saturating_add(source.size);
            *folder.formats.entry(source.facts.format.clone()).or_default() += 1;
            folder.best.get_or_insert_with(|| source.clone());
            folder.tracks.insert(
                (track.disc.unwrap_or(1), track.no.unwrap_or(0), track.title.clone()),
                track.clone(),
            );
            let song = songs.entry(track.id.clone()).or_insert_with(|| Song {
                track,
                sources: Vec::new(),
            });
            song.sources.push(source);
        }
    }
    Merged { songs, folders, files }
}

fn song_of(r: &slskd::Response, f: &slskd::File) -> Option<(Track, Source)> {
    let ext = files::extension(&f.filename)?;
    let facts = files::facts(
        &ext,
        f.size,
        Attributes {
            bit_rate: f.bit_rate,
            bit_depth: f.bit_depth,
            sample_rate: f.sample_rate,
            variable: f.is_variable_bit_rate,
            length: f.length,
        },
    )?;
    let song = files::song(&f.filename)?;
    let track = Track {
        id: TrackId::of(&song.artist, &song.album, &song.title),
        title: song.title,
        artist_id: ArtistId::of(&song.artist),
        album_id: AlbumId::of(&song.album_artist, &song.album),
        artist: song.artist,
        album: song.album,
        year: song.year,
        no: song.no,
        disc: song.disc,
        duration: f.length,
        tags: Vec::new(),
        saved: false,
        file: Some(facts.clone()),
    };
    let source = Source {
        user: r.username.clone(),
        path: f.filename.clone(),
        size: f.size,
        ext,
        facts,
        upload_speed: r.upload_speed,
        free_slot: r.has_free_upload_slot,
        queue: u32::try_from(r.queue_length).unwrap_or(u32::MAX),
        length: f.length.filter(|l| *l > 0),
    };
    Some((track, source))
}

fn album_of(t: &Track) -> Album {
    Album {
        id: t.album_id.clone(),
        title: t.album.clone(),
        artist_id: t.artist_id.clone(),
        artist: t.artist.clone(),
        year: t.year,
        kind: Some("album".into()),
        released: t.year.map(|y| y.to_string()),
        label: None,
        tags: Vec::new(),
        saved: Saved::None,
        track_ids: Vec::new(),
        duration: None,
        musicbrainz_id: None,
    }
}

struct AlbumHit {
    album: Album,
    tracks: Vec<Track>,
    folders: Vec<FolderView>,
}

/// Folders holding enough of one album become album rows; the same album on many peers is one
/// row with each peer's folder listed, best first.
fn albums(folders: HashMap<(String, String), Folder>, sources: &Sources) -> Vec<AlbumHit> {
    let mut by_album: HashMap<AlbumId, AlbumHit> = HashMap::new();
    for f in folders.into_values().filter(|f| f.tracks.len() >= ALBUM_MIN_TRACKS) {
        let Some(best) = f.best.clone() else { continue };
        let ranked = sources.rank(best);
        let format = f
            .formats
            .iter()
            .max_by_key(|(_, n)| **n)
            .map(|(label, _)| label.clone())
            .unwrap_or_default();
        let view = FolderView {
            peer: f.user.clone(),
            path: f.path.clone(),
            format,
            tracks: f.tracks.len(),
            size_bytes: f.size,
            speed: ranked.speed,
            free_slot: ranked.free_slot,
            queue: ranked.queue,
            score: ranked.score,
            cover: f.cover,
        };
        let hit = by_album.entry(f.album.id.clone()).or_insert_with(|| AlbumHit {
            album: f.album.clone(),
            tracks: Vec::new(),
            folders: Vec::new(),
        });
        hit.folders.push(view);
        for t in f.tracks.into_values() {
            if !hit.tracks.iter().any(|known| known.id == t.id) {
                hit.tracks.push(t);
            }
        }
    }
    let mut hits: Vec<AlbumHit> = by_album
        .into_values()
        .map(|mut h| {
            h.folders.sort_by_key(|f| std::cmp::Reverse(f.score));
            h.folders.truncate(6);
            h.tracks.sort_by_key(|t| (t.disc.unwrap_or(1), t.no.unwrap_or(0)));
            h.album.track_ids = h.tracks.iter().map(|t| t.id.clone()).collect();
            h.album.duration = h.tracks.iter().map(|t| t.duration).sum();
            h
        })
        .collect();
    hits.sort_by(|a, b| {
        b.folders.len().cmp(&a.folders.len()).then(
            b.folders
                .first()
                .map(|f| f.score)
                .cmp(&a.folders.first().map(|f| f.score)),
        )
    });
    hits.truncate(MAX_ALBUMS);
    hits
}
