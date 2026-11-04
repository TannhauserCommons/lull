//! The taste model (docs/context/taste.md): artist and tag weights learned from what the
//! listener does, each signal fading with a 45-day half-life, so this month counts more than
//! last year. Small and explainable on purpose: every weight is a sum of the signals below.

use std::collections::{HashMap, HashSet};

use lull_core::ids::{AlbumId, ArtistId, TrackId, loose_artist, loose_title};
use serde::Serialize;

use crate::radio::credited_names;

/// Days after which a signal counts half.
pub const HALF_LIFE_DAYS: f64 = 45.0;

/// What each signal is worth at full strength.
pub mod weight {
    pub const COMPLETED: f64 = 1.0;
    /// Played past the scrobble point (half), not to the end.
    pub const MOSTLY: f64 = 0.5;
    /// Skipped before the scrobble point.
    pub const SKIPPED: f64 = -0.5;
    pub const LIKE: f64 = 3.0;
    pub const SAVE: f64 = 2.0;
    /// "More like this" / "less like this".
    pub const FEEDBACK: f64 = 3.0;
    /// Following is a standing wish, so it doesn't fade.
    pub const FOLLOW: f64 = 4.0;
    /// The most played artist in the Last.fm history; others scale down by log plays.
    pub const HISTORY: f64 = 2.0;
}

/// Everything the model learns from, as plain data (read by `store`, built here).
#[derive(Debug, Default)]
pub struct Signals {
    pub plays: Vec<Play>,
    pub likes: Vec<Mark>,
    pub saves: Vec<Mark>,
    /// More-like (`positive`) and less-like feedback on a track or an artist.
    pub feedback: Vec<Feedback>,
    pub followed_artists: Vec<String>,
    pub followed_tags: Vec<String>,
    /// Last.fm history: artist and play count.
    pub history: Vec<(String, u64)>,
    pub hidden: HashSet<ArtistId>,
    /// The songs played or liked, by name: the same song on another release is still known.
    pub songs: Vec<Named>,
    /// Tags of the tracks and artists named above; a track without tags uses its artist's.
    pub track_tags: HashMap<TrackId, Vec<String>>,
    pub artist_tags: HashMap<ArtistId, Vec<String>>,
}

#[derive(Debug, Clone)]
pub struct Play {
    pub track: TrackId,
    pub artist: String,
    pub at: i64,
    pub played_s: u32,
    pub duration_s: u32,
    pub completed: bool,
    pub skipped: bool,
}

#[derive(Debug, Clone)]
pub struct Mark {
    pub track: TrackId,
    pub artist: String,
    pub at: i64,
}

/// A song as the listener met it: artist credit, title and album.
#[derive(Debug, Clone)]
pub struct Named {
    pub artist: String,
    pub title: String,
    pub album: String,
}

#[derive(Debug, Clone)]
pub struct Feedback {
    pub track: Option<TrackId>,
    pub artist: String,
    pub at: i64,
    pub positive: bool,
}

/// Umbrella genres: true of so much music that digging into them finds charts, not niches.
/// Picks and radio seeds skip them; the profile itself keeps them.
const UMBRELLA_TAGS: &[&str] = &[
    "rock",
    "pop",
    "electronic",
    "electronica",
    "hip hop",
    "hip-hop",
    "rap",
    "jazz",
    "soul",
    "r&b",
    "rnb",
    "folk",
    "metal",
    "punk",
    "indie",
    "alternative",
    "alternative rock",
    "indie rock",
    "indie pop",
    "classical",
    "country",
    "blues",
    "dance",
    "experimental",
    "instrumental",
    "singer-songwriter",
    "funk",
    "reggae",
    "world",
    "seen live",
    "favorites",
    "female vocalists",
    "male vocalists",
];

/// Whether a tag is specific enough to dig into (not an umbrella genre).
pub fn is_specific(tag: &str) -> bool {
    !UMBRELLA_TAGS.contains(&tag)
}

/// Liked artists a tag must run through before picks dig into it: one artist's labels (often a
/// namesake's, or a single song tagged "rapper") are that artist, not a taste.
pub const TAG_ARTISTS: usize = 2;

/// `[id, weight]` pairs, as Stats shows them.
pub type Weights<K> = Vec<(K, f64)>;

#[derive(Debug, Clone, Serialize)]
pub struct Weighted {
    pub id: ArtistId,
    pub name: String,
    pub weight: f64,
}

