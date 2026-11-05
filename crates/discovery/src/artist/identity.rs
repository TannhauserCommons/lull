//! Who an artist is: the exact ids lull holds for it (Apple's artistId, Deezer's artist id, its
//! MBID), never a name (docs/context/discovery.md#artist-identity). Artist ids are name hashes, so
//! namesakes share one record: by name, lull took Doudou Masta for the Chinese group DOUDOU whose
//! song the listener played, and BIJI's tags from Last.fm's page for every "Biji".
//!
//! An id enters only from an exact reference:
//! - **the listener's songs:** a played or liked song's own catalog artist (the tracklist of the
//!   catalog album lull found it on), and the library's MBID for a saved artist;
//! - **links:** MusicBrainz's URL relationships between an MBID and an artist's Apple Music or
//!   Deezer page, either way, to exactly one artist;
//! - **credits that carry ids:** an ISRC's recordings, whose credit is that one artist, or lists
//!   it among others by an MBID that links to one of its catalog ids;
//! - **recommenders' own ids** (`offer`): ListenBrainz's recording and release artists, Deezer's
//!   related artists and radio, Last.fm's similar artists that come with an MBID;
//! - **search rows' own ids** (`Store::artist_sources`): a catalog's artist row, a song row's
//!   artist.
//!
//! The listener's songs come first: an artist they have songs of takes only the ids those give
//! (and their links), and a recommender's or search's id never joins it. A recommender or search seeds an artist lull
//! holds no id for; another source joins only through a link. Two different ids of one source
//! cancel each other. No name is compared anywhere. Each feature uses whichever id serves it.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::future::Future;
use std::sync::Arc;

use lull_core::Result;
use lull_core::ids::{AlbumId, ArtistId, TrackId, loose_title};
use serde::{Deserialize, Serialize};

use crate::sources::musicbrainz::Links;
use crate::sources::{AlbumSource, ArtistInfo, ArtistSource, Song};
use crate::{Service, feed};

/// The listener's evidence for an artist is weighed again after this: new songs, new links.
const RECHECK_S: i64 = 7 * 86_400;
/// The rule's version: a new one weighs every stored artist again, once (`recheck`).
const RULE: &str = "exact-ids-1";

/// A source of artist ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Itunes,
    Deezer,
    Musicbrainz,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Itunes => "itunes",
            Source::Deezer => "deezer",
            Source::Musicbrainz => "musicbrainz",
        }
    }

    fn of(s: ArtistSource) -> Source {
        match s {
            ArtistSource::Itunes(_) => Source::Itunes,
            ArtistSource::Deezer(_) => Source::Deezer,
        }
    }
}

/// Where an id came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "from")]
pub enum Evidence {
    /// A song of the listener's names its artist by this catalog id.
    Song { track: TrackId },
    /// The library's saved files carry this MBID.
    Library,
    /// MusicBrainz links it to the artist's id of `source`.
    Link { source: Source },
    /// A recording with this ISRC credits the artist by this MBID.
    Isrc { isrc: String },
    /// A recommender named the artist by this id.
    Recommended { by: String },
    /// A search row (the catalog's own artist row, or a song's) named the artist by this id.
    Searched,
}

/// One id of an artist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdRow {
    pub source: Source,
    pub id: String,
    pub evidence: Evidence,
}

impl IdRow {
    /// A recommender's id for an artist (`offer`).
    pub fn recommended(source: Source, id: impl ToString, by: &str) -> IdRow {
        IdRow {
            source,
            id: id.to_string(),
            evidence: Evidence::Recommended { by: by.to_owned() },
        }
    }
}

/// The exact ids lull holds for an artist, at most one per source.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Ids(Vec<IdRow>);

impl Ids {
    pub fn get(&self, source: Source) -> Option<&IdRow> {
        self.0.iter().find(|r| r.source == source)
    }

    /// Sets (or replaces) the id of the row's source.
    pub fn set(&mut self, row: IdRow) {
        self.0.retain(|r| r.source != row.source);
        self.0.push(row);
        self.0.sort_by_key(|r| r.source);
    }

