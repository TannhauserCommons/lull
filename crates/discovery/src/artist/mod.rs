//! `GET /api/artists/{id}`: an artist page as complete as a streaming service's. Who the artist
//! is comes first (the MBID a release named, else Last.fm's, else MusicBrainz's); then, side by
//! side, each in a task of its own:
//! - the catalog: MusicBrainz's links (tags, and the iTunes and Deezer artists it points to,
//!   which tell namesakes apart), iTunes's artist lookup, Deezer where it is open, and
//!   MusicBrainz's release groups, merged into one discography (`discography`);
//! - Last.fm's play counts, placed on the catalog's songs (`popular`);
//! - similar artists (Last.fm, else ListenBrainz);
//! - photos for the artist and the similar artists (`photo`), once the covers are known.
//!
//! The page answers within `PAGE_DEADLINE`. A section not in by then is named in `pending`, and
//! `artist.updated` is announced once every late section has ended, so the UI reads the page
//! again (from the caches the sections filled).

pub mod discography;
pub mod photo;
pub mod popular;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use axum::Json;
use axum::extract::{Path, State};
use lull_core::ids::{AlbumId, ArtistId, TrackId, normalize_artist};
use lull_core::{Error, Result};
use serde::Serialize;
use tokio::sync::oneshot;
use tokio::time::Instant;

use crate::Service;
use crate::art::{self, Pictured, PicturedArtist};
use crate::pages::{Errors, PAGE_DEADLINE, checked};
use crate::search::catalog::Choice;
use crate::search::merge::{AlbumHit, SongHit};
use crate::sources::listenbrainz::Mode;
use crate::sources::musicbrainz::Links;
use crate::sources::{ArtistInfo, Song};
use crate::store::{self, entities};
use discography::{Entry, Group};
use popular::Counts;

