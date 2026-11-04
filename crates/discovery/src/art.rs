//! `GET /api/catalog/art/{id}?size=96|256|512|1024|2048`: covers and artist photos discovery
//! found (Deezer, iTunes, the Cover Art Archive, Wikimedia Commons), fetched once per size at the
//! source's best for it, scaled down to it (never up: `lull_core::art`) and served from lull, so
//! the UI never loads an outside image and the CSP stays same-origin. Where each
//! picture comes from is `lull_core::catalog`'s; an album with a MusicBrainz release group and
//! no other source gets the Cover Art Archive's. Entities point at this route as `art` (albums)
//! whenever a source is known, and as `image` (artists) only for a vetted photo
//! (`artist::photo`); the route serves nothing else for an artist.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path as FsPath, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use axum::extract::{Path, Query, State};
use axum::response::Response;
use lull_core::ids::{AlbumId, ArtistId, TrackId};
use lull_core::model::{Album, Artist, Entities, Saved, Track};
use lull_core::warm::{Priority, Warmer};
use lull_core::{Error, Result, art, catalog};
use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::Service;

pub(crate) use lull_core::art::SIZES;
const DEFAULT_SIZE: u32 = 256;
/// How long a request waits for a picture lull doesn't have yet. The fetch goes on after it,
/// and the next request is served from the cache.
const WAIT: Duration = Duration::from_secs(3);
/// One try per picture, in the background.
const FETCH_TIMEOUT: Duration = Duration::from_secs(20);
/// A fetched picture is used this long before it is fetched again.
const KEEP: Duration = Duration::from_secs(30 * 24 * 3600);
/// "The source has none" is believed this long.
const NONE_FOR: Duration = Duration::from_secs(24 * 3600);
/// The largest Deezer picture asked for. Its CDN serves up to 1900 px whatever the master is,
/// and 1400 px is the smallest master it takes from labels, so this size is never blown up.
const DEEZER_LARGEST: u32 = 1400;

/// An album as the UI gets it from discovery: with `art` when lull serves its cover.
#[derive(Debug, Clone, Serialize)]
pub struct PicturedAlbum {
    #[serde(flatten)]
    pub album: Album,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub art: Option<String>,
}

/// An artist with `image` when lull serves a photo.
#[derive(Debug, Clone, Serialize)]
pub struct PicturedArtist {
    #[serde(flatten)]
    pub artist: Artist,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
}

/// `Entities` with pictures (docs/context/api.md#conventions).
#[derive(Debug, Clone, Default, Serialize)]
pub struct Pictured {
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub tracks: BTreeMap<TrackId, Track>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub albums: BTreeMap<AlbumId, PicturedAlbum>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub artists: BTreeMap<ArtistId, PicturedArtist>,
}

/// Same-origin picture URLs for what the catalog has a picture source for, except where the
/// source said it has none (a new release the Cover Art Archive has no cover for): the UI shows
/// its placeholder at once instead of asking for a 404 on every load.
pub async fn pictured(svc: &Service, e: Entities) -> Result<Pictured> {
    let (mut out, unvetted) = svc
        .ctx
        .db
        .call(move |c| {
            let mut unvetted = Vec::new();
            for (id, artist) in &e.artists {
                if crate::artist::photo::vetted(c, id)?.is_none() {
                    unvetted.push(artist.clone());
                }
            }
            Ok((picture(c, e)?, unvetted))
        })
        .await?;
    svc.photos.ask(unvetted);
    let pictures = &svc.pictures;
    for (id, album) in &mut out.albums {
        if album.art.is_some() && pictures.has_none(id.as_str()).await {
            album.art = None;
        }
    }
    for (id, artist) in &mut out.artists {
        if artist.image.is_some() && pictures.has_none(id.as_str()).await {
            artist.image = None;
        }
    }
    // What this answer shows is fetched now, before the page asks: rows, tiles and heroes.
    let shown = out
        .albums
        .iter()
        .filter(|(_, a)| a.art.is_some())
        .map(|(id, _)| id.as_str());
    for id in shown.chain(
        out.artists
            .iter()
            .filter(|(_, a)| a.image.is_some())
            .map(|(id, _)| id.as_str()),
    ) {
        pictures.warm(id, SMALL, Priority::Page);
    }
    let from_library: Vec<AlbumId> = out
        .albums
        .iter()
        .filter(|(_, a)| a.art.is_none() && a.album.saved != Saved::None)
        .map(|(id, _)| id.clone())
        .collect();
    svc.store.warm_covers(&from_library);
    Ok(out)
}