    pub fn rows(&self) -> &[IdRow] {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn mbid(&self) -> Option<&str> {
        self.get(Source::Musicbrainz).map(|r| r.id.as_str())
    }

    pub fn itunes(&self) -> Option<u64> {
        self.get(Source::Itunes).and_then(|r| r.id.parse().ok())
    }

    pub fn deezer(&self) -> Option<u64> {
        self.get(Source::Deezer).and_then(|r| r.id.parse().ok())
    }

    fn catalog(&self) -> Vec<ArtistSource> {
        let itunes = self.itunes().map(ArtistSource::Itunes);
        itunes
            .into_iter()
            .chain(self.deezer().map(ArtistSource::Deezer))
            .collect()
    }
}

/// What the listener's own songs say of an artist.
#[derive(Debug, Default, Clone)]
pub struct Listened {
    /// The MBID in the saved files' tags.
    pub library: Option<String>,
    /// Each song's own catalog artist.
    pub catalog: Vec<(ArtistSource, TrackId)>,
    /// Those songs' ISRCs.
    pub isrcs: Vec<String>,
}

impl Listened {
    fn is_empty(&self) -> bool {
        self.library.is_none() && self.catalog.is_empty() && self.isrcs.is_empty()
    }
}

/// The artist's ids: the listener's evidence weighed when it has songs or saved files (again
/// once `RECHECK_S` old), else what recommenders gave; filled through MusicBrainz's links. A
/// lookup that fails fails this call, and nothing is kept, so the next call asks again.
pub async fn ids(svc: &Service, id: &ArtistId) -> Result<Ids> {
    let now = lull_core::now();
    let songs = listener_songs(svc).await?.remove(id).unwrap_or_default();
    let library = svc.store.library_mbid(id).await?;
    if !songs.is_empty() || library.is_some() {
        let weighed = svc.store.ids_checked(id.clone()).await?;
        if weighed.is_some_and(|at| now - at < RECHECK_S) {
            return svc.store.artist_ids(id.clone()).await;
        }
        return weigh(svc, id, library, &songs, now).await;
    }
    let held = svc.store.artist_ids(id.clone()).await?;
    if held.is_empty() {
        return Ok(held);
    }
    let mut filled = held.clone();
    expand(
        &mut filled,
        svc.itunes.country(),
        |u| svc.mb.url_artists(u),
        |m| links(svc, m),
    )
    .await?;
    let new: Vec<IdRow> = filled
        .rows()
        .iter()
        .filter(|r| !held.rows().contains(r))
        .cloned()
        .collect();
    if new.is_empty() {
        return Ok(held);
    }
    svc.store.add_artist_ids(id.clone(), new).await
}

/// Recommenders' ids for the artists they name, each by artist name. One joins only an artist
/// lull holds no id for and the listener has no songs of: theirs decide who it is.
pub async fn offer(svc: &Service, offers: Vec<(String, IdRow)>) -> Result<()> {
    if offers.is_empty() {
        return Ok(());
    }
    let listened: HashSet<ArtistId> = listener_songs(svc).await?.into_keys().collect();
    let named: Vec<ArtistId> = offers.iter().map(|(n, _)| ArtistId::of(n)).collect();
    let held = svc.store.identified(named).await?;
    for (id, row) in accepted(offers, &listened, &held) {
        svc.store.add_artist_ids(id, vec![row]).await?;
    }
    Ok(())
}

/// The offers that seed an artist: one the listener has no songs of and lull holds no id for,
/// the first offer for it.
fn accepted(
    offers: Vec<(String, IdRow)>,
    listened: &HashSet<ArtistId>,
    held: &HashSet<ArtistId>,
) -> Vec<(ArtistId, IdRow)> {
    let mut seeded = HashSet::new();
    offers
        .into_iter()
        .map(|(name, row)| (ArtistId::of(&name), row))
        .filter(|(id, _)| !listened.contains(id) && !held.contains(id) && seeded.insert(id.clone()))
        .collect()
}

/// Weighs every remembered artist the listener has songs of again when the rule changed (once
/// per rule), then rebuilds the feed: its seeds and tags came from the old identities.
pub async fn recheck(svc: Arc<Service>) {
    if let Err(e) = recheck_all(&svc).await {
        tracing::warn!(error = %e, "artists not identified again; the next start tries again");
    }
}

async fn recheck_all(svc: &Arc<Service>) -> Result<()> {
    if svc.store.state("identity_rule").await?.as_deref() == Some(RULE) {
        return Ok(());
    }
    let now = lull_core::now();
    let mut songs = listener_songs(svc).await?;
    let mut library: HashMap<ArtistId, String> = svc
        .store
        .library_artists()
        .await?
        .into_iter()
        .filter_map(|a| Some((a.id, a.musicbrainz_id?)))
        .collect();
    let (mut weighed, mut with_mbid, mut undecided) = (0, 0, 0);
    for (id, name) in svc.store.remembered_artists().await? {
        let (theirs, saved) = (songs.remove(&id).unwrap_or_default(), library.remove(&id));
        if theirs.is_empty() && saved.is_none() {
            continue;
        }
        match weigh(svc, &id, saved, &theirs, now).await {
            Ok(ids) => {
                weighed += 1;
                with_mbid += usize::from(ids.mbid().is_some());
            }
            Err(e) => {
                undecided += 1;
                tracing::info!(artist = %name, error = %e, "artist left undecided; asked again when shown");
            }
        }
    }
    svc.store.set_state("identity_rule", RULE).await?;
    tracing::info!(
        weighed,
        with_mbid,
        undecided,
        "the listener's artists identified again by exact ids"
    );
    feed::refresh_soon(svc);
    Ok(())
}

/// Decides the artist's ids from the listener's evidence and keeps them. Its tags then come
/// from MusicBrainz by its MBID, and its songs' own tags from Last.fm by artist and title.
async fn weigh(svc: &Service, id: &ArtistId, library: Option<String>, songs: &[TrackId], now: i64) -> Result<Ids> {
    let listened = gather(svc, id, library, songs).await?;
    let ids = if listened.is_empty() {
        Ids::default()
    } else {
        decide(
            &listened,
            svc.itunes.country(),
            |u| svc.mb.url_artists(u),
            |m| links(svc, m),
            |isrc| async move { svc.mb.isrc_credits(&isrc).await },
        )
        .await?
    };
    svc.store.set_artist_ids(id.clone(), ids.clone(), now).await?;
    if let Some(mbid) = ids.mbid() {
        let tags = svc.mb.artist_links(mbid).await?.tags;
        svc.store.set_artist_tags(id.clone(), tags).await?;
    }
    song_tags(svc, songs).await;
    Ok(ids)
}

/// The songs' own tags where the catalog has none: Last.fm's for that artist and title.
async fn song_tags(svc: &Service, songs: &[TrackId]) {
    let Some(lastfm) = svc.lastfm() else { return };
    for id in songs {
        let track = match svc.store.track(id.clone()).await {
            Ok(Some(t)) if t.tags.is_empty() => t,
            Ok(_) => continue,
            Err(e) => {
                tracing::info!(error = %e, "a song's tags not read");
                continue;
            }
        };
        match lastfm.track_tags(&track.artist, &track.title).await {
            Ok(tags) if !tags.is_empty() => {
                if let Err(e) = svc.store.set_track_tags(id.clone(), tags).await {
                    tracing::info!(error = %e, "a song's tags not kept");
                }
            }
            Ok(_) => {}
            Err(e) => tracing::info!(error = %e, song = %track.title, "no Last.fm tags for a song this time"),
        }
    }
}

/// MusicBrainz's links of an MBID.
async fn links(svc: &Service, mbid: String) -> Result<Links> {
    svc.mb.artist_links(&mbid).await
}

/// The exact ids the listener's songs credited to the artist carry: each song's catalog artist
/// (from the tracklist of the catalog album lull found it on, kept a week) and its ISRC.
async fn gather(svc: &Service, id: &ArtistId, library: Option<String>, songs: &[TrackId]) -> Result<Listened> {
    let mut listened = Listened {
        library,
        ..Listened::default()
    };
    let mut wanted: HashMap<AlbumId, Vec<(String, TrackId)>> = HashMap::new();
    for track_id in songs {
        if let Some(isrc) = svc.store.isrc(track_id.clone()).await? {
            listened.isrcs.push(isrc);
        }
        if let Some(track) = svc.store.track(track_id.clone()).await? {
            wanted
                .entry(track.album_id)
                .or_default()
                .push((loose_title(&track.title), track_id.clone()));
        }
    }
    for (album, titles) in wanted {
        let Some(source) = svc.store.album_source(album).await? else {
            continue;
        };
        let listed = match source {
            AlbumSource::Itunes(collection) => svc.itunes.tracklist_in_turn(collection).await?,
            AlbumSource::Deezer(album) => svc.deezer.tracklist(album).await?,
        };
        for (song, track) in theirs(listed, id, &titles) {
            listened.catalog.extend(song.artist_source.map(|s| (s, track.clone())));
            listened.isrcs.extend(song.isrc);
        }
    }
    Ok(listened)
}

/// The listed songs that are the listener's: credited to the artist (the same credit, so the
/// same id), with a title they played.
fn theirs(listed: Vec<Song>, id: &ArtistId, titles: &[(String, TrackId)]) -> Vec<(Song, TrackId)> {
    listed
        .into_iter()
        .filter(|s| ArtistId::of(&s.artist) == *id)
        .filter_map(|s| {
            let title = loose_title(&s.title);
            let track = titles.iter().find(|(t, _)| *t == title)?.1.clone();
            Some((s, track))
        })
        .collect()
}

/// The songs the listener played or liked, by the artist they are credited to.
async fn listener_songs(svc: &Service) -> Result<HashMap<ArtistId, Vec<TrackId>>> {
    let mut by_artist: HashMap<ArtistId, Vec<TrackId>> = HashMap::new();
    for (track, artist) in svc.store.listener_songs().await? {
        by_artist.entry(ArtistId::of(&artist)).or_default().push(track);
    }
    Ok(by_artist)
}

/// The ids the listener's evidence proves, and what MusicBrainz links to them. `url_artists`
/// answers the artists linked from each page URL asked (one it doesn't know is absent), `links`
/// an MBID's Apple and Deezer artists, `credits` the artist credit of each recording with an
/// ISRC. A lookup that fails fails the decision.
pub async fn decide<U, UF, K, KF, C, CF>(
    listened: &Listened,
    country: &str,
    url_artists: U,
    links: K,
    credits: C,
) -> Result<Ids>
where
    U: Fn(Vec<String>) -> UF,
    UF: Future<Output = Result<HashMap<String, Vec<ArtistInfo>>>>,
    K: Fn(String) -> KF,
    KF: Future<Output = Result<Links>>,
    C: Fn(String) -> CF,
    CF: Future<Output = Result<Vec<Vec<ArtistInfo>>>>,
{
    let mut ids = Ids::default();
    if let Some(mbid) = &listened.library {
        ids.set(IdRow {
            source: Source::Musicbrainz,
            id: mbid.clone(),
            evidence: Evidence::Library,
        });
    }
    for source in [Source::Itunes, Source::Deezer] {
        let named: Vec<&(ArtistSource, TrackId)> = listened
            .catalog
            .iter()
            .filter(|(s, _)| Source::of(*s) == source)
            .collect();
        let distinct: BTreeSet<ArtistSource> = named.iter().map(|(s, _)| *s).collect();
        // Songs naming two of one catalog's artists: neither is surely this one.
        if let (1, Some((only, track))) = (distinct.len(), named.first()) {
            ids.set(IdRow {
                source,
                id: catalog_id(*only),
                evidence: Evidence::Song { track: track.clone() },
            });
        }
    }
    expand(&mut ids, country, &url_artists, &links).await?;
    if ids.mbid().is_none() {
        for isrc in listened.isrcs.iter().collect::<BTreeSet<_>>() {
            if let Some(mbid) = credited(&ids, credits(isrc.clone()).await?, &links).await? {
                ids.set(IdRow {
                    source: Source::Musicbrainz,
                    id: mbid,
                    evidence: Evidence::Isrc { isrc: isrc.clone() },
                });
                expand(&mut ids, country, &url_artists, &links).await?;
                break;
            }
        }
    }
    Ok(ids)
}

/// Fills the sources `ids` has none of through MusicBrainz's links: its catalog ids to the one
/// artist they all link to, its MBID to the Apple and Deezer artists it links to.
pub async fn expand<U, UF, K, KF>(ids: &mut Ids, country: &str, url_artists: U, links: K) -> Result<()>
where
    U: Fn(Vec<String>) -> UF,
    UF: Future<Output = Result<HashMap<String, Vec<ArtistInfo>>>>,
    K: Fn(String) -> KF,
    KF: Future<Output = Result<Links>>,
{
    if ids.mbid().is_none() && !ids.catalog().is_empty() {
        let catalog = ids.catalog();
        let linked = url_artists(catalog.iter().flat_map(|s| resources(*s, country)).collect()).await?;
        let mut found: Vec<(String, Source)> = Vec::new();
        for s in catalog {
            let mut artists: Vec<&String> = resources(s, country)
                .iter()
                .filter_map(|url| linked.get(url))
                .flatten()
                .filter_map(|a| a.mbid.as_ref())
                .collect();
            artists.sort();
            artists.dedup();
            if let [only] = artists.as_slice() {
                found.push(((*only).clone(), Source::of(s)));
            }
        }
        let agreed = found.first().filter(|(m, _)| found.iter().all(|(o, _)| o == m));
        if let Some((mbid, from)) = agreed.cloned() {
            ids.set(IdRow {
                source: Source::Musicbrainz,
                id: mbid,
                evidence: Evidence::Link { source: from },
            });
        }
    }
    if let Some(mbid) = ids.mbid().map(str::to_owned)
        && (ids.itunes().is_none() || ids.deezer().is_none())
    {
        let linked = links(mbid).await?;
        for (source, id) in [(Source::Itunes, linked.itunes), (Source::Deezer, linked.deezer)] {
            if let (None, Some(id)) = (ids.get(source), id) {
                ids.set(IdRow {
                    source,
                    id: id.to_string(),
                    evidence: Evidence::Link {
                        source: Source::Musicbrainz,
                    },
                });
            }
        }
    }
    Ok(())
}

/// The artist's MBID among recordings' credits: a credit that is one artist alone, or the one
/// member whose MBID MusicBrainz links to one of the artist's catalog ids. All the credits must
/// agree.
async fn credited<K, KF>(ids: &Ids, credits: Vec<Vec<ArtistInfo>>, links: K) -> Result<Option<String>>
where
    K: Fn(String) -> KF,
    KF: Future<Output = Result<Links>>,
{
    let mut found = BTreeSet::new();
    for credit in credits {
        let members: Vec<&String> = credit.iter().filter_map(|a| a.mbid.as_ref()).collect();
        if let [only] = members.as_slice() {
            found.insert((*only).clone());
            continue;
        }
        for mbid in members {
            let linked = links(mbid.clone()).await?;
            let ours = linked.itunes.is_some_and(|i| ids.itunes() == Some(i))
                || linked.deezer.is_some_and(|d| ids.deezer() == Some(d));
            if ours {
                found.insert(mbid.clone());
            }
        }
    }
    let mut found = found.into_iter();
    Ok(match (found.next(), found.next()) {
        (Some(only), None) => Some(only),
        _ => None,
    })
}

/// The page URLs MusicBrainz may link a catalog artist from: Apple's in the hub's store country,
/// the US and the UK ones (MusicBrainz keeps the storefront an editor linked: BIJI's is the UK
/// one), its old iTunes form, and Deezer's. All name the same Apple id.
pub fn resources(source: ArtistSource, country: &str) -> Vec<String> {
    match source {
        ArtistSource::Itunes(id) => {
            let mut countries = vec![country.to_lowercase()];
            for c in ["us", "gb"] {
                if !countries.iter().any(|k| k == c) {
                    countries.push(c.into());
                }
            }
            let mut urls: Vec<String> = countries
                .iter()
                .map(|c| format!("https://music.apple.com/{c}/artist/{id}"))
                .collect();
            urls.push(format!("https://itunes.apple.com/us/artist/id{id}"));
            urls
        }
        ArtistSource::Deezer(id) => vec![format!("https://www.deezer.com/artist/{id}")],
    }
}

fn catalog_id(s: ArtistSource) -> String {
    match s {
        ArtistSource::Itunes(id) | ArtistSource::Deezer(id) => id.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use lull_core::Result;
    use lull_core::ids::{ArtistId, TrackId};

    use super::*;

    // Real ids, checked 2026-11-04: BIJI's "Boya" and DOUDOU's "A Rusty Dream" on Apple, the
    // pages MusicBrainz links them from, and the artist lull once took DOUDOU for by name.
    const BIJI_APPLE: u64 = 1_689_784_894;
    const BIJI: &str = "712b5d5f-78ec-41c8-acfe-d4c1210e14c5";
    const BIJI_DEEZER: u64 = 329_421_301;
    const DOUDOU_APPLE: u64 = 1_691_467_301;
    const DOUDOU: &str = "f0fa7600-2702-4060-a540-32a60db78d58";
    const DOUDOU_MASTA: &str = "562b88d2-94b4-4632-b00c-c23fd8978c65";

    fn artist(name: &str, mbid: &str) -> ArtistInfo {
        ArtistInfo {
            mbid: Some(mbid.into()),
            name: name.into(),
        }
    }

    /// MusicBrainz's URL relationships as they are (BIJI's on the UK storefront only).
    async fn url_artists(asked: Vec<String>) -> Result<HashMap<String, Vec<ArtistInfo>>> {
        let known = HashMap::from([
            (
                format!("https://music.apple.com/gb/artist/{BIJI_APPLE}"),
                vec![artist("Biji", BIJI), artist("Biji", BIJI)],
            ),
            (
                format!("https://music.apple.com/us/artist/{DOUDOU_APPLE}"),
                vec![artist("DOUDOU", DOUDOU)],
            ),
            (
                "https://www.deezer.com/artist/999".to_owned(),
                vec![
                    artist("Split Name", "6100aaaa-0000-4000-8000-000000000002"),
                    artist("Split Name", "6100aaaa-0000-4000-8000-000000000003"),
                ],
            ),
        ]);
        Ok(asked
            .into_iter()
            .filter_map(|u| Some((u.clone(), known.get(&u)?.clone())))
            .collect())
    }

    /// An MBID's Apple and Deezer artists.
    async fn links(mbid: String) -> Result<Links> {
        Ok(match mbid.as_str() {
            BIJI => Links {
                itunes: Some(BIJI_APPLE),
                deezer: Some(BIJI_DEEZER),
                ..Links::default()
            },
            "6100aaaa-0000-4000-8000-0000000000c1" => Links {
                deezer: Some(777),
                ..Links::default()
            },
            _ => Links::default(),
        })
    }

    async fn no_credits(_: String) -> Result<Vec<Vec<ArtistInfo>>> {
        Ok(Vec::new())
    }

    fn song(source: ArtistSource) -> (ArtistSource, TrackId) {
        (source, TrackId::of("anyone", "", "a song"))
    }

    async fn decided(listened: Listened) -> Result<Ids> {
        decide(&listened, "US", url_artists, links, no_credits).await
    }

    #[tokio::test]
    async fn biji_is_who_its_own_apple_id_links_to_with_its_deezer_id() -> Result<()> {
        // Live, BIJI's taste tags were kurdish and iraq: Last.fm's page for every "Biji". Its
        // own song names its Apple id, which MusicBrainz links (on the UK storefront) to BIJI.
        let ids = decided(Listened {
            catalog: vec![song(ArtistSource::Itunes(BIJI_APPLE))],
            ..Listened::default()
        })
        .await?;
        assert_eq!(ids.itunes(), Some(BIJI_APPLE));
        assert_eq!(ids.mbid(), Some(BIJI));
        assert_eq!(
            ids.deezer(),
            Some(BIJI_DEEZER),
            "through MusicBrainz's link, for related artists"
        );
        assert_eq!(
            ids.get(Source::Musicbrainz).map(|r| &r.evidence),
            Some(&Evidence::Link { source: Source::Itunes })
        );
        Ok(())
    }

    #[tokio::test]
    async fn doudou_is_the_group_its_song_names_never_doudou_masta() -> Result<()> {
        // Live, DOUDOU was stored as Doudou Masta (French hip hop, reggae), Last.fm's MBID for
        // the name.
        let ids = decided(Listened {
            catalog: vec![song(ArtistSource::Itunes(DOUDOU_APPLE))],
            ..Listened::default()
        })
        .await?;
        assert_eq!(ids.mbid(), Some(DOUDOU));
        assert_ne!(ids.mbid(), Some(DOUDOU_MASTA));
        Ok(())
    }

    #[tokio::test]
    async fn a_catalog_id_no_link_names_keeps_the_artist_by_that_id() -> Result<()> {
        // No MBID, and still who it is: its discography and related artists come by this id.
        let ids = decided(Listened {
            catalog: vec![song(ArtistSource::Deezer(4242))],
            ..Listened::default()
        })
        .await?;
        assert_eq!((ids.deezer(), ids.mbid()), (Some(4242), None));
        Ok(())
    }

    #[tokio::test]
    async fn two_ids_of_one_catalog_or_a_link_to_two_artists_prove_nothing() -> Result<()> {
        let two = decided(Listened {
            catalog: vec![song(ArtistSource::Deezer(1)), song(ArtistSource::Deezer(2))],
            ..Listened::default()
        })
        .await?;
        assert!(two.is_empty(), "two Deezer artists of this name: neither is surely it");
        let split = decided(Listened {
            catalog: vec![song(ArtistSource::Deezer(999))],
            ..Listened::default()
        })
        .await?;
        assert_eq!((split.deezer(), split.mbid()), (Some(999), None));
        Ok(())
    }

    #[tokio::test]
    async fn the_library_comes_first() -> Result<()> {
        let ids = decided(Listened {
            library: Some("6100aaaa-0000-4000-8000-0000000000ff".into()),
            catalog: vec![song(ArtistSource::Itunes(DOUDOU_APPLE))],
            ..Listened::default()
        })
        .await?;
        assert_eq!(ids.mbid(), Some("6100aaaa-0000-4000-8000-0000000000ff"));
        assert_eq!(
            ids.get(Source::Musicbrainz).map(|r| &r.evidence),
            Some(&Evidence::Library)
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_collaboration_is_matched_by_the_contributor_id_never_the_name() -> Result<()> {
        // "BIJI & Mira Vale": the ISRC's credit lists both. The member whose MBID links to the
        // artist's Deezer id is it; the names are never compared (both are spelled to match).
        let credits = |_: String| async {
            Ok(vec![vec![
                artist("Mira Vale", "6100aaaa-0000-4000-8000-0000000000c2"),
                artist("Mira Vale", "6100aaaa-0000-4000-8000-0000000000c1"),
            ]])
        };
        let listened = Listened {
            catalog: vec![song(ArtistSource::Deezer(777))],
            isrcs: vec!["QZABC2600001".into()],
            ..Listened::default()
        };
        let ids = decide(&listened, "US", url_artists, links, credits).await?;
        assert_eq!(ids.mbid(), Some("6100aaaa-0000-4000-8000-0000000000c1"));
        assert_eq!(
            ids.get(Source::Musicbrainz).map(|r| &r.evidence),
            Some(&Evidence::Isrc {
                isrc: "QZABC2600001".into()
            })
        );
        Ok(())
    }

    #[test]
    fn a_recommenders_id_seeds_only_an_artist_lull_holds_nothing_for() {
        let row = |id: &str| IdRow::recommended(Source::Musicbrainz, id, "ListenBrainz");
        let listened = HashSet::from([ArtistId::of("BIJI")]);
        let held = HashSet::from([ArtistId::of("Glow Tape")]);
        let kept = accepted(
            vec![
                ("BIJI".into(), row("namesake")),
                ("Glow Tape".into(), row("another")),
                ("Pale Harbor".into(), row("first")),
                ("Pale Harbor".into(), row("second")),
            ],
            &listened,
            &held,
        );
        assert_eq!(
            kept,
            [(ArtistId::of("Pale Harbor"), row("first"))],
            "the listener's own artist and one already identified take no recommender's id"
        );
    }
}
