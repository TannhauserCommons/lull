//! `GET /api/search?q=`: the catalog search (ADR-0008, docs/context/discovery.md#search).
//!
//! - **Sources** run side by side: the fast catalog (`catalog`: Deezer, or iTunes plus Deezer's
//!   artists where Deezer is closed) and the saved library, then MusicBrainz (one recording
//!   search) and Last.fm. Their rows are merged and ranked by `merge`.
//! - **Fast first:** the answer goes out once every fast source is in, or at 1.5 s (iTunes and
//!   Deezer take 0.6-1 s from the homelab), so its top result and order come from all of them.
//!   What arrives later is announced as `search.more`, with only rows not shown yet.
//! - **One run per query for 10 minutes:** the same words again answer at once, joining a run
//!   that is still going, with the run's list as shown so far (`Board`): everyone searching the
//!   same words sees the same rows in the same order. A run with a failed source isn't reused.
//! - **A submitted query** (`submitted=1`: Enter, or the rest that starts Soulseek) also asks
//!   MusicBrainz for release groups by title, so Bandcamp-only EPs and albums are found.
//! - **`search.catalog`** names the best tracks once every source is in, for Soulseek.

pub mod catalog;
mod matching;
pub mod merge;
mod playlists;
pub mod resolve;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex, PoisonError};
use std::time::Duration;

use axum::Json;
use axum::extract::{Query as Params, State};
use lull_core::ids::{AlbumId, ArtistId, TrackId};
use lull_core::model::Track;
use lull_core::{Error, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, Notify};
use tokio::time::Instant;

use crate::Service;
use crate::art::{self, Pictured};
use crate::store::{self, entities};
use catalog::Choice;
use merge::{Fit, Found, Kind, Merged, Query, Score, SongHit, Source, Touched};

/// The longest the first answer waits for the fast sources.
const FIRST_ANSWER: Duration = Duration::from_millis(1500);
const FAST: [Source; 4] = [Source::Deezer, Source::Itunes, Source::Library, Source::DeezerPlaylists];
const KEEP: Duration = Duration::from_secs(600);
/// The longest any one source is waited for.
const SOURCE_LIMIT: Duration = Duration::from_secs(8);
const TRACKS: usize = 50;
const ALBUMS: usize = 20;
const ARTISTS: usize = 20;
const SAVED: usize = 25;
/// Shorter words ask only Deezer and the library: MusicBrainz's one call a second is shared
/// with every lull crate.
const SLOW_SOURCES_FROM: usize = 3;
/// How many of a query's best tracks `search.catalog` names.
const CATALOG_TRACKS: usize = 50;

#[derive(Default)]
pub struct Searches {
    runs: StdMutex<HashMap<String, Arc<Run>>>,
}

struct Run {
    query: String,
    started: Instant,
    state: Mutex<RunState>,
    arrived: Notify,
    failed: AtomicBool,
    /// The release-group search has been started (`submitted`).
    albums_asked: AtomicBool,
}

#[derive(Default)]
struct RunState {
    merged: Merged,
    pending: BTreeSet<Source>,
    errors: BTreeMap<Source, String>,
    /// What answers and `search.more` have shown; `None` until the first answer.
    board: Option<Board>,
}

/// A playlist row of an answer (`playlists.rs`): not an entity, as it isn't in lull's catalog.
#[derive(Debug, Clone, Serialize)]
pub struct PlaylistRow {
    /// Its link key (`youtube:playlist:<id>`), what the preview and bookmarks take.
    id: String,
    site: String,
    title: String,
    owner: String,
    /// Same-origin, through `/api/catalog/art` (`art::playlist_picture`).
    cover: Option<String>,
    tracks: Option<u32>,
    fans: Option<u64>,
}

#[derive(Deserialize)]
pub struct SearchParams {
    q: String,
    /// `1` when the listener submitted the words (not just typed them).
    submitted: Option<String>,
}

