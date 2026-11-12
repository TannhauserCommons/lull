//! `POST /api/catalog/resolve { trackIds }`: the songs the UI is about to show without a cover
//! (docs/context/discovery.md#bare-songs). Any crate may record a song bare, as its source names
//! it (`catalog::remember_named`); the UI asks here for each one it draws without a picture, so
//! every path is covered, today's and later ones, and no crate resolves songs itself.
//!
//! - **At once:** a song lull already has a picture for (a resolved one, a song whose album's
//!   cover is known, a source's own picture) comes back in the answer's `entities`, with its
//!   album pictured.
//! - **Lull's own songs first:** a bare song lull already holds as the same take on an album whose
//!   cover it knows (`twin`: catalog search, an album page, a Soulseek folder of a known album put
//!   it there) is that song, at once, whatever an earlier lookup missed. Live, Soulseek's "HEAVY
//!   LOVE" by Odetari (no album in its tags) showed a placeholder beside the catalog's HEAVY LOVE
//!   on VARIA 0.5, cover known: the lookup asked only outside services, missed, and was
//!   believed for a day.
//! - **When a cover becomes known** (`news`): whoever records it, the catalog notes the artist
//!   and album (`catalog::pictured_news`); the artist's bare songs are matched again, and the
//!   songs on the album a page drew without a cover are sent, so rows already drawn update.
//! - **In the background:** a bare song due a lookup (never tried, or missed a day ago) is
//!   looked up as `Catalog::find_song` does (the library, Deezer by ISRC, the catalog by name),
//!   and on a match the bare record is enriched in place (`catalog::enrich`: the catalog's
//!   spelling, album and cover, the same id). Each batch goes out as `catalog.tracks
//!   { entities }`, which the UI absorbs and repaints. Songs are looked up in the order their
//!   covers stand on the listener's screens (`Pictures::rank`, ADR-0033): what is visible first;
//!   the rest wait, oldest first, since the UI asks about a song once.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use lull_core::db::Db;
use lull_core::ids::{AlbumId, ArtistId, TrackId, loose_title};
use lull_core::model::{Album, Entities, SongQuery, Track};
use lull_core::pictures::Rank;
use lull_core::seams::Catalog;
use lull_core::{Error, Result, catalog};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::Notify;

use super::matching::is_take;
use crate::Service;
use crate::art;
use crate::radio::credited_names;

/// Ids one request may name: a long playlist asks in a few requests.
const MAX_IDS: usize = 200;
/// A batch of resolved songs goes out at most this often.
const REPORT_EVERY: Duration = Duration::from_secs(1);
/// Covers that became known are looked at this often (`news`).
const NEWS_EVERY: Duration = Duration::from_secs(2);
/// News taken per round, and rounds per look (matching songs makes news of its own).
const NEWS_BATCH: usize = 64;
const NEWS_ROUNDS: usize = 3;
/// Bare songs of one artist matched per news, and songs of one album sent.
const PER_ARTIST: usize = 500;
const PER_ALBUM: usize = 200;

/// A bare song waiting, with the album its cover is drawn as.
type Waiting = (TrackId, AlbumId);

/// The bare songs waiting for a lookup, each once, oldest first.
#[derive(Default)]
pub struct Queue {
    waiting: Mutex<(Vec<Waiting>, HashSet<TrackId>)>,
    arrived: Notify,
}

impl Queue {
    fn push(&self, songs: Vec<Waiting>) {
        let mut w = self.waiting.lock().unwrap_or_else(PoisonError::into_inner);
        for (id, album) in songs {
            if w.1.insert(id.clone()) {
                w.0.push((id, album));
            }
        }
        drop(w);
        self.arrived.notify_one();
    }

