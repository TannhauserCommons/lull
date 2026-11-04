//! Every peer file a search has turned up, by song: what the source picker lists and what a
//! stream falls back through, in the order the listener's profile sets (ADR-0008). Held in
//! memory (searches are cheap to repeat, peers come and go) and bounded by age.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use lull_core::ids::TrackId;
use lull_core::model::{FileFacts, SourceRef};
use lull_core::quality::{Minimum, Pick, Quality, format_of};
use serde::Serialize;

use crate::peers::Peers;
use crate::rank::{self, Factors, Offer};
use crate::rules::{Purpose, Rules};

/// Peer answers go stale: slots fill, peers leave. Older ones are dropped.
const KEEP_FOR: Duration = Duration::from_secs(6 * 3600);

/// One file on one peer, as a search reported it.
#[derive(Debug, Clone)]
pub struct Source {
    pub user: String,
    pub path: String,
    pub size: u64,
    pub ext: String,
    pub facts: FileFacts,
    /// Bytes per second.
    pub upload_speed: u64,
    pub free_slot: bool,
    pub queue: u32,
    /// Seconds, when the peer reports it.
    pub length: Option<u32>,
}

impl Source {
    pub fn source_ref(&self) -> SourceRef {
        SourceRef {
            user: self.user.clone(),
            path: self.path.clone(),
            size: self.size,
        }
    }

    pub fn is(&self, r: &SourceRef) -> bool {
        self.user == r.user && self.path == r.path
    }

    /// The bitrate from the peer's attributes, else from the size and `duration` (the song's).
    pub fn quality(&self, duration: Option<u32>) -> Quality {
        match self.facts.kbps {
            Some(_) => Quality::of(Some(&self.facts)),
            None => Quality::of_file(&self.path, self.size, self.length.or(duration)),
        }
    }

    pub fn meets(&self, min: &Minimum, duration: Option<u32>) -> bool {
        self.quality(duration).meets(format_of(&self.ext), min)
    }

    /// The order `highest` sorts by: lossless first, hi-res lossless above CD, then bitrate.
    fn quality_rank(&self, duration: Option<u32>) -> (bool, u32) {
        let q = self.quality(duration);
        match q.lossless {
            true => {
                let hi_res = self.facts.bits.is_some_and(|b| b > 16) || self.facts.khz.is_some_and(|k| k > 48.0);
                (true, u32::from(hi_res))
            }
            false => (false, q.kbps),
        }
    }

    /// Busy, with more people waiting than the listener allows: never picked automatically.
    pub fn queue_too_long(&self, max: u32) -> bool {
        !self.free_slot && self.queue > max
    }
}

/// A source with its score, as `GET /api/tracks/{id}/sources` returns it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Ranked {
    pub peer: String,
    pub path: String,
    pub format: String,
    pub size_bytes: u64,
    /// MB/s, one decimal: what lull measured from the peer, else what the peer reports.
    pub speed: f32,
    /// `speed` is lull's measurement (ADR-0015), not the peer's own figure.
    pub speed_measured: bool,
    /// When it was last measured (unix seconds).
    pub measured_at: Option<i64>,
    /// Bytes a second ranking expects: measured, else the report as reports have proved here.
    #[serde(skip)]
    pub expected: f64,
    /// The same in MB/s, one decimal: what the picker sorts by speed on.
    pub expected_speed: f32,
    pub free_slot: bool,
    pub queue: u32,
    pub score: u8,
    pub factors: Factors,
    pub preferred: bool,
    pub stalled_before: bool,
    /// Passes the minimum of the profile the list was ordered for.
    pub meets: bool,
    /// More people waiting than Settings › Sources allows: listed, never picked automatically.
    pub queue_too_long: bool,
    /// Above the streaming cap (`maxKbps`; lossless is above any cap): taken only after every
    /// copy within it. Always false in the saving order.
    pub above_max: bool,
    #[serde(skip)]
    pub source: Source,
}

#[derive(Clone)]
pub struct Sources {
    inner: Arc<RwLock<HashMap<TrackId, Held>>>,
    peers: Peers,
    preferred: Arc<[String]>,
}

struct Held {
    sources: Vec<Source>,
    seen: Instant,
}

impl Sources {
    pub fn new(peers: Peers, preferred: Vec<String>) -> Self {
        Sources {
            inner: Arc::default(),
            peers,
            preferred: preferred.into(),
        }
    }

    /// Adds what a search found; a file seen again replaces its older report.
    pub fn add(&self, found: HashMap<TrackId, Vec<Source>>) {
        let Ok(mut map) = self.inner.write() else { return };
        let now = Instant::now();
        map.retain(|_, h| now.duration_since(h.seen) < KEEP_FOR);
        for (track, sources) in found {
            let held = map.entry(track).or_insert_with(|| Held {
                sources: Vec::new(),
                seen: now,
            });
            held.seen = now;
            for s in sources {
                held.sources.retain(|old| !(old.user == s.user && old.path == s.path));
                held.sources.push(s);
            }
        }
    }

