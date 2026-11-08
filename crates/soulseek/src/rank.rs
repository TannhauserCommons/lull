//! Source ranking: a 0-100 score from four factors (format, speed, free slot, queue), minus a
//! penalty for peers that stalled on us before. The UI shows the breakdown, so each factor is
//! a plain number that adds up to the score (docs/context/soulseek.md#ranking).

use serde::Serialize;

/// Points per factor at best.
const FORMAT_MAX: f32 = 40.0;
const SPEED_MAX: f32 = 30.0;
const SLOT_FREE: u8 = 15;
const SLOT_BUSY: u8 = 4;
const QUEUE_MAX: u8 = 15;
/// Each preference step down costs this much of the format factor.
const FORMAT_STEP: f32 = 8.0;
/// A peer that stalled recently loses this many points.
const STALL_PENALTY: u8 = 25;
/// MB/s at which the speed factor is full; FLAC streams at ~0.15 MB/s, so this leaves headroom.
const FULL_SPEED_MBPS: f32 = 3.3;

/// What a peer offers for one file.
#[derive(Debug, Clone)]
pub struct Offer<'a> {
    /// Lowercase extension.
    pub ext: &'a str,
    pub lossless: bool,
    pub kbps: Option<u32>,
    pub khz: Option<f32>,
    pub bits: Option<u8>,
    /// Bytes per second lull expects from the peer: what it measured, else the peer's own
    /// figure as such figures have proved on this hub (ADR-0015).
    pub speed: u64,
    pub free_slot: bool,
    pub queue: u32,
    pub stalled_before: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Factors {
    pub format: u8,
    pub speed: u8,
    pub slot: u8,
    pub queue: u8,
    /// Zero, or minus the penalty for a recent stall.
    pub stalled: i8,
}

impl Factors {
    pub fn score(&self) -> u8 {
        let sum = i16::from(self.format) + i16::from(self.speed) + i16::from(self.slot) + i16::from(self.queue);
        (sum + i16::from(self.stalled)).clamp(0, 100) as u8
    }
}

/// `preferred` is `LULL_PREFERRED_FORMATS`: lowercase extensions, most wanted first. `max_kbps`:
/// the streaming cap the format factor scales toward when copies are ranked for streaming
/// (`sound`); 0 for none, or for a saving or search ranking.
pub fn factors(offer: &Offer, preferred: &[String], max_kbps: u32) -> Factors {
    Factors {
        format: sound(offer, preferred, max_kbps),
        speed: (offer.speed as f32 / 1_000_000.0 / FULL_SPEED_MBPS * SPEED_MAX)
            .round()
            .min(SPEED_MAX) as u8,
        slot: if offer.free_slot { SLOT_FREE } else { SLOT_BUSY },
        queue: if offer.free_slot {
            QUEUE_MAX
        } else {
            QUEUE_MAX.saturating_sub(offer.queue.saturating_mul(2).min(u32::from(QUEUE_MAX)) as u8)
        },
        stalled: if offer.stalled_before {
            -(STALL_PENALTY as i8)
        } else {
            0
        },
    }
}

/// The bitrate a lossy file earns full format points at, without a streaming cap.
const FULL_KBPS: u32 = 320;

/// How a copy sounds, by the listener's own terms and nothing about getting it: the format
/// factor, its lossy part scaled toward the streaming cap `max_kbps` (0 = none, then toward
/// 320). The score adds speed, slot and queue to it (what `pick: fastest` starts by); a better
/// copy mid-song is ranked by it alone (`policy`).
pub fn sound(offer: &Offer, preferred: &[String], max_kbps: u32) -> u8 {
    format_points(offer, preferred, if max_kbps > 0 { max_kbps } else { FULL_KBPS })
}

