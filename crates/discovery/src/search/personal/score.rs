//! How much the listener's own music lifts a search row: one number from 0 to 1 per song, album
//! and artist, a pure function of what taste already reads (`Signals`) and of the ids lull holds
//! for the listener's artists (docs/context/taste.md#search-affinity).
//!
//! - **Song** (the strongest tier): the sum of its own signals at taste's weights and fade (a
//!   play to the end, past half or skipped; a like; a save; a playlist), as `sum / (sum + 3)`: a
//!   like alone is a half, and more only closes the gap to 1. A skip can take a song to nothing,
//!   never below.
//! - **Album:** from its songs, `1 - ∏(1 - song)`: each kept or played song adds to it.
//! - **Artist** (the medium tier): taste's artist weight as `weight / (weight + 4)`, reached
//!   **only through ids**: a catalog's own artist id on a search row, matched to an id lull holds
//!   for a listener's artist. A name is never compared, so a namesake gets nothing; an artist the
//!   listener hid gets nothing.
//! - **A song's or album's row** also carries half of its artist's score, `1 - (1 - own)(1 - ½ × artist)`.
//!
//! Every step is bounded by 1, rises with each signal and never falls below 0, so a search never
//! sinks a row the words match, and each number can be explained from the listener's actions.

use std::collections::HashMap;

use lull_core::ids::{ArtistId, TrackId};

use crate::artist::identity::Source;
use crate::taste::{Signals, Taste, fade, play_weight, weight};

/// A song's raw sum that scores a half: one like.
const SONG_HALF: f64 = weight::LIKE;
/// An artist's weight that scores a half: one follow.
const ARTIST_HALF: f64 = weight::FOLLOW;
/// What a song in a playlist is worth: as a save, a standing choice that doesn't fade.
const PLAYLISTED: f64 = weight::SAVE;
/// The share of an artist's score a song or album of theirs carries.
const ARTIST_SHARE: f64 = 0.5;

/// A catalog's own id for an artist: Apple's, Deezer's, or MusicBrainz's.
pub type CatalogId = (Source, String);

/// What the listener's music lifts a row by.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Affinity {
    /// 0 to 1.
    pub score: f64,
    /// The listener has this very take (played, liked, kept or in a playlist): another version
    /// of a song they play is their song, not an "other version" (`merge::Score::with_affinity`).
    pub own: bool,
}

/// How the listener keeps a song apart from playing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keep {
    /// It is in the saved library now.
    Saved,
    /// It is in one of their playlists.
    Playlisted,
}

#[derive(Debug, Clone)]
pub struct Kept {
    pub track: TrackId,
    pub artist: String,
    pub how: Keep,
}

/// Each song's score, by the id the listener met it under. Songs of an artist they hid, and
/// songs with nothing in their favour, are not listed.
pub fn songs(signals: &Signals, kept: &[Kept], now: i64) -> HashMap<TrackId, f64> {
    let mut sums: HashMap<TrackId, f64> = HashMap::new();
    let mut add = |track: &TrackId, artist: &str, w: f64| {
        let shown = signals
            .who(artist, Some(track))
            .is_some_and(|a| !signals.hidden.contains(&a));
        if shown {
            *sums.entry(track.clone()).or_default() += w;
        }
    };
    for p in &signals.plays {
        add(&p.track, &p.artist, play_weight(p) * fade(now - p.at));
    }
    for m in &signals.likes {
        add(&m.track, &m.artist, weight::LIKE * fade(now - m.at));
    }
    for k in kept {
        let w = match k.how {
            Keep::Saved => weight::SAVE,
            Keep::Playlisted => PLAYLISTED,
        };
        add(&k.track, &k.artist, w);
    }
    sums.into_iter()
        .filter(|&(_, sum)| sum > 0.0)
        .map(|(track, sum)| (track, saturate(sum, SONG_HALF)))
        .collect()
}

/// An album's score from its songs' (`songs`).
pub fn album(songs: impl IntoIterator<Item = f64>) -> f64 {
    1.0 - songs.into_iter().fold(1.0, |rest, s| rest * (1.0 - s))
}

/// Each artist's score from taste's weight; a hidden artist, and one the listener only skipped, has none.
pub fn artists(taste: &Taste) -> HashMap<ArtistId, f64> {
    taste
        .artists
        .iter()
        .filter(|a| a.weight > 0.0 && !taste.hidden.contains(&a.id))
        .map(|a| (a.id.clone(), saturate(a.weight, ARTIST_HALF)))
        .collect()
}

/// The artists' scores by each catalog id lull holds for them (`ids`, per artist): the only way
/// a search row's artist is reached. The best score wins where two artists hold one id.
pub fn by_catalog_id(
    artists: &HashMap<ArtistId, f64>,
    ids: &HashMap<ArtistId, Vec<CatalogId>>,
) -> HashMap<CatalogId, f64> {
    let mut scores: HashMap<CatalogId, f64> = HashMap::new();
    for (artist, score) in artists {
        for id in ids.get(artist).into_iter().flatten() {
            let best = scores.entry(id.clone()).or_default();
            *best = best.max(*score);
        }
    }
    scores
}

/// A song's or album's own score with its artist's share.
pub fn with_artist(own: f64, artist: f64) -> f64 {
    1.0 - (1.0 - own) * (1.0 - ARTIST_SHARE * artist)
}