/// The learned profile. Artists and tags are sorted by weight, highest first; negative
/// weights (mostly skipped, "less like this") stay in, so ranking can push them down.
#[derive(Debug, Default)]
pub struct Taste {
    pub artists: Vec<Weighted>,
    pub tags: Vec<(String, f64)>,
    /// How many liked artists (positive weight) carry each tag.
    pub tag_artists: HashMap<String, usize>,
    pub hidden: HashSet<ArtistId>,
    /// Tracks already played or liked: not news to the listener.
    pub known: HashSet<TrackId>,
    /// The same, by loose artist and title: every credited artist with the song's title.
    known_songs: HashSet<(String, String)>,
    /// Albums the listener played or liked a song from.
    known_albums: HashSet<AlbumId>,
}

impl Taste {
    pub fn build(s: &Signals, now: i64) -> Taste {
        let mut acc = Accumulator::new(s);
        for p in &s.plays {
            acc.add(&p.artist, Some(&p.track), play_weight(p) * fade(now - p.at));
        }
        for m in &s.likes {
            acc.add(&m.artist, Some(&m.track), weight::LIKE * fade(now - m.at));
        }
        for m in &s.saves {
            acc.add(&m.artist, Some(&m.track), weight::SAVE * fade(now - m.at));
        }
        for f in &s.feedback {
            let sign = if f.positive { 1.0 } else { -1.0 };
            acc.add(&f.artist, f.track.as_ref(), sign * weight::FEEDBACK * fade(now - f.at));
        }
        for a in &s.followed_artists {
            acc.add(a, None, weight::FOLLOW);
        }
        let most = s.history.iter().map(|(_, n)| *n).max().unwrap_or(0);
        for (artist, plays) in &s.history {
            acc.add(artist, None, weight::HISTORY * log_share(*plays, most));
        }

        let (mut tags, tag_artists) = acc.tag_weights();
        for t in &s.followed_tags {
            *tags.entry(t.to_lowercase()).or_default() += weight::FOLLOW;
        }
        let mut artists: Vec<Weighted> = acc
            .artists
            .into_iter()
            .map(|(id, (name, weight))| Weighted { id, name, weight })
            .collect();
        artists.sort_by(|a, b| b.weight.total_cmp(&a.weight));
        let mut tags: Vec<(String, f64)> = tags.into_iter().collect();
        tags.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let known = s.plays.iter().map(|p| &p.track).chain(s.likes.iter().map(|m| &m.track));
        Taste {
            artists,
            tags,
            tag_artists,
            hidden: s.hidden.clone(),
            known: known.cloned().collect(),
            known_songs: s
                .songs
                .iter()
                .flat_map(|n| credited_names(&n.artist).into_iter().map(|a| song_key(&a, &n.title)))
                .collect(),
            known_albums: s
                .songs
                .iter()
                .filter(|n| !n.album.is_empty())
                .map(|n| AlbumId::of(&n.artist, &n.album))
                .collect(),
        }
    }

    /// Whether the listener has played or liked this song, on any release, by any of its artists.
    pub fn knows_song(&self, artist: &str, title: &str) -> bool {
        credited_names(artist)
            .iter()
            .any(|a| self.known_songs.contains(&song_key(a, title)))
    }

    /// Whether the listener has met this release already: played or liked a song from it, or,
    /// for a single, the song it is named after.
    pub fn knows_release(&self, artist: &str, title: &str) -> bool {
        self.known_albums.contains(&AlbumId::of(artist, title)) || self.knows_song(artist, title)
    }

    /// Tags worth digging into, strongest first: specific ones that run through at least
    /// `TAG_ARTISTS` liked artists.
    pub fn dig_tags(&self) -> impl Iterator<Item = &(String, f64)> {
        self.top_tags(usize::MAX)
            .filter(|(t, _)| is_specific(t) && self.tag_artists.get(t).copied().unwrap_or(0) >= TAG_ARTISTS)
    }

    pub fn is_empty(&self) -> bool {
        self.artists.iter().all(|a| a.weight <= 0.0) && self.tags.iter().all(|t| t.1 <= 0.0)
    }

    pub fn artist_weight(&self, id: &ArtistId) -> f64 {
        self.artists.iter().find(|a| &a.id == id).map_or(0.0, |a| a.weight)
    }

    /// The best-liked artists (positive weight only).
    pub fn top_artists(&self, n: usize) -> impl Iterator<Item = &Weighted> {
        self.artists.iter().filter(|a| a.weight > 0.0).take(n)
    }

    pub fn top_tags(&self, n: usize) -> impl Iterator<Item = &(String, f64)> {
        self.tags.iter().filter(|t| t.1 > 0.0).take(n)
    }