/// "The source has none", one file for every size: a source missing at one size has none at all
/// (an archive thumbnail it lacks falls back to its original first).
pub(crate) fn none_file(dir: &FsPath, id: &str) -> PathBuf {
    dir.join(format!("{id}.none"))
}

/// `pictured` on a connection. An album with a release group and no known source gets the Cover
/// Art Archive's, recorded so the route serves it.
pub fn picture(c: &rusqlite::Connection, e: Entities) -> Result<Pictured> {
    let mut out = Pictured {
        tracks: e.tracks,
        ..Pictured::default()
    };
    for (id, album) in e.albums {
        let mut source = catalog::album_art(c, &id)?;
        if source.is_none()
            && let Some(group) = &album.musicbrainz_id
        {
            let archive = cover_archive(group);
            catalog::remember_album(c, &album, Some(&archive))?;
            source = Some(archive);
        }
        let art = source.map(|_| url(id.as_str()));
        out.albums.insert(id, PicturedAlbum { album, art });
    }
    for (id, artist) in e.artists {
        // Only a vetted photo: what search recorded may be a cover (artist/photo.rs). Its version
        // makes a browser holding an old picture ask again.
        let image = crate::artist::photo::image_of(c, &id)?
            .map(|source| format!("{}?v={}", url(id.as_str()), crate::artist::photo::version(&source)));
        out.artists.insert(id, PicturedArtist { artist, image });
    }
    Ok(out)
}

/// The Cover Art Archive's front cover of a release group, as a picture source.
pub fn cover_archive(release_group: &str) -> String {
    format!("https://coverartarchive.org/release-group/{release_group}/front-{{size}}")
}

/// The prefix of a playlist picture's id: `pls-` and the playlist's link key in hex, so the id
/// says which picture it is and stays a plain path segment.
const PLAYLIST_PICTURE: &str = "pls-";

/// The route for an outside playlist's picture (search's Playlists section), its source
/// remembered for the route a month.
pub async fn playlist_picture(svc: &Service, link: &str, source: &str) -> Result<String> {
    let id = format!("{PLAYLIST_PICTURE}{}", hex_of(link));
    svc.cache
        .put(format!("playlist-picture:{id}"), &source.to_owned())
        .await?;
    Ok(url(&id))
}

fn hex_of(s: &str) -> String {
    s.bytes().map(|b| format!("{b:02x}")).collect()
}

/// The route for an album's or artist's picture; the UI adds `?size=`. Relative to the page, as
/// the UI's own requests are, so lull also works behind a reverse-proxy sub-path.
pub fn url(id: &str) -> String {
    format!("api/catalog/art/{id}")
}

#[derive(Deserialize)]
pub struct Size {
    #[serde(default = "default_size")]
    size: u32,
}

fn default_size() -> u32 {
    DEFAULT_SIZE
}

pub async fn get(State(svc): State<Arc<Service>>, Path(id): Path<String>, Query(q): Query<Size>) -> Result<Response> {
    art::check(q.size)?;
    let pictures = &svc.pictures;
    let source = pictures
        .source_of(&id)
        .await?
        .ok_or_else(|| Error::NotFound(format!("lull has no picture for {id}")))?;
    let path = pictures.file(&id, q.size);
    if let Some(bytes) = read_fresh(&path, KEEP).await {
        return Ok(art::response(bytes));
    }
    if pictures.has_none(&id).await {
        return Ok(art::none(&id, NONE_FOR));
    }
    let mut done = pictures.ensure(&id, &source, q.size);
    // Ended (or the hub stopping) either way; what matters is what is on disk now.
    let _ = tokio::time::timeout(WAIT, done.wait_for(|d| *d)).await;
    Ok(match read_fresh(&path, KEEP).await {
        Some(bytes) => art::response(bytes),
        None if pictures.has_none(&id).await => art::none(&id, NONE_FOR),
        None => art::not_yet(&id),
    })
}

/// The size fetched from outside for `size`: one 256 px picture makes 96 and 256 (rows and tiles,
/// the smaller scaled here), so a cover the page shows costs one small outside fetch; 512 and up
/// are fetched as they are wanted.
fn tier(size: u32) -> u32 {
    size.max(SMALL)
}
const SMALL: u32 = 256;

