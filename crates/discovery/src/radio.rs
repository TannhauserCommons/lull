//! `GET /api/radio`: the next tracks for a radio seeded by a track, an artist or a tag, steered
//! from familiar (0) to adventurous (1). ListenBrainz picks the candidates (artist radio for
//! similar artists, tag radio within a popularity window); the taste model orders them.
//!
//! An artist radio needs the artist's MBID. A credit ("A feat. B & C") is tried whole, then
//! artist by artist, and each name through what lull remembers, MusicBrainz (a transient error
//! retried once), then Last.fm. When none names an MBID, or ListenBrainz has nothing for it, the
//! radio comes from Last.fm's similar artists and the tags of the track and artist instead.

use std::collections::{HashMap, HashSet};
use std::future::Future;

use lull_core::ids::{ArtistId, TrackId};
use lull_core::model::{Entities, Track};
use lull_core::{Error, Result};
use serde::Deserialize;

use crate::sources::listenbrainz::{Mode, Popularity};
use crate::store::entities;
use crate::{Service, mix, taste};

/// Tracks returned per call; the UI asks again as the queue drains.
const BATCH: usize = 12;
/// Candidates looked at per call (one metadata request).
const POOL: usize = 50;
/// Per artist in one batch, except the seed artist of an artist radio.
const PER_ARTIST: usize = 2;

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

pub async fn next(
    svc: &Service,
    seed: Seed,
    steer: f64,
    exclude: HashSet<TrackId>,
) -> Result<(Vec<TrackId>, Entities)> {
    let taste = svc.taste().await?;
    let (mut mbids, seed_artist) = candidates(svc, &seed, steer).await?;
    mix::shuffle(&mut mbids, mix::seed());
    mbids.truncate(POOL);
    let songs = svc.lb.songs(&mbids).await?;
    let tracks = svc.store.tracks(songs).await?;

    let mut ranked: Vec<(f64, Track)> = tracks
        .into_iter()
        .filter(|t| !exclude.contains(&t.id) && !taste.hidden.contains(&t.artist_id))
        .filter(|t| taste.artist_weight(&t.artist_id) >= 0.0)
        .enumerate()
        .map(|(i, t)| {
            // Shuffled order, nudged towards known artists when familiar and away when adventurous.
            let known = if taste.artist_weight(&t.artist_id) > 0.0 {
                1.0
            } else {
                0.0
            };
            let nudge = (1.0 - 2.0 * steer) * known * 10.0;
            (i as f64 - nudge, t)
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
            *n <= PER_ARTIST || seed_artist.as_ref() == Some(&t.artist_id)
        })
        .take(BATCH)
        .collect();
    Ok((
        picked.iter().map(|t| t.id.clone()).collect(),
        entities(picked, Vec::new(), Vec::new()),
    ))
}

/// Candidate recordings for the seed, and the seed's artist when it has one.
async fn candidates(svc: &Service, seed: &Seed, steer: f64) -> Result<(Vec<String>, Option<ArtistId>)> {
    let pop = popularity(steer);
    match seed.kind {
        SeedKind::Tag => Ok((svc.lb.tag_radio(&seed.id.to_lowercase(), pop, 100).await?, None)),
        SeedKind::Artist => {
            let artist = svc
                .store
                .artist_ref(ArtistId(seed.id.clone()))
                .await?
                .ok_or_else(|| Error::NotFound(format!("Artist {} is unknown to lull.", seed.id)))?;
            let found = by_artist(svc, &artist.name, &artist.tags, steer, pop).await?;
            Ok((found, Some(artist.id)))
        }
        SeedKind::Track => {
            let track = svc
                .store
                .track(TrackId(seed.id.clone()))
                .await?
                .ok_or_else(|| Error::NotFound(format!("Track {} is unknown to lull.", seed.id)))?;
            let mut found = by_artist(svc, &track.artist, &track.tags, steer, pop).await?;
            if let Some(tag) = track.tags.iter().find(|t| taste::is_specific(t)) {
                // A bonus on top of the artist's radio: its failure doesn't stop the radio.
                match svc.lb.tag_radio(tag, pop, 50).await {
                    Ok(more) => found.extend(more),
                    Err(e) => tracing::info!(tag, error = %e, "track radio without its tag"),
                }
            }
            Ok((found, None))
        }
    }
}