    /// The Stats "taste" block: positive weights scaled so the strongest is 1.
    pub fn summary(&self, n: usize) -> (Weights<ArtistId>, Weights<String>) {
        let scale = |max: f64, w: f64| (w / max * 100.0).round() / 100.0;
        let max_a = self.artists.first().map_or(1.0, |a| a.weight.max(f64::EPSILON));
        let max_t = self.tags.first().map_or(1.0, |t| t.1.max(f64::EPSILON));
        (
            self.top_artists(n)
                .map(|a| (a.id.clone(), scale(max_a, a.weight)))
                .collect(),
            self.top_tags(n).map(|(t, w)| (t.clone(), scale(max_t, *w))).collect(),
        )
    }
}

struct Accumulator<'a> {
    signals: &'a Signals,
    artists: HashMap<ArtistId, (String, f64)>,
    /// The tags each artist carries: those of its tracks the signals name, else its own.
    carried: HashMap<ArtistId, HashSet<String>>,
}

impl<'a> Accumulator<'a> {
    fn new(signals: &'a Signals) -> Self {
        Accumulator {
            signals,
            artists: HashMap::new(),
            carried: HashMap::new(),
        }
    }

    /// Each tag weighs what the artists carrying it weigh, together, and counts the liked ones:
    /// a tag on many liked artists beats one artist's labels, however few labels that artist has.
    fn tag_weights(&self) -> (HashMap<String, f64>, HashMap<String, usize>) {
        let mut weights: HashMap<String, f64> = HashMap::new();
        let mut liked: HashMap<String, usize> = HashMap::new();
        for (id, tags) in &self.carried {
            let w = self.artists.get(id).map_or(0.0, |a| a.1);
            for t in tags {
                *weights.entry(t.clone()).or_default() += w;
                if w > 0.0 {
                    *liked.entry(t.clone()).or_default() += 1;
                }
            }
        }
        (weights, liked)
    }

    /// Adds `w` to the artist, which carries the track's (else the artist's own) tags.
    fn add(&mut self, artist: &str, track: Option<&TrackId>, w: f64) {
        if w == 0.0 || artist.is_empty() {
            return;
        }
        let id = ArtistId::of(artist);
        if self.signals.hidden.contains(&id) {
            return;
        }
        let tags = track
            .and_then(|t| self.signals.track_tags.get(t))
            .filter(|t| !t.is_empty())
            .or_else(|| self.signals.artist_tags.get(&id));
        if let Some(tags) = tags.filter(|t| !t.is_empty()) {
            let carried = self.carried.entry(id.clone()).or_default();
            carried.extend(tags.iter().map(|t| t.to_lowercase()));
        }
        let entry = self.artists.entry(id).or_insert_with(|| (artist.to_owned(), 0.0));
        entry.1 += w;
    }
}

fn play_weight(p: &Play) -> f64 {
    if p.completed {
        weight::COMPLETED
    } else if p.skipped {
        weight::SKIPPED
    } else if p.duration_s > 0 && p.played_s.saturating_mul(2) >= p.duration_s {
        weight::MOSTLY
    } else {
        0.0
    }
}

fn song_key(artist: &str, title: &str) -> (String, String) {
    (loose_artist(artist), loose_title(title))
}

/// 1 now, 1/2 after one half-life, 1/4 after two...
pub fn fade(age_s: i64) -> f64 {
    let days = age_s.max(0) as f64 / 86_400.0;
    0.5_f64.powf(days / HALF_LIFE_DAYS)
}

