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
    /// Bytes per second, as the peer advertises.
    pub upload_speed: u64,
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

/// `preferred` is `LULL_PREFERRED_FORMATS`: lowercase extensions, most wanted first.
pub fn factors(offer: &Offer, preferred: &[String]) -> Factors {
    Factors {
        format: format_points(offer, preferred),
        speed: (offer.upload_speed as f32 / 1_000_000.0 / FULL_SPEED_MBPS * SPEED_MAX)
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

/// Where the format sits in the preference list, then how good the file is within it: lossy
/// files scale with bitrate (an MP3 128 is not an MP3 320); hi-res lossless costs a little,
/// since its files are three times the size for no audible gain while streaming.
fn format_points(offer: &Offer, preferred: &[String]) -> u8 {
    let Some(rank) = preferred.iter().position(|p| p == offer.ext) else {
        return 0;
    };
    let base = (FORMAT_MAX - FORMAT_STEP * rank as f32).max(FORMAT_STEP / 2.0);
    let quality = if offer.lossless {
        let hi_res = offer.bits.is_some_and(|b| b > 16) || offer.khz.is_some_and(|k| k > 48.0);
        if hi_res { 0.95 } else { 1.0 }
    } else {
        offer.kbps.map_or(0.6, |k| (k as f32 / 320.0).clamp(0.3, 1.0))
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
            upload_speed: 2_000_000,
            free_slot: true,
            queue: 0,
            stalled_before: false,
        }
    }

    fn score(o: &Offer) -> u8 {
        factors(o, &prefs()).score()
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
        assert_eq!(factors(&wav, &prefs()).format, 0);
        // The order is the listener's: prefer mp3 and the same files rank the other way round.
        let mp3_first = ["mp3", "flac"].map(String::from);
        assert!(factors(&mp3, &mp3_first).format > factors(&flac, &mp3_first).format);
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
        let f = factors(&queued_flac, &prefs());
        assert_eq!((f.slot, f.queue), (SLOT_BUSY, 3));
        // Real peers report queues in the thousands: the factor bottoms out at zero.
        let swamped = Offer {
            free_slot: false,
            queue: 5_000,
            ..offer("flac")
        };
        assert_eq!(factors(&swamped, &prefs()).queue, 0);
    }

    #[test]
    fn peers_that_stalled_before_are_demoted_and_the_breakdown_still_adds_up() {
        let fresh = offer("flac");
        let stalled = Offer {
            stalled_before: true,
            ..offer("flac")
        };
        assert_eq!(score(&fresh) - score(&stalled), STALL_PENALTY);
        let f = factors(&stalled, &prefs());
        let sum =
            i16::from(f.format) + i16::from(f.speed) + i16::from(f.slot) + i16::from(f.queue) + i16::from(f.stalled);
        assert_eq!(sum, i16::from(f.score()));
        // Speed saturates: a fast peer can't buy back a bad format.
        let fast = Offer {
            upload_speed: 50_000_000,
            ..offer("flac")
        };
        assert_eq!(factors(&fast, &prefs()).speed, SPEED_MAX as u8);
    }
}
