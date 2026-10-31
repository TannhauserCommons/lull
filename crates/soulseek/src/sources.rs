//! Every peer file a search has turned up, by song: what the source picker lists and what a
//! stream falls back through. Held in memory (searches are cheap to repeat, peers come and go)
//! and bounded by age.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use lull_core::ids::TrackId;
use lull_core::model::{FileFacts, SourceRef};
use serde::Serialize;

use crate::peers::Peers;
use crate::rank::{self, Factors, Offer};

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

    /// The track's sources, best first.
    pub fn ranked(&self, track: &TrackId) -> Vec<Ranked> {
        let sources = self
            .inner
            .read()
            .ok()
            .and_then(|m| m.get(track).map(|h| h.sources.clone()))
            .unwrap_or_default();
        let mut ranked: Vec<Ranked> = sources.into_iter().map(|s| self.rank(s)).collect();
        ranked.sort_by(|a, b| b.score.cmp(&a.score).then(b.speed.total_cmp(&a.speed)));
        ranked
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
            source: s,
        }
    }
}
