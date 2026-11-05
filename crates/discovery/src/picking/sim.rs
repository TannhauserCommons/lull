//! Simulated listeners (ADR-0100): how much picks move after what a listener does, measured with
//! the real taste model, seed choice, mix and carry, over fake sources. Nothing outside is asked.
//!
//! - **Seed shift:** the total-variation distance between two builds' seed distributions (each
//!   seed's share of 2,000 draws): 0 is the same seeds as often, 1 entirely different ones.
//! - **Turnover:** the share of the 12 picks that changed from one build to the next.
//! - **Edge picks:** picks a fresh-edge seed found.
//!
//! `before` runs the rules main had before (`legacy_seeds`: the 4 strongest artists and tags of an
//! undamped profile, no core or edge) on the same listeners, for comparison. `cargo test -p
//! lull-discovery sim -- --nocapture` prints the table the ADR quotes.

use std::collections::{HashMap, HashSet};

use lull_core::ids::{ArtistId, TrackId};

use super::{Candidate, Fared, PICKS, carry, choose_seeds, edge_share, mix};
use crate::feed::{Pick, Reason, ReasonKind, RecentPicks};
use crate::sources::Song;
use crate::store;
use crate::taste::{Evidence, Mark, Play, Signals, Taste};

const DAY: i64 = 86_400;
const START: i64 = 1_800_000_000;
const COLD: &[&str] = &["dream pop", "slowcore", "city pop"];
const GENRES: &[&str] = &[
    "dream pop",
    "shoegaze",
    "acid jazz",
    "minimal techno",
    "city pop",
    "doom metal",
    "freak folk",
    "dark ambient",
];
const DRAWS: u64 = 2_000;

/// An invented artist of a genre.
fn artist(genre: &str, k: usize) -> String {
    format!("{genre} artist {k}")
}

/// A listener's signals as days go by.
#[derive(Default, Clone)]
struct Listener {
    s: Signals,
    followed_tags: Vec<String>,
}

impl Listener {
    fn new() -> Self {
        let mut s = Signals::default();
        for g in GENRES {
            for k in 0..15 {
                s.artist_tags.insert(ArtistId::of(&artist(g, k)), vec![(*g).to_owned()]);
            }
        }
        Listener {
            s,
            followed_tags: Vec::new(),
        }
    }

    fn play(&mut self, artist: &str, day: i64, n: usize, skipped: bool) {
        for i in 0..n {
            let title = format!("{artist} song {}", (day as usize * 7 + i) % 40);
            self.s.plays.push(Play {
                track: TrackId::of(artist, "", &title),
                artist: artist.to_owned(),
                at: START + day * DAY + 3_600 + i as i64 * 240,
                played_s: if skipped { 10 } else { 200 },
                duration_s: 200,
                completed: !skipped,
                skipped,
            });
        }
    }

    fn like(&mut self, artist: &str, day: i64) {
        self.s.likes.push(Mark {
            track: TrackId::of(artist, "", "liked"),
            artist: artist.to_owned(),
            at: START + day * DAY,
        });
    }

    fn taste(&self, day: i64) -> Taste {
        Taste::build(&self.s, START + day * DAY + DAY / 2)
    }
}

/// The songs a seed's source would offer: 6 by artists the listener has never met.
fn pool(seed: &str, edge: bool, kind: ReasonKind) -> Vec<Candidate> {
    (0..6)
        .map(|k| Candidate {
            song: Song {
                mbid: None,
                title: format!("{seed} find {k}"),
                artist: format!("near {seed} {k}"),
                artist_mbid: None,
                album: String::new(),
                album_artist: None,
                album_mbid: None,
                year: None,
                no: None,
                disc: None,
                duration_s: None,
                tags: Vec::new(),
                artist_source: None,
                isrc: None,
            },
            reason: Reason {
                kind,
                name: seed.to_owned(),
                reference: Some(seed.to_owned()),
                evidence: Evidence::default(),
            },
            via: "simulated",
            edge,
        })
        .collect()
}

/// One build as `feed::build` makes it: carry (unless `refresh`), then mix the seeds' pools.
struct Build {
    picks: Vec<Pick>,
    edge: usize,
}

