//! Every peer file a search has turned up, by song: what the source picker lists and what a
//! stream falls back through, in the order the listener's profile sets (ADR-0008). Held in
//! memory (searches are cheap to repeat, peers come and go) and bounded by age.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use lull_core::ids::TrackId;
use lull_core::model::{FileFacts, SourceRef};
use lull_core::quality::{Minimum, Pick, Quality};
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
        self.quality(duration).meets(&self.ext, min)
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
    /// MB/s, one decimal.
    pub speed: f32,
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
        let factors = rank::factors(
            &Offer {
                ext: &s.ext,
                lossless: s.facts.lossless,
                kbps: s.facts.kbps,
                khz: s.facts.khz,
                bits: s.facts.bits,
                upload_speed: s.upload_speed,
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
            speed: (s.upload_speed as f32 / 100_000.0).round() / 10.0,
            free_slot: s.free_slot,
            queue: s.queue,
            score: factors.score(),
            factors,
            preferred: self.preferred.first() == Some(&s.ext),
            stalled_before,
            meets: true,
            queue_too_long: false,
            source: s,
        }
    }
}

/// Sorts ranked sources for a profile and marks which pass its minimum.
pub fn order(mut ranked: Vec<Ranked>, duration: Option<u32>, rules: &Rules, purpose: Purpose) -> Vec<Ranked> {
    let min = rules.minimum(purpose);
    for r in &mut ranked {
        r.meets = r.source.meets(min, duration);
        r.queue_too_long = r.source.queue_too_long(rules.max_peer_queue);
    }
    let fastest = |a: &Ranked, b: &Ranked| b.score.cmp(&a.score).then(b.speed.total_cmp(&a.speed));
    ranked.sort_by(|a, b| {
        a.queue_too_long
            .cmp(&b.queue_too_long)
            .then(b.meets.cmp(&a.meets))
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
                upload_speed: source.upload_speed,
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
            free_slot,
            queue,
            score: factors.score(),
            factors,
            preferred: false,
            stalled_before: false,
            meets: true,
            queue_too_long: false,
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