    /// The track's sources in the order the profile for `purpose` takes them: copies that
    /// pass its minimum by its `pick`, then the rest by the same order, then peers whose queue
    /// is too long. `duration` (the song's) judges files whose bitrate the peer didn't report.
    pub fn ordered(&self, track: &TrackId, duration: Option<u32>, rules: &Rules, purpose: Purpose) -> Vec<Ranked> {
        let sources = self
            .inner
            .read()
            .ok()
            .and_then(|m| m.get(track).map(|h| h.sources.clone()))
            .unwrap_or_default();
        let ranked = sources.into_iter().map(|s| self.rank(s)).collect();
        order(ranked, duration, rules, purpose)
    }

    /// One known source of the track, by reference.
    pub fn get(&self, track: &TrackId, r: &SourceRef) -> Option<Source> {
        let map = self.inner.read().ok()?;
        map.get(track)?.sources.iter().find(|s| s.is(r)).cloned()
    }

    /// Whether any source is known for the track.
    pub fn known(&self, track: &TrackId) -> bool {
        self.inner
            .read()
            .is_ok_and(|m| m.get(track).is_some_and(|h| !h.sources.is_empty()))
    }

    pub fn rank(&self, s: Source) -> Ranked {
        let stalled_before = self.peers.stalled_before(&s.user);
        let (expected, measured) = self.peers.expected(&s.user, s.upload_speed);
        let shown = measured.map_or(s.upload_speed as f64, |m| m.rate);
        let factors = rank::factors(
            &Offer {
                ext: &s.ext,
                lossless: s.facts.lossless,
                kbps: s.facts.kbps,
                khz: s.facts.khz,
                bits: s.facts.bits,
                speed: expected as u64,
                free_slot: s.free_slot,
                queue: s.queue,
                stalled_before,
            },
            &self.preferred,
        );
        Ranked {
            peer: s.user.clone(),
            path: s.path.clone(),
            format: s.facts.format.clone(),
            size_bytes: s.size,
            speed: (shown as f32 / 100_000.0).round() / 10.0,
            speed_measured: measured.is_some(),
            measured_at: measured.map(|m| m.at),
            expected,
            expected_speed: (expected as f32 / 100_000.0).round() / 10.0,
            free_slot: s.free_slot,
            queue: s.queue,
            score: factors.score(),
            factors,
            preferred: self.preferred.first() == Some(&s.ext),
            stalled_before,
            meets: true,
            queue_too_long: false,
            above_max: false,
            source: s,
        }
    }
}

/// Sorts ranked sources for a profile and marks which pass its minimum. Streaming also puts
/// copies within its bitrate cap before those above it, among passing copies and among the rest.
pub fn order(mut ranked: Vec<Ranked>, duration: Option<u32>, rules: &Rules, purpose: Purpose) -> Vec<Ranked> {
    let min = rules.minimum(purpose);
    let cap = match purpose {
        Purpose::Stream => rules.stream.max_kbps,
        Purpose::Save => 0,
    };
    for r in &mut ranked {
        r.meets = r.source.meets(min, duration);
        r.queue_too_long = r.source.queue_too_long(rules.max_peer_queue);
        r.above_max = !r.source.quality(duration).within(cap);
    }
    let fastest = |a: &Ranked, b: &Ranked| b.score.cmp(&a.score).then(b.expected.total_cmp(&a.expected));
    ranked.sort_by(|a, b| {
        a.queue_too_long
            .cmp(&b.queue_too_long)
            .then(b.meets.cmp(&a.meets))
            .then(a.above_max.cmp(&b.above_max))
            .then_with(|| match rules.pick(purpose) {
                Pick::Fastest => fastest(a, b),
                Pick::Highest => b
                    .source
                    .quality_rank(duration)
                    .cmp(&a.source.quality_rank(duration))
                    .then_with(|| fastest(a, b)),
            })
    });
    ranked
}

/// Why the copy playing is a lesser one, worth switching away from mid-song.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lesser {
    /// YouTube's copy (a fallback, or start fast).
    Youtube,
    /// Below the streaming minimum (`playBest`).
    Below,
    /// A peer the fetch fell back to after better ones failed.
    FellBack,
}

/// What a better copy has to beat: the copy playing, why it is a lesser one, and the peers
/// that already failed this play.
#[derive(Debug, Clone)]
pub struct Bar {
    pub playing: SourceRef,
    pub why: Lesser,
    pub avoid: Vec<String>,
}