/// Everything about catalog pictures: where they are cached, the fetches under way (one per
/// picture and tier), the background queue that fetches them before they are asked for, and
/// `art.ready`, which tells the pages one has come in.
pub struct Pictures {
    ctx: lull_core::Ctx,
    cache: crate::sources::cache::Cache,
    dir: PathBuf,
    fetches: Fetches,
    warmer: Arc<Warmer>,
    ready: Arc<art::Ready>,
    /// The songs `playing` last warmed.
    playing: Mutex<Vec<TrackId>>,
    /// Failed fetches by picture and tier, and how often (`again_later`).
    failed: Mutex<HashMap<String, u32>>,
}

/// A failed fetch is tried again after this (twice that the second time).
const RETRY_AFTER: Duration = Duration::from_secs(20);
const RETRIES: u32 = 2;

/// Pictures fetched at once in the background: polite to the archive and the CDNs, and still a
/// page of covers in a few seconds.
const WARM_WORKERS: usize = 3;

impl Pictures {
    pub fn new(ctx: &lull_core::Ctx, cache: crate::sources::cache::Cache) -> Arc<Pictures> {
        Arc::new(Pictures {
            ctx: ctx.clone(),
            cache,
            dir: ctx.config.data_dir.join("catalog-art"),
            fetches: Fetches::default(),
            warmer: Warmer::start(ctx, WARM_WORKERS),
            ready: art::Ready::new(ctx.bus.clone(), ctx.shutdown.clone()),
            playing: Mutex::default(),
            failed: Mutex::default(),
        })
    }

    pub(crate) fn dir(&self) -> &FsPath {
        &self.dir
    }

    fn file(&self, id: &str, size: u32) -> PathBuf {
        self.dir.join(format!("{id}-{size}"))
    }

    /// Whether the picture's source said lately that it has none.
    async fn has_none(&self, id: &str) -> bool {
        read_fresh(&none_file(&self.dir, id), NONE_FOR).await.is_some()
    }