const SIMILAR: usize = 12;
/// Popular songs sent; the UI shows the first five.
const POPULAR: usize = 10;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtistPage {
    artist: PicturedArtist,
    /// The artist's most popular songs, best first.
    top_tracks: Vec<TrackId>,
    /// Last.fm's counts for the popular songs it knows.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    counts: BTreeMap<TrackId, Counts>,
    /// Where the popular songs' order comes from: lastfm, deezer, itunes or listenbrainz.
    #[serde(skip_serializing_if = "Option::is_none")]
    popular_via: Option<&'static str>,
    /// Every release, newest first.
    albums: Vec<AlbumId>,
    discography: Discography,
    similar: Vec<ArtistId>,
    /// Sections still being fetched; `artist.updated` follows when they are in.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pending: Vec<&'static str>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    errors: Errors,
    entities: Pictured,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct Discography {
    albums: Vec<AlbumId>,
    singles: Vec<AlbumId>,
    compilations: Vec<AlbumId>,
    appears_on: Vec<AlbumId>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ArtistUpdated {
    artist_id: ArtistId,
}

/// What the catalogs know of the artist.
#[derive(Default)]
struct Catalog {
    links: Links,
    releases: Vec<Entry>,
    /// Songs credited to the artist, most popular first: Deezer's top, then Apple's order.
    songs: Vec<SongHit>,
    songs_via: Option<&'static str>,
    /// How the store writes the name ("BIJI").
    store_name: Option<String>,
}

pub async fn page(State(svc): State<Arc<Service>>, Path(id): Path<String>) -> Result<Json<ArtistPage>> {
    let id = ArtistId(checked(id, ArtistId::PREFIX)?);
    let mut artist = svc
        .store
        .artist_ref(id.clone())
        .await?
        .ok_or_else(|| Error::NotFound(format!("Artist {id} is unknown to lull.")))?;
    artist.id = id.clone();
    let started = Instant::now();
    let until = started + PAGE_DEADLINE;
    let mut errors = Errors::new();
    let mut pending = Vec::new();
    let late = Late::new(&svc, id.clone());

    // Who the artist is: Last.fm's profile (asked by MBID when known) and, without an MBID,
    // MusicBrainz's artist of that name, side by side. With the MBID known, the sections don't
    // wait for them.
    let mut profile_rx = svc.lastfm().is_some().then(|| {
        let (s, name, mbid) = (svc.clone(), artist.name.clone(), artist.musicbrainz_id.clone());
        late.run(async move {
            match s.lastfm() {
                Some(lastfm) => lastfm.artist(&name, mbid.as_deref()).await,
                None => Err(off("Last.fm")),
            }
        })
    });
    let known = artist.musicbrainz_id.is_some();
    let mut profile = None;
    if !known {
        let found = {
            let (s, name) = (svc.clone(), artist.name.clone());
            late.run(async move { s.mb.find_artist(&name).await })
        };
        profile = match profile_rx.take() {
            Some(rx) => Some(wait(until, rx).await),
            None => None,
        };
        match wait(until, found).await {
            Ok(Some(a)) => artist.musicbrainz_id = a.mbid,
            Ok(None) => {}
            Err(Missing::Late) => pending.push("artist"),
            Err(Missing::Failed(e)) => {
                errors.insert("artist", e.to_string());
            }
        }
        if artist.musicbrainz_id.is_none()
            && let Some(Ok(p)) = &profile
        {
            artist.musicbrainz_id = p.mbid.clone();
            errors.remove("artist");
        }
    }
    let info = ArtistInfo {
        mbid: artist.musicbrainz_id.clone(),
        name: artist.name.clone(),
    };

    // Every section at once. The catalog and similar artists go on to the photos, which need
    // the covers and the similar artists' names.
    let (catalog_tx, catalog_rx) = oneshot::channel();
    let (similar_tx, similar_rx) = oneshot::channel();
    let (photos_tx, photos_rx) = oneshot::channel::<Result<()>>();
    late.open(3);
    {
        let (svc, late, info, artist) = (svc.clone(), late.clone(), info.clone(), artist.clone());
        svc.ctx.clone().spawn(async move {
            let (catalog, similar) = tokio::join!(
                async {
                    let c = catalog(&svc, &info).await;
                    let asked = c.as_ref().ok().map(|c| (covers(&c.releases), c.links.deezer));
                    late.deliver(catalog_tx, c);
                    asked
                },
                async {
                    let s = similar_artists(&svc, &info).await;
                    let names = s.as_ref().ok().cloned().unwrap_or_default();
                    late.deliver(similar_tx, s);
                    names
                }
            );
            let (covers, deezer) = catalog.unwrap_or_default();
            let mut asks = vec![photo::Ask { artist, deezer, covers }];
            asks.extend(similar.into_iter().map(|a| photo::Ask {
                artist: store::artist(a),
                deezer: None,
                covers: Vec::new(),
            }));
            photo::find(&svc, asks).await;
            late.deliver(photos_tx, Ok(()));
        });
    }
    let counted = svc.lastfm().is_some().then(|| {
        let (s, i) = (svc.clone(), info.clone());
        late.run(async move {
            match s.lastfm() {
                Some(lastfm) => lastfm.top_tracks(&i.name, i.mbid.as_deref()).await,
                None => Err(off("Last.fm")),
            }
        })
    });
    if known {
        profile = match profile_rx.take() {
            Some(rx) => Some(wait(until, rx).await),
            None => None,
        };
    }

    // A profile asked by name can be a namesake's: one naming another MBID isn't this artist's.
    match profile {
        Some(Ok(p)) => {
            let theirs = p.mbid.as_ref().zip(info.mbid.as_ref()).is_some_and(|(t, o)| t != o);
            if theirs {
                tracing::info!(artist = %artist.name, "Last.fm's artist of that name is another; its profile is left out");
            } else {
                artist.listeners = p.listeners;
                artist.bio = p.bio;
                artist.tags = p.tags;
            }
        }
        Some(Err(Missing::Late)) => pending.push("bio"),
        Some(Err(Missing::Failed(e))) => {
            errors.insert("bio", e.to_string());
        }
        None => {}
    }
    let catalog = match wait(until, catalog_rx).await {
        Ok(c) => Some(c),
        Err(Missing::Late) => {
            pending.extend(["albums", "topTracks"]);
            None
        }
        Err(Missing::Failed(e)) => {
            errors.insert("albums", e.to_string());
            None
        }
    };
    let counted = match counted {
        Some(rx) => match wait(until, rx).await {
            Ok(c) => c,
            Err(Missing::Late) => {
                pending.push("topTracks");
                Vec::new()
            }
            Err(Missing::Failed(e)) => {
                tracing::info!(error = %e, artist = %artist.name, "popular songs without Last.fm's counts");
                Vec::new()
            }
        },
        None => Vec::new(),
    };
    let similar = match wait(until, similar_rx).await {
        Ok(s) => s,
        Err(Missing::Late) => {
            pending.push("similar");
            Vec::new()
        }
        Err(Missing::Failed(e)) => {
            errors.insert("similar", e.to_string());
            Vec::new()
        }
    };
    if let Err(Missing::Late) = wait(until, photos_rx).await {
        pending.push("image");
    }
    pending.sort_unstable();
    pending.dedup();

    let catalog = catalog.unwrap_or_default();
    if let Some(name) = catalog
        .store_name
        .as_ref()
        .filter(|n| normalize_artist(n) == normalize_artist(&artist.name))
    {
        artist.name = name.clone();
    }
    if !catalog.links.tags.is_empty() {
        artist.tags = catalog.links.tags.clone();
    }

    // Popular songs, recorded with their releases so their covers show.
    let rows = popular::rank(&artist.name, &counted, &catalog.songs, POPULAR);
    let popular_via = match (rows.iter().any(|(_, c)| c.is_some()), rows.is_empty()) {
        (_, true) => None,
        (true, _) => Some("lastfm"),
        (false, _) => catalog.songs_via,
    };
    let on_albums: Vec<AlbumHit> = rows.iter().filter_map(|(s, _)| s.album_hit()).collect();
    remember(&svc, on_albums).await?;
    let tracks = svc.store.tracks(rows.iter().map(|(s, _)| s.song()).collect()).await?;
    let counts = tracks
        .iter()
        .zip(&rows)
        .filter_map(|(t, (_, c))| Some((t.id.clone(), (*c)?)))
        .collect();

    // The discography, merged and grouped.
    let grouped = discography::merge(&artist.name, catalog.releases);
    let albums = remember(&svc, grouped.iter().map(|(_, a)| a.clone()).collect()).await?;
    let mut disco = Discography::default();
    for ((group, _), album) in grouped.iter().zip(&albums) {
        let list = match group {
            Group::Albums => &mut disco.albums,
            Group::Singles => &mut disco.singles,
            Group::Compilations => &mut disco.compilations,
            Group::AppearsOn => &mut disco.appears_on,
        };
        list.push(album.id.clone());
    }

    let similar = svc
        .store
        .artists(similar.into_iter().take(SIMILAR).map(store::artist).collect())
        .await?;
    let mut all_artists = svc.store.artists(vec![artist.clone()]).await?;
    all_artists.extend(similar.iter().cloned());
    let top_tracks = tracks.iter().map(|t| t.id.clone()).collect();
    let album_ids = albums.iter().map(|a| a.id.clone()).collect();
    let similar = similar.iter().map(|a| a.id.clone()).collect();
    let entities = art::pictured(&svc, entities(tracks, albums, all_artists)).await?;
    tracing::info!(artist = %artist.name, ms = started.elapsed().as_millis() as u64, ?pending, "artist page answered");
    Ok(Json(ArtistPage {
        top_tracks,
        counts,
        popular_via,
        albums: album_ids,
        discography: disco,
        similar,
        pending,
        artist: entities
            .artists
            .get(&artist.id)
            .cloned()
            .unwrap_or(PicturedArtist { artist, image: None }),
        errors,
        entities,
    }))
}

/// Everything the catalogs know of the artist, the sources side by side. One failing leaves the
/// others; all failing is an error.
async fn catalog(svc: &Service, info: &ArtistInfo) -> Result<Catalog> {
    let open = svc.catalog.choice() == Choice::Deezer;
    let (stores, from_musicbrainz) = tokio::join!(
        async {
            let (links, itunes, deezer) = tokio::join!(
                async {
                    match &info.mbid {
                        Some(mbid) => svc.mb.artist_links(mbid).await,
                        None => Ok(Links::default()),
                    }
                },
                svc.itunes.artists(&info.name),
                async {
                    if open {
                        svc.deezer.artists_named(&info.name).await
                    } else {
                        Ok(Vec::new())
                    }
                }
            );
            let links = links.unwrap_or_else(|e| {
                tracing::info!(error = %e, "artist links from MusicBrainz missing");
                Links::default()
            });
            // The linked artist, else the store's best-known one of that name; Deezer's only
            // when it is the only one of that name.
            let itunes = itunes.map(|found| match links.itunes {
                Some(id) => found
                    .into_iter()
                    .find(|a| a.id == id)
                    .or(Some(crate::sources::itunes::StoreArtist {
                        id,
                        name: info.name.clone(),
                    })),
                None => found.into_iter().next(),
            });
            let deezer = deezer.map(|found| {
                links.deezer.or(match found.as_slice() {
                    [only] => Some(only.id),
                    _ => None,
                })
            });
            let (from_itunes, from_deezer) = tokio::join!(
                async {
                    match itunes {
                        Ok(Some(a)) => svc.itunes.artist_catalog(a.id).await.map(|c| Some((a.name, c))),
                        Ok(None) => Ok(None),
                        Err(e) => Err(e),
                    }
                },
                async {
                    match deezer {
                        Ok(Some(id)) if open => {
                            let (releases, top) =
                                tokio::join!(svc.deezer.artist_releases(id), svc.deezer.artist_top(id));
                            Ok(Some((releases?, top?)))
                        }
                        Ok(_) => Ok(None),
                        Err(e) => Err(e),
                    }
                }
            );
            (links, from_itunes, from_deezer)
        },
        svc.mb.discography(info)
    );
    let (links, from_itunes, from_deezer) = stores;
    let mut out = Catalog {
        links,
        ..Catalog::default()
    };
    let mut failed = Vec::new();
    match from_deezer {
        Ok(Some((releases, top))) => {
            out.releases.extend(releases);
            out.songs_via = (!top.is_empty()).then_some("deezer");
            out.songs.extend(top);
        }
        Ok(None) => {}
        Err(e) => failed.push(e),
    }
    match from_itunes {
        Ok(Some((name, c))) => {
            out.store_name = Some(name);
            out.releases.extend(c.releases);
            out.songs_via = out.songs_via.or((!c.songs.is_empty()).then_some("itunes"));
            out.songs.extend(
                c.songs
                    .into_iter()
                    .filter(|s| discography::credits(&s.artist, &info.name)),
            );
        }
        Ok(None) => {}
        Err(e) => failed.push(e),
    }
    match from_musicbrainz {
        Ok(releases) => out.releases.extend(
            releases
                .into_iter()
                .map(|r| discography::musicbrainz(r, info.mbid.as_deref())),
        ),
        Err(e) => failed.push(e),
    }
    if out.songs.is_empty()
        && let Some(mbid) = &info.mbid
    {
        match listenbrainz_top(svc, mbid).await {
            Ok(songs) if !songs.is_empty() => {
                out.songs = songs.into_iter().map(song_hit).collect();
                out.songs_via = Some("listenbrainz");
            }
            Ok(_) => {}
            Err(e) => tracing::info!(error = %e, "no popular songs from ListenBrainz"),
        }
    }
    if out.releases.is_empty()
        && let Some(e) = failed.pop()
    {
        return Err(e);
    }
    for e in failed {
        tracing::info!(error = %e, artist = %info.name, "a discography source failed; the page shows the others");
    }
    Ok(out)
}

/// The covers of the artist's own releases, newest first: what a Deezer "photo" is checked
/// against.
fn covers(releases: &[Entry]) -> Vec<String> {
    let mut own: Vec<&Entry> = releases.iter().filter(|e| e.own != Some(false)).collect();
    own.sort_by(|a, b| b.album.date.cmp(&a.album.date));
    let mut out: Vec<String> = Vec::new();
    for art in own.into_iter().filter_map(|e| e.album.art.clone()) {
        if !out.contains(&art) {
            out.push(art);
        }
    }
    out
}

/// Releases as albums, remembered with their covers and the catalog album behind each, so their
/// pages and pictures resolve.
async fn remember(svc: &Service, hits: Vec<AlbumHit>) -> Result<Vec<lull_core::model::Album>> {
    let albums = svc.store.albums(hits.iter().map(AlbumHit::release).collect()).await?;
    svc.store
        .remember_sources(
            albums
                .iter()
                .zip(&hits)
                .filter_map(|(a, h)| Some((a.id.clone(), h.source?)))
                .collect(),
        )
        .await?;
    svc.store
        .remember_pictures(
            albums.iter().cloned().zip(hits.into_iter().map(|h| h.art)).collect(),
            Vec::new(),
        )
        .await?;
    Ok(albums)
}

/// The artist's most played recordings on ListenBrainz (artist radio with no similar artists),
/// for an artist no catalog lists songs of.
async fn listenbrainz_top(svc: &Service, mbid: &str) -> Result<Vec<Song>> {
    let mut picks = svc.lb.artist_radio(mbid, Mode::Easy, 0, 30, (0, 100)).await?;
    picks.retain(|p| p.artist_mbid == mbid);
    picks.sort_by_key(|p| std::cmp::Reverse(p.listens));
    let mbids: Vec<String> = picks.into_iter().map(|p| p.recording_mbid).collect();
    let mut songs = svc.lb.songs(&mbids).await?;
    // The same song on several releases is one row.
    let mut seen = std::collections::HashSet::new();
    songs.retain(|s| seen.insert(lull_core::ids::normalize(&s.title)));
    songs.truncate(POPULAR);
    Ok(songs)
}

fn song_hit(s: Song) -> SongHit {
    SongHit {
        title: s.title,
        artist: s.artist,
        album: s.album,
        album_artist: s.album_artist,
        year: s.year,
        duration: s.duration_s,
        mbid: s.mbid,
        ..SongHit::default()
    }
}

/// Last.fm's similar artists when configured, else ListenBrainz's.
async fn similar_artists(svc: &Service, artist: &ArtistInfo) -> Result<Vec<ArtistInfo>> {
    let similar = match (svc.lastfm(), &artist.mbid) {
        (Some(lastfm), mbid) => lastfm.similar_artists(&artist.name, mbid.as_deref()).await?,
        (None, Some(mbid)) => svc.lb.similar_artists(mbid).await?,
        (None, None) => Vec::new(),
    };
    let similar = without_credits(&artist.name, similar);
    Ok(similar.into_iter().take(SIMILAR).collect())
}

/// Last.fm lists credit strings among similar artists ("Kalman Chodakiewitz, Eli Shiffman &
/// Petite Meller" beside Petite Meller): a name made of several artists, one of them listed
/// already or the page's own, isn't an artist of its own.
fn without_credits(artist: &str, similar: Vec<ArtistInfo>) -> Vec<ArtistInfo> {
    let listed: std::collections::HashSet<String> = similar
        .iter()
        .map(|a| normalize_artist(&a.name))
        .chain([normalize_artist(artist)])
        .collect();
    similar
        .into_iter()
        .filter(|a| {
            let names = crate::radio::credited_names(&a.name);
            names.len() < 3
                || !names
                    .iter()
                    .skip(1)
                    .any(|n| listed.contains(&normalize_artist(n)) && normalize_artist(n) != normalize_artist(&a.name))
        })
        .collect()
}

fn off(service: &'static str) -> Error {
    Error::unavailable(service, "it is switched off in Settings > Discovery")
}

/// The sections of one page build still running, and whether the page stopped waiting for any:
/// then `artist.updated` goes out once the last has ended.
#[derive(Clone)]
struct Late(Arc<LateInner>);

struct LateInner {
    svc: Arc<Service>,
    id: ArtistId,
    open: AtomicUsize,
    missed: AtomicBool,
}

impl Late {
    fn new(svc: &Arc<Service>, id: ArtistId) -> Self {
        Late(Arc::new(LateInner {
            svc: svc.clone(),
            id,
            open: AtomicUsize::new(0),
            missed: AtomicBool::new(false),
        }))
    }

    /// `n` more sections will be delivered.
    fn open(&self, n: usize) {
        self.0.open.fetch_add(n, Ordering::SeqCst);
    }

    /// Runs a section in a task of its own; its answer is cached even when the page stops waiting.
    fn run<T: Send + 'static>(
        &self,
        section: impl std::future::Future<Output = Result<T>> + Send + 'static,
    ) -> oneshot::Receiver<Result<T>> {
        let (tx, rx) = oneshot::channel();
        self.open(1);
        let late = self.clone();
        self.0.svc.ctx.spawn(async move { late.deliver(tx, section.await) });
        rx
    }

    fn deliver<T>(&self, tx: oneshot::Sender<T>, answer: T) {
        if tx.send(answer).is_err() {
            self.0.missed.store(true, Ordering::SeqCst);
        }
        if self.0.open.fetch_sub(1, Ordering::SeqCst) == 1 && self.0.missed.load(Ordering::SeqCst) {
            tracing::info!(artist = %self.0.id, "late artist sections are in");
            self.0.svc.ctx.bus.publish(
                "artist.updated",
                ArtistUpdated {
                    artist_id: self.0.id.clone(),
                },
            );
        }
    }
}

/// Why a section isn't on the page.
enum Missing {
    /// Still being fetched: `artist.updated` follows.
    Late,
    Failed(Error),
}

/// A section's answer by `until`.
async fn wait<T>(until: Instant, rx: oneshot::Receiver<Result<T>>) -> std::result::Result<T, Missing> {
    match tokio::time::timeout_at(until, rx).await {
        Ok(Ok(answer)) => answer.map_err(Missing::Failed),
        Ok(Err(_)) => Err(Missing::Failed(Error::unavailable("discovery", "is restarting"))),
        Err(_) => Err(Missing::Late),
    }
}

#[cfg(test)]
mod tests {
    use super::discography::{self, Group};
    use super::popular;
    use crate::sources::itunes::artist_catalog;

    #[test]
    fn credit_strings_are_not_similar_artists() {
        let names = [
            "Naser Razzazi",
            "Kalman Chodakiewitz, Eli Shiffman & Petite Meller",
            "Petite Meller",
            "Simon & Garfunkel",
        ];
        let similar = names
            .iter()
            .map(|n| crate::sources::ArtistInfo {
                mbid: None,
                name: (*n).into(),
            })
            .collect();
        let kept: Vec<String> = super::without_credits("Biji", similar)
            .into_iter()
            .map(|a| a.name)
            .collect();
        assert_eq!(kept, ["Naser Razzazi", "Petite Meller", "Simon & Garfunkel"]);
    }

    #[test]
    fn bijis_page_holds_everything_search_finds() {
        // Live, 2026-11-02: the page showed two singles; search found these.
        let albums = serde_json::from_str(include_str!("fixtures/itunes-artist-albums-biji.json"));
        let songs = serde_json::from_str(include_str!("fixtures/itunes-artist-songs-biji.json"));
        let (Ok(albums), Ok(songs)) = (albums, songs) else {
            unreachable!("the fixtures are JSON");
        };
        let catalog = artist_catalog(albums, songs, 1_689_784_894);
        let Ok(catalog) = catalog else {
            unreachable!("Apple's answer parses")
        };
        let grouped = discography::merge("BIJI", catalog.releases);
        let titles: Vec<&str> = grouped.iter().map(|(_, a)| a.title.as_str()).collect();
        for searched in [
            "BIJI, Pt. 1",
            "BIJI, Pt. 2",
            "Boya (from Cyberpunk: Edgerunners 2)",
            "Zo",
            "ESMERE",
            "Left Right",
            "AHA",
            "Be Shan",
            "LEGENDS",
        ] {
            assert!(titles.contains(&searched), "{searched} is on the page");
        }
        let appears: Vec<&str> = grouped
            .iter()
            .filter(|(g, _)| *g == Group::AppearsOn)
            .map(|(_, a)| a.title.as_str())
            .collect();
        assert!(appears.contains(&"Cyberpunk: Edgerunners 2 (Original Series Soundtrack)"));
        assert!(appears.contains(&"In Transit"), "Conducta's EP BIJI is on");
        assert!(!titles.iter().any(|t| t.contains("DJ Mix")), "DJ sets aren't releases");
        let first = grouped.first().map(|(g, a)| (*g, a.title.as_str()));
        assert_eq!(
            first,
            Some((Group::Singles, "Boya (from Cyberpunk: Edgerunners 2)")),
            "newest first"
        );

        let rows = popular::rank("BIJI", &[], &catalog.songs, 10);
        let popular: Vec<&str> = rows.iter().map(|(s, _)| s.title.as_str()).collect();
        for song in ["Zigidi", "Zo", "ESMERE"] {
            assert!(popular.contains(&song), "{song} is popular");
        }
        assert_eq!(
            rows.first().map(|(s, _)| s.album.as_str()),
            Some("Boya (from Cyberpunk: Edgerunners 2)"),
            "Boya leads, on BIJI's own single"
        );
    }
}
