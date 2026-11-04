//! The player's rules as web/player-rules.js has them, which the page's player follows: what the
//! queue moves to, what a play records, how loud, when the next song loads, what the hub
//! preloads, when radio adds songs, how far a seek may go and when a better copy is switched to.
//! Each function here is the one of the same name there, and tests/player-rules.json holds the
//! cases both are tested against (here and in tests/web/player-rules.test.mjs), so a rule changed
//! on one side fails until the other matches. The engine (engine.rs, queue.rs, listen.rs,
//! mixer.rs) decides by these.

use super::listen::How;
use super::queue::Repeat;
use serde_json::{Value, json};
use std::f64::consts::FRAC_PI_2;
use std::time::Duration;

/// The player's clock.
pub const TICK: Duration = Duration::from_millis(250);
/// A song no peer delivers gives way to the next after this.
pub const SKIP_FAILED: Duration = Duration::from_millis(2500);
/// Preloading waits this long after the last change.
pub const PREFETCH: Duration = Duration::from_millis(800);
/// A step of the position this long or longer isn't listening.
pub const LISTEN_GAP_S: f64 = 1.5;
/// Previous past this restarts the song.
pub const PREVIOUS_RESTARTS_S: f64 = 3.0;
/// Gapless continues only when the song ended this close to its end.
pub const GAPLESS_WITHIN_S: f64 = 2.0;
/// A Soulseek response ending further than this from the end is a fallback in progress.
pub const ENDED_EARLY_S: f64 = 2.0;
/// Radio keeps this many songs after the playing one; "radio when the queue ends" steers this far.
pub const RADIO_AHEAD: usize = 3;
pub const RADIO_STEER: f64 = 0.4;
/// Measured loudness levels after this many readings (one a tick: 3 s), within these bounds.
pub const LOUDNESS_AFTER: f64 = 12.0;
pub const LOUDNESS_MIN_DB: f64 = -12.0;
pub const LOUDNESS_MAX_DB: f64 = 6.0;
/// A better copy needs this much of the song left, and to have arrived this far past where the
/// song is; it is tried this many times, faded in over this long, at a quiet moment or after this
/// long at most.
pub const SWITCH_LEAD_S: f64 = 45.0;
pub const SWITCH_AHEAD_S: f64 = 20.0;
pub const SWITCH_TRIES: u32 = 3;
pub const SWITCH_FADE_S: f64 = 0.2;
pub const SWITCH_QUIET_S: f64 = 3.0;
/// After the fade, the copy's level glides from the song's to its own with this time constant.
pub const SWITCH_GLIDE_S: f64 = 1.5;

/// The place the queue moves to next: repeat one (only when a song ends by itself), the next
/// entry, the first again with repeat all, or none.
pub fn next_index(index: usize, length: usize, repeat: Repeat, by_itself: bool) -> Option<usize> {
    if by_itself && repeat == Repeat::One {
        return Some(index);
    }
    if index + 1 < length {
        return Some(index + 1);
    }
    (repeat == Repeat::All).then_some(0)
}

/// Previous: past 3 s, or on the first song, the song starts again (None); else the one before.
pub fn previous(index: usize, position: f64) -> Option<usize> {
    if position > PREVIOUS_RESTARTS_S || index == 0 {
        None
    } else {
        Some(index - 1)
    }
}

/// The seconds a step of the position adds to what a play heard.
pub fn heard(step: f64, playing: bool) -> f64 {
    if step > 0.0 && step < LISTEN_GAP_S && playing {
        step
    } else {
        0.0
    }
}

/// The body that starts a play (`POST /api/plays`).
pub fn play_start(track: &str, duration: Option<f64>, recommended: bool) -> Value {
    let mut body = json!({ "trackId": track, "recommended": recommended });
    if let Some(d) = duration.filter(|d| *d != 0.0) {
        body["duration"] = json!(d.round() as i64);
    }
    body
}

/// The body that ends one (`PATCH /api/plays/{id}`).
pub fn play_end(heard: f64, how: How) -> Value {
    json!({
        "playedS": heard.round() as i64,
        "completed": how == How::Completed,
        "skipped": how == How::Skipped,
    })
}