    /// Where the picture for `id` comes from: an album's catalog source, an artist's vetted
    /// photo, or a search playlist's picture; `None` for an id lull has no picture for.
    async fn source_of(&self, id: &str) -> Result<Option<String>> {
        let hex = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_hexdigit());
        Ok(match id {
            i if i.strip_prefix(AlbumId::PREFIX).is_some_and(hex) => {
                let id = AlbumId(id.to_owned());
                self.ctx.db.call(move |c| catalog::album_art(c, &id)).await?
            }
            i if i.strip_prefix(ArtistId::PREFIX).is_some_and(hex) => {
                let id = ArtistId(id.to_owned());
                self.ctx
                    .db
                    .call(move |c| crate::artist::photo::image_of(c, &id))
                    .await?
            }
            i if i.strip_prefix(PLAYLIST_PICTURE).is_some_and(hex) => {
                self.cache
                    .fresh::<String>(format!("playlist-picture:{id}"), 30 * crate::sources::cache::DAY)
                    .await?
            }
            _ => None,
        })
    }

    /// Queues the picture for `id` at `size` to be fetched before anyone asks, unless it is here
    /// or its source has none.
    pub fn warm(self: &Arc<Self>, id: &str, size: u32, priority: Priority) {
        let (me, id) = (self.clone(), id.to_owned());
        self.warmer.push(format!("{id} {}", tier(size)), priority, async move {
            if read_fresh(&me.file(&id, size), KEEP).await.is_some() || me.has_none(&id).await {
                return;
            }
            if let Ok(Some(source)) = me.source_of(&id).await {
                let _ = me.ensure(&id, &source, size).wait_for(|d| *d).await;
            }
        });
    }

    /// Makes the picture for `id` at `size`'s tier: the rows' size scaled here from the 256 px
    /// picture when that is cached, else fetched from `source` (with "none" remembered when the source has none).
    /// One job per picture and tier, shared by every request and the queue; the receiver turns
    /// `true` when it ends. A picture fetched from outside is announced (`art.ready`).
    fn ensure(self: &Arc<Self>, id: &str, source: &str, size: u32) -> watch::Receiver<bool> {
        let tier = tier(size);
        let key = format!("{id} {tier}");
        self.fetches.join(&key.clone(), |done| {
            let (me, id, url) = (self.clone(), id.to_owned(), sized(source, tier));
            self.ctx.spawn(async move {
                let master = me.file(&id, tier);
                let (bytes, fetched) = match read_fresh(&master, KEEP).await {
                    Some(bytes) => (Some(bytes), false),
                    None => match me.fetch_into(&url, tier, &master, &id).await {
                        Ok(bytes) => (bytes, true),
                        Err(()) => {
                            me.again_later(&key, &id, tier);
                            (None, false)
                        }
                    },
                };
                if let Some(bytes) = bytes {
                    if tier == SMALL {
                        me.derive(&id, bytes).await;
                    }
                    if fetched {
                        me.ready.announce(id);
                    }
                }
                me.fetches.finish(&key, done);
            });
        })
    }

    /// A fetch that failed (a source timing out while a long page's covers come in) is queued
    /// again after a while, twice at most, so the pages waiting for it still hear `art.ready`.
    fn again_later(self: &Arc<Self>, key: &str, id: &str, tier: u32) {
        let tries = {
            let mut failed = self.failed.lock().unwrap_or_else(PoisonError::into_inner);
            let n = failed.entry(key.to_owned()).or_insert(0);
            *n += 1;
            *n
        };
        if tries > RETRIES {
            self.failed.lock().unwrap_or_else(PoisonError::into_inner).remove(key);
            return;
        }
        let (me, id) = (self.clone(), id.to_owned());
        self.ctx.spawn(async move {
            tokio::time::sleep(RETRY_AFTER * tries).await;
            me.warm(&id, tier, Priority::Page);
        });
    }

    /// The source's picture scaled to `tier` and cached at `path` (`Ok(None)`: the source has
    /// none, remembered; an archive thumbnail it lacks falls back to its original first). A
    /// failure caches nothing: `Err`.
    async fn fetch_into(
        &self,
        url: &str,
        tier: u32,
        path: &FsPath,
        id: &str,
    ) -> std::result::Result<Option<Vec<u8>>, ()> {
        let mut fetched = fetch(&self.ctx, url).await;
        if let (Ok(None), Some(original)) = (&fetched, archive_original(url)) {
            fetched = fetch(&self.ctx, &original).await;
        }
        match fetched {
            Ok(Some(bytes)) => match tokio::task::spawn_blocking(move || art::fit(bytes, tier)).await {
                Ok(bytes) => {
                    write(path, &bytes).await;
                    self.failed
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .remove(&format!("{id} {tier}"));
                    Ok(Some(bytes))
                }
                Err(e) => {
                    tracing::warn!(error = %e, "picture scaling failed");
                    Ok(None)
                }
            },
            Ok(None) => {
                write(&none_file(&self.dir, id), &[]).await;
                Ok(None)
            }
            Err(e) => {
                tracing::info!(url, error = %e, "picture not fetched; tried again later");
                Err(())
            }
        }
    }

    /// The sizes below `SMALL`, scaled from it.
    async fn derive(&self, id: &str, master: Vec<u8>) {
        for size in SIZES.into_iter().filter(|&s| s < SMALL) {
            let path = self.file(id, size);
            if read_fresh(&path, KEEP).await.is_some() {
                continue;
            }
            let from = master.clone();
            if let Ok(bytes) = tokio::task::spawn_blocking(move || art::fit(from, size)).await {
                write(&path, &bytes).await;
            }
        }
    }
}

