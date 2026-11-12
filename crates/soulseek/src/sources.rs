//! Every peer file a search has turned up, by song: what the source picker lists and what a
//! stream falls back through, in the order the listener's profile sets (ADR-0008). Held in
//! memory (searches are cheap to repeat, peers come and go) and bounded by age.
//!
//! Files are held under the `TrackId` their path names, and that names the album folder; a
//! song's pool is every held file that is the song by `matching`, whatever its folder (live,
//! "Drag Me Down" offered 2 copies from soundtrack folders while 6 more peers shared it under
//! "Santa Cruz - Single - 2016 - Drag Me Down" and others).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use lull_core::ids::{TrackId, loose_title};
use lull_core::model::{FileFacts, SourceRef, Track};
use lull_core::quality::{Minimum, Pick, Quality, format_of};
use serde::Serialize;

use crate::files;
use crate::matching::Wanted;
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

    /// Whether its format is one `min` leaves on (`Minimum::allows`).
    pub fn allowed(&self, min: &Minimum) -> bool {
        min.allows(format_of(&self.ext))
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

/// What ranking knows of a copy (`rank::factors`, `rank::sound`): its format and quality, the
/// bitrate from its size and the song's length when the peer didn't report one. No speed.
pub fn offer(s: &Source, duration: Option<u32>) -> Offer<'_> {
    let quality = s.quality(duration);
    Offer {
        ext: &s.ext,
        lossless: quality.lossless,
        kbps: Some(quality.kbps).filter(|k| *k > 0),
        khz: s.facts.khz,
        bits: s.facts.bits,
        speed: 0,
        free_slot: s.free_slot,
        queue: s.queue,
        stalled_before: false,
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
    /// In a format the streaming profile switched off (`policy::allowed`): listed last, never
    /// streamed unless the listener picks it. Always false in the saving order.
    #[serde(skip)]
    pub left_out: bool,
    #[serde(skip)]
    pub source: Source,
}

#[derive(Clone)]
pub struct Sources {
    inner: Arc<RwLock<Map>>,
    peers: Peers,
    preferred: Arc<[String]>,
}

#[derive(Default)]
struct Map {
    held: HashMap<TrackId, Held>,
    /// The ids held, by the title their files name (`title_key`): where a song's copies under
    /// other folders' ids are looked for.
    titled: HashMap<String, HashSet<TrackId>>,
    /// Songs a search for the song itself has looked for lately (`searched`).
    searched: HashMap<TrackId, Instant>,
    /// Copies whose sound turned out not to be the song (ADR-0016), by (peer, path).
    wrong: HashMap<TrackId, HashSet<(String, String)>>,
}

struct Held {
    sources: Vec<Source>,
    seen: Instant,
}

/// A title as `matching` compares titles, spacing aside.
fn title_key(title: &str) -> String {
    loose_title(title).split(' ').collect()
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
        let Map {
            held, titled, searched, ..
        } = &mut *map;
        held.retain(|_, h| now.duration_since(h.seen) < KEEP_FOR);
        titled.retain(|_, ids| {
            ids.retain(|id| held.contains_key(id));
            !ids.is_empty()
        });
        searched.retain(|_, at| now.duration_since(*at) < KEEP_FOR);
        for (track, sources) in found {
            if let Some(song) = sources.first().and_then(|s| files::song(&s.path)) {
                titled.entry(title_key(&song.title)).or_default().insert(track.clone());
            }
            let h = held.entry(track).or_insert_with(|| Held {
                sources: Vec::new(),
                seen: now,
            });
            h.seen = now;
            for s in sources {
                h.sources.retain(|old| !(old.user == s.user && old.path == s.path));
                h.sources.push(s);
            }
        }
    }

    /// Every held file that is this song: its own id's, then those under other folders' ids
    /// that `matching` takes for it (artist and title loosely, lengths within 6 s, a remix or a
    /// cover another song).
    fn pool(&self, track: &Track) -> Vec<Source> {
        let Ok(map) = self.inner.read() else {
            return Vec::new();
        };
        let wrong = map.wrong.get(&track.id);
        let is_wrong = |s: &Source| wrong.is_some_and(|w| w.contains(&(s.user.clone(), s.path.clone())));
        let mut pool: Vec<Source> = map.held.get(&track.id).map(|h| h.sources.clone()).unwrap_or_default();
        pool.retain(|s| !is_wrong(s));
        let wanted = Wanted::of(track);
        // In one order, so ties in the profile's order don't shuffle between asks.
        let mut ids: Vec<&TrackId> = map
            .titled
            .get(&title_key(&track.title))
            .into_iter()
            .flatten()
            .filter(|id| **id != track.id)
            .collect();
        ids.sort_by(|a, b| a.0.cmp(&b.0));
        let others = ids.into_iter().filter_map(|id| map.held.get(id));
        for s in others.flat_map(|h| &h.sources) {
            let is_song =
                || files::song(&s.path).is_some_and(|f| wanted.matches_named(&f.artist, &f.title, s.length, &s.path));
            if !pool.iter().any(|p| p.user == s.user && p.path == s.path) && !is_wrong(s) && is_song() {
                pool.push(s.clone());
            }
        }
        pool
    }

    /// A search for the song itself has run lately (`resolve`), not only searches that came
    /// across some of its files (an album's, another query's).
    pub fn searched(&self, track: &TrackId) -> bool {
        self.inner.read().is_ok_and(|m| m.searched.contains_key(track))
    }

    /// A copy whose sound isn't the song: never offered for it again (until the hub restarts).
    pub fn mark_wrong(&self, track: &TrackId, copy: &SourceRef) {
        if let Ok(mut map) = self.inner.write() {
            map.wrong
                .entry(track.clone())
                .or_default()
                .insert((copy.user.clone(), copy.path.clone()));
        }
    }

    /// A search for the song itself ended after `since`.
    pub fn searched_since(&self, track: &TrackId, since: Instant) -> bool {
        self.inner
            .read()
            .is_ok_and(|m| m.searched.get(track).is_some_and(|at| *at >= since))
    }

    pub fn mark_searched(&self, track: &TrackId) {
        if let Ok(mut map) = self.inner.write() {
            map.searched.insert(track.clone(), Instant::now());
        }
    }

    /// The track's sources in the order the profile for `purpose` takes them: copies that
    /// pass its minimum by its `pick`, then the rest by the same order, then peers whose queue
    /// is too long. `duration` (the song's) judges files whose bitrate the peer didn't report.
    pub fn ordered(&self, track: &Track, rules: &Rules, purpose: Purpose) -> Vec<Ranked> {
        let cap = match purpose {
            Purpose::Stream => rules.stream.max_kbps,
            Purpose::Save => 0,
        };
        let ranked = self
            .pool(track)
            .into_iter()
            .map(|s| self.rank_for(s, cap, track.duration))
            .collect();
        order(ranked, track.duration, rules, purpose)
    }

    /// The listener's formats, most wanted first (`LULL_PREFERRED_FORMATS`).
    pub fn preferred(&self) -> &[String] {
        &self.preferred
    }

    /// One known source of the track, by reference.
    pub fn get(&self, track: &Track, r: &SourceRef) -> Option<Source> {
        self.pool(track).into_iter().find(|s| s.is(r))
    }

    /// Whether any source is known for the track.
    pub fn known(&self, track: &Track) -> bool {
        !self.pool(track).is_empty()
    }

    /// A copy ranked with no streaming cap: search results and saving.
    pub fn rank(&self, s: Source) -> Ranked {
        self.rank_for(s, 0, None)
    }

    /// A copy ranked for streaming under the cap `max_kbps` (0 = none), `duration` the song's.
    pub fn rank_for(&self, s: Source, max_kbps: u32, duration: Option<u32>) -> Ranked {
        let stalled_before = self.peers.stalled_before(&s.user);
        let (expected, measured) = self.peers.expected(&s.user, s.upload_speed);
        let shown = measured.map_or(s.upload_speed as f64, |m| m.rate);
        let factors = rank::factors(
            &Offer {
                speed: expected as u64,
                stalled_before,
                ..offer(&s, duration)
            },
            &self.preferred,
            max_kbps,
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
            left_out: false,
            source: s,
        }
    }
}