#[derive(Serialize)]
pub struct Answer {
    query: String,
    top: Option<Top>,
    artists: Vec<ArtistId>,
    albums: Vec<AlbumId>,
    tracks: Vec<TrackId>,
    playlists: Vec<PlaylistRow>,
    pending: Vec<Source>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    errors: BTreeMap<Source, String>,
    entities: Pictured,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Top {
    kind: &'static str,
    id: String,
}

impl Top {
    fn of(kind: &'static str, id: String) -> Self {
        Top { kind, id }
    }
}

#[derive(Serialize)]
struct More<'a> {
    query: &'a str,
    artists: Vec<ArtistId>,
    albums: Vec<AlbumId>,
    tracks: Vec<TrackId>,
    playlists: Vec<PlaylistRow>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    errors: &'a BTreeMap<Source, String>,
    entities: Pictured,
    done: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Catalog<'a> {
    query: &'a str,
    track_ids: Vec<TrackId>,
}

pub async fn search(State(svc): State<Arc<Service>>, Params(p): Params<SearchParams>) -> Result<Json<Answer>> {
    let query = key(&p.q);
    if query.is_empty() {
        return Err(Error::BadRequest("Type something to search for.".into()));
    }
    let run = svc.searches.run(&svc, query);
    if p.submitted.as_deref().is_some_and(|s| s == "1" || s == "true") {
        run.ask_albums(&svc).await;
    }
    run.first_answer().await;
    let mut st = run.state.lock().await;
    let view = view(&svc, &run.query, &mut st, None).await?;
    st.merged.freeze();
    if st.pending.is_empty() {
        publish_catalog(&svc, &run.query, &st.merged);
    }
    tracing::info!(
        query = %run.query,
        ms = run.started.elapsed().as_millis() as u64,
        pending = ?st.pending,
        "search answered"
    );
    Ok(Json(Answer {
        query: run.query.clone(),
        top: view.top,
        artists: view.artists,
        albums: view.albums,
        tracks: view.tracks,
        playlists: view.playlists,
        pending: st.pending.iter().copied().collect(),
        errors: st.errors.clone(),
        entities: view.entities,
    }))
}