/// Songs played next, from `device.state`: the playing song's cover is fetched at every size its
/// Now Playing view may want, and the next songs' at the player bar's and Now Playing's, ahead
/// of everything a page asked for. A state that names the same songs again changes nothing.
pub async fn playing(svc: &Service, data: serde_json::Value) -> Result<()> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Playing {
        track_id: Option<TrackId>,
        #[serde(default)]
        queue: Vec<TrackId>,
        queue_index: Option<usize>,
    }
    let p: Playing = serde_json::from_value(data).map_err(|e| Error::internal(format!("device.state payload: {e}")))?;
    let next = p.queue_index.map_or(0, |i| i + 1);
    let songs: Vec<TrackId> = p
        .track_id
        .into_iter()
        .chain(p.queue.into_iter().skip(next).take(UP_NEXT))
        .collect();
    let pictures = &svc.pictures;
    {
        let mut last = pictures.playing.lock().unwrap_or_else(PoisonError::into_inner);
        if *last == songs {
            return Ok(());
        }
        last.clone_from(&songs);
    }
    let albums = svc
        .ctx
        .db
        .call(move |c| {
            let mut albums = Vec::new();
            for song in &songs {
                if let Some(track) = catalog::track(c, song)?
                    && catalog::album_art(c, &track.album_id)?.is_some()
                    && !albums.contains(&track.album_id)
                {
                    albums.push(track.album_id);
                }
            }
            Ok(albums)
        })
        .await?;
    pictures.warmer.clear(Priority::Playing);
    for (i, album) in albums.iter().enumerate() {
        let sizes: &[u32] = if i == 0 { &[SMALL, 512, 1024] } else { &[SMALL, 512] };
        for &size in sizes {
            pictures.warm(album.as_str(), size, Priority::Playing);
        }
    }
    Ok(())
}

/// Songs after the playing one whose covers are fetched ahead.
const UP_NEXT: usize = 3;

/// Picture fetches under way, one per picture and tier, shared by every request for it.
#[derive(Default)]
pub struct Fetches(Mutex<HashMap<String, watch::Receiver<bool>>>);

impl Fetches {
    /// A receiver that turns `true` when the fetch of `key` ends; `start` begins one when none is
    /// under way, and must call `finish` at its end.
    pub fn join(&self, key: &str, start: impl FnOnce(watch::Sender<bool>)) -> watch::Receiver<bool> {
        let mut map = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(done) = map.get(key) {
            return done.clone();
        }
        let (tx, rx) = watch::channel(false);
        map.insert(key.to_owned(), rx.clone());
        drop(map);
        start(tx);
        rx
    }

    pub fn finish(&self, key: &str, done: watch::Sender<bool>) {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).remove(key);
        let _ = done.send(true);
    }
}

/// The source URL at its best for `size`, scaled down after. The Cover Art Archive has its own
/// sizes: the smallest thumbnail that covers the request (250 counts for 256), else its original.
/// Deezer is asked for at most `DEEZER_LARGEST`; iTunes and Wikimedia Commons serve any size up
/// to their original and never more.
pub fn sized(source: &str, size: u32) -> String {
    if source.starts_with("https://coverartarchive.org/") {
        return match art::archive_size(size) {
            Some(thumbnail) => source.replace("{size}", &thumbnail.to_string()),
            None => source.replace("-{size}", ""),
        };
    }
    let size = if source.contains(".dzcdn.net/") {
        size.min(DEEZER_LARGEST)
    } else {
        size
    };
    source.replace("{size}", &size.to_string())
}

/// The archive's original for one of its thumbnails (`…/front-1200` → `…/front`).
fn archive_original(url: &str) -> Option<String> {
    if !url.starts_with("https://coverartarchive.org/") {
        return None;
    }
    let (front, size) = url.rsplit_once('-')?;
    size.parse::<u32>().ok().map(|_| front.to_owned())
}

pub(crate) async fn fetch(ctx: &lull_core::Ctx, url: &str) -> Result<Option<Vec<u8>>> {
    let service = if url.starts_with("https://coverartarchive.org/") {
        "Cover Art Archive"
    } else if url.contains(".mzstatic.com/") {
        "iTunes"
    } else if url.starts_with("https://commons.wikimedia.org/") {
        "Wikimedia Commons"
    } else {
        "Deezer"
    };
    let resp = ctx
        .http
        .client()
        .get(url)
        .timeout(FETCH_TIMEOUT)
        .send()
        .await
        .map_err(|e| Error::unavailable(service, e.without_url().to_string()))?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let resp = lull_core::http::ok(service, resp)?;
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| Error::upstream(service, format!("picture cut short: {e}")))?;
    Ok(Some(bytes.to_vec()))
}

async fn read_fresh(path: &FsPath, keep: Duration) -> Option<Vec<u8>> {
    let modified = tokio::fs::metadata(path).await.ok()?.modified().ok()?;
    let age = SystemTime::now().duration_since(modified).unwrap_or_default();
    if age > keep {
        return None;
    }
    tokio::fs::read(path).await.ok()
}

