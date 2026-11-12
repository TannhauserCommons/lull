//! The streaming policy (ADR-0012, ADR-0013): what a song starts on, and what it switches to
//! mid-song, decided by the streaming profile alone (the asking device's source preset's: `minimum`,
//! `below`, `pick`, `maxKbps`). A copy is a copy, whatever its source (a Soulseek peer's,
//! YouTube's): `youtube.mode` only says when YouTube is searched (`fetch.rs::route`), never how
//! its copy is judged. Pure: the fetch, race and upgrade code gather the copies and ask here.
//!
//! - **Starting** (`start`). At a song's start lull can't know yet whether a copy passing the
//!   minimum exists, so a copy below it arriving first means "none is good enough yet", and
//!   `below` decides. `playBest`: the first copy whose bytes are ready plays, above or below the
//!   minimum (a passing one ready at the same moment wins), and the upgrade fetches the best
//!   passing copy. `keepLooking`: only passing copies play, the first ready wins, and with none
//!   it searches on. `ask`: the first passing copy plays; once the search ends with none, the
//!   listener picks. `pick` orders the copies tried first (`sources::order`), and so breaks ties.
//!   `maxKbps` bounds all of it: a copy above the cap starts the song only when no copy within it
//!   can, and a later play takes a cached copy within it first.
//! - **Formats switched off** (`allowed`). A copy in a format the minimum's `formats` leaves out
//!   never streams, whatever `below` says: not at the start, in the race, as a better copy or
//!   from the cache. It is the listener's own choice (data on a phone); only their pick of that
//!   copy plays it (2026-11-12).
//! - **Switching mid-song** (`better`, `switch_to`; ADR-0018). A better copy is one in a higher
//!   quality band (`lull_core::quality::Band`) than the copy playing, in a band the profile
//!   allows (`StreamProfile::bands`), passing the minimum and within the cap. A few kbps more in
//!   the same band is no switch. They are listed highest band first, then by how they sound
//!   (`rank::sound`); the race (`fetch/race.rs`) decides which of them arrives.

use lull_core::quality::{Band, StreamBelow, StreamProfile};

use crate::rank;
use crate::sources::{self, Ranked, Source};

/// Whether `copy` passes the streaming minimum (kbps floor, formats, lossless only).
pub fn passes(p: &StreamProfile, copy: &Source, duration: Option<u32>) -> bool {
    copy.meets(&p.minimum, duration)
}

/// Whether `copy` is within the streaming cap (`maxKbps`; lossless counts as above any cap).
pub fn within_cap(p: &StreamProfile, copy: &Source, duration: Option<u32>) -> bool {
    copy.quality(duration).within(p.max_kbps)
}

/// Whether `copy`'s format is one streaming leaves on (`Minimum::allows`). A hard rule, unlike
/// the rest of the minimum, which `below` may set aside.
pub fn allowed(p: &StreamProfile, copy: &Source) -> bool {
    copy.allowed(&p.minimum)
}

/// Whether `copy` is one the profile streams gladly: a format it leaves on, within its cap and
/// passing its minimum. A device of another preset joins a fetch only on such a copy (ADR-0017).
pub fn suits(p: &StreamProfile, copy: &Source, duration: Option<u32>) -> bool {
    allowed(p, copy) && within_cap(p, copy, duration) && passes(p, copy, duration)
}

/// Whether `copy` may be fetched to start the song: one passing the minimum always, one below it
/// only under `playBest`, and never one in a format switched off.
pub fn may_start(p: &StreamProfile, copy: &Source, duration: Option<u32>) -> bool {
    allowed(p, copy) && (passes(p, copy, duration) || p.below == StreamBelow::PlayBest)
}

/// A copy of the song found so far, in the order the profile tries them (`pick`), and whether its
/// first bytes are here.
#[derive(Debug, Clone, Copy)]
pub struct Seen<'a> {
    pub copy: &'a Source,
    pub ready: bool,
}

/// What the song's start does now.
#[derive(Debug)]
pub enum Start<'a> {
    /// This copy plays (marked below the minimum when it fails it).
    Play(&'a Source),
    /// A copy that may start the song is on its way, or the search still runs.
    Wait,
    /// `keepLooking` with no copy passing: search on.
    Search,
    /// `ask` with no copy passing: the listener picks one of those found.
    Ask,
    /// Nothing was found to play.
    Fail,
}

