//! Which MusicBrainz release a file came from (docs/context/save-flow.md#matching). Pure: the
//! search results come in, ranked candidates and a decision come out.

use lull_core::ids::normalize;
use serde::{Deserialize, Serialize};

use crate::musicbrainz::{Recording, RecordingRelease, credit_name};

/// What is known about the file: its tags, else the search result it was saved from.
#[derive(Debug, Clone)]
pub struct Wanted {
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub duration_s: Option<f64>,
    pub track: Option<u32>,
}

/// One release the file may come from: a row in the release picker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    /// The release's MusicBrainz id.
    pub mbid: String,
    pub recording: String,
    pub release_group: Option<String>,
    pub title: String,
    pub artist: String,
    /// "album, CD", "single, Digital Media", "album (compilation), CD".
    pub kind: String,
    pub date: Option<String>,
    pub country: Option<String>,
    pub tracks: Option<u32>,
    /// 0-1: how well title, artist, album and length agree.
    pub score: f32,
}

#[derive(Debug, PartialEq)]
pub enum Decision {
    Sure(Candidate),
    /// For the release picker, best first; empty when MusicBrainz knows nothing like it.
    Unsure(Vec<Candidate>),
}

/// Below this, a match is a guess.
const SURE: f32 = 0.85;
/// How far the best album must lead any other album (release group) to be taken without asking.
const LEAD: f32 = 0.08;
/// Picker rows.
const SHOWN: usize = 8;

pub fn decide(wanted: &Wanted, recordings: &[Recording]) -> Decision {
    let ranked = rank(wanted, recordings);
    let Some(best) = ranked.first() else {
        return Decision::Unsure(vec![]);
    };
    let runner_up = ranked.iter().find(|c| c.release_group != best.release_group);
    if best.score >= SURE && runner_up.is_none_or(|r| best.score - r.score >= LEAD) {
        Decision::Sure(best.clone())
    } else {
        Decision::Unsure(ranked.into_iter().take(SHOWN).collect())
    }
}

/// One candidate per release, best first. Within a release group the official, earliest
/// release wins ties, so the choice is stable.
fn rank(wanted: &Wanted, recordings: &[Recording]) -> Vec<Candidate> {
    let mut all: Vec<(Candidate, bool)> = recordings
        .iter()
        .flat_map(|rec| rec.releases.iter().map(move |rel| score(wanted, rec, rel)))
        .collect();
    all.sort_by(|(a, a_official), (b, b_official)| {
        b.score
            .total_cmp(&a.score)
            .then(b_official.cmp(a_official))
            .then_with(|| date_key(&a.date).cmp(date_key(&b.date)))
            .then_with(|| a.mbid.cmp(&b.mbid))
    });
    let mut seen = std::collections::HashSet::new();
    all.into_iter()
        .map(|(c, _)| c)
        .filter(|c| seen.insert(c.mbid.clone()))
        .collect()
}

fn score(wanted: &Wanted, rec: &Recording, rel: &RecordingRelease) -> (Candidate, bool) {
    let credited = credit_name(&rec.artist_credit);
    let first_artist = rec.artist_credit.first().map(|c| c.name.as_str()).unwrap_or("");
    let mut parts = vec![
        (0.35, similarity(&wanted.title, &rec.title)),
        (
            0.25,
            similarity(&wanted.artist, &credited).max(similarity(&wanted.artist, first_artist)),
        ),
    ];
    if let Some(album) = &wanted.album {
        parts.push((0.20, similarity(album, &rel.title)));
    }
    if let (Some(want), Some(ms)) = (wanted.duration_s, rec.length) {
        parts.push((0.20, length_agreement(want, ms as f64 / 1000.0)));
    }
    let weight: f32 = parts.iter().map(|(w, _)| w).sum();
    let mut score = parts.iter().map(|(w, s)| w * s).sum::<f32>() / weight;
    let same_number = wanted.track.is_some_and(|n| {
        rel.media
            .iter()
            .flat_map(|m| &m.tracks)
            .any(|t| t.number.as_deref().and_then(|x| x.parse().ok()) == Some(n))
    });
    if same_number {
        score = (score + 0.02).min(1.0);
    }
    let official = rel.status.as_deref() == Some("Official");
    let candidate = Candidate {
        mbid: rel.id.clone(),
        recording: rec.id.clone(),
        release_group: rel.release_group.as_ref().map(|g| g.id.clone()),
        title: rel.title.clone(),
        artist: credited,
        kind: kind(rel),
        date: rel.date.clone().filter(|d| !d.is_empty()),
        country: rel.country.clone(),
        tracks: rel.track_count,
        score,
    };
    (candidate, official)
}

fn kind(rel: &RecordingRelease) -> String {
    let group = rel.release_group.as_ref();
    let mut kind = group
        .and_then(|g| g.primary_type.clone())
        .unwrap_or_else(|| "release".into())
        .to_lowercase();
    if let Some(g) = group.filter(|g| !g.secondary_types.is_empty()) {
        kind = format!("{kind} ({})", g.secondary_types.join(", ").to_lowercase());
    }
    match rel.media.first().and_then(|m| m.format.clone()) {
        Some(format) => format!("{kind}, {format}"),
        None => kind,
    }
}

/// Unknown dates sort last.
fn date_key(date: &Option<String>) -> &str {
    date.as_deref().filter(|d| !d.is_empty()).unwrap_or("9999")
}

/// 1 within 2 s, 0 beyond 15 s: same recording, not a live take or an edit.
fn length_agreement(a: f64, b: f64) -> f32 {
    let diff = (a - b).abs();
    (1.0 - ((diff - 2.0) / 13.0).clamp(0.0, 1.0)) as f32
}