/// The output's volume: a perceptual curve.
pub fn output_volume(volume: f64, muted: bool) -> f64 {
    if muted { 0.0 } else { volume * volume }
}

pub fn gain(db: f64) -> f64 {
    10f64.powf(db / 20.0)
}

/// The level for a song measured at `rms` after `readings` readings, levelled toward the target
/// (None: keep the level it has).
pub fn loudness(target_db: f64, rms: f64, readings: f64) -> Option<f64> {
    if readings < LOUDNESS_AFTER || rms < 1e-4 {
        return None;
    }
    Some((gain(target_db) / rms).clamp(gain(LOUDNESS_MIN_DB), gain(LOUDNESS_MAX_DB)))
}

/// Repeat one never crossfades.
pub fn crossfade_s(repeat: Repeat, crossfade: f64) -> f64 {
    if repeat == Repeat::One { 0.0 } else { crossfade }
}

/// How close to its end the playing song sets the next: a Soulseek song still transferring keeps
/// the bandwidth to itself until its last half minute.
pub fn preload_lead(crossfade: f64, soulseek: bool, received: f64) -> f64 {
    (crossfade + 15.0).max(30.0) + if soulseek && received < 1.0 { 0.0 } else { 60.0 }
}

pub fn preload_now(lead: f64, duration: Option<f64>, position: f64) -> bool {
    duration.is_some_and(|d| d != 0.0 && d - position <= lead)
}

/// The output moves into the next song only when the playing one really reached its end.
pub fn gapless(duration: Option<f64>, position: f64) -> bool {
    duration.is_none_or(|d| d == 0.0 || d - position <= GAPLESS_WITHIN_S)
}

/// A Soulseek response ending this far from the end is a fallback in progress, not the song's end.
pub fn ended_early(soulseek: bool, complete: bool, duration: Option<f64>, position: f64) -> bool {
    soulseek && !complete && duration.is_some_and(|d| d - position > ENDED_EARLY_S)
}

/// The crossfade's levels (going, coming) at `k` of the way: equal power.
pub fn fade(k: f64) -> (f64, f64) {
    ((k * FRAC_PI_2).cos(), (k * FRAC_PI_2).sin())
}

/// A crossfade lasts what is left of the song, at least a second.
pub fn crossfade_length_s(left: f64) -> f64 {
    left.max(1.0)
}

/// The next few songs the hub fetches ahead: unsaved, distinct, not the playing one, up to `n`
/// (the next only, with shuffle; none with repeat one).
pub fn upcoming(
    index: usize,
    ids: &[String],
    shuffle: bool,
    repeat: Repeat,
    saved: impl Fn(&str) -> bool,
    n: usize,
) -> Vec<String> {
    if n == 0 || repeat == Repeat::One {
        return Vec::new();
    }
    let current = ids.get(index);
    let at: Vec<usize> = if shuffle {
        next_index(index, ids.len(), repeat, true).into_iter().collect()
    } else {
        (index + 1..ids.len()).collect()
    };
    let mut out: Vec<String> = Vec::new();
    for id in at.into_iter().filter_map(|i| ids.get(i)) {
        if out.len() < n && !saved(id) && !out.contains(id) && Some(id) != current {
            out.push(id.clone());
        }
    }
    out
}

/// Radio: with "radio when the queue ends" on, repeat off and nothing after the playing song, the
/// playing song seeds a radio; a radio that keeps adding asks for more while fewer than three songs
/// follow. `keeps_adding`: the radio there is (None: none). Returns (seed one, ask).
pub fn radio(
    index: usize,
    length: usize,
    repeat: Repeat,
    keeps_adding: Option<bool>,
    radio_when_queue_ends: bool,
) -> (bool, bool) {
    let left = length.saturating_sub(index + 1);
    let seed = keeps_adding.is_none() && radio_when_queue_ends && repeat == Repeat::Off && length > 0 && left == 0;
    let active = seed || keeps_adding == Some(true);
    (seed, active && left < RADIO_AHEAD)
}

