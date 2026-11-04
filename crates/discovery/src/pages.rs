//! `GET /api/albums/{id}` and `/api/tags/{id}` (the artist page is `artist`). An id is resolved
//! through what discovery has shown before or the catalog; the page is then filled from
//! MusicBrainz, ListenBrainz and (when configured) Last.fm. A section whose source failed is
//! empty and named in `errors`, so the rest of the page still shows.
//!
//! An album a catalog found (iTunes, Deezer) takes its tracklist from that catalog in one call;
//! MusicBrainz, paced and shared with every lull crate, then only adds its facts in the
//! background. The album page answers within `ALBUM_DEADLINE` with what lull has, and
//! announces the full tracklist as `album.updated` when it arrives later.

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::Json;
use axum::extract::{Path, State};
use lull_core::ids::{AlbumId, ArtistId};
use lull_core::model::Album;
use lull_core::model::Track;
use lull_core::{Error, Result};
use serde::Serialize;
use tokio::sync::oneshot;
use tokio::time::Instant;

use crate::art::{self, Pictured};
use crate::artist::photo;
use crate::sources::{AlbumSource, ArtistInfo, Release, Song, top_tags};
use crate::store::{self, entities};
use crate::{Service, taste};

const TAG_ARTISTS: usize = 12;
const TAG_RELEASES: usize = 12;
/// "New" on a tag page: first released within this many days.
const TAG_NEW_DAYS: i64 = 90;

/// How long the album page waits for its tracklist before answering with what lull has.
const ALBUM_DEADLINE: Duration = Duration::from_secs(3);
/// How long the artist and tag pages wait for their sections.
pub(crate) const PAGE_DEADLINE: Duration = Duration::from_secs(4);
/// How long a section a page answered without may go on: longer than any one source's own bound
/// (iTunes waits at most its budget's minute for a turn). Then it ends as failed, so the page's
/// `*.updated` always follows (api.md).
pub(crate) const LATE_LIMIT: Duration = Duration::from_secs(90);

/// `section`, ended as failed once it has run `LATE_LIMIT`.
pub(crate) async fn bounded<T>(section: impl Future<Output = Result<T>>) -> Result<T> {
    match tokio::time::timeout(LATE_LIMIT, section).await {
        Ok(answer) => answer,
        Err(_) => Err(Error::unavailable(
            "this part",
            format!("didn't arrive within {} s", LATE_LIMIT.as_secs()),
        )),
    }
}

