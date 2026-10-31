//! `GET /api/radio`: the next tracks for a radio seeded by a track, an artist or a tag, steered
//! from familiar (0) to adventurous (1). ListenBrainz picks the candidates (artist radio for
//! similar artists, tag radio within a popularity window); the taste model orders them.

use std::collections::{HashMap, HashSet};

use lull_core::ids::{ArtistId, TrackId};
use lull_core::model::{Entities, Track};
use lull_core::{Error, Result};
use serde::Deserialize;

use crate::sources::listenbrainz::{Mode, Popularity};
use crate::store::entities;
use crate::{Service, feed, mix, taste};

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
            let found = by_artist(svc, &artist.name, steer, pop).await?;
            Ok((found, Some(artist.id)))
        }
        SeedKind::Track => {
            let track = svc
                .store
                .track(TrackId(seed.id.clone()))
                .await?
                .ok_or_else(|| Error::NotFound(format!("Track {} is unknown to lull.", seed.id)))?;
            let mut found = by_artist(svc, &track.artist, steer, pop).await?;
            if let Some(tag) = track.tags.iter().find(|t| taste::is_specific(t)) {
                found.extend(svc.lb.tag_radio(tag, pop, 50).await?);
            }
            Ok((found, None))
        }
    }
}

async fn by_artist(svc: &Service, name: &str, steer: f64, pop: Popularity) -> Result<Vec<String>> {
    let mbid = feed::artist_mbid(svc, name).await?.ok_or_else(|| {
        Error::NotFound(format!(
            "MusicBrainz doesn't know {name}, so there is no radio for it yet."
        ))
    })?;
    let picks = svc.lb.artist_radio(&mbid, mode(steer), 15, 4, pop).await?;
    Ok(picks.into_iter().map(|p| p.recording_mbid).collect())
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