/// Kept back from what has arrived, so the output doesn't stall the moment a seek lands.
pub const SEEK_MARGIN: f64 = 0.01;

/// What of the song can play now (parts of 0-1): all of it, or for a Soulseek song still
/// arriving, the parts the hub has (`arrived`: one from 0 until a seek ahead, ADR-0014) and what the
/// output holds (`buffered`), merged, each less the margin at its end (a part reaching the end
/// keeps it).
pub fn playable(soulseek: bool, arrived: &[(f64, f64)], buffered: &[(f64, f64)]) -> Vec<(f64, f64)> {
    if !soulseek {
        return vec![(0.0, 1.0)];
    }
    let mut all: Vec<(f64, f64)> = arrived.iter().chain(buffered).copied().collect();
    all.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for r in all {
        match merged.last_mut() {
            Some(last) if r.0 <= last.1 => last.1 = last.1.max(r.1),
            _ => merged.push(r),
        }
    }
    merged
        .into_iter()
        .map(|(a, b)| (a, if b >= 1.0 { 1.0 } else { b - SEEK_MARGIN }))
        .filter(|(a, b)| b > a)
        .collect()
}

/// What a seek does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeekHow {
    /// Inside what can play: it lands now.
    Now,
    /// Past it, where the hub can start the transfer (`aheadS`) and the transfer running would
    /// take longer to get there: it lands now, and the hub fetches from the byte asked for.
    Fetches,
    /// Otherwise: the song plays on and the seek is kept, to land once that far has arrived.
    Waits,
}

/// Where a seek to `fraction` goes, and how: `wait_s` is how long the transfer running takes to
/// get there (unknown counts as longer), `ahead_s` the hub's `seekAheadS`.
pub fn seek_to(fraction: f64, playable: &[(f64, f64)], wait_s: Option<f64>, ahead_s: Option<f64>) -> (f64, SeekHow) {
    let fraction = fraction.clamp(0.0, 1.0);
    let how = if playable.iter().any(|r| r.0 <= fraction && fraction <= r.1) {
        SeekHow::Now
    } else if ahead_s.is_some_and(|ahead| wait_s.is_none_or(|wait| wait > ahead)) {
        SeekHow::Fetches
    } else {
        SeekHow::Waits
    };
    (fraction, how)
}

/// About how long the transfer takes to bring a seek at `fraction` (and the margin), in whole
/// seconds rounded up: the bytes from the end of the part arriving toward it, at the transfer's
/// `speed` in bytes a second; none while the speed isn't known.
pub fn seek_wait_s(fraction: f64, arrived: &[(f64, f64)], size: f64, speed: f64) -> Option<f64> {
    let to = (fraction + SEEK_MARGIN).min(1.0);
    let from = arrived
        .iter()
        .filter(|r| r.0 <= to)
        .fold(0.0, |m: f64, r| m.max(r.1.min(to)));
    let left = (to - from) * size;
    if left <= 0.0 {
        return Some(0.0);
    }
    (speed > 0.0).then(|| (left / speed).ceil())
}

/// What a better copy offered while a song plays does now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Switch {
    /// Line it up and switch now.
    Now,
    Wait,
    /// It plays from the song's next play instead.
    NextPlay,
}

/// `picked`: a copy the listener chose plays; `can_switch`: the output can line copies up;
/// `steady`: playing, sound flowing, no crossfade; `arrived`: 0-1 of the copy.
pub fn switch_now(
    picked: bool,
    can_switch: bool,
    steady: bool,
    duration: Option<f64>,
    position: f64,
    arrived: f64,
) -> Switch {
    let duration = duration.filter(|d| *d != 0.0);
    if picked {
        return Switch::Wait;
    }
    if !can_switch {
        return Switch::NextPlay;
    }
    if !steady {
        return Switch::Wait;
    }
    if duration.is_some_and(|d| d - position < SWITCH_LEAD_S) {
        return Switch::NextPlay;
    }
    if arrived < 1.0 && duration.is_none_or(|d| arrived * d < position + SWITCH_AHEAD_S) {
        return Switch::Wait;
    }
    Switch::Now
}