/// Sorts ranked sources for a profile and marks which pass its minimum. Streaming puts every
/// copy within its bitrate cap before those above it, passing copies first on each side: the cap
/// bounds what streams, and a copy above it is taken only when none within it is (2026-11-12).
/// A copy in a format streaming switched off goes last, and is never taken (`left_out`).
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
        r.left_out = purpose == Purpose::Stream && !r.source.allowed(min);
    }
    let fastest = |a: &Ranked, b: &Ranked| b.score.cmp(&a.score).then(b.expected.total_cmp(&a.expected));
    ranked.sort_by(|a, b| {
        a.left_out
            .cmp(&b.left_out)
            .then(a.queue_too_long.cmp(&b.queue_too_long))
            .then(a.above_max.cmp(&b.above_max))
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
pub(crate) mod tests {
    use super::*;
    use lull_core::quality::{Minimum, Pick};

    /// A peer's copy with a free slot at 1 MB/s, ranked as `copy` ranks it.
    pub(crate) fn ranked_copy(user: &str, ext: &str, kbps: u32) -> Ranked {
        copy(user, ext, kbps, 1.0, true, 0)
    }

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
            0,
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
            left_out: false,
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
        let at = |peer: &str| ordered.iter().position(|r| r.peer == peer);
        assert!(
            at("fast-128") < at("fast-320"),
            "a copy within the cap, below the minimum, before one above the cap"
        );

        let save = order(peers(), Some(240), &capped, Purpose::Save);
        assert_eq!(save[0].peer, "slow-flac", "the saving order has no cap");
        assert!(save.iter().all(|r| !r.above_max));
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