    /// The song whose cover is best placed on a screen; among the others, the oldest.
    fn pop(&self, rank: impl Fn(&AlbumId) -> Option<Rank>) -> Option<TrackId> {
        let mut w = self.waiting.lock().unwrap_or_else(PoisonError::into_inner);
        let best =
            w.0.iter()
                .enumerate()
                .min_by_key(|(i, (_, album))| match rank(album) {
                    Some(r) => (false, Some(r), *i),
                    None => (true, None, *i),
                })
                .map(|(i, _)| i)?;
        let (id, _) = w.0.remove(best);
        w.1.remove(&id);
        Some(id)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Body {
    track_ids: Vec<TrackId>,
}

/// `{ entities }` for the songs lull can picture now; the others are queued.
pub async fn post(
    State(svc): State<Arc<Service>>,
    body: std::result::Result<Json<Body>, JsonRejection>,
) -> Result<Json<Value>> {
    let Json(body) = body.map_err(|e| Error::BadRequest(e.body_text()))?;
    if body.track_ids.len() > MAX_IDS {
        return Err(Error::BadRequest(format!("Ask for at most {MAX_IDS} songs at once.")));
    }
    let now = lull_core::now();
    let (known, due) = svc
        .ctx
        .db
        .call(move |c| {
            let (mut known, mut due) = (Vec::new(), Vec::new());
            let mut held = Held::default();
            for id in body.track_ids {
                let Some(track) = catalog::track(c, &id)? else { continue };
                if !catalog::is_bare(c, &track)? {
                    known.push(track);
                    continue;
                }
                // Whatever an earlier lookup missed: lull may have learnt the song since.
                if let Some(found) = held.adopt(c, &track, now)? {
                    known.push(found);
                    continue;
                }
                catalog::asked(c, &id)?;
                if catalog::resolution(c, &id, now)?.1 {
                    due.push((id, track.album_id.clone()));
                }
                // Still bare: its album, named, is looked up by name when drawn.
                known.push(track);
            }
            Ok((known, due))
        })
        .await?;
    svc.resolver.push(due);
    Ok(Json(json!({ "entities": pictured(&svc, known).await? })))
}

/// Covers that became known, for as long as lull runs: each `NEWS_EVERY`, the artists' bare songs
/// lull now holds pictured are enriched, and they and the songs of albums that gained a cover go
/// out as `catalog.tracks`, so every page showing them draws the cover. "None" believed for such
/// an album is dropped.
pub async fn news(svc: Arc<Service>) {
    loop {
        tokio::time::sleep(NEWS_EVERY).await;
        match svc.ctx.db.call(|c| sweep(c, lull_core::now())).await {
            Ok((tracks, albums)) => {
                for album in &albums {
                    svc.ctx.pictures.forget_none(album.as_str()).await;
                }
                announce(&svc, tracks).await;
            }
            Err(e) => tracing::warn!(error = %e, "covers that became known weren't looked at"),
        }
    }
}

/// One look at the covers that became known: the songs to send, and the albums that gained one.
pub fn sweep(c: &rusqlite::Connection, now: i64) -> Result<(Vec<Track>, Vec<AlbumId>)> {
    let mut out: BTreeMap<TrackId, Track> = BTreeMap::new();
    let mut albums = Vec::new();
    for _ in 0..NEWS_ROUNDS {
        let news = catalog::pictured_news(c, NEWS_BATCH)?;
        if news.is_empty() {
            break;
        }
        // Read afresh each round: what the last round enriched is held now.
        let mut held = Held::default();
        for n in news {
            match n {
                catalog::News::Album(id) => {
                    for t in catalog::asked_songs_on(c, &id, PER_ALBUM)? {
                        out.insert(t.id.clone(), t);
                    }
                    albums.push(id);
                }
                catalog::News::Artist(artist) => {
                    for bare in catalog::bare_songs_by(c, &artist, PER_ARTIST)? {
                        if let Some(t) = held.adopt(c, &bare, now)? {
                            out.insert(t.id.clone(), t);
                        }
                    }
                }
            }
        }
    }
    Ok((out.into_values().collect(), albums))
}

/// The songs lull holds on pictured albums, by artist, read once per request or look.
#[derive(Default)]
struct Held(HashMap<ArtistId, Vec<(Track, Album)>>);

impl Held {
    /// `bare` enriched as the song lull already holds pictured (`twin`), if it holds it.
    fn adopt(&mut self, c: &rusqlite::Connection, bare: &Track, now: i64) -> Result<Option<Track>> {
        let artists = artists_of(bare);
        for artist in &artists {
            if !self.0.contains_key(artist) {
                let songs = catalog::pictured_songs_by(c, artist)?;
                self.0.insert(artist.clone(), songs);
            }
        }
        let candidates = artists.iter().filter_map(|a| self.0.get(a)).flatten();
        match twin(bare, candidates) {
            Some(found) => {
                let found = found.clone();
                catalog::enrich(c, &bare.id, &found, now)
            }
            None => Ok(None),
        }
    }
}

/// The artists a song may be held under: its own id, then each name its credit holds.
fn artists_of(song: &Track) -> Vec<ArtistId> {
    let mut out = vec![song.artist_id.clone()];
    for name in credited_names(&song.artist) {
        let id = ArtistId::of(&name);
        if !out.contains(&id) {
            out.push(id);
        }
    }
    out
}

/// The song `bare` is among the songs lull holds on pictured albums: the same take by the rule
/// `find_song` matches by (`matching::is_take`). Of several, the one on the album the bare song
/// names, then one its album lists, then the earliest release.
fn twin<'a>(bare: &Track, held: impl IntoIterator<Item = &'a (Track, Album)>) -> Option<&'a Track> {
    let query = SongQuery {
        artist: bare.artist.clone(),
        title: bare.title.clone(),
        album: None,
        duration: bare.duration,
        isrc: None,
    };
    let named = loose_title(&bare.album);
    held.into_iter()
        .filter(|(t, _)| t.id != bare.id && !t.saved && is_take(&query, t))
        .min_by_key(|(t, album)| {
            (
                named.is_empty() || loose_title(&album.title) != named,
                !album.track_ids.contains(&t.id),
                album.year.unwrap_or(u16::MAX),
                album.id.clone(),
            )
        })
        .map(|(t, _)| t)
}