fn build(taste: &Taste, l: &Listener, last: &[Pick], recent: &mut RecentPicks, refresh: bool, seed: u64) -> Build {
    let artists: HashMap<TrackId, ArtistId> = last
        .iter()
        .map(|p| (p.track_id.clone(), ArtistId::of(&p.reasons[0].name)))
        .collect();
    let carried = if refresh {
        Vec::new()
    } else {
        carry(last, &artists, taste, recent)
    };
    let skip: HashSet<TrackId> = recent.shelves.iter().flatten().cloned().collect();
    let seeds = choose_seeds(taste, &l.followed_tags, &Fared::default(), COLD, START, seed);
    let mut pools: Vec<Vec<Candidate>> = Vec::new();
    for (_, name, edge) in &seeds.artists {
        pools.push(pool(name, *edge, ReasonKind::Like));
    }
    for (tag, edge) in &seeds.tags {
        pools.push(pool(tag, *edge, ReasonKind::Tag));
    }
    let slots = PICKS - carried.len();
    let chosen = mix(pools, taste, &skip, slots, edge_share(taste, slots), seed);
    let edge = chosen.iter().filter(|c| c.edge).count();
    let mut picks: Vec<Pick> = chosen
        .into_iter()
        .map(|c| Pick {
            track_id: store::track(c.song).id,
            reasons: vec![c.reason],
            via: c.via.to_owned(),
            avail: Vec::new(),
            no_source: false,
        })
        .collect();
    picks.extend(carried);
    recent.shelves.push(picks.iter().map(|p| p.track_id.clone()).collect());
    if recent.shelves.len() > super::SHELVES {
        recent.shelves.remove(0);
    }
    Build { picks, edge }
}

/// Each seed's share of `DRAWS` seed choices.
fn seed_shares(taste: &Taste, l: &Listener) -> HashMap<String, f64> {
    let mut counts: HashMap<String, f64> = HashMap::new();
    let mut total: f64 = 0.0;
    for d in 1..=DRAWS {
        let s = choose_seeds(
            taste,
            &l.followed_tags,
            &Fared::default(),
            COLD,
            START,
            d.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1,
        );
        for (id, _, _) in &s.artists {
            *counts.entry(format!("a:{}", id.0)).or_default() += 1.0;
            total += 1.0;
        }
        for (t, _) in &s.tags {
            *counts.entry(format!("t:{t}")).or_default() += 1.0;
            total += 1.0;
        }
    }
    counts.values_mut().for_each(|c| *c /= total.max(1.0));
    counts
}

/// The rules before: the 4 strongest artists and the strongest followed and dug-into tags of
/// a profile that counts every play and every artist, every build the same.
fn legacy_shares(l: &Listener, day: i64) -> HashMap<String, f64> {
    let now = START + day * DAY + DAY / 2;
    let mut w: HashMap<String, f64> = HashMap::new();
    let mut tags: HashMap<String, (f64, usize)> = HashMap::new();
    for p in &l.s.plays {
        let v = if p.completed {
            1.0
        } else if p.skipped {
            -0.5
        } else {
            0.0
        };
        *w.entry(p.artist.clone()).or_default() += v * crate::taste::fade(now - p.at);
    }
    for m in &l.s.likes {
        *w.entry(m.artist.clone()).or_default() += 3.0 * crate::taste::fade(now - m.at);
    }
    for (a, v) in &w {
        for t in l.s.artist_tags.get(&ArtistId::of(a)).into_iter().flatten() {
            let e = tags.entry(t.clone()).or_default();
            e.0 += v;
            e.1 += usize::from(*v > 0.0);
        }
    }
    let mut artists: Vec<(String, f64)> = w.into_iter().filter(|(_, v)| *v > 0.0).collect();
    artists.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let mut tags: Vec<(String, f64)> = tags
        .into_iter()
        .filter(|(_, (_, n))| *n >= 2)
        .map(|(t, (v, _))| (t, v))
        .collect();
    tags.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let items: Vec<String> = artists
        .iter()
        .take(4)
        .map(|(a, _)| format!("a:{}", ArtistId::of(a).0))
        .chain(l.followed_tags.iter().map(|t| format!("t:{t}")))
        .chain(tags.iter().take(4).map(|(t, _)| format!("t:{t}")))
        .take(8)
        .collect();
    let share = 1.0 / items.len().max(1) as f64;
    items.into_iter().map(|i| (i, share)).collect()
}

fn tvd(a: &HashMap<String, f64>, b: &HashMap<String, f64>) -> f64 {
    let keys: HashSet<&String> = a.keys().chain(b.keys()).collect();
    keys.into_iter()
        .map(|k| (a.get(k).copied().unwrap_or(0.0) - b.get(k).copied().unwrap_or(0.0)).abs())
        .sum::<f64>()
        / 2.0
}

fn turnover(a: &[Pick], b: &[Pick]) -> f64 {
    let a: HashSet<&TrackId> = a.iter().map(|p| &p.track_id).collect();
    let kept = b.iter().filter(|p| a.contains(&p.track_id)).count();
    1.0 - kept as f64 / PICKS as f64
}