/// `plays` as a share of `most` on a log scale, so a long tail still counts.
fn log_share(plays: u64, most: u64) -> f64 {
    if most == 0 {
        return 0.0;
    }
    (1.0 + plays as f64).ln() / (1.0 + most as f64).ln()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;
    const DAY: i64 = 86_400;

    fn play(artist: &str, title: &str, days_ago: i64, completed: bool, skipped: bool) -> Play {
        Play {
            track: TrackId::of(artist, "", title),
            artist: artist.into(),
            at: NOW - days_ago * DAY,
            played_s: if completed { 200 } else { 20 },
            duration_s: 200,
            completed,
            skipped,
        }
    }

    fn weight_of(taste: &Taste, artist: &str) -> f64 {
        taste.artist_weight(&ArtistId::of(artist))
    }

    #[test]
    fn recent_signals_outweigh_old_ones_and_follows_never_fade() {
        let like = |artist: &str, days_ago| Mark {
            track: TrackId::of(artist, "", "song"),
            artist: artist.into(),
            at: NOW - days_ago * DAY,
        };
        let signals = Signals {
            likes: vec![like("Now", 0), like("Then", 45), like("Long Ago", 450)],
            followed_artists: vec!["Followed Years Ago".into()],
            ..Default::default()
        };
        let taste = Taste::build(&signals, NOW);
        assert!((weight_of(&taste, "Now") - weight::LIKE).abs() < 1e-9);
        assert!(
            (weight_of(&taste, "Then") - weight::LIKE / 2.0).abs() < 1e-9,
            "one half-life"
        );
        assert!(weight_of(&taste, "Long Ago") < 0.01, "ten half-lives is all but gone");
        assert_eq!(weight_of(&taste, "Followed Years Ago"), weight::FOLLOW);
        assert_eq!(taste.artists[0].name, "Followed Years Ago");
    }

    #[test]
    fn completions_lift_skips_sink_and_artists_carry_their_tags() {
        let mut signals = Signals {
            plays: vec![
                play("Finisher", "a", 0, true, false),
                play("Finisher", "b", 0, true, false),
                play("Skipped", "c", 0, false, true),
                play("Skipped", "d", 0, false, true),
                play("Hidden", "e", 0, true, false),
            ],
            hidden: [ArtistId::of("Hidden")].into(),
            ..Default::default()
        };
        signals.track_tags.insert(
            TrackId::of("Finisher", "", "a"),
            vec!["slowcore".into(), "Dream Pop".into()],
        );
        signals
            .artist_tags
            .insert(ArtistId::of("Finisher"), vec!["slowcore".into()]);
        let taste = Taste::build(&signals, NOW);

        assert_eq!(weight_of(&taste, "Finisher"), 2.0 * weight::COMPLETED);
        assert_eq!(weight_of(&taste, "Skipped"), 2.0 * weight::SKIPPED);
        assert_eq!(weight_of(&taste, "Hidden"), 0.0, "hidden artists are out of the model");
        // Track a carries two tags, track b none (so its artist's): Finisher carries both, and
        // each weighs all of Finisher's 2.0.
        assert_eq!(taste.tags, vec![("dream pop".into(), 2.0), ("slowcore".into(), 2.0)]);
        assert!(taste.known.contains(&TrackId::of("Skipped", "", "c")));
        let (artists, tags) = taste.summary(10);
        assert_eq!(artists, vec![(ArtistId::of("Finisher"), 1.0)]);
        assert_eq!(tags[1], ("slowcore".into(), 1.0));
    }

    #[test]
    fn a_tag_counts_by_the_liked_artists_it_runs_through() {
        // Live, one namesake's two labels (kurdish, iraq) and one song tagged only "rapper" topped
        // the profile, because a signal's weight was split among however few tags it had.
        let like = |artist: &str| Mark {
            track: TrackId::of(artist, "", "song"),
            artist: artist.into(),
            at: NOW,
        };
        let mut signals = Signals {
            likes: ["Namesake", "Glow Tape", "Tidal Bloom", "Night Arcade"]
                .map(like)
                .into(),
            ..Default::default()
        };
        let tags = |list: &[&str]| list.iter().map(|t| (*t).to_owned()).collect::<Vec<_>>();
        for (artist, carried) in [
            ("Namesake", tags(&["kurdish"])),
            ("Glow Tape", tags(&["chillwave", "bedroom pop", "synth-pop", "lo-fi"])),
            (
                "Tidal Bloom",
                tags(&["chillwave", "dream pop", "bedroom pop", "shoegaze"]),
            ),
            ("Night Arcade", tags(&["chillwave", "pop"])),
        ] {
            signals.artist_tags.insert(ArtistId::of(artist), carried);
        }
        let taste = Taste::build(&signals, NOW);

        assert_eq!(taste.tags[0], ("chillwave".into(), 3.0 * weight::LIKE));
        assert_eq!(taste.tags[1], ("bedroom pop".into(), 2.0 * weight::LIKE));
        let dug: Vec<&str> = taste.dig_tags().map(|(t, _)| t.as_str()).collect();
        assert_eq!(
            dug,
            ["chillwave", "bedroom pop"],
            "one artist's tag isn't dug into, nor an umbrella genre"
        );
    }

    #[test]
    fn a_song_is_known_on_any_release_and_by_any_of_its_artists() {
        let signals = Signals {
            songs: vec![Named {
                artist: "Tidal Bloom & Mira Vale".into(),
                title: "Cold Harbour".into(),
                album: "Cold Harbour".into(),
            }],
            ..Default::default()
        };
        let taste = Taste::build(&signals, NOW);
        assert!(taste.knows_song("Tidal Bloom", "Cold Harbour"));
        assert!(taste.knows_song("Mira Vale", "cold harbour (remastered)"));
        assert!(!taste.knows_song("Tidal Bloom", "Lanterns"));
        assert!(
            taste.knows_release("Tidal Bloom", "Cold Harbour"),
            "the single of a known song"
        );
        assert!(!taste.knows_release("Tidal Bloom", "Ginger Sleep"));
    }
}