/// The copies in `ordered` (the streaming order) better than the one playing, best first: they
/// pass the minimum, aren't busy or failed already, and, when the copy playing passes too (a
/// peer fallen back to), come before it in the order.
pub fn better(ordered: Vec<Ranked>, bar: &Bar) -> Vec<Ranked> {
    let playing_at = ordered.iter().position(|r| r.source.is(&bar.playing));
    ordered
        .into_iter()
        .enumerate()
        .filter(|(i, r)| {
            let above = bar.why != Lesser::FellBack || playing_at.is_none_or(|at| *i < at);
            above && r.meets && !r.queue_too_long && !r.source.is(&bar.playing) && !bar.avoid.contains(&r.peer)
        })
        .map(|(_, r)| r)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lull_core::quality::{Minimum, Pick};

    /// A peer's copy: extension, bitrate (0 = unreported), MB/s, free slot, queue.
    fn copy(user: &str, ext: &str, kbps: u32, mbps: f32, free_slot: bool, queue: u32) -> Ranked {
        let lossless = matches!(ext, "flac" | "wav");
        let source = Source {
            user: user.into(),
            path: format!("Music\\A\\B\\01 - Song.{ext}"),
            size: 10_000_000,
            ext: ext.into(),
            facts: FileFacts {
                format: ext.to_uppercase(),
                codec: ext.to_uppercase(),
                lossless,
                khz: Some(44.1),
                bits: lossless.then_some(16),
                kbps: (kbps > 0).then_some(kbps),
                size_bytes: Some(10_000_000),
            },
            upload_speed: (mbps * 1_000_000.0) as u64,
            free_slot,
            queue,
            length: None,
        };
        let factors = rank::factors(
            &Offer {
                ext,
                lossless,
                kbps: source.facts.kbps,
                khz: source.facts.khz,
                bits: source.facts.bits,
                speed: source.upload_speed,
                free_slot,
                queue,
                stalled_before: false,
            },
            &["flac", "mp3"].map(String::from),
        );
        Ranked {
            peer: user.into(),
            path: source.path.clone(),
            format: source.facts.format.clone(),
            size_bytes: source.size,
            speed: mbps,
            speed_measured: false,
            measured_at: None,
            expected: source.upload_speed as f64,
            expected_speed: mbps,
            free_slot,
            queue,
            score: factors.score(),
            factors,
            preferred: false,
            stalled_before: false,
            meets: true,
            queue_too_long: false,
            above_max: false,
            source,
        }
    }

    fn peers() -> Vec<Ranked> {
        vec![
            copy("slow-flac", "flac", 0, 0.2, true, 0),
            copy("fast-320", "mp3", 320, 3.0, true, 0),
            copy("fast-128", "mp3", 128, 3.3, true, 0),
            copy("mid-256", "mp3", 256, 1.5, true, 0),
            copy("busy-flac", "flac", 0, 3.0, false, 400),
            copy("unknown-mp3", "mp3", 0, 3.3, true, 0),
        ]
    }

    fn rules(kbps: u32, pick: Pick) -> Rules {
        let mut r = Rules::default();
        r.stream.minimum = Minimum {
            lossless_only: false,
            kbps,
            formats: vec![],
        };
        r.stream.pick = pick;
        r
    }

    fn users(ranked: &[Ranked]) -> Vec<&str> {
        ranked.iter().map(|r| r.peer.as_str()).collect()
    }

    #[test]
    fn the_minimum_gates_by_bitrate_kind_and_format() {
        let ordered = order(peers(), Some(240), &rules(256, Pick::Fastest), Purpose::Stream);
        let passing: Vec<&str> = ordered.iter().filter(|r| r.meets).map(|r| r.peer.as_str()).collect();
        assert!(passing.contains(&"slow-flac"), "lossless passes any lossy floor");
        assert!(passing.contains(&"mid-256"));
        assert!(!passing.contains(&"fast-128"));
        assert!(
            passing.contains(&"unknown-mp3"),
            "an unreported bitrate comes from size and length: 10 MB over 240 s"
        );
        let unknown_length = order(peers(), None, &rules(256, Pick::Fastest), Purpose::Stream);
        assert!(
            unknown_length.iter().any(|r| r.peer == "unknown-mp3" && !r.meets),
            "a lossy file of unknown bitrate can't be shown to pass"
        );
        let mut flac_only = rules(0, Pick::Fastest);
        flac_only.stream.minimum.formats = vec!["flac".into()];
        let ordered = order(peers(), Some(240), &flac_only, Purpose::Stream);
        assert!(ordered.iter().filter(|r| r.meets).all(|r| r.source.ext == "flac"));
    }

    #[test]
    fn fastest_takes_the_best_score_and_highest_the_best_quality_among_passing_copies() {
        let fastest = order(peers(), Some(240), &rules(256, Pick::Fastest), Purpose::Stream);
        let highest = order(peers(), Some(240), &rules(256, Pick::Highest), Purpose::Stream);
        assert_ne!(fastest[0].peer, "slow-flac", "a slow peer isn't the fastest");
        assert!(fastest[0].meets);
        assert_eq!(highest[0].peer, "slow-flac", "lossless first, however slow");
        // Below the minimum comes after every passing copy, in either order.
        for list in [&fastest, &highest] {
            let first_below = list.iter().position(|r| !r.meets && !r.queue_too_long);
            let last_passing = list.iter().rposition(|r| r.meets && !r.queue_too_long);
            assert!(first_below > last_passing, "{:?}", users(list));
        }
    }

    #[test]
    fn a_streaming_cap_takes_copies_within_it_first_and_leaves_saving_alone() {
        let mut capped = rules(192, Pick::Highest);
        capped.stream.max_kbps = 256;
        let ordered = order(peers(), Some(240), &capped, Purpose::Stream);
        let passing: Vec<&str> = ordered
            .iter()
            .filter(|r| r.meets && !r.queue_too_long)
            .map(|r| r.peer.as_str())
            .collect();
        assert_eq!(
            passing,
            ["mid-256", "slow-flac", "unknown-mp3", "fast-320"],
            "within the cap by quality, then lossless and 320 kbps above it"
        );
        assert!(ordered.iter().any(|r| r.peer == "fast-320" && r.above_max));
        assert!(ordered.iter().any(|r| r.peer == "fast-128" && !r.meets && !r.above_max));

        let save = order(peers(), Some(240), &capped, Purpose::Save);
        assert_eq!(save[0].peer, "slow-flac", "the saving order has no cap");
        assert!(save.iter().all(|r| !r.above_max));
    }

    #[test]
    fn a_better_copy_passes_the_minimum_and_beats_a_peer_fallen_back_to_only_by_coming_first() {
        let ordered = || order(peers(), Some(240), &rules(256, Pick::Fastest), Purpose::Stream);
        let all = ordered();
        let bar = |why, playing: &str, avoid: &[&str]| Bar {
            playing: all
                .iter()
                .find(|r| r.peer == playing)
                .map(|r| r.source.source_ref())
                .unwrap_or_else(|| SourceRef {
                    user: playing.into(),
                    path: "youtube:abc:251.webm".into(),
                    size: 1,
                }),
            why,
            avoid: avoid.iter().map(|a| (*a).to_owned()).collect(),
        };

        let from_youtube = better(ordered(), &bar(Lesser::Youtube, "YouTube", &[]));
        assert!(!from_youtube.is_empty() && from_youtube.iter().all(|r| r.meets && !r.queue_too_long));
        assert!(
            from_youtube.iter().all(|r| r.peer != "fast-128"),
            "a copy below the minimum is no better than YouTube's"
        );

        let from_below = better(ordered(), &bar(Lesser::Below, "fast-128", &["mid-256"]));
        assert!(from_below.iter().all(|r| r.peer != "fast-128" && r.peer != "mid-256"));
        assert_eq!(
            from_below.len(),
            from_youtube.len() - 1,
            "a peer that failed this play is avoided"
        );

        let last_passing = all.iter().rposition(|r| r.meets && !r.queue_too_long).unwrap_or(0);
        let fell_back_to = all[last_passing].peer.clone();
        let from_fallback = better(ordered(), &bar(Lesser::FellBack, &fell_back_to, &[]));
        assert_eq!(
            users(&from_fallback),
            users(&all[..last_passing]),
            "only copies the profile ranks above the one playing"
        );
        let first = all[0].peer.clone();
        assert!(
            better(ordered(), &bar(Lesser::FellBack, &first, &[])).is_empty(),
            "nothing beats the profile's first choice"
        );
    }

    #[test]
    fn fallback_follows_the_profile_order_and_long_queues_come_last() {
        let highest = order(peers(), Some(240), &rules(256, Pick::Highest), Purpose::Stream);
        assert_eq!(highest.last().map(|r| r.peer.as_str()), Some("busy-flac"));
        assert!(highest.last().is_some_and(|r| r.queue_too_long));
        let passing: Vec<&str> = highest
            .iter()
            .filter(|r| r.meets && !r.queue_too_long)
            .map(|r| r.peer.as_str())
            .collect();
        assert_eq!(passing, ["slow-flac", "unknown-mp3", "fast-320", "mid-256"]);
        // Saving uses its own profile: 320 kbps by default, so the 256 kbps copy drops below.
        let save = order(peers(), Some(240), &rules(256, Pick::Highest), Purpose::Save);
        assert!(save.iter().any(|r| r.peer == "mid-256" && !r.meets));
    }
}