/// A steady listener of one genre: 60 days of dream pop, 2 of 8 artists a day, 6 songs, some liked.
fn steady() -> Listener {
    let mut l = Listener::new();
    for day in 0..60 {
        for k in 0..2 {
            l.play(&artist("dream pop", (day as usize * 3 + k) % 8), day, 3, false);
        }
        if day % 10 == 0 {
            l.like(&artist("dream pop", day as usize % 8), day);
        }
    }
    l
}

/// Prints one row of the report.
fn row(persona: &str, event: &str, measured: &str, before: &str) {
    println!("| {persona} | {event} | {measured} | {before} |");
}

#[test]
fn steady_listener_gets_a_stable_shelf_with_a_fresh_edge() {
    let l = steady();
    let taste = l.taste(59);
    let mut recent = RecentPicks::default();
    let first = build(&taste, &l, &[], &mut recent, true, 11);
    let second = build(&taste, &l, &first.picks, &mut recent, false, 12);
    let shift = tvd(&seed_shares(&l.taste(58), &l), &seed_shares(&taste, &l));
    let t = turnover(&first.picks, &second.picks);
    row(
        "steady, one genre",
        "next automatic build",
        &format!(
            "turnover {:.0}%, seed shift {shift:.3}, edge picks {}",
            t * 100.0,
            first.edge
        ),
        &format!("seed shift {:.3}", tvd(&legacy_shares(&l, 58), &legacy_shares(&l, 59))),
    );
    assert_eq!(first.picks.len(), PICKS);
    assert!((0.30..=0.50).contains(&t), "turnover {t}");
    assert!(shift < 0.10, "seed shift {shift}");
    assert!((2..=3).contains(&first.edge), "edge picks {}", first.edge);
}

#[test]
fn an_eclectic_listener_is_seeded_from_many_artists_over_a_week() {
    let mut l = Listener::new();
    for day in 0..60 {
        for k in 0..3 {
            let g = GENRES[(day as usize * 3 + k) % GENRES.len()];
            l.play(&artist(g, (day as usize * 5 + k) % 15), day, 3, false);
        }
    }
    let mut seen = HashSet::new();
    let mut recent = RecentPicks::default();
    let mut last = Vec::new();
    let mut turnovers = Vec::new();
    let mut edges = Vec::new();
    for day in 53..60 {
        let b = build(&l.taste(day), &l, &last, &mut recent, false, day as u64 * 31 + 1);
        for p in &b.picks {
            if p.reasons[0].kind == ReasonKind::Like {
                seen.insert(p.reasons[0].name.clone());
            }
        }
        if !last.is_empty() {
            turnovers.push(turnover(&last, &b.picks));
        }
        edges.push(b.edge);
        last = b.picks;
    }
    let mean = turnovers.iter().sum::<f64>() / turnovers.len() as f64;
    row(
        "wide, eclectic",
        "7 days of builds",
        &format!(
            "{} seed artists, mean turnover {:.0}%, edge picks {:?}",
            seen.len(),
            mean * 100.0,
            edges
        ),
        "4 seed artists every build",
    );
    assert!(seen.len() >= 8, "seed artists over a week: {}", seen.len());
    assert!((0.30..=0.50).contains(&mean), "turnover {mean}");
}

#[test]
fn a_day_of_bingeing_one_new_artist_barely_moves_the_seeds() {
    let mut l = steady();
    let before = l.taste(59);
    let shares = seed_shares(&before, &l);
    let legacy = legacy_shares(&l, 59);
    let binge = artist("doom metal", 0);
    l.play(&binge, 60, 40, false);
    let after = l.taste(60);
    let shift = tvd(&shares, &seed_shares(&after, &l));
    let legacy_shift = tvd(&legacy, &legacy_shares(&l, 60));
    let key = format!("a:{}", ArtistId::of(&binge).0);
    let seeds_binge = |t: &Taste, l: &Listener| seed_shares(t, l).get(&key).copied().unwrap_or(0.0);
    let one_day = seeds_binge(&after, &l);
    for day in 61..63 {
        l.play(&binge, day, 40, false);
    }
    let three_days = seeds_binge(&l.taste(62), &l);
    row(
        "binges a new artist",
        "one day (40 plays), then 3 days",
        &format!("seed shift {shift:.3}; its seed share {one_day:.2} after a day, {three_days:.2} after 3 days"),
        &format!("seed shift {legacy_shift:.3}"),
    );
    assert_eq!(one_day, 0.0, "one day doesn't make a seed");
    assert!(shift < 0.15, "seed shift {shift}");
    assert!(three_days > 0.0, "three days do");
}

