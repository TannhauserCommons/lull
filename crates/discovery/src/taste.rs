//! The taste model (docs/context/taste.md): artist and tag weights learned from what the
//! listener does, each signal fading with a 45-day half-life, so this month counts more than
//! last year. Small and explainable on purpose: every weight is a sum of the signals below.

use std::collections::{HashMap, HashSet};

use lull_core::ids::{AlbumId, ArtistId, TrackId, loose_artist, loose_title};
use serde::{Deserialize, Serialize};

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
    /// The artist with the most saved albums; others scale down by log albums. About one song
    /// heard to the end: what the listener keeps says who they are before they have played
    /// anything here, and their listening soon outweighs it.
    pub const LIBRARY: f64 = 1.0;
}

/// Everything the model learns from, as plain data (read by `store`, built here).
#[derive(Debug, Default, Clone)]
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
    /// The saved library: artist and how many of their albums it holds.
    pub library: Vec<(String, u64)>,
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

/// Damping, so one day can't swing the profile (docs/context/taste.md#damping): a day of
/// listening counts, not each play. At most this many heard plays per artist per day count...
pub const HEARD_PER_DAY: u32 = 3;
/// ...and this many skips.
pub const SKIPS_PER_DAY: u32 = 2;
/// Separate days an artist must be heard on before it is established (seeds picks, carries its
/// tags), unless the listener acted on it (a like, a save, a follow, "more like this", saved
/// albums, the Last.fm history).
pub const ESTABLISHED_DAYS: usize = 2;

/// What the listener did that an artist's weight (or a tag's) comes from, as a reason says it
/// (`feed::Reason::evidence`): counts, never a guess. Zero and false are left out of the API.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Evidence {
    /// Songs of the artist heard (to the end or past half) in the last year.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub plays: u32,
    /// Its songs liked.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub liked: u32,
    /// Its songs saved through lull.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub saved: u32,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub followed: bool,
    /// Its albums in the saved library.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub library: u32,
    /// Plays in the Last.fm history.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub lastfm: u32,
    /// For a tag: the liked artists that carry it.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub artists: u32,
}

impl Evidence {
    pub fn is_empty(&self) -> bool {
        *self == Evidence::default()
    }
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

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
    /// What each artist's weight comes from.
    evidence: HashMap<ArtistId, Evidence>,
    /// Artists the listener came back to (`ESTABLISHED_DAYS`) or acted on: only they seed picks
    /// and count for a tag.
    established: HashSet<ArtistId>,
    /// Artists kept out of picks and radio: "less like this", or skipped on 2 or more days. A
    /// skip or two on one day doesn't hold an artist back.
    held_back: HashSet<ArtistId>,
}