/// Best effort: a picture that can't be cached is fetched again next time.
async fn write(path: &FsPath, bytes: &[u8]) {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".part");
    let tmp = PathBuf::from(tmp);
    let written = async {
        if let Some(dir) = path.parent() {
            tokio::fs::create_dir_all(dir).await?;
        }
        tokio::fs::write(&tmp, bytes).await?;
        tokio::fs::rename(&tmp, path).await
    };
    if let Err(e) = written.await {
        tracing::warn!(error = %e, path = %path.display(), "could not cache a picture");
    }
}

#[cfg(test)]
mod tests {
    use lull_core::db::Db;
    use lull_core::ids::{AlbumId, ArtistId};
    use lull_core::model::{Album, Artist, Entities, Saved};
    use lull_core::{Result, catalog};

    use super::{Fetches, Pictures, archive_original, picture, sized};

    fn test_ctx() -> Result<lull_core::Ctx> {
        let data = std::env::temp_dir().join(format!("lull-discovery-art-{}", std::process::id()));
        let vars = [
            ("LULL_SLSKD_URL", "http://slskd"),
            ("LULL_SLSKD_API_KEY", "k"),
            ("LULL_DATA_DIR", data.to_str().unwrap_or_default()),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
        let config =
            lull_core::config::Config::from_vars(vars).map_err(|e| lull_core::Error::internal(e.to_string()))?;
        Ok(lull_core::Ctx::new(
            config,
            Db::memory()?,
            lull_core::http::Http::new()?,
            lull_core::events::Bus::new(),
        ))
    }

    fn album(title: &str, group: Option<&str>) -> Album {
        Album {
            id: AlbumId::of("Velvet Orchard", title),
            title: title.into(),
            artist_id: ArtistId::of("Velvet Orchard"),
            artist: "Velvet Orchard".into(),
            year: None,
            kind: Some("album".into()),
            released: None,
            label: None,
            tags: vec![],
            saved: Saved::None,
            track_ids: vec![],
            duration: None,
            musicbrainz_id: group.map(str::to_owned),
        }
    }

    #[tokio::test]
    async fn an_artist_picture_search_recorded_is_not_advertised_until_vetted() -> Result<()> {
        // Live, BIJI's first page visit advertised Deezer's "photo" (the Edgerunners cover).
        let db = Db::memory()?;
        db.migrate(&catalog::MIGRATIONS)?;
        db.migrate(&crate::MIGRATIONS)?;
        let biji = Artist {
            id: ArtistId::of("BIJI"),
            name: "BIJI".into(),
            tags: vec![],
            listeners: None,
            bio: None,
            musicbrainz_id: None,
        };
        let id = biji.id.clone();
        let e = || Entities {
            artists: [(biji.id.clone(), biji.clone())].into(),
            ..Entities::default()
        };
        let (first, e1) = (id.clone(), e());
        let unvetted = db
            .call(move |c| {
                catalog::remember_artist(c, &e1.artists[&first], Some("https://cdn.example/cover/{size}.jpg"))?;
                Ok(picture(c, e1)?.artists[&first].image.clone())
            })
            .await?;
        assert_eq!(unvetted, None, "search's picture may be a cover");
        let (then, e2) = (id.clone(), e());
        let vetted = db
            .call(move |c| {
                c.execute(
                    "INSERT INTO discovery_artist_photos (artist_id, image, checked_at) VALUES (?1, ?2, 0)",
                    [then.as_str(), "https://commons.example/biji.jpg?width={size}"],
                )?;
                Ok(picture(c, e2)?.artists[&then].image.clone())
            })
            .await?;
        assert!(vetted.is_some_and(|u| u.starts_with("api/catalog/art/ar-") && u.contains("?v=")));
        Ok(())
    }

    #[tokio::test]
    async fn a_new_release_with_a_release_group_gets_the_archives_cover() -> Result<()> {
        // Live, Home's new releases had no `art`, and both picture routes answered 404.
        let db = Db::memory()?;
        db.migrate(&catalog::MIGRATIONS)?;
        let known = album("Satellite Porch", Some("0f3c0c64-5b0c-4d0b-9c0c-9a3a2c5b1e11"));
        let unknown = album("Paper Moon Static", None);
        let (known_id, unknown_id) = (known.id.clone(), unknown.id.clone());
        let e = Entities {
            albums: [(known.id.clone(), known), (unknown.id.clone(), unknown)].into(),
            ..Entities::default()
        };
        let asked = known_id.clone();
        let (out, source) = db
            .call(move |c| Ok((picture(c, e)?, catalog::album_art(c, &asked)?)))
            .await?;
        assert!(out.albums[&known_id].art.is_some());
        assert!(out.albums[&unknown_id].art.is_none());
        assert_eq!(
            source.as_deref(),
            Some("https://coverartarchive.org/release-group/0f3c0c64-5b0c-4d0b-9c0c-9a3a2c5b1e11/front-{size}")
        );
        Ok(())
    }

    #[test]
    fn each_source_is_asked_for_its_best_at_the_size() {
        let archive = "https://coverartarchive.org/release-group/0f3c0c64-5b0c/front-{size}";
        assert!(sized(archive, 96).ends_with("front-250"));
        assert!(sized(archive, 256).ends_with("front-250"));
        assert!(sized(archive, 512).ends_with("front-500"));
        assert!(sized(archive, 1024).ends_with("front-1200"));
        assert!(sized(archive, 2048).ends_with("/0f3c0c64-5b0c/front"), "the original");
        let deezer = "https://cdn-images.dzcdn.net/images/cover/d06e/{size}x{size}-000000-80-0-0.jpg";
        assert!(sized(deezer, 1024).contains("/1024x1024-"));
        // Live, its CDN answered 403 above 1900 px and may blow up a small master below that.
        assert!(sized(deezer, 2048).contains("/1400x1400-"));
        let itunes = "https://is1-ssl.mzstatic.com/image/thumb/x/1.png/{size}x{size}bb.jpg";
        // Live, iTunes answered 2048 at 2048 px and 5000 at its 4000 px original.
        assert!(sized(itunes, 2048).ends_with("/2048x2048bb.jpg"));
    }

    #[test]
    fn an_archive_thumbnail_falls_back_to_the_original() {
        assert_eq!(
            archive_original("https://coverartarchive.org/release-group/0f3c0c64-5b0c/front-1200").as_deref(),
            Some("https://coverartarchive.org/release-group/0f3c0c64-5b0c/front")
        );
        assert_eq!(
            archive_original("https://coverartarchive.org/release-group/0f3c0c64-5b0c/front"),
            None
        );
        assert_eq!(archive_original("https://cdn.example/1200"), None);
    }

    #[tokio::test]
    async fn a_rows_cover_is_scaled_from_the_tiles_without_another_outside_fetch() -> Result<()> {
        let ctx = test_ctx()?;
        let pictures = Pictures::new(&ctx, crate::sources::cache::Cache::new(ctx.db.clone()));
        let tile = image::RgbImage::from_pixel(256, 256, image::Rgb([200, 120, 40]));
        let mut jpeg = Vec::new();
        let encoded = image::DynamicImage::ImageRgb8(tile)
            .write_to(&mut std::io::Cursor::new(&mut jpeg), image::ImageFormat::Jpeg);
        assert!(encoded.is_ok());
        super::write(&pictures.file("al-0a", 256), &jpeg).await;
        // A source nobody can reach: the 96 px cover must come from the cached 256 px one.
        let mut done = pictures.ensure("al-0a", "https://unreachable.invalid/{size}", 96);
        assert!(done.wait_for(|d| *d).await.is_ok());
        let row = tokio::fs::read(pictures.file("al-0a", 96))
            .await
            .map_err(|e| lull_core::Error::internal(e.to_string()))?;
        assert_eq!(lull_core::art::dimensions(&row), Some((96, 96)));
        let _ = tokio::fs::remove_dir_all(pictures.dir()).await;
        Ok(())
    }

    #[tokio::test]
    async fn requests_for_one_picture_share_one_fetch() {
        // Live, a cold cover took 6 s and answered 503; now the fetch runs once in the background.
        let fetches = Fetches::default();
        let mut started = Vec::new();
        let mut first = fetches.join("https://img.example/a", |done| started.push(done));
        let second = fetches.join("https://img.example/a", |done| started.push(done));
        assert_eq!(started.len(), 1, "the second request joins the first fetch");
        let Some(done) = started.pop() else { return };
        fetches.finish("https://img.example/a", done);
        assert!(first.wait_for(|d| *d).await.is_ok());
        assert!(*second.borrow());
        let _again = fetches.join("https://img.example/a", |done| started.push(done));
        assert_eq!(started.len(), 1, "after it ends, a new fetch can start");
    }
}