/// What the song starts on, given the copies found so far (`copies`, in the profile's order;
/// those that failed left out) and whether the search for more still runs.
pub fn start<'a>(p: &StreamProfile, copies: &[Seen<'a>], searching: bool, duration: Option<u32>) -> Start<'a> {
    // A format switched off never starts the song, nor is it offered.
    let copies: Vec<Seen<'a>> = copies.iter().filter(|s| allowed(p, s.copy)).copied().collect();
    let copies = copies.as_slice();
    // Within the cap first: a copy above it starts the song only when none within it can.
    let within = |s: &&Seen| within_cap(p, s.copy, duration);
    let mut ready = copies.iter().filter(|s| s.ready);
    if let Some(s) = ready.clone().filter(within).find(|s| passes(p, s.copy, duration)) {
        return Start::Play(s.copy);
    }
    if p.below == StreamBelow::PlayBest
        && let Some(s) = ready.clone().find(within)
    {
        return Start::Play(s.copy);
    }
    if copies
        .iter()
        .any(|s| !s.ready && within(&s) && may_start(p, s.copy, duration))
    {
        return Start::Wait;
    }
    if let Some(s) = ready.clone().find(|s| passes(p, s.copy, duration)) {
        return Start::Play(s.copy);
    }
    if p.below == StreamBelow::PlayBest
        && let Some(s) = ready.next()
    {
        return Start::Play(s.copy);
    }
    let coming = copies.iter().any(|s| !s.ready && may_start(p, s.copy, duration));
    if searching || coming {
        return Start::Wait;
    }
    match p.below {
        // Any copy found would have been ready or coming.
        StreamBelow::PlayBest => Start::Fail,
        StreamBelow::KeepLooking => Start::Search,
        StreamBelow::Ask if copies.is_empty() => Start::Fail,
        StreamBelow::Ask => Start::Ask,
    }
}

/// How `copy` sounds by the profile: `rank::sound`, toward `maxKbps`, the same format factor the
/// start's score adds speed, slot and queue to.
pub fn sound(p: &StreamProfile, preferred: &[String], copy: &Source, duration: Option<u32>) -> u8 {
    rank::sound(&sources::offer(copy, duration), preferred, p.max_kbps)
}

/// The quality band of `copy` (ADR-0018): a bitrate lull only estimated from size and length
/// counts one band down.
pub fn band(copy: &Source, duration: Option<u32>) -> Band {
    Band::of(copy.quality(duration), copy.facts.kbps.is_none())
}

/// Whether the song should switch from `playing` to `copy` mid-song: `copy` is in a higher band
/// than `playing`, one the profile allows, passes the minimum, is within the cap (lossless counts
/// as above any) and is in a format left on.
pub fn switch_to(p: &StreamProfile, copy: &Source, playing: &Source, duration: Option<u32>) -> bool {
    let band = band(copy, duration);
    band > self::band(playing, duration)
        && p.bands().contains(&band)
        && allowed(p, copy)
        && passes(p, copy, duration)
        && within_cap(p, copy, duration)
}

/// What a better copy has to beat: the copy playing, and the peers that failed this play.
#[derive(Debug, Clone)]
pub struct Bar {
    pub playing: Source,
    pub avoid: Vec<String>,
}

/// The copies worth switching to from the one playing (`switch_to`), highest band first, then by
/// how they sound (`sound`), the speed expected only breaking ties. Not one whose queue is too long
/// (`Ranked::queue_too_long`, from `sources::order`), the copy playing, or a peer that failed.
pub fn better(
    p: &StreamProfile,
    preferred: &[String],
    copies: Vec<Ranked>,
    bar: &Bar,
    duration: Option<u32>,
) -> Vec<Ranked> {
    let mut worth: Vec<(Band, u8, Ranked)> = copies
        .into_iter()
        .filter(|r| {
            !r.queue_too_long
                && !r.source.is(&bar.playing.source_ref())
                && !bar.avoid.contains(&r.peer)
                && switch_to(p, &r.source, &bar.playing, duration)
        })
        .map(|r| (band(&r.source, duration), sound(p, preferred, &r.source, duration), r))
        .collect();
    worth.sort_by(|(ba, a, x), (bb, b, y)| bb.cmp(ba).then(b.cmp(a)).then(y.expected.total_cmp(&x.expected)));
    worth.into_iter().map(|(_, _, r)| r).collect()
}