impl Taste {
    pub fn build(s: &Signals, now: i64) -> Taste {
        let mut acc = Accumulator::new(s);
        // A day at a time per artist, earliest first: the first plays of a day count.
        let mut plays: Vec<&Play> = s.plays.iter().collect();
        plays.sort_by_key(|p| p.at);
        let mut today: HashMap<(ArtistId, i64, bool), u32> = HashMap::new();
        for p in plays {
            let w = play_weight(p);
            let day = p.at.div_euclid(86_400);
            let id = ArtistId::of(&p.artist);
            let n = today.entry((id.clone(), day, w > 0.0)).or_default();
            *n += 1;
            let counts = if w > 0.0 {
                *n <= HEARD_PER_DAY
            } else {
                w == 0.0 || *n <= SKIPS_PER_DAY
            };
            if counts {
                acc.add(&p.artist, Some(&p.track), w * fade(now - p.at));
            }
            if w > 0.0 {
                acc.note(&p.artist, |e| e.plays += 1);
                acc.heard_days.entry(id).or_default().insert(day);
            } else if w < 0.0 {
                acc.skip_days.entry(id).or_default().insert(day);
            }
        }
        for m in &s.likes {
            acc.add(&m.artist, Some(&m.track), weight::LIKE * fade(now - m.at));
            acc.note(&m.artist, |e| e.liked += 1);
            acc.acted(&m.artist);
        }
        for m in &s.saves {
            acc.add(&m.artist, Some(&m.track), weight::SAVE * fade(now - m.at));
            acc.note(&m.artist, |e| e.saved += 1);
            acc.acted(&m.artist);
        }
        for f in &s.feedback {
            let sign = if f.positive { 1.0 } else { -1.0 };
            acc.add(&f.artist, f.track.as_ref(), sign * weight::FEEDBACK * fade(now - f.at));
            if f.positive {
                acc.acted(&f.artist);
            } else {
                acc.disliked.insert(ArtistId::of(&f.artist));
            }
        }
        for a in &s.followed_artists {
            acc.add(a, None, weight::FOLLOW);
            acc.note(a, |e| e.followed = true);
            acc.acted(a);
        }
        let most = s.history.iter().map(|(_, n)| *n).max().unwrap_or(0);
        for (artist, plays) in &s.history {
            acc.add(artist, None, weight::HISTORY * log_share(*plays, most));
            acc.note(artist, |e| e.lastfm = u32::try_from(*plays).unwrap_or(u32::MAX));
            acc.acted(artist);
        }
        let most = s.library.iter().map(|(_, n)| *n).max().unwrap_or(0);
        for (artist, albums) in &s.library {
            acc.add(artist, None, weight::LIBRARY * log_share(*albums, most));
            acc.note(artist, |e| e.library = u32::try_from(*albums).unwrap_or(u32::MAX));
            acc.acted(artist);
        }
        acc.settle();

        let (mut tags, tag_artists) = acc.tag_weights();
        for t in &s.followed_tags {
            *tags.entry(t.to_lowercase()).or_default() += weight::FOLLOW;
        }
        let evidence = std::mem::take(&mut acc.evidence);
        let established = std::mem::take(&mut acc.established);
        let held_back: HashSet<ArtistId> = acc
            .artists
            .iter()
            .filter(|(id, (_, w))| {
                *w < 0.0 && (acc.disliked.contains(*id) || acc.skip_days.get(*id).is_some_and(|d| d.len() >= 2))
            })
            .map(|(id, _)| id.clone())
            .chain(acc.disliked.iter().cloned())
            .collect();
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
            evidence,
            established,
            held_back,
        }
    }

    /// The best-liked artists that may seed picks: established ones (heard on separate days, or
    /// acted on), strongest first.
    pub fn seed_artists(&self, n: usize) -> impl Iterator<Item = &Weighted> {
        self.artists
            .iter()
            .filter(|a| a.weight > 0.0 && self.established.contains(&a.id))
            .take(n)
    }

    /// Whether the artist is kept out of picks: "less like this", or skipped on separate days.
    pub fn held_back(&self, id: &ArtistId) -> bool {
        self.held_back.contains(id)
    }

    /// What the artist's weight comes from.
    pub fn evidence(&self, id: &ArtistId) -> Evidence {
        self.evidence.get(id).cloned().unwrap_or_default()
    }

    /// What a tag's weight comes from: the liked artists that carry it, and a follow.
    pub fn tag_evidence(&self, tag: &str, followed: bool) -> Evidence {
        Evidence {
            artists: self
                .tag_artists
                .get(tag)
                .map_or(0, |n| u32::try_from(*n).unwrap_or(u32::MAX)),
            followed,
            ..Evidence::default()
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
    evidence: HashMap<ArtistId, Evidence>,
    /// The days each artist was heard on, and skipped on.
    heard_days: HashMap<ArtistId, HashSet<i64>>,
    skip_days: HashMap<ArtistId, HashSet<i64>>,
    /// Artists the listener acted on, and those they asked for less of.
    acted: HashSet<ArtistId>,
    disliked: HashSet<ArtistId>,
    established: HashSet<ArtistId>,
}

impl<'a> Accumulator<'a> {
    fn new(signals: &'a Signals) -> Self {
        Accumulator {
            signals,
            artists: HashMap::new(),
            carried: HashMap::new(),
            evidence: HashMap::new(),
            heard_days: HashMap::new(),
            skip_days: HashMap::new(),
            acted: HashSet::new(),
            disliked: HashSet::new(),
            established: HashSet::new(),
        }
    }

    fn acted(&mut self, artist: &str) {
        if !artist.is_empty() {
            self.acted.insert(ArtistId::of(artist));
        }
    }

    /// Decides who is established, once every signal is in.
    fn settle(&mut self) {
        let returned = self
            .heard_days
            .iter()
            .filter(|(_, days)| days.len() >= ESTABLISHED_DAYS)
            .map(|(id, _)| id.clone());
        self.established = self.acted.iter().cloned().chain(returned).collect();
        self.established.retain(|id| !self.signals.hidden.contains(id));
    }

    /// Records what the listener did with the artist (a hidden one isn't kept).
    fn note(&mut self, artist: &str, change: impl FnOnce(&mut Evidence)) {
        let id = ArtistId::of(artist);
        if artist.is_empty() || self.signals.hidden.contains(&id) {
            return;
        }
        change(self.evidence.entry(id).or_default());
    }

    /// Each tag weighs what the artists carrying it weigh, together, and counts the liked,
    /// established ones: a tag on many liked artists beats one artist's labels, and a stray play
    /// never unlocks a tag.
    fn tag_weights(&self) -> (HashMap<String, f64>, HashMap<String, usize>) {
        let mut weights: HashMap<String, f64> = HashMap::new();
        let mut liked: HashMap<String, usize> = HashMap::new();
        for (id, tags) in &self.carried {
            let w = self.artists.get(id).map_or(0.0, |a| a.1);
            for t in tags {
                *weights.entry(t.clone()).or_default() += w;
                if w > 0.0 && self.established.contains(id) {
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
    fn a_saved_library_is_a_taste_before_anything_is_played_and_listening_soon_leads() {
        // A listener who connected 2,000 albums and played nothing here got random starter tags.
        let mut signals = Signals {
            library: vec![
                ("Velvet Orchard".into(), 40),
                ("Glow Tape".into(), 3),
                ("Pale Harbor".into(), 1),
            ],
            ..Default::default()
        };
        signals
            .artist_tags
            .insert(ArtistId::of("Velvet Orchard"), vec!["dream pop".into()]);
        let taste = Taste::build(&signals, NOW);
        assert!(!taste.is_empty(), "not a first run");
        assert_eq!(taste.artists[0].name, "Velvet Orchard", "the most kept leads");
        assert!(weight_of(&taste, "Velvet Orchard") <= weight::LIBRARY + 1e-9);
        assert!(
            weight_of(&taste, "Pale Harbor") > 0.0,
            "every saved artist counts a little"
        );
        assert_eq!(taste.top_tags(1).next().map(|(t, _)| t.as_str()), Some("dream pop"));

        signals.plays = vec![
            play("Quiet Ferns", "a", 0, true, false),
            play("Quiet Ferns", "b", 0, true, false),
        ];
        let taste = Taste::build(&signals, NOW);
        assert_eq!(
            taste.artists[0].name, "Quiet Ferns",
            "two songs heard outweigh the whole shelf"
        );
    }

    #[test]
    fn a_reason_counts_what_the_listener_did_and_nothing_else() {
        // "An artist you like" was said of one only ever played; now the reason says what it was.
        let like = |artist: &str| Mark {
            track: TrackId::of(artist, "", "song"),
            artist: artist.into(),
            at: NOW,
        };
        let mut signals = Signals {
            plays: vec![
                play("Glow Tape", "a", 1, true, false),
                play("Glow Tape", "b", 2, true, false),
                play("Glow Tape", "c", 3, false, true),
            ],
            likes: vec![like("Pale Harbor")],
            followed_artists: vec!["Quiet Ferns".into()],
            library: vec![("Velvet Orchard".into(), 4)],
            ..Default::default()
        };
        for a in ["Glow Tape", "Pale Harbor"] {
            signals.artist_tags.insert(ArtistId::of(a), vec!["dream pop".into()]);
        }
        let taste = Taste::build(&signals, NOW);
        let e = |a: &str| taste.evidence(&ArtistId::of(a));
        assert_eq!(
            e("Glow Tape"),
            Evidence {
                plays: 2,
                ..Evidence::default()
            },
            "a skip isn't a play heard"
        );
        assert_eq!(
            e("Pale Harbor"),
            Evidence {
                liked: 1,
                ..Evidence::default()
            }
        );
        assert_eq!(
            e("Quiet Ferns"),
            Evidence {
                followed: true,
                ..Evidence::default()
            }
        );
        assert_eq!(
            e("Velvet Orchard"),
            Evidence {
                library: 4,
                ..Evidence::default()
            }
        );
        assert!(e("Nobody").is_empty());
        assert_eq!(taste.tag_evidence("dream pop", false).artists, 2);
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