/// How each page section went the last time it ended, when it failed, by page and section. A
/// page read again (after `*.updated`) names such a section in `errors` when it is late again,
/// rather than as still coming: so a section whose source keeps failing slowly ends in "couldn't
/// load", never in an endless skeleton. A section that arrives clears it.
#[derive(Default)]
pub struct Unfinished(Mutex<HashMap<(String, &'static str), String>>);

impl Unfinished {
    /// Records how a section went when it ended.
    pub fn ended<T>(&self, page: &str, sections: &[&'static str], answer: &Result<T>) {
        let mut failed = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        for section in sections {
            let key = (page.to_owned(), *section);
            match answer {
                Ok(_) => failed.remove(&key),
                Err(e) => failed.insert(key, e.to_string()),
            };
        }
    }

    /// Why the section failed the last time it ended, if it did.
    pub fn failed(&self, page: &str, section: &'static str) -> Option<String> {
        let failed = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        failed.get(&(page.to_owned(), section)).cloned()
    }

    /// A section not in by the page's deadline: still coming (`pending`), or, when it failed
    /// the last time it ended, in `errors`.
    pub fn late(&self, page: &str, section: &'static str, pending: &mut Vec<&'static str>, errors: &mut Errors) {
        match self.failed(page, section) {
            Some(why) => {
                errors.insert(section, why);
            }
            None => pending.push(section),
        }
    }
}

pub(crate) type Errors = BTreeMap<&'static str, String>;

#[derive(Serialize)]
pub struct AlbumPage {
    album: Album,
    /// The tracklist wasn't in by the deadline: the tracks lull had are shown, and
    /// `album.updated` follows when it has arrived or failed.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    partial: bool,
    /// `tracks` when the tracklist failed the last time it was fetched and is late again.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    errors: Errors,
    entities: Pictured,
}

/// An album with its tracklist, and whether a catalog (not MusicBrainz) gave it.
struct Filled {
    album: Album,
    tracks: Vec<Track>,
    from_catalog: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AlbumUpdated {
    album_id: AlbumId,
}

pub async fn album(State(svc): State<Arc<Service>>, Path(id): Path<String>) -> Result<Json<AlbumPage>> {
    let id = AlbumId(checked(id, AlbumId::PREFIX)?);
    let mut known = svc
        .store
        .album_ref(id.clone())
        .await?
        .ok_or_else(|| Error::NotFound(format!("Album {id} is unknown to lull.")))?;
    known.id = id.clone();

    // Filled in a task of its own, so a tracklist that misses the deadline is still fetched,
    // remembered and announced: arrived or failed, within `LATE_LIMIT`.
    let started = Instant::now();
    let (tx, rx) = oneshot::channel();
    let task = (svc.clone(), known.clone());
    svc.ctx.spawn(async move {
        let (svc, known) = task;
        let album_id = known.id.clone();
        let filled = bounded(fill(&svc, known)).await;
        // A page read while it had failed already says so: failing again is no news for it.
        let news = filled.is_ok() || svc.unfinished.failed(&album_id.0, "tracks").is_none();
        svc.unfinished.ended(&album_id.0, &["tracks"], &filled);
        let to_enrich = filled.as_ref().ok().filter(|f| f.from_catalog).map(|f| f.album.clone());
        if tx.send(filled).is_err() && news {
            tracing::info!(album = %album_id, ms = started.elapsed().as_millis() as u64, "album tracklist ended after the page");
            svc.ctx.bus.publish("album.updated", AlbumUpdated { album_id });
        }
        if let Some(album) = to_enrich {
            enrich(&svc, album).await;
        }
    });
    let mut errors = Errors::new();
    let (album, tracks, partial) = match tokio::time::timeout(ALBUM_DEADLINE, rx).await {
        Ok(Ok(filled)) => {
            let f = filled?;
            (f.album, f.tracks, false)
        }
        Ok(Err(_)) => return Err(Error::unavailable("discovery", "is restarting")),
        Err(_) => {
            let tracks = svc.store.catalog_album_tracks(id.clone()).await?;
            let mut late = Vec::new();
            svc.unfinished.late(&id.0, "tracks", &mut late, &mut errors);
            (store::with_tracks(known, &tracks), tracks, !late.is_empty())
        }
    };
    tracing::info!(album = %id, ms = started.elapsed().as_millis() as u64, partial, "album page answered");
    let artist = svc
        .store
        .artists(vec![store::artist(ArtistInfo {
            mbid: None,
            name: album.artist.clone(),
        })])
        .await?;
    Ok(Json(AlbumPage {
        entities: art::pictured(&svc, entities(tracks, vec![album.clone()], artist)).await?,
        partial,
        errors,
        album,
    }))
}

/// The album's tracklist, remembered with it: from the catalog that found it, else from
/// MusicBrainz, else the catalog tracks lull already has of it.
async fn fill(svc: &Service, known: Album) -> Result<Filled> {
    let id = known.id.clone();
    if let Some(source) = svc.store.album_source(id.clone()).await? {
        match from_catalog(svc, source, &known).await {
            Ok(songs) if !songs.is_empty() => {
                let tracks = svc.store.tracks(songs).await?;
                let album = svc.store.full_album(known, &tracks).await?;
                return Ok(Filled {
                    album,
                    tracks,
                    from_catalog: true,
                });
            }
            Ok(_) => tracing::info!(album = %id, ?source, "the catalog lists no tracks for it"),
            Err(e) => tracing::info!(error = %e, album = %id, ?source, "album tracklist from MusicBrainz instead"),
        }
    }
    let (mut album, songs) = match from_musicbrainz(svc, &known).await {
        Ok(Some(found)) => found,
        Ok(None) => (known, Vec::new()),
        Err(e) => {
            tracing::info!(error = %e, album = %id, "album from the catalog only");
            (known, Vec::new())
        }
    };
    album.id = id.clone();
    let tracks = if songs.is_empty() {
        svc.store.catalog_album_tracks(id).await?
    } else {
        svc.store.tracks(songs).await?
    };
    Ok(Filled {
        album: svc.store.full_album(album, &tracks).await?,
        tracks,
        from_catalog: false,
    })
}

/// The tracklist of the catalog album behind `known`, as songs of `known` (its title and artist,
/// so the tracks carry its id).
async fn from_catalog(svc: &Service, source: AlbumSource, known: &Album) -> Result<Vec<Song>> {
    let songs = match source {
        AlbumSource::Itunes(id) => svc.itunes.tracklist(id).await?,
        AlbumSource::Deezer(id) => svc.deezer.tracklist(id).await?,
    };
    Ok(songs
        .into_iter()
        .map(|s| Song {
            album: known.title.clone(),
            album_artist: Some(known.artist.clone()),
            album_mbid: known.musicbrainz_id.clone(),
            year: known.year.or(s.year),
            tags: known.tags.clone(),
            ..s
        })
        .collect())
}

/// Adds what MusicBrainz knows of an album a catalog gave (its MBID, tags, type, date).
async fn enrich(svc: &Service, album: Album) {
    if !svc.mb.switch.is_on() {
        return;
    }
    let group = match &album.musicbrainz_id {
        Some(mbid) => svc.mb.release(mbid).await.map(Some),
        None => svc.mb.find_release(&album.artist, &album.title).await,
    };
    let outcome = match group {
        Ok(Some(group)) => svc.store.enrich_album(album.id.clone(), group).await,
        Ok(None) => Ok(()),
        Err(e) => Err(e),
    };
    if let Err(e) = outcome {
        tracing::info!(error = %e, album = %album.id, "album not enriched from MusicBrainz");
    }
}

/// The album's release group and tracklist, when MusicBrainz knows it.
async fn from_musicbrainz(svc: &Service, known: &Album) -> Result<Option<(Album, Vec<Song>)>> {
    let group = match &known.musicbrainz_id {
        Some(mbid) => Some(svc.mb.release(mbid).await?),
        None => svc.mb.find_release(&known.artist, &known.title).await?,
    };
    let Some(group) = group else { return Ok(None) };
    let songs = svc.mb.tracklist(&group).await?;
    Ok(Some((store::album(group), songs)))
}

/// Runs a page section in a task of its own, so a fetch that misses the page's deadline still
/// ends and fills the cache for the next visit.
fn background<T: Send + 'static>(
    svc: &Service,
    section: impl Future<Output = Result<T>> + Send + 'static,
) -> oneshot::Receiver<Result<T>> {
    let (tx, rx) = oneshot::channel();
    svc.ctx.spawn(async move {
        // The page stopped waiting: the answer is cached all the same.
        let _ = tx.send(section.await);
    });
    rx
}

/// A section's answer by `until`, or why it is missing.
async fn by<T>(until: Instant, section: oneshot::Receiver<Result<T>>) -> Result<T> {
    match tokio::time::timeout_at(until, section).await {
        Ok(Ok(answer)) => answer,
        Ok(Err(_)) => Err(Error::unavailable("discovery", "is restarting")),
        Err(_) => Err(Error::unavailable(
            "this section",
            format!(
                "took over {} s; it shows when the page is opened again",
                PAGE_DEADLINE.as_secs()
            ),
        )),
    }
}

#[derive(Serialize)]
pub struct TagInfo {
    id: String,
    name: String,
    desc: Option<String>,
    related: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TagPage {
    tag: TagInfo,
    artists: Vec<ArtistId>,
    new_releases: Vec<AlbumId>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    errors: Errors,
    entities: Pictured,
}

pub async fn tag(State(svc): State<Arc<Service>>, Path(id): Path<String>) -> Result<Json<TagPage>> {
    let tag = id.trim().to_lowercase();
    if tag.is_empty() {
        return Err(Error::BadRequest("a tag needs a name".into()));
    }
    let until = Instant::now() + PAGE_DEADLINE;
    let mut errors = Errors::new();
    let since = iso_date(lull_core::now() - TAG_NEW_DAYS * 86_400);
    let releases = {
        let (s, t) = (svc.clone(), tag.clone());
        background(&svc, async move { s.mb.tagged_releases(&t, &since).await })
    };
    let lastfm = svc.lastfm().map(|_| {
        let (s, t) = (svc.clone(), tag.clone());
        background(&svc, async move {
            let Some(lastfm) = s.lastfm() else {
                return Err(Error::unavailable(
                    "Last.fm",
                    "it is switched off in Settings > Discovery",
                ));
            };
            let (desc, related, artists) = tokio::join!(
                lastfm.tag_description(&t),
                lastfm.similar_tags(&t),
                lastfm.tag_artists(&t)
            );
            Ok((desc, related, artists))
        })
    });
    let radio = lastfm.is_none().then(|| {
        let (s, t) = (svc.clone(), tag.clone());
        background(&svc, async move { tag_artists_from_radio(&s, &t).await })
    });

    let mut releases = section(&mut errors, "newReleases", by(until, releases).await).unwrap_or_default();
    let co_tagged = co_tags(&tag, &releases);
    releases.truncate(TAG_RELEASES);
    let (desc, related, artists) = match (lastfm, radio) {
        (Some(rx), _) => match section(&mut errors, "artists", by(until, rx).await) {
            Some((desc, related, artists)) => (
                section(&mut errors, "desc", desc).flatten(),
                section(&mut errors, "related", related).unwrap_or_default(),
                section(&mut errors, "artists", artists).unwrap_or_default(),
            ),
            None => (None, Vec::new(), Vec::new()),
        },
        (None, Some(rx)) => (
            None,
            Vec::new(),
            section(&mut errors, "artists", by(until, rx).await).unwrap_or_default(),
        ),
        (None, None) => (None, Vec::new(), Vec::new()),
    };
    let related = if related.is_empty() { co_tagged } else { related };
    if errors.is_empty() && desc.is_none() && releases.is_empty() && artists.is_empty() && related.is_empty() {
        return Err(Error::NotFound(format!("No source knows the tag {tag}.")));
    }

    let albums = svc.store.albums(releases).await?;
    let artists = svc
        .store
        .artists(artists.into_iter().take(TAG_ARTISTS).map(store::artist).collect())
        .await?;
    // Their photos, as on artist pages (found once, then kept); what misses the deadline shows
    // next time.
    let photos = {
        let (s, asks) = (
            svc.clone(),
            artists
                .iter()
                .map(|a| photo::Ask {
                    artist: a.clone(),
                    deezer: None,
                    covers: Vec::new(),
                })
                .collect(),
        );
        background(&svc, async move {
            photo::find(&s, asks).await;
            Ok(())
        })
    };
    let _ = by(until, photos).await;
    Ok(Json(TagPage {
        tag: TagInfo {
            id: tag.clone(),
            name: tag,
            desc,
            related: related.into_iter().take(8).collect(),
        },
        artists: artists.iter().map(|a| a.id.clone()).collect(),
        new_releases: albums.iter().map(|a| a.id.clone()).collect(),
        errors,
        entities: art::pictured(&svc, entities(Vec::new(), albums, artists)).await?,
    }))
}

/// Without Last.fm: the artists behind the tag's well-played recordings on ListenBrainz.
async fn tag_artists_from_radio(svc: &Service, tag: &str) -> Result<Vec<ArtistInfo>> {
    let mbids = svc.lb.tag_radio(tag, (50, 100), 50).await?;
    let songs = svc.lb.songs(&mbids).await?;
    let mut counts: HashMap<String, (usize, Option<String>)> = HashMap::new();
    for s in songs {
        let e = counts.entry(s.artist).or_insert((0, s.artist_mbid));
        e.0 += 1;
    }
    let mut ranked: Vec<_> = counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.0.cmp(&a.1.0).then_with(|| a.0.cmp(&b.0)));
    Ok(ranked
        .into_iter()
        .map(|(name, (_, mbid))| ArtistInfo { mbid, name })
        .collect())
}

/// Specific tags that appear alongside `tag` on at least two of its releases, most often first.
fn co_tags(tag: &str, releases: &[Release]) -> Vec<String> {
    let mut counts: HashMap<&str, i64> = HashMap::new();
    for t in releases.iter().flat_map(|r| &r.tags) {
        if t != tag && taste::is_specific(t) {
            *counts.entry(t).or_default() += 1;
        }
    }
    top_tags(
        counts
            .into_iter()
            .filter(|(_, n)| *n >= 2)
            .map(|(t, n)| (t.to_owned(), n))
            .collect(),
        8,
    )
}

/// Keeps a section's value, or records why it is missing.
fn section<T>(errors: &mut Errors, name: &'static str, result: Result<T>) -> Option<T> {
    match result {
        Ok(v) => Some(v),
        Err(e) => {
            errors.insert(name, e.to_string());
            None
        }
    }
}

pub(crate) fn checked(id: String, prefix: &str) -> Result<String> {
    if id.starts_with(prefix) {
        Ok(id)
    } else {
        Err(Error::BadRequest(format!("{id} is not an {prefix}… id")))
    }
}

/// Unix seconds to YYYY-MM-DD (UTC), by Howard Hinnant's civil-from-days.
fn iso_date(unix: i64) -> String {
    let z = unix.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use lull_core::{Error, Result};

    use super::{Errors, LATE_LIMIT, Unfinished, bounded};

    #[test]
    fn a_part_that_failed_late_is_named_failed_when_late_again_never_coming_for_good() {
        // Live, an artist's discography stayed a skeleton: read again after `artist.updated`, the
        // part was late again and named still coming, each time.
        let unfinished = Unfinished::default();
        let late = |page: &str| {
            let (mut pending, mut errors) = (Vec::new(), Errors::new());
            unfinished.late(page, "albums", &mut pending, &mut errors);
            (pending, errors)
        };
        assert_eq!(late("ar-1").0, ["albums"], "never ended: still coming");
        let failed: Result<()> = Err(Error::unavailable("iTunes", "lull's calls for this minute are spent"));
        unfinished.ended("ar-1", &["albums", "topTracks"], &failed);
        let (pending, errors) = late("ar-1");
        assert!(pending.is_empty());
        assert!(errors["albums"].contains("calls for this minute are spent"));
        assert_eq!(late("ar-2").0, ["albums"], "another page's parts are its own");
        unfinished.ended("ar-1", &["albums"], &Ok(()));
        assert_eq!(late("ar-1").0, ["albums"], "arrived once: coming again, not failed");
    }

    #[tokio::test(start_paused = true)]
    async fn a_late_part_ends_within_its_bound() {
        let hung = bounded(std::future::pending::<Result<()>>()).await;
        let Err(e) = hung else {
            unreachable!("a part that never answers can't succeed")
        };
        assert!(e.to_string().contains(&format!("within {} s", LATE_LIMIT.as_secs())));
    }

    #[test]
    fn iso_date_matches_the_calendar() {
        assert_eq!(super::iso_date(0), "1970-01-01");
        assert_eq!(super::iso_date(1_793_404_800), "2026-10-31");
        assert_eq!(super::iso_date(951_782_400), "2000-02-29");
    }
}