/// Where the format sits in the preference list, then how good the file is within it: lossy
/// files scale with bitrate up to `full_kbps` (an MP3 128 is not an MP3 320); hi-res lossless
/// costs a little, since its files are three times the size for no audible gain while streaming.
fn format_points(offer: &Offer, preferred: &[String], full_kbps: u32) -> u8 {
    let Some(rank) = preferred.iter().position(|p| p == offer.ext) else {
        return 0;
    };
    let base = (FORMAT_MAX - FORMAT_STEP * rank as f32).max(FORMAT_STEP / 2.0);
    let quality = if offer.lossless {
        let hi_res = offer.bits.is_some_and(|b| b > 16) || offer.khz.is_some_and(|k| k > 48.0);
        if hi_res { 0.95 } else { 1.0 }
    } else {
        offer
            .kbps
            .map_or(0.6, |k| (k as f32 / full_kbps as f32).clamp(0.3, 1.0))
    };
    (base * quality).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prefs() -> Vec<String> {
        ["flac", "mp3", "opus", "ogg", "m4a"].map(String::from).to_vec()
    }

    fn offer(ext: &str) -> Offer<'_> {
        let lossless = ext == "flac";
        Offer {
            ext,
            lossless,
            kbps: Some(if lossless { 1000 } else { 320 }),
            khz: Some(44.1),
            bits: lossless.then_some(16),
            speed: 2_000_000,
            free_slot: true,
            queue: 0,
            stalled_before: false,
        }
    }

    fn score(o: &Offer) -> u8 {
        factors(o, &prefs(), 0).score()
    }

    #[test]
    fn the_format_factor_scales_toward_the_streaming_cap_for_the_start_and_the_switch_alike() {
        let mp3 = |k| Offer {
            kbps: Some(k),
            ..offer("mp3")
        };
        let p = prefs();
        assert_eq!(
            factors(&mp3(256), &p, 256).format,
            factors(&mp3(320), &p, 0).format,
            "under a 256 cap a 256 kbps copy earns the full points"
        );
        assert!(
            factors(&mp3(256), &p, 0).format < factors(&mp3(320), &p, 0).format,
            "no cap: toward 320"
        );
        assert!(
            factors(&mp3(192), &p, 256).format < factors(&mp3(256), &p, 256).format,
            "nearer the cap, higher"
        );
        assert_eq!(
            sound(&mp3(192), &p, 256),
            factors(&mp3(192), &p, 256).format,
            "one function"
        );
    }

    #[test]
    fn preferred_format_outranks_others_at_equal_peer_quality() {
        let flac = offer("flac");
        let mp3 = offer("mp3");
        let mp3_128 = Offer {
            kbps: Some(128),
            ..offer("mp3")
        };
        let wav = offer("wav");
        assert!(score(&flac) > score(&mp3));
        assert!(score(&mp3) > score(&mp3_128));
        // A format the listener didn't list gets no format points at all.
        assert_eq!(factors(&wav, &prefs(), 0).format, 0);
        // The order is the listener's: prefer mp3 and the same files rank the other way round.
        let mp3_first = ["mp3", "flac"].map(String::from);
        assert!(factors(&mp3, &mp3_first, 0).format > factors(&flac, &mp3_first, 0).format);
    }

    #[test]
    fn a_free_slot_and_short_queue_beat_a_slightly_better_file_behind_a_long_queue() {
        let queued_flac = Offer {
            free_slot: false,
            queue: 6,
            ..offer("flac")
        };
        let free_mp3 = offer("mp3");
        assert!(score(&free_mp3) > score(&queued_flac));
        let f = factors(&queued_flac, &prefs(), 0);
        assert_eq!((f.slot, f.queue), (SLOT_BUSY, 3));
        // Real peers report queues in the thousands: the factor bottoms out at zero.
        let swamped = Offer {
            free_slot: false,
            queue: 5_000,
            ..offer("flac")
        };
        assert_eq!(factors(&swamped, &prefs(), 0).queue, 0);
    }

    #[test]
    fn peers_that_stalled_before_are_demoted_and_the_breakdown_still_adds_up() {
        let fresh = offer("flac");
        let stalled = Offer {
            stalled_before: true,
            ..offer("flac")
        };
        assert_eq!(score(&fresh) - score(&stalled), STALL_PENALTY);
        let f = factors(&stalled, &prefs(), 0);
        let sum =
            i16::from(f.format) + i16::from(f.speed) + i16::from(f.slot) + i16::from(f.queue) + i16::from(f.stalled);
        assert_eq!(sum, i16::from(f.score()));
        // Speed saturates: a fast peer can't buy back a bad format.
        let fast = Offer {
            speed: 50_000_000,
            ..offer("flac")
        };
        assert_eq!(factors(&fast, &prefs(), 0).speed, SPEED_MAX as u8);
    }
}