/// Looks queued songs up, one at a time (the catalog paces itself), for as long as lull runs.
pub async fn work(svc: Arc<Service>) {
    let mut batch: Vec<Track> = Vec::new();
    let mut last = Instant::now();
    loop {
        let Some(id) = svc.resolver.pop(|album| svc.ctx.pictures.rank(album.as_str())) else {
            announce(&svc, std::mem::take(&mut batch)).await;
            svc.resolver.arrived.notified().await;
            continue;
        };
        match resolve(&svc.ctx.db, svc.as_ref(), &id).await {
            Ok(Some(track)) => batch.push(track),
            Ok(None) => {}
            Err(e) => tracing::info!(error = %e, track = %id, "a bare song wasn't resolved; asked again when shown"),
        }
        if last.elapsed() >= REPORT_EVERY {
            announce(&svc, std::mem::take(&mut batch)).await;
            last = Instant::now();
        }
    }
}

/// Looks the songs up now, one at a time as the worker does, for as long as `within` allows: how
/// a feed build resolves its picks, so they come with their catalog album and cover, not when
/// first played. A failed lookup, and the songs not reached in time, are left for the worker
/// (it asks again when the song is shown). The songs found, enriched in place.
pub async fn now(db: &Db, catalog: &dyn Catalog, ids: &[TrackId], within: Duration) -> Vec<Track> {
    let until = Instant::now() + within;
    let mut found = Vec::new();
    for id in ids {
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        // A lookup cut off here has written nothing of its own: `resolve` writes in one step, after
        // the catalog answers (what the catalog records itself is whole facts about an album).
        match tokio::time::timeout(left, resolve(db, catalog, id)).await {
            Ok(Ok(track)) => found.extend(track),
            Ok(Err(e)) => tracing::info!(error = %e, track = %id, "a pick wasn't resolved; asked again when shown"),
            Err(_) => break,
        }
    }
    found
}

/// The song as the catalog knows it, enriched in place; `None` when nothing matches (noted, so
/// outside services aren't asked again for a day; lull's own songs always are) or it no longer
/// needs it.
async fn resolve(db: &Db, catalog: &dyn Catalog, id: &TrackId) -> Result<Option<Track>> {
    enum Next {
        Done(Option<Track>),
        Ask(Track, Option<String>),
    }
    let (bare, isrc) = {
        let id = id.clone();
        let now = lull_core::now();
        let next = db
            .call(move |c| {
                let Some(t) = catalog::track(c, &id)? else {
                    return Ok(Next::Done(None));
                };
                if !catalog::is_bare(c, &t)? {
                    return Ok(Next::Done(None));
                }
                // A song lull already holds pictured needs no outside service, whatever one missed.
                if let Some(found) = Held::default().adopt(c, &t, now)? {
                    return Ok(Next::Done(Some(found)));
                }
                let (isrc, due) = catalog::resolution(c, &id, now)?;
                Ok(if due { Next::Ask(t, isrc) } else { Next::Done(None) })
            })
            .await?;
        match next {
            Next::Ask(t, isrc) => (t, isrc),
            Next::Done(found) => return Ok(found),
        }
    };
    let query = SongQuery {
        artist: bare.artist.clone(),
        title: bare.title.clone(),
        album: Some(bare.album.clone()).filter(|a| !a.is_empty()),
        duration: bare.duration,
        isrc,
    };
    let found = catalog.find_song(&query).await?;
    let id = id.clone();
    db.call(move |c| {
        let now = lull_core::now();
        match found {
            // The catalog's song under the same id: its record and album are the catalog's now.
            Some(f) if f.id == id => Ok(catalog::track(c, &id)?.filter(|t| !t.saved)),
            Some(f) => catalog::enrich(c, &id, &f, now),
            None => {
                catalog::missed(c, &id, now)?;
                Ok(None)
            }
        }
    })
    .await
}

