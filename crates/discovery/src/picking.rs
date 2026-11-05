//! How picks are chosen, without the sources (`feed::picks` asks them): stable at the core,
//! fresh at the edge (docs/context/discovery.md#stable-and-fresh, ADR-0100). The evidence and the
//! engineering choices behind the numbers are in the ADR.
//!
//! - **Seeds:** the listener's 3 strongest established artists and 2 strongest tags always seed
//!   picks (the core); 1 more artist and up to 2 more tags are drawn by weight from the next ones
//!   (the edge, `mix::draw`). How each seed's picks fared moves its weight, within bounds.
//! - **Mix:** about a fifth of the shelf comes from the edge (3 of 12 for a listener of few tags,
//!   2 for a broad one); the rest from the core. One song per artist, originals first, nothing
//!   the listener knows, has hidden or holds back, nothing by an artist they already like.
//! - **Turnover:** an automatic rebuild keeps up to half the shelf's picks not taken yet; a pick
//!   gives way once it has been on 3 shelves. Refresh, the listener's own, keeps none.

use std::collections::{HashMap, HashSet};

use lull_core::ids::{ArtistId, TrackId};

use crate::feed::{Pick, Reason, ReasonKind, RecentPicks};
use crate::sources::Song;
use crate::taste::Taste;
use crate::{mix, radio, store};

pub const PICKS: usize = 12;
const CORE_ARTISTS: usize = 3;
const EDGE_ARTISTS: usize = 1;
const CORE_TAGS: usize = 2;
const EDGE_TAGS: usize = 2;
/// Established artists and tags the core and edge are taken from.
const SEED_POOL: usize = 12;
/// Picks from the edge: more for a listener of few tags (a broad shelf helps them, and slightly
/// hurts listeners already broad).
const EDGE_PICKS_NARROW: usize = 3;
const EDGE_PICKS_BROAD: usize = 2;
/// A listener with fewer tags dug into than this is narrow.
const BROAD_TAGS: usize = 4;
/// Picks not taken yet an automatic rebuild keeps, at most: half the shelf.
pub const CARRY: usize = PICKS / 2;
/// Shelves a pick may be on before it gives way.
pub const SHELVES: usize = 3;

/// A pick before it is a track: the song, why, from where, and whether an edge seed found it.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub song: Song,
    pub reason: Reason,
    pub via: &'static str,
    pub edge: bool,
}

/// What seeds this build's picks.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Seeds {
    /// Artist id, name, from the edge.
    pub artists: Vec<(ArtistId, String, bool)>,
    /// Tag, from the edge.
    pub tags: Vec<(String, bool)>,
}

/// How each seed's picks fared (`store::seed_outcomes`, already shrunk towards none): a seed
/// whose picks come back weighs up to twice as much, one whose picks are skipped down to half.
#[derive(Debug, Default)]
pub struct Fared(pub HashMap<(String, String), f64>);

impl Fared {
    /// Twice the weight for every 3 points, within half and twice.
    pub fn factor(&self, kind: &str, seed: &str) -> f64 {
        let score = self.0.get(&(kind.to_owned(), seed.to_owned())).copied().unwrap_or(0.0);
        2f64.powf((score / 3.0).clamp(-1.0, 1.0))
    }
}

/// The seeds: the core always, the edge drawn with `seed`. On a cold start (no tag known),
/// three of `cold_tags`, a different three each day.
pub fn choose_seeds(
    taste: &Taste,
    followed: &[String],
    fared: &Fared,
    cold_tags: &[&str],
    now: i64,
    seed: u64,
) -> Seeds {
    let mut artists: Vec<(&crate::taste::Weighted, f64)> = taste
        .seed_artists(SEED_POOL)
        .map(|a| (a, a.weight * fared.factor("artist", &a.id.0)))
        .collect();
    artists.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.name.cmp(&b.0.name)));
    let rest = artists.split_off(CORE_ARTISTS.min(artists.len()));
    let mut out = Seeds {
        artists: artists
            .iter()
            .map(|(a, _)| (a.id.clone(), a.name.clone(), false))
            .collect(),
        tags: Vec::new(),
    };
    for a in mix::draw(rest, EDGE_ARTISTS, seed) {
        out.artists.push((a.id.clone(), a.name.clone(), true));
    }

    // Followed tags first (a standing wish), then the strongest dug into.
    let weight = |t: &str| taste.tags.iter().find(|(x, _)| x == t).map_or(0.0, |(_, w)| *w);
    let mut tags: Vec<(String, f64)> = followed
        .iter()
        .map(|t| {
            (
                t.clone(),
                weight(t).max(crate::taste::weight::FOLLOW) * fared.factor("tag", t),
            )
        })
        .collect();
    tags.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let mut dug: Vec<(String, f64)> = taste
        .dig_tags()
        .filter(|(t, _)| !followed.contains(t))
        .take(SEED_POOL)
        .map(|(t, w)| (t.clone(), w * fared.factor("tag", t)))
        .collect();
    dug.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    tags.extend(dug);
    let rest = tags.split_off(CORE_TAGS.min(tags.len()));
    out.tags = tags.into_iter().map(|(t, _)| (t, false)).collect();
    out.tags.extend(
        mix::draw(rest, EDGE_TAGS, seed.rotate_left(17) | 1)
            .into_iter()
            .map(|t| (t, true)),
    );
    if out.tags.is_empty() {
        let day = now.div_euclid(86_400) as usize;
        out.tags = (0..3)
            .map(|i| (cold_tags[(day * 3 + i) % cold_tags.len()].to_owned(), false))
            .collect();
    }
    out
}