/// The words as runs are kept by: lowercase, single spaces.
fn key(q: &str) -> String {
    q.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

impl Searches {
    /// The run for `query`: a recent one, or a new one with its sources started.
    fn run(&self, svc: &Arc<Service>, query: String) -> Arc<Run> {
        let mut runs = self.runs.lock().unwrap_or_else(PoisonError::into_inner);
        runs.retain(|_, r| r.started.elapsed() < KEEP && !r.failed.load(Ordering::Relaxed));
        if let Some(run) = runs.get(&query) {
            return run.clone();
        }
        let sources = sources(svc, &query);
        let run = Arc::new(Run {
            query: query.clone(),
            started: Instant::now(),
            state: Mutex::new(RunState {
                pending: sources.iter().copied().collect(),
                ..RunState::default()
            }),
            arrived: Notify::new(),
            failed: AtomicBool::new(false),
            albums_asked: AtomicBool::new(false),
        });
        runs.insert(query, run.clone());
        drop(runs);
        for source in sources {
            start(svc, &run, source);
        }
        run
    }
}

/// Runs one source for the run. One that hasn't answered within `SOURCE_LIMIT` counts as failed,
/// so every run ends with `done: true`.
fn start(svc: &Arc<Service>, run: &Arc<Run>, source: Source) {
    let (svc, run) = (svc.clone(), run.clone());
    svc.ctx.clone().spawn(async move {
        let found = tokio::time::timeout(SOURCE_LIMIT, fetch(&svc, source, &run.query))
            .await
            .unwrap_or_else(|_| {
                Err(Error::unavailable(
                    source.service(),
                    format!("didn't answer within {} s", SOURCE_LIMIT.as_secs()),
                ))
            });
        arrive(&svc, &run, source, found).await;
    });
}

/// The sources in use: the fast catalog and the library; for 3+ characters also MusicBrainz
/// unless switched off, Last.fm when configured and on, and Deezer's and YouTube's playlists.
fn sources(svc: &Service, query: &str) -> Vec<Source> {
    let mut sources = vec![Source::Deezer, Source::Library];
    if svc.catalog.choice() == Choice::Itunes {
        sources.push(Source::Itunes);
    }
    let slow = query.chars().count() >= SLOW_SOURCES_FROM;
    if slow && svc.mb.switch.is_on() {
        sources.push(Source::MusicBrainz);
    }
    if slow && svc.lastfm().is_some() {
        sources.push(Source::LastFm);
    }
    if slow {
        sources.extend([Source::DeezerPlaylists, Source::YoutubePlaylists]);
    }
    sources
}

impl Run {
    /// Adds MusicBrainz's release groups by title to the run, once.
    async fn ask_albums(self: &Arc<Self>, svc: &Arc<Service>) {
        let worth_it = self.query.chars().count() >= SLOW_SOURCES_FROM && svc.mb.switch.is_on();
        if !worth_it || self.albums_asked.swap(true, Ordering::Relaxed) {
            return;
        }
        self.state.lock().await.pending.insert(Source::MusicBrainzAlbums);
        start(svc, self, Source::MusicBrainzAlbums);
    }

    /// Returns once every fast source is in, or at 1.5 s. Not sooner: an answer with only
    /// Deezer's artists in fixes the order and the top result before iTunes's songs and albums
    /// arrive ("blinding lights" led with a small artist of that name).
    async fn first_answer(&self) {
        let until = self.started + FIRST_ANSWER;
        loop {
            // Made before the check, so an arrival in between still wakes it.
            let arrived = self.arrived.notified();
            let waiting = self.state.lock().await.pending.iter().any(|s| FAST.contains(s));
            if !waiting || tokio::time::timeout_at(until, arrived).await.is_err() {
                return;
            }
        }
    }
}

async fn fetch(svc: &Service, source: Source, q: &str) -> Result<Found> {
    match source {
        Source::Deezer => {
            let found = match svc.catalog.choice() {
                Choice::Deezer => svc.deezer.search(q).await,
                Choice::Itunes => svc.deezer.artists(q).await,
            };
            catalog::report(
                svc,
                match &found {
                    Ok(f) => f.problem.as_ref().map(|p| format!("Deezer {p}")),
                    Err(e) => Some(format!("Search goes on without Deezer: {e}")),
                },
            );
            found
        }
        Source::Itunes => {
            let found = svc.itunes.search(q).await;
            if let Err(e) = &found {
                catalog::report(svc, Some(format!("Search goes on without iTunes: {e}")));
            }
            found
        }
        Source::Library => Ok(Found {
            songs: svc.store.search_saved(q, SAVED).await?.into_iter().map(saved).collect(),
            ..Found::default()
        }),
        Source::MusicBrainz => {
            let mut found = svc.mb.search(q).await?;
            // Its query is broad; rows whose words don't match are someone else's. An album
            // holding most of them stays (a soundtrack named without the "season" typed).
            let query = Query::new(q);
            found.songs.retain(|s| query.song(s) >= Fit::Words);
            found.albums.retain(|a| query.album(a) >= Fit::Most);
            found.artists.retain(|a| query.artist(a) >= Fit::Words);
            Ok(found)
        }
        Source::MusicBrainzAlbums => {
            let mut found = svc.mb.search_releases(q).await?;
            let query = Query::new(q);
            found.albums.retain(|a| query.album(a) >= Fit::Most);
            Ok(found)
        }
        Source::LastFm => match svc.lastfm() {
            Some(lastfm) => lastfm.search(q).await,
            None => Ok(Found::default()),
        },
        Source::DeezerPlaylists => Ok(Found {
            playlists: svc.deezer.playlists(q).await?,
            ..Found::default()
        }),
        Source::YoutubePlaylists => Ok(Found {
            playlists: youtube_playlists(svc, q).await?,
            ..Found::default()
        }),
    }
}

/// YouTube's playlists for `q`, kept a day: a yt-dlp run takes seconds, so a query asked again
/// gets them in its first answer.
async fn youtube_playlists(svc: &Service, q: &str) -> Result<Vec<merge::PlaylistHit>> {
    svc.cache
        .get(format!("yt:playlists:{q}"), crate::sources::cache::DAY, || async {
            let found = svc.youtube.search_playlists(q, playlists::LISTED).await?;
            Ok(found
                .into_iter()
                .enumerate()
                .map(|(i, p)| merge::PlaylistHit {
                    site: "youtube".into(),
                    id: p.id,
                    title: p.title,
                    owner: p.owner,
                    cover: p.thumbnails.into_iter().next(),
                    tracks: None,
                    fans: None,
                    rank: i as u32,
                })
                .collect())
        })
        .await
}

fn saved(t: Track) -> SongHit {
    SongHit {
        title: t.title.clone(),
        artist: t.artist.clone(),
        album: t.album.clone(),
        year: t.year,
        duration: t.duration,
        saved: Some(t),
        ..SongHit::default()
    }
}

/// Merges a source's answer; after the first answer, announces what it added.
async fn arrive(svc: &Service, run: &Run, source: Source, found: Result<Found>) {
    let mut st = run.state.lock().await;
    st.pending.remove(&source);
    let touched = match found {
        Ok(mut found) => {
            if let Some(problem) = found.problem.take() {
                st.errors.insert(source, problem);
            }
            st.merged.add(found)
        }
        Err(e) => {
            tracing::info!(query = %run.query, ?source, error = %e, "a search source failed");
            run.failed.store(true, Ordering::Relaxed);
            st.errors.insert(source, e.to_string());
            Touched::default()
        }
    };
    if st.board.is_some() {
        match view(svc, &run.query, &mut st, Some(&touched)).await {
            Ok(v) => svc.ctx.bus.publish(
                "search.more",
                More {
                    query: &run.query,
                    artists: v.artists,
                    albums: v.albums,
                    tracks: v.tracks,
                    playlists: v.playlists,
                    errors: &st.errors,
                    entities: v.entities,
                    done: st.pending.is_empty(),
                },
            ),
            Err(e) => tracing::warn!(query = %run.query, error = %e, "search additions not announced"),
        }
        st.merged.freeze();
        if st.pending.is_empty() {
            publish_catalog(svc, &run.query, &st.merged);
        }
    }
    drop(st);
    run.arrived.notify_waiters();
}

/// `search.catalog`: the query's best tracks, for Soulseek's pre-warming.
fn publish_catalog(svc: &Service, query: &str, merged: &Merged) {
    let q = Query::new(query);
    let fits: Vec<Score> = merged.songs.iter().map(|s| Score::song(&q, s)).collect();
    let track_ids = merge::rank(&fits, |i| merged.songs[i].saved.is_some())
        .into_iter()
        .take(CATALOG_TRACKS)
        .map(|i| merged.songs[i].id())
        .collect();
    svc.ctx.bus.publish("search.catalog", Catalog { query, track_ids });
}

/// What one answer or announcement holds.
struct View {
    top: Option<Top>,
    artists: Vec<ArtistId>,
    albums: Vec<AlbumId>,
    tracks: Vec<TrackId>,
    playlists: Vec<PlaylistRow>,
    entities: Pictured,
}

/// The rows of an answer or an announcement (`touched`), with their entities. The run's first
/// answer fixes its top result and order (`Board`); an announcement lists only rows not on the
/// board, which go after it, and its entities also carry shown rows that gained facts. Every
/// later answer is the board as it stands.
async fn view(svc: &Service, query: &str, st: &mut RunState, touched: Option<&Touched>) -> Result<View> {
    let RunState { merged: m, board, .. } = st;
    let rows = match (board.as_ref(), touched) {
        (Some(shown), None) => shown.clone(),
        _ => {
            let (liked, followed) = svc.store.own().await?;
            let ranked = rank_rows(&Query::new(query), m, &liked, &followed);
            match board {
                Some(shown) => shown.add(ranked),
                None => board.insert(Board::first(ranked)).clone(),
            }
        }
    };
    let (mut d_songs, mut d_albums, mut d_artists) = (rows.songs.clone(), rows.albums.clone(), rows.artists.clone());
    d_albums.extend(rows.top_album.filter(|i| !rows.albums.contains(i)));
    if let (Some(t), Some(shown)) = (touched, board.as_ref()) {
        let improved = |touched: &[usize], listed: &[usize], on_board: &[usize]| -> Vec<usize> {
            touched
                .iter()
                .copied()
                .filter(|i| on_board.contains(i) && !listed.contains(i))
                .collect()
        };
        d_songs.extend(improved(&t.songs, &rows.songs, &shown.songs));
        d_albums.extend(improved(&t.albums, &rows.albums, &shown.albums));
        d_artists.extend(improved(&t.artists, &rows.artists, &shown.artists));
    }

    let entities = describe(svc, m, &d_songs, &d_albums, &d_artists).await?;
    let mut playlists = Vec::with_capacity(rows.playlists.len());
    for &i in &rows.playlists {
        let p = &m.playlists[i];
        let cover = match &p.cover {
            Some(source) => Some(art::playlist_picture(svc, &p.link(), source).await?),
            None => None,
        };
        playlists.push(PlaylistRow {
            id: p.link(),
            site: p.site.clone(),
            title: p.title.clone(),
            owner: p.owner.clone(),
            cover,
            tracks: p.tracks,
            fans: p.fans,
        });
    }
    Ok(View {
        top: rows.top,
        playlists,
        tracks: rows.songs.iter().map(|&i| m.songs[i].id()).collect(),
        albums: rows.albums.iter().map(|&i| m.albums[i].id()).collect(),
        artists: rows.artists.iter().map(|&i| m.artists[i].id()).collect(),
        entities,
    })
}

/// Every row so far, best first (positions in `Merged`), and the top result.
#[derive(Debug, Default)]
struct Ranked {
    top: Option<Top>,
    /// The album the top result is.
    top_album: Option<usize>,
    songs: Vec<usize>,
    albums: Vec<usize>,
    artists: Vec<usize>,
    playlists: Vec<usize>,
}

/// Ranks the merged rows for the query; `liked` and `followed` are the listener's own, which
/// break ties (`merge`). Misspelt artists are left out.
fn rank_rows(q: &Query, m: &Merged, liked: &HashSet<TrackId>, followed: &HashSet<ArtistId>) -> Ranked {
    let fits: Vec<Score> = m.songs.iter().map(|s| Score::song(q, s)).collect();
    let songs = merge::rank(&fits, |i| {
        let s = &m.songs[i];
        s.saved.is_some() || liked.contains(&s.id()) || followed.contains(&ArtistId::of(&s.artist))
    });
    let album_fits: Vec<Score> = m.albums.iter().map(|a| Score::album(q, a)).collect();
    let albums = merge::rank(&album_fits, |i| followed.contains(&ArtistId::of(&m.albums[i].artist)));
    let mut artist_fits: Vec<Score> = m.artists.iter().map(|a| Score::artist(q, a)).collect();
    merge::overshadow(&m.artists, &mut artist_fits);
    let misspelt = merge::misspelt(&m.artists);
    let mut artists = merge::rank(&artist_fits, |i| followed.contains(&m.artists[i].id()));
    artists.retain(|&i| !misspelt[i]);

    // A query that names a film, show or game: its soundtrack, album or playlist, leads.
    let ranked_albums: Vec<(usize, &merge::AlbumHit)> = albums.iter().map(|&i| (i, &m.albums[i])).collect();
    let (top, top_album) = match playlists::work_top(q, &ranked_albums, &m.playlists) {
        Some(playlists::WorkTop::Album(i)) => (Some(Top::of("album", m.albums[i].id().0)), Some(i)),
        Some(playlists::WorkTop::Playlist(i)) => (Some(Top::of("playlist", m.playlists[i].link())), None),
        None => {
            let kind = merge::top(
                artists.first().map(|&i| artist_fits[i]),
                albums.first().map(|&i| album_fits[i]),
                songs.first().map(|&i| fits[i]),
            );
            let top = kind.map(|kind| match kind {
                Kind::Artist => Top::of("artist", m.artists[artists[0]].id().0),
                Kind::Album => Top::of("album", m.albums[albums[0]].id().0),
                Kind::Track => Top::of("track", m.songs[songs[0]].id().0),
            });
            (top, None)
        }
    };
    Ranked {
        top,
        top_album,
        songs,
        albums,
        artists,
        playlists: playlists::rank(q, &m.playlists),
    }
}

/// The rows a run has shown (positions in `Merged`) in the order shown: the first answer's best,
/// then each announcement's after them. A later answer is the board as it stands, so two people
/// searching the same words see one list, and a source arriving late moves nothing.
#[derive(Debug, Clone, Default, PartialEq)]
struct Board {
    top: Option<Top>,
    top_album: Option<usize>,
    songs: Vec<usize>,
    albums: Vec<usize>,
    artists: Vec<usize>,
    playlists: Vec<usize>,
}

impl Board {
    /// The first answer: the best rows of each kind, and the top result.
    fn first(r: Ranked) -> Board {
        Board {
            top: r.top,
            top_album: r.top_album,
            songs: r.songs.into_iter().take(TRACKS).collect(),
            albums: r.albums.into_iter().take(ALBUMS).collect(),
            artists: r.artists.into_iter().take(ARTISTS).collect(),
            playlists: r.playlists.into_iter().take(playlists::LISTED).collect(),
        }
    }

    /// An announcement: the best rows not on the board yet, put after it. Its top result stays.
    fn add(&mut self, r: Ranked) -> Board {
        fn new(order: Vec<usize>, shown: &mut Vec<usize>, limit: usize) -> Vec<usize> {
            let added: Vec<usize> = order.into_iter().filter(|i| !shown.contains(i)).take(limit).collect();
            shown.extend(&added);
            added
        }
        Board {
            top: None,
            top_album: None,
            songs: new(r.songs, &mut self.songs, TRACKS),
            albums: new(r.albums, &mut self.albums, ALBUMS),
            artists: new(r.artists, &mut self.artists, ARTISTS),
            playlists: new(r.playlists, &mut self.playlists, playlists::LISTED),
        }
    }
}

/// Entities for rows: tracks recorded in the catalog with saved state, albums (the rows' and
/// the tracks') and artists remembered with their pictures, so their ids resolve later.
async fn describe(svc: &Service, m: &Merged, songs: &[usize], albums: &[usize], artists: &[usize]) -> Result<Pictured> {
    let mut tracks: Vec<Track> = songs.iter().filter_map(|&i| m.songs[i].saved.clone()).collect();
    let found: Vec<_> = songs
        .iter()
        .filter(|&&i| m.songs[i].saved.is_none())
        .map(|&i| m.songs[i].song())
        .collect();
    tracks.extend(svc.store.tracks(found).await?);

    let album_hits: Vec<_> = albums
        .iter()
        .map(|&i| m.albums[i].clone())
        .chain(songs.iter().filter_map(|&i| m.songs[i].album_hit()))
        .collect();
    let album_rows = svc
        .store
        .albums(album_hits.iter().map(merge::AlbumHit::release).collect())
        .await?;
    svc.store
        .remember_sources(
            album_rows
                .iter()
                .zip(&album_hits)
                .filter_map(|(row, hit)| Some((row.id.clone(), hit.source?)))
                .collect(),
        )
        .await?;
    let artist_hits: Vec<_> = artists.iter().map(|&i| m.artists[i].clone()).collect();
    let artist_rows = svc
        .store
        .artists(artist_hits.iter().map(|a| store::artist(a.info())).collect())
        .await?;
    svc.store
        .remember_pictures(
            album_rows
                .iter()
                .cloned()
                .zip(album_hits.into_iter().map(|a| a.art))
                .collect(),
            artist_rows
                .iter()
                .cloned()
                .zip(artist_hits.into_iter().map(|a| a.image))
                .collect(),
        )
        .await?;
    art::pictured(svc, entities(tracks, album_rows, artist_rows)).await
}