/// Sends a batch as `catalog.tracks { entities }`.
async fn announce(svc: &Service, tracks: Vec<Track>) {
    if tracks.is_empty() {
        return;
    }
    match pictured(svc, tracks).await {
        Ok(entities) => svc.ctx.bus.publish("catalog.tracks", json!({ "entities": entities })),
        Err(e) => tracing::warn!(error = %e, "resolved songs weren't announced"),
    }
}

/// The songs with their albums, pictured as search pictures them.
async fn pictured(svc: &Service, tracks: Vec<Track>) -> Result<art::Pictured> {
    let entities = svc
        .ctx
        .db
        .call(move |c| {
            let mut e = Entities::default();
            for t in tracks {
                // A bare song's album, known only from the song, is recorded so its cover can be
                // looked up by name (`art::picture` gives it `art`).
                let album = match catalog::album(c, &t.album_id)? {
                    Some(a) => Some(a),
                    None => catalog::album_named_by(c, &t)?,
                };
                if let Some(album) = album {
                    e.albums.insert(album.id.clone(), album);
                }
                e.tracks.insert(t.id.clone(), t);
            }
            Ok(e)
        })
        .await?;
    art::pictured(svc, entities).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_song_waits_once_however_often_it_is_asked_for() {
        let q = Queue::default();
        let (a, b) = (TrackId("t-a".into()), TrackId("t-b".into()));
        let album = AlbumId("al-0a".into());
        let song = |id: &TrackId| (id.clone(), album.clone());
        q.push(vec![song(&a), song(&b), song(&a)]);
        q.push(vec![song(&b)]);
        let none = |_: &AlbumId| None;
        assert_eq!(
            (q.pop(none), q.pop(none), q.pop(none)),
            (Some(a.clone()), Some(b), None)
        );
        q.push(vec![song(&a)]);
        assert_eq!(q.pop(none), Some(a), "asked again once looked up");
    }

    #[test]
    fn songs_are_looked_up_in_the_order_their_covers_stand_on_screen() {
        // Live, a 5,000-song import resolved oldest first, about 12 a minute: the rows on screen
        // waited behind the whole list above them.
        let pictures = lull_core::pictures::Pictures::new(
            std::env::temp_dir(),
            lull_core::http::Http::new().unwrap_or_else(|e| unreachable!("{e}")),
            lull_core::events::Bus::new(),
            Default::default(),
        );
        let report = lull_core::pictures::Report {
            generation: 1,
            visible: vec![("art/al-0c".into(), 96), ("art/al-0b".into(), 96)],
            near: vec![],
        };
        let reported = pictures.report("laptop", report);
        assert!(reported.is_ok());
        let q = Queue::default();
        let songs = ["0a", "0b", "0c", "0d"].map(|k| (TrackId(format!("t-{k}")), AlbumId(format!("al-{k}"))));
        q.push(songs.to_vec());
        let rank = |album: &AlbumId| pictures.rank(album.as_str());
        let order: Vec<_> = std::iter::from_fn(|| q.pop(rank)).map(|t| t.0).collect();
        assert_eq!(
            order,
            ["t-0c", "t-0b", "t-0a", "t-0d"],
            "on screen top to bottom, then oldest first"
        );
    }

    /// An outside catalog that knows nothing (iTunes had none of Odetari's 2026 songs).
    struct Misses;

    #[async_trait::async_trait]
    impl Catalog for Misses {
        async fn find_song(&self, _: &SongQuery) -> Result<Option<Track>> {
            Ok(None)
        }

        fn same_take(&self, _: &SongQuery, _: &Track) -> bool {
            false
        }
    }

    fn odetari(album: &str, title: &str, duration: Option<u32>) -> Track {
        Track {
            id: TrackId::of("Odetari", album, title),
            title: title.into(),
            artist_id: ArtistId::of("Odetari"),
            artist: "Odetari".into(),
            album_id: AlbumId::of("Odetari", album),
            album: album.into(),
            year: None,
            no: None,
            disc: None,
            duration,
            tags: vec![],
            saved: false,
            file: None,
        }
    }

    fn release(title: &str, year: u16, songs: &[&Track]) -> Album {
        Album {
            id: AlbumId::of("Odetari", title),
            title: title.into(),
            artist_id: ArtistId::of("Odetari"),
            artist: "Odetari".into(),
            year: Some(year),
            kind: Some("single".into()),
            released: None,
            label: None,
            tags: vec![],
            saved: lull_core::model::Saved::None,
            track_ids: songs.iter().map(|t| t.id.clone()).collect(),
            duration: None,
            musicbrainz_id: None,
        }
    }

    const COVER: &str = "https://coverartarchive.org/release-group/b50586b0/front-{size}";

    async fn catalog_db() -> Result<Db> {
        let db = Db::memory()?;
        db.migrate(&catalog::MIGRATIONS)?;
        Ok(db)
    }

    #[tokio::test]
    async fn a_bare_song_lull_holds_pictured_is_that_song_whatever_a_lookup_missed() -> Result<()> {
        // Live (2026-11-12, "odetari blackhole"): Soulseek's songs with no album in their tags
        // stood on Odetari's album "", no cover. Lull held HEAVY LOVE on VARIA 0.5 and the single
        // I'M A BLACKHOLE (STAR), covers known; the lookup asked only iTunes, missed, and the miss
        // was believed for a day.
        let db = catalog_db().await?;
        let heavy = odetari("", "HEAVY LOVE", None);
        let star = odetari("", "I'M A BLACKHOLE (STAR)", Some(198));
        let slowed = odetari("", "I'M A BLACKHOLE (STAR) (Slowed & Reverbed)", Some(248));
        let ids = [heavy.id.clone(), star.id.clone(), slowed.id.clone()];
        let on_varia = odetari("VARIA 0.5", "HEAVY LOVE", Some(167));
        let on_single = odetari("I'M A BLACKHOLE (STAR)", "I'M A BLACKHOLE (STAR)", Some(198));
        let single_slowed = odetari(
            "I'M A BLACKHOLE (STAR)",
            "I'M A BLACKHOLE (STAR) (Slowed & Reverbed)",
            Some(248),
        );
        let on_compilation = odetari("PHAZON VARIA (V2)", "I'M A BLACKHOLE (STAR)", Some(198));
        db.call({
            let bare = vec![heavy.clone(), star.clone(), slowed.clone()];
            let (v, s, ss, p) = (
                on_varia.clone(),
                on_single.clone(),
                single_slowed.clone(),
                on_compilation.clone(),
            );
            move |c| {
                catalog::remember(c, &bare)?;
                for t in &bare {
                    catalog::missed(c, &t.id, lull_core::now())?;
                }
                catalog::remember_album(c, &release("VARIA 0.5", 2025, &[&v]), Some(COVER))?;
                catalog::remember_album(c, &release("I'M A BLACKHOLE (STAR)", 2026, &[&s, &ss]), Some(COVER))?;
                catalog::remember_album(c, &release("PHAZON VARIA (V2)", 2026, &[]), Some(COVER))?;
                catalog::remember(c, &[v, s, ss, p])
            }
        })
        .await?;

        let found = now(&db, &Misses, &ids, Duration::from_secs(5)).await;
        let albums: Vec<_> = found
            .iter()
            .map(|t| (t.id.clone(), t.title.clone(), t.album_id.clone()))
            .collect();
        assert_eq!(
            albums,
            vec![
                (heavy.id, "HEAVY LOVE".into(), on_varia.album_id),
                (star.id, "I'M A BLACKHOLE (STAR)".into(), on_single.album_id.clone()),
                (
                    slowed.id,
                    "I'M A BLACKHOLE (STAR) (Slowed & Reverbed)".into(),
                    on_single.album_id
                ),
            ],
            "each its own take, on the release that lists it, under its own id"
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_cover_learnt_later_reaches_the_bare_songs_and_the_rows_drawn_without_it() -> Result<()> {
        let db = catalog_db().await?;
        let bare = odetari("", "INCOMPLETE", None);
        let folder = odetari("VARIA 0.5", "BABY I'M HOME", Some(150));
        let id = bare.id.clone();
        db.call({
            let (b, f) = (bare.clone(), folder.clone());
            move |c| {
                // A page drew both without a cover; the lookup missed.
                catalog::remember(c, &[b.clone(), f.clone()])?;
                catalog::asked(c, &f.id)?;
                catalog::missed(c, &b.id, lull_core::now())
            }
        })
        .await?;
        assert!(
            now(&db, &Misses, std::slice::from_ref(&id), Duration::from_secs(5))
                .await
                .is_empty()
        );
        let nothing = db.call(|c| sweep(c, lull_core::now())).await?;
        assert_eq!(nothing, (vec![], vec![]), "no cover became known");

        // Later the album page brings VARIA 0.5's tracklist and cover.
        let listed = odetari("VARIA 0.5", "INCOMPLETE", Some(190));
        let album = release("VARIA 0.5", 2025, &[&listed, &folder]);
        db.call({
            let (album, listed) = (album.clone(), listed.clone());
            move |c| {
                catalog::remember_album(c, &album, Some(COVER))?;
                catalog::remember(c, &[listed])
            }
        })
        .await?;
        let (sent, albums) = db.call(|c| sweep(c, lull_core::now())).await?;
        let sent: Vec<_> = sent.into_iter().map(|t| (t.id, t.album_id)).collect();
        assert_eq!(
            sent,
            {
                let mut want = vec![(id.clone(), album.id.clone()), (folder.id.clone(), album.id.clone())];
                want.sort();
                want
            },
            "the bare song is the album's now, and the row drawn on the album gets its cover"
        );
        assert_eq!(albums, vec![album.id.clone()], "its believed \"none\" goes");
        let again = db.call(|c| sweep(c, lull_core::now())).await?;
        assert_eq!(again, (vec![], vec![]), "each once");
        Ok(())
    }

    #[test]
    fn a_twin_is_the_same_take_never_another() {
        let plain = odetari("", "I'M A BLACKHOLE (STAR)", None);
        let slowed = odetari("X", "I'M A BLACKHOLE (STAR) (Slowed & Reverbed)", None);
        let remix = odetari("X", "I'M A BLACKHOLE (STAR) - Remix", Some(198));
        let other = odetari("X", "HEAVY LOVE", Some(198));
        let album = release("X", 2026, &[&slowed, &remix, &other]);
        let held = [(slowed, album.clone()), (remix, album.clone()), (other, album)];
        assert!(twin(&plain, &held).is_none());
    }

    /// A catalog that answers after `0`, as one that is slow does.
    struct Slow(Duration);

    #[async_trait::async_trait]
    impl Catalog for Slow {
        async fn find_song(&self, _: &SongQuery) -> Result<Option<Track>> {
            tokio::time::sleep(self.0).await;
            Ok(None)
        }

        fn same_take(&self, _: &SongQuery, _: &Track) -> bool {
            true
        }
    }

    #[tokio::test]
    async fn a_lookup_slower_than_the_budget_is_cut_off_with_the_pick_left_bare() -> Result<()> {
        // A first Home waits for its build, which resolves picks within AVAILABILITY_WAIT: one slow
        // lookup must not hold it past that.
        let db = Db::memory()?;
        db.migrate(&catalog::MIGRATIONS)?;
        let song = Track {
            id: TrackId::of("Velvet Orchard", "Satellite Porch", "Paper Moon Static"),
            title: "Paper Moon Static".into(),
            artist_id: lull_core::ids::ArtistId::of("Velvet Orchard"),
            artist: "Velvet Orchard".into(),
            album_id: AlbumId::of("Velvet Orchard", "Satellite Porch"),
            album: "Satellite Porch".into(),
            year: None,
            no: None,
            disc: None,
            duration: None,
            tags: vec![],
            saved: false,
            file: None,
        };
        let id = song.id.clone();
        db.call(move |c| catalog::remember(c, &[song])).await?;

        let budget = Duration::from_millis(300);
        let started = Instant::now();
        let found = now(&db, &Slow(Duration::from_secs(30)), std::slice::from_ref(&id), budget).await;
        let took = started.elapsed();
        assert!(found.is_empty());
        assert!(took < budget + Duration::from_millis(500), "returned in {took:?}");

        let asked = id.clone();
        let (bare, due) = db
            .call(move |c| {
                let t = catalog::track(c, &asked)?.ok_or_else(|| Error::internal("pick gone"))?;
                Ok((
                    catalog::is_bare(c, &t)?,
                    catalog::resolution(c, &asked, lull_core::now())?.1,
                ))
            })
            .await?;
        assert!(bare, "the pick stays bare");
        assert!(due, "and nothing was written: the worker looks it up when shown");
        Ok(())
    }
}