/// How many of `slots` come from the edge.
pub fn edge_share(taste: &Taste, slots: usize) -> usize {
    let n = if taste.dig_tags().count() < BROAD_TAGS {
        EDGE_PICKS_NARROW
    } else {
        EDGE_PICKS_BROAD
    };
    (n * slots).div_ceil(PICKS).min(slots)
}

/// Fills `slots` from the pools, `edge` of them from edge pools: round-robin within each side,
/// each pool in a new order (`seed`), originals first, one song per artist, nothing in `skip`.
/// A side that runs out leaves its slots to the other.
pub fn mix(
    pools: Vec<Vec<Candidate>>,
    taste: &Taste,
    skip: &HashSet<TrackId>,
    slots: usize,
    edge: usize,
    seed: u64,
) -> Vec<Candidate> {
    let (mut edges, mut cores): (Vec<_>, Vec<_>) = pools.into_iter().partition(|p| p.first().is_some_and(|c| c.edge));
    for (i, pool) in edges.iter_mut().chain(cores.iter_mut()).enumerate() {
        mix::shuffle(pool, seed.rotate_left(i as u32 + 1) | 1);
        pool.sort_by_key(|c| radio::is_version(&c.song.title));
        pool.reverse(); // pop() takes from the front
    }
    let mut artists = HashSet::new();
    let mut out = Vec::new();
    let from_edges = take(&mut edges, edge, taste, skip, &mut artists);
    let from_cores = take(&mut cores, slots - from_edges.len(), taste, skip, &mut artists);
    let short = slots.saturating_sub(from_edges.len() + from_cores.len());
    let more_edges = take(&mut edges, short, taste, skip, &mut artists);
    out.extend(from_cores);
    out.extend(from_edges);
    out.extend(more_edges);
    out
}

/// Up to `n` round-robin from `pools`, by the pick rules.
fn take(
    pools: &mut [Vec<Candidate>],
    n: usize,
    taste: &Taste,
    skip: &HashSet<TrackId>,
    artists: &mut HashSet<ArtistId>,
) -> Vec<Candidate> {
    let mut out = Vec::new();
    while out.len() < n && pools.iter().any(|p| !p.is_empty()) {
        for pool in pools.iter_mut() {
            while let Some(c) = pool.pop() {
                let t = store::track(c.song.clone());
                let familiar = taste.artist_weight(&t.artist_id) > 0.0 && c.reason.kind != ReasonKind::Fresh;
                if taste.known.contains(&t.id)
                    || skip.contains(&t.id)
                    || taste.knows_song(&c.song.artist, &c.song.title)
                    || taste.hidden.contains(&t.artist_id)
                    || taste.held_back(&t.artist_id)
                    || familiar
                    || !artists.insert(t.artist_id)
                {
                    continue;
                }
                out.push(c);
                break;
            }
            if out.len() >= n {
                break;
            }
        }
    }
    out
}

/// The last shelf's picks an automatic rebuild keeps: not heard or liked since, not by an artist
/// hidden or held back, on fewer than `SHELVES` shelves, at most `CARRY`. `artists` names each
/// pick's artist (from the last shelf's tracks).
pub fn carry(last: &[Pick], artists: &HashMap<TrackId, ArtistId>, taste: &Taste, recent: &RecentPicks) -> Vec<Pick> {
    last.iter()
        .filter(|p| {
            let shelves = recent.shelves.iter().filter(|s| s.contains(&p.track_id)).count();
            let artist = artists.get(&p.track_id);
            shelves < SHELVES
                && !taste.known.contains(&p.track_id)
                && artist.is_none_or(|a| !taste.hidden.contains(a) && !taste.held_back(a))
        })
        .take(CARRY)
        .cloned()
        .collect()
}

#[cfg(test)]
mod sim;