#[test]
fn one_stray_play_in_an_unusual_genre_moves_nothing() {
    let mut l = steady();
    let shares = seed_shares(&l.taste(59), &l);
    let legacy = legacy_shares(&l, 59);
    let stray = artist("dark ambient", 3);
    l.play(&stray, 60, 1, false);
    let after = seed_shares(&l.taste(60), &l);
    let shift = tvd(&shares, &after);
    row(
        "one stray play",
        "next build",
        &format!("seed shift {shift:.3}"),
        &format!("seed shift {:.3}", tvd(&legacy, &legacy_shares(&l, 60))),
    );
    assert!(shift < 0.02, "seed shift {shift}");
    assert!(
        !after
            .keys()
            .any(|k| k.contains(&ArtistId::of(&stray).0) || k == "t:dark ambient")
    );
}

#[test]
fn a_new_listener_starts_from_starter_tags_or_their_library() {
    let l = Listener::new();
    let taste = l.taste(0);
    let seeds = choose_seeds(&taste, &[], &Fared::default(), COLD, START, 5);
    assert!(taste.is_empty());
    assert!(seeds.artists.is_empty());
    assert_eq!(seeds.tags.len(), 3, "three starter tags");

    let mut l = Listener::new();
    l.s.library = (0..12).map(|k| (artist("city pop", k), 3 + k as u64)).collect();
    let taste = l.taste(0);
    let seeds = choose_seeds(&taste, &[], &Fared::default(), COLD, START, 5);
    row(
        "new listener",
        "first build, nothing / a library",
        &format!(
            "starter tags / {} library artists and {:?} seed",
            seeds.artists.len(),
            seeds.tags.iter().map(|t| &t.0).collect::<Vec<_>>()
        ),
        "starter tags / starter tags",
    );
    assert!(!taste.is_empty(), "a library is a taste");
    assert!(seeds.artists.iter().all(|(_, n, _)| n.starts_with("city pop")));
    assert!(!seeds.artists.is_empty());
}

#[test]
fn a_listener_back_after_months_keeps_their_anchors_and_new_listening_leads_within_a_week() {
    let mut l = steady();
    l.s.followed_artists = vec![artist("dream pop", 1)];
    l.s.library = (0..6).map(|k| (artist("shoegaze", k), 4)).collect();
    let back = 180;
    let taste = l.taste(back);
    let shares = seed_shares(&taste, &l);
    assert!(!shares.is_empty(), "the follows and library still seed");
    for day in back..back + 7 {
        for k in 0..3 {
            l.play(&artist("city pop", k), day, 4, false);
        }
    }
    let first_day = {
        let mut once = l.clone();
        once.s.plays.retain(|p| p.at < START + (back + 1) * DAY);
        tvd(&shares, &seed_shares(&once.taste(back), &once))
    };
    let week = l.taste(back + 6);
    let lead = week.seed_artists(1).next().map(|a| a.name.clone()).unwrap_or_default();
    row(
        "back after 4 months",
        "first day back, then a week",
        &format!("anchors seed; seed shift {first_day:.3} after a day; leading after a week: {lead}"),
        "n/a",
    );
    assert!(first_day < 0.3, "seed shift {first_day}");
    assert!(lead.starts_with("city pop"), "new listening leads after a week: {lead}");
}

#[test]
fn a_heavy_skipper_still_gets_a_full_shelf() {
    let mut l = Listener::new();
    for day in 0..30 {
        for k in 0..10 {
            let a = artist(GENRES[k % 4], (day as usize + k) % 15);
            l.play(&a, day, 1, k % 10 < 6);
        }
    }
    let taste = l.taste(29);
    let held: Vec<_> = (0..15)
        .flat_map(|k| GENRES[..4].iter().map(move |g| artist(g, k)))
        .filter(|a| taste.held_back(&ArtistId::of(a)))
        .collect();
    let mut recent = RecentPicks::default();
    let b = build(&taste, &l, &[], &mut recent, true, 3);
    row(
        "heavy skipper (60%)",
        "a month",
        &format!("{} picks; {} artists held back", b.picks.len(), held.len()),
        "n/a",
    );
    assert!(b.picks.len() >= 10, "picks {}", b.picks.len());
    for a in held {
        let days: HashSet<i64> =
            l.s.plays
                .iter()
                .filter(|p| p.artist == a && p.skipped)
                .map(|p| p.at.div_euclid(DAY))
                .collect();
        assert!(days.len() >= 2, "{a} held back after skips on {} day(s)", days.len());
    }
}