/// `x / (x + half)`: 0 at 0, a half at `half`, never 1.
fn saturate(x: f64, half: f64) -> f64 {
    x / (x + half)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::taste::{Mark, Play};
    use std::collections::HashSet;

    const NOW: i64 = 1_800_000_000;
    const DAY: i64 = 86_400;

    fn id(artist: &str, title: &str) -> TrackId {
        TrackId::of(artist, "", title)
    }

    fn play(artist: &str, title: &str, days_ago: i64, completed: bool, skipped: bool) -> Play {
        Play {
            track: id(artist, title),
            artist: artist.into(),
            at: NOW - days_ago * DAY,
            played_s: if completed { 200 } else { 20 },
            duration_s: 200,
            completed,
            skipped,
        }
    }

    fn like(artist: &str, title: &str, days_ago: i64) -> Mark {
        Mark {
            track: id(artist, title),
            artist: artist.into(),
            at: NOW - days_ago * DAY,
        }
    }

    fn score_of(scores: &HashMap<TrackId, f64>, artist: &str, title: &str) -> f64 {
        scores.get(&id(artist, title)).copied().unwrap_or(0.0)
    }

    #[test]
    fn a_like_is_a_half_and_every_further_signal_closes_the_gap_to_one_without_reaching_it() {
        let signals = Signals {
            likes: vec![like("A", "liked", 0), like("A", "loved", 0)],
            plays: vec![play("A", "loved", 0, true, false), play("A", "loved", 0, true, false)],
            ..Default::default()
        };
        let kept = [Kept {
            track: id("A", "loved"),
            artist: "A".into(),
            how: Keep::Saved,
        }];
        let scores = songs(&signals, &kept, NOW);
        assert!((score_of(&scores, "A", "liked") - 0.5).abs() < 1e-9);
        let loved = score_of(&scores, "A", "loved");
        assert!(loved > 0.5 && loved < 1.0, "{loved}");
    }

    #[test]
    fn a_recent_play_counts_more_than_an_old_one_and_a_skip_never_sinks_a_song_below_nothing() {
        let signals = Signals {
            plays: vec![
                play("A", "now", 0, true, false),
                play("A", "then", 45, true, false),
                play("A", "skipped", 0, false, true),
                play("A", "kept then skipped", 0, false, true),
            ],
            likes: vec![like("A", "kept then skipped", 0)],
            ..Default::default()
        };
        let scores = songs(&signals, &[], NOW);
        assert!(score_of(&scores, "A", "now") > score_of(&scores, "A", "then"));
        assert_eq!(
            score_of(&scores, "A", "skipped"),
            0.0,
            "a skipped song has nothing, not less"
        );
        let liked = score_of(&scores, "A", "kept then skipped");
        assert!(
            liked > 0.0 && liked < 0.5,
            "a skip lowers a like's lift but never to a penalty: {liked}"
        );
    }

    #[test]
    fn a_saved_song_and_a_song_in_a_playlist_do_not_fade() {
        let kept = [
            Kept {
                track: id("A", "saved"),
                artist: "A".into(),
                how: Keep::Saved,
            },
            Kept {
                track: id("A", "listed"),
                artist: "A".into(),
                how: Keep::Playlisted,
            },
        ];
        let early = songs(&Signals::default(), &kept, NOW);
        let late = songs(&Signals::default(), &kept, NOW + 900 * DAY);
        assert_eq!(early, late);
        assert!(score_of(&early, "A", "saved") > 0.0 && score_of(&early, "A", "listed") > 0.0);
    }

    #[test]
    fn songs_of_a_hidden_artist_get_nothing() {
        let signals = Signals {
            likes: vec![like("Hidden", "song", 0), like("Shown", "song", 0)],
            hidden: HashSet::from([ArtistId::of("Hidden")]),
            ..Default::default()
        };
        let scores = songs(&signals, &[], NOW);
        assert_eq!(score_of(&scores, "Hidden", "song"), 0.0);
        assert!(score_of(&scores, "Shown", "song") > 0.0);
    }

    #[test]
    fn an_album_scores_from_its_songs_and_each_song_adds() {
        assert_eq!(album([]), 0.0);
        assert!((album([0.5]) - 0.5).abs() < 1e-9);
        assert!(album([0.5, 0.25]) > album([0.5]));
        assert!(album([0.9; 40]) < 1.0 + 1e-12);
    }

    #[test]
    fn an_artist_is_scored_only_through_the_ids_lull_holds_for_them() {
        let signals = Signals {
            followed_artists: vec!["Ra".into(), "Hidden".into()],
            hidden: HashSet::from([ArtistId::of("Hidden")]),
            ..Default::default()
        };
        let taste = Taste::build(&signals, NOW);
        let ra: CatalogId = (Source::Deezer, "27".into());
        let namesake: CatalogId = (Source::Deezer, "999".into());
        let ids = HashMap::from([
            (ArtistId::of("Ra"), vec![ra.clone()]),
            (ArtistId::of("Hidden"), vec![(Source::Itunes, "5".into())]),
        ]);
        let scores = by_catalog_id(&artists(&taste), &ids);
        assert!((scores[&ra] - 0.5).abs() < 1e-9, "a follow is a half");
        assert!(
            !scores.contains_key(&namesake),
            "a namesake shares the name, not the id"
        );
        assert!(
            scores.len() == 1,
            "no hidden artist, and nothing for an artist with no ids: {scores:?}"
        );
        assert!(
            by_catalog_id(&artists(&taste), &HashMap::new()).is_empty(),
            "no ids, no lift"
        );
    }

    #[test]
    fn a_songs_artist_adds_half_of_its_score_and_never_more_than_one() {
        assert_eq!(with_artist(0.0, 0.0), 0.0);
        assert!((with_artist(0.0, 1.0) - 0.5).abs() < 1e-9);
        assert!(with_artist(0.4, 0.8) > 0.4);
        assert!(with_artist(1.0, 1.0) <= 1.0);
    }
}