/// 0-1 after normalising. A bracketed suffix ("(Remastered 2011)", "[FLAC]") costs a little
/// rather than everything, so folder names and remaster titles still match.
pub fn similarity(a: &str, b: &str) -> f32 {
    let full = ratio(&normalize(a), &normalize(b));
    let bare = ratio(&normalize(&strip_brackets(a)), &normalize(&strip_brackets(b)));
    full.max(bare * 0.92)
}

fn strip_brackets(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0usize;
    for c in s.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            c if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

/// 1 - edit distance / longer length.
fn ratio(a: &str, b: &str) -> f32 {
    if a == b {
        return 1.0;
    }
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let longest = a.len().max(b.len());
    if longest == 0 {
        return 1.0;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            let swap = prev[j] + usize::from(ca != cb);
            cur[j + 1] = swap.min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        prev = cur;
    }
    1.0 - prev[b.len()] as f32 / longest as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn release(id: &str, title: &str, group: &str, kind: &str, date: &str, track_no: &str) -> Value {
        json!({
            "id": id, "title": title, "status": "Official", "date": date, "country": "XW", "track-count": 10,
            "release-group": { "id": group, "primary-type": kind },
            "media": [{ "position": 1, "format": "Digital Media", "track": [{ "id": format!("{id}-t"), "number": track_no }] }]
        })
    }

    fn recording(id: &str, title: &str, length_ms: u64, releases: Vec<Value>) -> Value {
        json!({
            "id": id, "title": title, "length": length_ms,
            "artist-credit": [{ "name": "Glass Harbour", "joinphrase": "", "artist": { "id": "a-1" } }],
            "releases": releases
        })
    }

    fn parse(recordings: Vec<Value>) -> Result<Vec<Recording>, serde_json::Error> {
        recordings.into_iter().map(serde_json::from_value).collect()
    }

    fn wanted(album: Option<&str>, duration_s: f64) -> Wanted {
        Wanted {
            title: "Rope & Tide".into(),
            artist: "Glass Harbour".into(),
            album: album.map(str::to_owned),
            duration_s: Some(duration_s),
            track: Some(3),
        }
    }

    /// The studio recording on the album, a single and a compilation, plus a live take.
    fn catalogue() -> Result<Vec<Recording>, serde_json::Error> {
        parse(vec![
            recording(
                "rec-studio",
                "Rope & Tide",
                241_000,
                vec![
                    release("rel-album-cd", "Lantern Season", "rg-album", "Album", "2024-05-01", "3"),
                    release(
                        "rel-album-digital",
                        "Lantern Season",
                        "rg-album",
                        "Album",
                        "2024-05-01",
                        "3",
                    ),
                    release("rel-single", "Rope & Tide", "rg-single", "Single", "2024-03-01", "1"),
                    release(
                        "rel-best-of",
                        "Harbour Lights: The Best Of",
                        "rg-best",
                        "Album",
                        "2030-01-01",
                        "7",
                    ),
                ],
            ),
            recording(
                "rec-live",
                "Rope & Tide (live)",
                312_000,
                vec![release(
                    "rel-live",
                    "Live at the Boathouse",
                    "rg-live",
                    "Album",
                    "2025-01-01",
                    "3",
                )],
            ),
        ])
    }

    #[test]
    fn known_album_and_length_pick_the_release_without_asking() -> Result<(), serde_json::Error> {
        // Soulseek folder names carry noise; a remaster suffix must not lose the match.
        let decision = decide(&wanted(Some("Lantern Season [2024 FLAC]"), 240.4), &catalogue()?);
        let Decision::Sure(c) = decision else {
            return Err(serde::de::Error::custom(format!(
                "expected a sure match, got {decision:?}"
            )));
        };
        assert_eq!(
            (c.recording.as_str(), c.release_group.as_deref()),
            ("rec-studio", Some("rg-album"))
        );
        // Two releases of the same album tie; the choice is stable (same date: lower id).
        assert_eq!(c.mbid, "rel-album-cd");
        assert_eq!(c.kind, "album, Digital Media");
        Ok(())
    }

    #[test]
    fn without_an_album_the_listener_picks_the_release() -> Result<(), serde_json::Error> {
        let Decision::Unsure(rows) = decide(&wanted(None, 240.4), &catalogue()?) else {
            return Err(serde::de::Error::custom(
                "album, single and best-of tie; that needs the picker",
            ));
        };
        let groups: std::collections::HashSet<_> = rows.iter().filter_map(|r| r.release_group.as_deref()).collect();
        assert!(groups.contains("rg-album") && groups.contains("rg-single"));
        assert_eq!(
            rows.last().map(|r| r.recording.as_str()),
            Some("rec-live"),
            "the live take ranks last"
        );
        Ok(())
    }

    #[test]
    fn length_tells_a_live_take_from_the_studio_recording() -> Result<(), serde_json::Error> {
        let live = Wanted {
            title: "Rope & Tide (live)".into(),
            ..wanted(Some("Live at the Boathouse"), 311.0)
        };
        let Decision::Sure(c) = decide(&live, &catalogue()?) else {
            return Err(serde::de::Error::custom("the live album should match"));
        };
        assert_eq!(c.mbid, "rel-live");
        Ok(())
    }

    #[test]
    fn nothing_found_asks_to_keep_existing_tags() {
        assert_eq!(
            decide(&wanted(Some("Lantern Season"), 240.0), &[]),
            Decision::Unsure(vec![])
        );
    }

    #[test]
    fn similarity_tolerates_noise_but_not_other_songs() {
        assert_eq!(similarity("The Cure", "the cure!"), 1.0);
        assert!(similarity("Lantern Season (Deluxe Edition)", "Lantern Season") > 0.9);
        assert!(similarity("Rope & Tide", "Tide Pools") < 0.6);
    }
}