/// Recordings for an artist radio on `credit`, else from its similar artists and `tags`.
async fn by_artist(svc: &Service, credit: &str, tags: &[String], steer: f64, pop: Popularity) -> Result<Vec<String>> {
    let names = credited_names(credit);
    let (mbid, mut failure) = resolve(
        &names,
        |name| remembered_mbid(svc, name),
        |name| musicbrainz_mbid(svc, name),
        |name| lastfm_mbid(svc, name),
    )
    .await;
    if let Some(mbid) = mbid {
        match svc.lb.artist_radio(&mbid, mode(steer), 15, 4, pop).await {
            Ok(picks) if !picks.is_empty() => return Ok(picks.into_iter().map(|p| p.recording_mbid).collect()),
            Ok(_) => tracing::info!(
                credit,
                "ListenBrainz has no artist radio; using similar artists and tags"
            ),
            Err(e) => failure = Some(e),
        }
    }
    let found = around(svc, main_and_featured(credit).0, tags, steer, pop, &mut failure).await;
    if !found.is_empty() {
        return Ok(found);
    }
    Err(failure
        .unwrap_or_else(|| Error::NotFound(format!("No music source knows {credit} well enough for a radio yet."))))
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
fn main_and_featured(credit: &str) -> (&str, &str) {
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

/// The first MBID any lookup names for `names`, in order: what lull remembers, MusicBrainz (once
/// it has failed, it isn't asked again), then Last.fm. Also MusicBrainz's failure, if any.
pub async fn resolve<R, M, L, RF, MF, LF>(
    names: &[String],
    remembered: R,
    musicbrainz: M,
    lastfm: L,
) -> (Option<String>, Option<Error>)
where
    R: Fn(String) -> RF,
    M: Fn(String) -> MF,
    L: Fn(String) -> LF,
    RF: Future<Output = Option<String>>,
    MF: Future<Output = Result<Option<String>>>,
    LF: Future<Output = Option<String>>,
{
    for name in names {
        if let Some(mbid) = remembered(name.clone()).await {
            return (Some(mbid), None);
        }
    }
    let mut failure = None;
    for name in names {
        let mut found = musicbrainz(name.clone()).await;
        if matches!(found, Err(Error::Unavailable { .. } | Error::Upstream { .. })) {
            // One retry: a dropped connection is usually gone a moment later.
            found = musicbrainz(name.clone()).await;
        }
        match found {
            Ok(Some(mbid)) => return (Some(mbid), None),
            Ok(None) => {}
            Err(e) => {
                tracing::info!(artist = %name, error = %e, "radio goes on without MusicBrainz");
                failure = Some(e);
                break;
            }
        }
    }
    for name in names {
        if let Some(mbid) = lastfm(name.clone()).await {
            return (Some(mbid), failure);
        }
    }
    (None, failure)
}

async fn remembered_mbid(svc: &Service, name: String) -> Option<String> {
    svc.store
        .artist_ref(ArtistId::of(&name))
        .await
        .ok()
        .flatten()
        .and_then(|a| a.musicbrainz_id)
}

async fn musicbrainz_mbid(svc: &Service, name: String) -> Result<Option<String>> {
    if !svc.mb.switch.is_on() {
        return Ok(None);
    }
    match svc.mb.find_artist(&name).await {
        Ok(found) => Ok(found.and_then(|a| a.mbid)),
        Err(Error::NotFound(_)) => Ok(None),
        Err(e) => Err(e),
    }
}

async fn lastfm_mbid(svc: &Service, name: String) -> Option<String> {
    svc.lastfm()?.artist(&name).await.ok()?.mbid
}

/// Radio without the artist's own: the artist radio of Last.fm's most similar artist that has an
/// MBID, and tag radio for the first specific tag of the seed or the artist. Failures are kept
/// in `failure` for when nothing at all answers.
async fn around(
    svc: &Service,
    name: &str,
    tags: &[String],
    steer: f64,
    pop: Popularity,
    failure: &mut Option<Error>,
) -> Vec<String> {
    let mut found = Vec::new();
    let mut tags: Vec<String> = tags.to_vec();
    if let Some(lastfm) = svc.lastfm() {
        match lastfm.similar_artists(name).await {
            Ok(similar) => {
                if let Some(mbid) = similar.into_iter().find_map(|a| a.mbid) {
                    match svc.lb.artist_radio(&mbid, mode(steer), 15, 4, pop).await {
                        Ok(picks) => found.extend(picks.into_iter().map(|p| p.recording_mbid)),
                        Err(e) => *failure = Some(e),
                    }
                }
            }
            Err(e) => *failure = Some(e),
        }
        if let Ok(profile) = lastfm.artist(name).await {
            tags.extend(profile.tags);
        }
    }
    if let Some(tag) = tags.iter().find(|t| taste::is_specific(t)) {
        match svc.lb.tag_radio(tag, pop, 50).await {
            Ok(more) => found.extend(more),
            Err(e) => *failure = Some(e),
        }
    }
    found
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

    use super::{credited_names, resolve};

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
        let (mbid, failure) = resolve(
            &names,
            |_| async { None },
            |name| async move { Ok((name == "Shawn Chrystopher").then(|| "mbid-shawn".to_owned())) },
            |_| async { None },
        )
        .await;
        assert_eq!(mbid.as_deref(), Some("mbid-shawn"));
        assert!(failure.is_none());
    }

    #[tokio::test]
    async fn with_musicbrainz_down_last_fm_names_the_artist_after_one_retry() {
        let names = credited_names("Shawn Chrystopher feat. Cults & Ashly");
        let asked = Cell::new(0);
        let (mbid, failure) = resolve(
            &names,
            |_| async { None },
            |_| {
                asked.set(asked.get() + 1);
                async { Err(Error::unavailable("MusicBrainz", "error sending request")) }
            },
            |name| async move { (name == "Shawn Chrystopher").then(|| "mbid-from-lastfm".to_owned()) },
        )
        .await;
        assert_eq!(mbid.as_deref(), Some("mbid-from-lastfm"));
        assert_eq!(
            asked.get(),
            2,
            "one retry, then MusicBrainz isn't asked for the other names"
        );
        assert!(failure.is_some());
    }

    #[tokio::test]
    async fn a_transient_musicbrainz_error_is_retried_once() {
        let asked = Cell::new(0);
        let (mbid, failure) = resolve(
            &["Radiohead".to_owned()],
            |_| async { None },
            |_| {
                asked.set(asked.get() + 1);
                let first = asked.get() == 1;
                async move {
                    if first {
                        Err(Error::unavailable("MusicBrainz", "error sending request"))
                    } else {
                        Ok(Some("mbid-radiohead".to_owned()))
                    }
                }
            },
            |_| async { None },
        )
        .await;
        assert_eq!((mbid.as_deref(), failure.is_none()), (Some("mbid-radiohead"), true));
    }

    #[tokio::test]
    async fn with_nothing_naming_the_artist_the_radio_falls_back_with_the_reason_kept() {
        let (mbid, failure) = resolve(
            &credited_names("Nobody feat. Anybody"),
            |_| async { None },
            |_| async { Err(Error::unavailable("MusicBrainz", "error sending request")) },
            |_| async { None },
        )
        .await;
        assert!(mbid.is_none());
        assert!(failure.is_some_and(|e| e.to_string().contains("MusicBrainz")));
    }
}