/// The order a later play takes cached copies in, best first: within the cap, passing the
/// minimum, then how it sounds. A cached copy needs no speed; one in a format switched off is
/// left out before (`allowed`).
pub fn cached_key(
    p: &StreamProfile,
    preferred: &[String],
    copy: &Source,
    duration: Option<u32>,
) -> (bool, bool, std::cmp::Reverse<u8>) {
    (
        !within_cap(p, copy, duration),
        !passes(p, copy, duration),
        std::cmp::Reverse(sound(p, preferred, copy, duration)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use lull_core::model::FileFacts;
    use lull_core::quality::{Minimum, Pick};

    const SONG: Option<u32> = Some(240);

    fn copy(user: &str, ext: &str, kbps: u32) -> Source {
        let lossless = ext == "flac";
        Source {
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
            upload_speed: 1_000_000,
            free_slot: true,
            queue: 0,
            length: None,
        }
    }

    fn profile(kbps: u32, below: StreamBelow) -> StreamProfile {
        StreamProfile {
            minimum: Minimum {
                lossless_only: false,
                kbps,
                formats: Vec::new(),
            },
            pick: Pick::Fastest,
            below,
            max_kbps: 0,
        }
    }

    /// Start fast as the race runs it: every copy is known from the start, each is fetched only
    /// when it may start the song and is ready at its time, and the search ends at `search_ends`.
    /// What the song starts on (a copy's user, or what `start` says), and when.
    fn race(p: &StreamProfile, arrivals: &[(&Source, u32)], search_ends: u32) -> (String, u32) {
        let mut times: Vec<u32> = arrivals.iter().map(|(_, t)| *t).chain([search_ends]).collect();
        times.sort_unstable();
        times.dedup();
        for t in times {
            let seen: Vec<Seen> = arrivals
                .iter()
                .map(|(c, at)| Seen {
                    copy: c,
                    ready: *at <= t && may_start(p, c, SONG),
                })
                .collect();
            match start(p, &seen, t < search_ends, SONG) {
                Start::Wait => {}
                Start::Play(c) => return (c.user.clone(), t),
                other => return (format!("{other:?}"), t),
            }
        }
        ("Wait".into(), u32::MAX)
    }

    /// A row of the start table: the `below` rule, the copies with the time each is ready, when the
    /// search ends, and what starts the song and when.
    type Row<'a> = (StreamBelow, Vec<(&'a Source, u32)>, u32, (&'a str, u32));

    #[test]
    fn a_format_switched_off_never_starts_the_song_whatever_the_below_rule() {
        let flac = copy("peer", "flac", 0);
        let mp3 = copy("other", "mp3", 128);
        use StreamBelow::*;
        for below in [PlayBest, KeepLooking, Ask] {
            let mut p = profile(192, below);
            p.minimum.formats = vec!["mp3".into(), "opus".into()];
            assert!(
                !may_start(&p, &flac, SONG),
                "{below:?}: the FLAC is never fetched to start"
            );
            // Only the FLAC ready, the search over: it doesn't play, and isn't offered.
            let (what, _) = race(&p, &[(&flac, 1)], 2);
            assert!(matches!(what.as_str(), "Fail" | "Search"), "{below:?}: {what}");
            // A copy in a format left on, below the minimum, ready after it: `playBest` plays it,
            // the others search on or ask; the FLAC never plays.
            let (what, at) = race(&p, &[(&flac, 1), (&mp3, 3)], 4);
            match below {
                PlayBest => assert_eq!((what.as_str(), at), ("other", 3)),
                KeepLooking | Ask => assert!(what == "Search" || what == "Ask", "{below:?}: {what}"),
            }
        }
    }

    #[test]
    fn start_fast_by_below_rule_and_arrival_order() {
        let yt_below = copy("YouTube", "webm", 160);
        let peer_320 = copy("peer", "mp3", 320);
        let peer_128 = copy("peer", "mp3", 128);
        use StreamBelow::*;
        // (below, arrivals in the profile's order, search ends, what starts the song and when)
        let table: Vec<Row> = vec![
            // YouTube's copy, below a 192 minimum, ready first; a passing peer's at 3.
            (PlayBest, vec![(&peer_320, 3), (&yt_below, 1)], 2, ("YouTube", 1)),
            (KeepLooking, vec![(&peer_320, 3), (&yt_below, 1)], 2, ("peer", 3)),
            (Ask, vec![(&peer_320, 3), (&yt_below, 1)], 2, ("peer", 3)),
            // The passing peer's copy first.
            (PlayBest, vec![(&peer_320, 1), (&yt_below, 2)], 2, ("peer", 1)),
            (KeepLooking, vec![(&peer_320, 1), (&yt_below, 2)], 2, ("peer", 1)),
            (Ask, vec![(&peer_320, 1), (&yt_below, 2)], 2, ("peer", 1)),
            // Both ready at the same moment: the passing copy wins.
            (PlayBest, vec![(&yt_below, 2), (&peer_320, 2)], 1, ("peer", 2)),
            (KeepLooking, vec![(&yt_below, 2), (&peer_320, 2)], 1, ("peer", 2)),
            (Ask, vec![(&yt_below, 2), (&peer_320, 2)], 1, ("peer", 2)),
            // Nothing passes: YouTube's at 1, a peer's 128 at 2, the search over at 3.
            (PlayBest, vec![(&peer_128, 2), (&yt_below, 1)], 3, ("YouTube", 1)),
            (KeepLooking, vec![(&peer_128, 2), (&yt_below, 1)], 3, ("Search", 3)),
            (Ask, vec![(&peer_128, 2), (&yt_below, 1)], 3, ("Ask", 3)),
            // Nothing found at all.
            (PlayBest, vec![], 3, ("Fail", 3)),
            (KeepLooking, vec![], 3, ("Search", 3)),
            (Ask, vec![], 3, ("Fail", 3)),
        ];
        for (below, arrivals, search_ends, (who, when)) in table {
            let p = profile(192, below);
            assert_eq!(
                race(&p, &arrivals, search_ends),
                (who.to_owned(), when),
                "{below:?}, arrivals {:?}",
                arrivals.iter().map(|(c, t)| (c.user.as_str(), *t)).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn a_copy_above_the_cap_starts_the_song_only_when_none_within_it_can() {
        // Live (2026-11-12): a 320 minimum and cap; a peer's FLAC sent first and started the song.
        let flac = copy("peer", "flac", 0);
        let yt = copy("YouTube", "webm", 160);
        let mp3 = copy("other", "mp3", 320);
        let capped = |below| {
            let mut p = profile(320, below);
            p.max_kbps = 320;
            p
        };
        use StreamBelow::*;
        let table: Vec<Row> = vec![
            // YouTube's Opus is within the cap, if below the minimum: under playBest it plays.
            (PlayBest, vec![(&flac, 1), (&yt, 2)], 3, ("YouTube", 2)),
            // A passing copy within the cap is waited for too.
            (PlayBest, vec![(&flac, 1), (&mp3, 2)], 3, ("other", 2)),
            // Nothing within the cap may start: the FLAC is the last resort.
            (KeepLooking, vec![(&flac, 1), (&yt, 2)], 3, ("peer", 1)),
            (PlayBest, vec![(&flac, 1)], 3, ("peer", 1)),
        ];
        for (below, arrivals, search_ends, (who, when)) in table {
            assert_eq!(
                race(&capped(below), &arrivals, search_ends),
                (who.to_owned(), when),
                "{below:?}, arrivals {:?}",
                arrivals.iter().map(|(c, t)| (c.user.as_str(), *t)).collect::<Vec<_>>()
            );
        }
        // A later play takes the cached copy within the cap, below the minimum or not.
        let p = capped(PlayBest);
        assert!(cached_key(&p, &[], &yt, SONG) < cached_key(&p, &[], &flac, SONG));
        assert!(cached_key(&p, &[], &mp3, SONG) < cached_key(&p, &[], &yt, SONG));
    }

    #[test]
    fn a_youtube_copy_passing_the_minimum_starts_the_song_when_it_is_first_whatever_the_rule() {
        let yt = copy("YouTube", "webm", 160);
        let peer = copy("peer", "mp3", 320);
        for below in [StreamBelow::PlayBest, StreamBelow::KeepLooking, StreamBelow::Ask] {
            let p = profile(128, below);
            assert_eq!(race(&p, &[(&peer, 3), (&yt, 1)], 2), ("YouTube".to_owned(), 1));
        }
    }

    #[test]
    fn two_passing_copies_ready_together_go_by_the_profiles_order() {
        let (a, b) = (copy("first", "mp3", 256), copy("second", "mp3", 320));
        let p = profile(192, StreamBelow::PlayBest);
        let seen = [Seen { copy: &a, ready: true }, Seen { copy: &b, ready: true }];
        assert!(matches!(start(&p, &seen, false, SONG), Start::Play(c) if std::ptr::eq(c, &a)));
    }

    #[test]
    fn a_switch_is_a_higher_band_passing_the_minimum_within_the_cap() {
        let mut p = profile(192, StreamBelow::PlayBest);
        p.max_kbps = 320;
        let mp3 = |kbps| copy("x", "mp3", kbps);
        // ADR-0018 R4: 256 -> 320 and 192 -> 224 are no longer switches (the old step test:
        // a fifth more bitrate); 192 -> 256 and 160 -> 320 cross into the high band.
        assert!(!switch_to(&p, &mp3(320), &mp3(256), SONG), "256 to 320 is a small step");
        assert!(switch_to(&p, &mp3(256), &mp3(192), SONG), "192 to 256 is a higher band");
        assert!(switch_to(&p, &mp3(320), &mp3(160), SONG), "160 to 320 is a higher band");
        assert!(switch_to(&p, &mp3(224), &mp3(192), SONG), "192 to 224 crosses 220");
        assert!(
            !switch_to(&p, &mp3(192), &mp3(160), SONG),
            "160 to 192 stay in the lower band"
        );
        assert!(!switch_to(&p, &mp3(256), &mp3(320), SONG), "never down");
        assert!(
            !switch_to(&p, &copy("x", "flac", 0), &mp3(192), SONG),
            "lossless is above a 320 cap"
        );
        assert!(
            !switch_to(&p, &mp3(160), &mp3(96), SONG),
            "a copy below the minimum is no target"
        );
        assert!(
            switch_to(&p, &mp3(256), &copy("YouTube", "webm", 128), SONG),
            "YouTube's copy is beaten as any"
        );
        p.max_kbps = 0;
        assert!(
            switch_to(&p, &copy("x", "flac", 0), &mp3(320), SONG),
            "with no cap, lossless is above 320"
        );
    }

    #[test]
    fn the_upgrade_target_is_the_best_sounding_copy_of_a_higher_band_however_slow() {
        let preferred = ["flac", "mp3"].map(String::from);
        let ranked = |user: &str, ext: &str, kbps: u32, speed: f64| Ranked {
            expected: speed,
            ..crate::sources::tests::ranked_copy(user, ext, kbps)
        };
        let mut p = profile(128, StreamBelow::PlayBest);
        p.max_kbps = 256;
        let bar = Bar {
            playing: copy("YouTube", "webm", 128),
            avoid: Vec::new(),
        };
        let copies = vec![
            ranked("fast-192", "mp3", 192, 3_000_000.0),
            ranked("slow-256", "mp3", 256, 100_000.0),
            ranked("flac", "flac", 0, 3_000_000.0),
            ranked("fast-320", "mp3", 320, 3_000_000.0),
        ];
        let got: Vec<String> = better(&p, &preferred, copies, &bar, SONG)
            .into_iter()
            .map(|r| r.peer)
            .collect();
        assert_eq!(
            got,
            ["slow-256"],
            "the 256 is at the cap; the 192 is no higher band; the FLAC and the 320 are above it"
        );
        // Format points scale toward the cap: a 256 earns them all under a 256 cap.
        let mp3_256 = copy("x", "mp3", 256);
        assert_eq!(
            sound(&p, &preferred, &mp3_256, SONG),
            sound(&p, &preferred, &copy("y", "mp3", 320), SONG)
        );
        p.max_kbps = 0;
        assert!(sound(&p, &preferred, &mp3_256, SONG) < sound(&p, &preferred, &copy("y", "mp3", 320), SONG));
    }
}