/// A quiet moment to switch in: the level now well under the median of the last ones, once there
/// are eight.
pub fn quiet(levels: &[f64]) -> bool {
    if levels.len() < 8 {
        return false;
    }
    let mut sorted = levels.to_vec();
    sorted.sort_by(f64::total_cmp);
    levels.last().is_some_and(|now| *now <= 0.6 * sorted[levels.len() / 2])
}

/// The copy's level matched to the song's for the fade (it then glides to its own).
pub fn level_match(song: f64, copy: f64) -> f64 {
    (song / copy.max(1e-6)).clamp(0.5, 2.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    const CASES: &str = include_str!("../../../../tests/player-rules.json");

    fn rules() -> Result<Value, String> {
        serde_json::from_str(CASES).map_err(|e| e.to_string())
    }

    fn cases<'a>(rules: &'a Value, section: &str) -> Result<&'a Vec<Value>, String> {
        let s = &rules[section];
        s["cases"]
            .as_array()
            .or_else(|| s.as_array())
            .ok_or_else(|| format!("no cases for {section}"))
    }

    fn num(v: &Value) -> Result<f64, String> {
        v.as_f64().ok_or_else(|| format!("not a number: {v}"))
    }

    fn opt(v: &Value) -> Option<f64> {
        v.as_f64()
    }

    fn flag(v: &Value) -> Result<bool, String> {
        v.as_bool().ok_or_else(|| format!("not a flag: {v}"))
    }

    fn repeat(v: &Value) -> Result<Repeat, String> {
        serde_json::from_value(v.clone()).map_err(|e| e.to_string())
    }

    fn index(v: &Value) -> Result<usize, String> {
        v.as_u64()
            .map(|n| n as usize)
            .ok_or_else(|| format!("not an index: {v}"))
    }

    /// -1 in the cases is none.
    fn place(v: Option<usize>) -> Value {
        v.map_or(json!(-1), |i| json!(i))
    }

    fn near(actual: &Value, expected: &Value, what: &str) -> Result<(), String> {
        let same = match (actual.as_f64(), expected.as_f64()) {
            (Some(a), Some(e)) => (a - e).abs() < 1e-9,
            _ => match (actual, expected) {
                (Value::Array(a), Value::Array(e)) if a.len() == e.len() => {
                    return a
                        .iter()
                        .zip(e)
                        .enumerate()
                        .try_for_each(|(i, (a, e))| near(a, e, &format!("{what}[{i}]")));
                }
                (Value::Object(a), Value::Object(e)) if a.len() == e.len() => {
                    return e.iter().try_for_each(|(k, e)| near(&a[k], e, &format!("{what}.{k}")));
                }
                _ => actual == expected,
            },
        };
        if same {
            Ok(())
        } else {
            Err(format!("{what}: {actual} vs {expected}"))
        }
    }

    /// Each section's cases through this side's rule.
    fn each(section: &str, rule: impl Fn(&Value) -> Result<Value, String>) -> Result<(), String> {
        let rules = rules()?;
        for (i, c) in cases(&rules, section)?.iter().enumerate() {
            near(&rule(c)?, &c["expect"], &format!("{section} #{i}"))?;
        }
        Ok(())
    }

    #[test]
    fn every_section_is_a_rule_here_or_says_why_not() -> Result<(), String> {
        let rules = rules()?;
        let here = [
            "nextIndex",
            "previous",
            "cycleRepeat",
            "listening",
            "playStart",
            "playEnd",
            "outputVolume",
            "gain",
            "loudness",
            "crossfadeS",
            "preloadLead",
            "preloadNow",
            "gapless",
            "endedEarly",
            "fade",
            "crossfadeLengthS",
            "upcoming",
            "radio",
            "playable",
            "seekTo",
            "seekWaitS",
            "switchNow",
            "quiet",
            "levelMatch",
        ];
        let sections = rules.as_object().ok_or("no sections")?;
        for (name, section) in sections {
            if name == "note" || name == "constants" {
                continue;
            }
            let taken = section["native"].is_string();
            assert!(
                here.contains(&name.as_str()) != taken,
                "{name}: a rule here, or `native` says why not"
            );
        }
        Ok(())
    }

    #[test]
    fn the_timings_are_the_shared_ones() -> Result<(), String> {
        let rules = rules()?;
        let ms = |d: Duration| json!(d.as_millis() as u64);
        let ours = [
            ("TICK_MS", ms(TICK)),
            ("SKIP_FAILED_MS", ms(SKIP_FAILED)),
            ("PREFETCH_MS", ms(PREFETCH)),
            ("LISTEN_GAP_S", json!(LISTEN_GAP_S)),
            ("PREVIOUS_RESTARTS_S", json!(PREVIOUS_RESTARTS_S)),
            ("GAPLESS_WITHIN_S", json!(GAPLESS_WITHIN_S)),
            ("ENDED_EARLY_S", json!(ENDED_EARLY_S)),
            ("RADIO_AHEAD", json!(RADIO_AHEAD)),
            ("RADIO_STEER", json!(RADIO_STEER)),
            ("LOUDNESS_AFTER", json!(LOUDNESS_AFTER)),
            ("LOUDNESS_MIN_DB", json!(LOUDNESS_MIN_DB)),
            ("LOUDNESS_MAX_DB", json!(LOUDNESS_MAX_DB)),
            ("SWITCH_LEAD_S", json!(SWITCH_LEAD_S)),
            ("SWITCH_AHEAD_S", json!(SWITCH_AHEAD_S)),
            ("SWITCH_TRIES", json!(SWITCH_TRIES)),
            ("SWITCH_FADE_S", json!(SWITCH_FADE_S)),
            ("SWITCH_QUIET_S", json!(SWITCH_QUIET_S)),
            ("SWITCH_GLIDE_S", json!(SWITCH_GLIDE_S)),
            ("SEEK_MARGIN", json!(SEEK_MARGIN)),
        ];
        let theirs = rules["constants"].as_object().ok_or("no constants")?;
        assert_eq!(theirs.len(), ours.len(), "every shared timing is here");
        for (name, value) in ours {
            near(&value, &theirs[name], name)?;
        }
        Ok(())
    }

    #[test]
    fn the_queue_moves_as_the_page_does() -> Result<(), String> {
        each("nextIndex", |c| {
            Ok(place(next_index(
                index(&c["index"])?,
                index(&c["length"])?,
                repeat(&c["repeat"])?,
                flag(&c["auto"])?,
            )))
        })?;
        each("previous", |c| {
            Ok(place(previous(index(&c["index"])?, num(&c["pos"])?)))
        })?;
        each("cycleRepeat", |c| Ok(json!(repeat(&c["mode"])?.cycled())))
    }

    #[test]
    fn plays_are_recorded_as_the_page_records_them() -> Result<(), String> {
        each("playStart", |c| {
            Ok(play_start(
                c["trackId"].as_str().unwrap_or_default(),
                opt(&c["duration"]),
                flag(&c["recommended"])?,
            ))
        })?;
        each("playEnd", |c| {
            let how = match c["how"].as_str() {
                Some("completed") => How::Completed,
                Some("skipped") => How::Skipped,
                _ => How::Stopped,
            };
            Ok(play_end(num(&c["heard"])?, how))
        })
    }

    #[test]
    fn levels_follow_the_pages() -> Result<(), String> {
        each("outputVolume", |c| {
            Ok(json!(output_volume(num(&c["volume"])?, flag(&c["muted"])?)))
        })?;
        each("gain", |c| Ok(json!(gain(num(&c["db"])?))))?;
        each("loudness", |c| {
            Ok(json!(loudness(
                num(&c["targetDb"])?,
                num(&c["rms"])?,
                num(&c["readings"])?
            )))
        })?;
        each("crossfadeS", |c| {
            Ok(json!(crossfade_s(repeat(&c["repeat"])?, num(&c["crossfade"])?)))
        })?;
        each("fade", |c| {
            let (going, coming) = fade(num(&c["k"])?);
            Ok(json!([going, coming]))
        })?;
        each("crossfadeLengthS", |c| Ok(json!(crossfade_length_s(num(&c["left"])?))))?;
        each("levelMatch", |c| {
            Ok(json!(level_match(num(&c["song"])?, num(&c["copy"])?)))
        })
    }

    #[test]
    fn songs_load_end_and_preload_as_the_pages_do() -> Result<(), String> {
        each("preloadLead", |c| {
            Ok(json!(preload_lead(
                num(&c["crossfade"])?,
                flag(&c["soulseek"])?,
                num(&c["received"])?
            )))
        })?;
        each("preloadNow", |c| {
            Ok(json!(preload_now(
                num(&c["lead"])?,
                opt(&c["duration"]),
                num(&c["pos"])?
            )))
        })?;
        each("gapless", |c| Ok(json!(gapless(opt(&c["duration"]), num(&c["pos"])?))))?;
        each("endedEarly", |c| {
            Ok(json!(ended_early(
                flag(&c["soulseek"])?,
                flag(&c["complete"])?,
                opt(&c["duration"]),
                num(&c["pos"])?
            )))
        })?;
        each("upcoming", |c| {
            let ids: Vec<String> = serde_json::from_value(c["ids"].clone()).map_err(|e| e.to_string())?;
            let saved: Vec<String> = serde_json::from_value(c["saved"].clone()).map_err(|e| e.to_string())?;
            Ok(json!(upcoming(
                index(&c["index"])?,
                &ids,
                flag(&c["shuffle"])?,
                repeat(&c["repeat"])?,
                |id| saved.iter().any(|s| s == id),
                index(&c["n"])?
            )))
        })?;
        each("radio", |c| {
            let (seed, ask) = radio(
                index(&c["index"])?,
                index(&c["length"])?,
                repeat(&c["repeat"])?,
                c["radio"]["keepsAdding"].as_bool(),
                flag(&c["radioWhenQueueEnds"])?,
            );
            Ok(json!({ "seed": seed, "ask": ask }))
        })
    }

    #[test]
    fn seeks_and_better_copies_follow_the_pages() -> Result<(), String> {
        let parts = |v: &Value| serde_json::from_value::<Vec<(f64, f64)>>(v.clone()).map_err(|e| e.to_string());
        each("playable", |c| {
            Ok(json!(playable(
                flag(&c["soulseek"])?,
                &parts(&c["arrived"])?,
                &parts(&c["buffered"])?
            )))
        })?;
        each("seekTo", |c| {
            let (fraction, how) = seek_to(
                num(&c["fraction"])?,
                &parts(&c["playable"])?,
                opt(&c["waitS"]),
                opt(&c["aheadS"]),
            );
            let how = match how {
                SeekHow::Now => "now",
                SeekHow::Fetches => "fetches",
                SeekHow::Waits => "waits",
            };
            Ok(json!({ "fraction": fraction, "how": how }))
        })?;
        each("seekWaitS", |c| {
            let wait = seek_wait_s(
                num(&c["fraction"])?,
                &parts(&c["arrived"])?,
                num(&c["size"])?,
                num(&c["speed"])?,
            );
            Ok(json!(wait))
        })?;
        each("switchNow", |c| {
            let now = switch_now(
                flag(&c["picked"])?,
                flag(&c["canSwitch"])?,
                flag(&c["steady"])?,
                opt(&c["duration"]),
                num(&c["pos"])?,
                num(&c["arrived"])?,
            );
            Ok(json!(match now {
                Switch::Now => "switch",
                Switch::Wait => "wait",
                Switch::NextPlay => "next-play",
            }))
        })?;
        each("quiet", |c| {
            let levels: Vec<f64> = serde_json::from_value(c["levels"].clone()).map_err(|e| e.to_string())?;
            Ok(json!(quiet(&levels)))
        })
    }
}
