//! Is this Soulseek file the song that was asked for? Artist and title are compared loosely,
//! with the normalization catalog merging uses (`ids::loose_*`: feat., version, remaster, edit
//! and live tags dropped), a little spelling slack, and the length when the file reports one
//! (ADR-0008, docs/context/soulseek.md#matching).

use lull_core::ids::{loose_artist, loose_title};
use lull_core::model::Track;

/// Lengths further apart than this are different recordings (an edit, a live take).
const LENGTH_SLACK_S: u32 = 6;
/// One spelling slip allowed per this many letters ("Hoppipola", "Hoppípolla").
const LETTERS_PER_SLIP: usize = 8;
/// Words that make another recording when only one side has them; a slip allowance must never
/// let "Song (Remix)" pass for "Song".
const OTHER_RECORDING: &[&str] = &[
    "remix",
    "mix",
    "rmx",
    "acoustic",
    "instrumental",
    "demo",
    "karaoke",
    "cover",
    "rework",
    "dub",
    "reprise",
    "unplugged",
    "interlude",
];

/// What is asked for: a catalog track, a queue item.
pub struct Wanted {
    artist: String,
    title: String,
    duration: Option<u32>,
}

impl Wanted {
    pub fn of(track: &Track) -> Self {
        Wanted {
            artist: loose_artist(&track.artist),
            title: loose_title(&track.title),
            duration: track.duration.filter(|d| *d > 0),
        }
    }

    /// The first search's words: the artist and the title as written.
    pub fn query(track: &Track) -> String {
        format!(
            "{} {}",
            lull_core::ids::normalize_artist(&track.artist),
            lull_core::ids::normalize(&track.title)
        )
    }

    /// The looser second search, for when the first finds nothing: the title without its tags
    /// and the artist's longest word (peers spell names differently: "Beyonce", "Sigur Ros").
    pub fn looser_query(&self, first: &str) -> Option<String> {
        let word = self
            .artist
            .split(' ')
            .max_by_key(|w| w.chars().count())
            .unwrap_or_default();
        let looser = format!("{} {}", word, self.title).trim().to_owned();
        (looser != first && looser.chars().count() >= 3).then_some(looser)
    }

    /// Whether a file found as `found` (parsed from `path`, `length` long) is this song.
    pub fn matches(&self, found: &Track, length: Option<u32>, path: &str) -> bool {
        self.title_matches(&loose_title(&found.title))
            && self.artist_matches(&found.artist, path)
            && self.length_matches(length)
    }

    fn title_matches(&self, title: &str) -> bool {
        title == self.title
            || (numbers(title) == numbers(&self.title)
                && same_recording_words(title, &self.title)
                && close(title, &self.title))
    }

    /// The credited artist, any one of several credited ("A, B & C"), or, when the path parser
    /// got the artist wrong (a compilation, an odd share), the artist's name in the path itself.
    fn artist_matches(&self, artist: &str, path: &str) -> bool {
        if self.artist.is_empty() {
            return true;
        }
        let whole = loose_artist(artist);
        if whole == self.artist || close(&whole, &self.artist) {
            return true;
        }
        if credits(artist).any(|a| a == self.artist) {
            return true;
        }
        self.artist.chars().count() >= 4 && contains_words(&loose_artist(path), &self.artist)
    }

    fn length_matches(&self, found: Option<u32>) -> bool {
        match (self.duration, found.filter(|d| *d > 0)) {
            (Some(want), Some(got)) => want.abs_diff(got) <= LENGTH_SLACK_S,
            _ => true,
        }
    }
}

/// Each artist of a credit such as "Daft Punk, Pharrell Williams & Nile Rodgers".
fn credits(artist: &str) -> impl Iterator<Item = String> + '_ {
    artist
        .split([',', '&', '/', ';', '+'])
        .flat_map(|part| part.split(" x ").flat_map(|p| p.split(" with ")))
        .map(loose_artist)
        .filter(|a| !a.is_empty())
}

/// The numbers in a name: "Part 1" and "Part 2" are different songs however alike they look.
fn numbers(s: &str) -> Vec<&str> {
    s.split(' ').filter(|w| w.bytes().all(|b| b.is_ascii_digit())).collect()
}

fn contains_words(haystack: &str, needle: &str) -> bool {
    format!(" {haystack} ").contains(&format!(" {needle} "))
}

fn same_recording_words(a: &str, b: &str) -> bool {
    let has = |s: &str, w: &str| s.split(' ').any(|x| x == w);
    OTHER_RECORDING.iter().all(|w| has(a, w) == has(b, w))
}

/// Alike but for spacing ("dont" and "don t") and a slip per `LETTERS_PER_SLIP` letters.
fn close(a: &str, b: &str) -> bool {
    let (a, b) = (a.replace(' ', ""), b.replace(' ', ""));
    let letters = a.chars().count().max(b.chars().count());
    letters > 0 && levenshtein(&a, &b) <= letters / LETTERS_PER_SLIP
}

fn levenshtein(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (diagonal + usize::from(ca != *cb)).min(above + 1).min(row[j] + 1);
            diagonal = above;
        }
    }
    row[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use lull_core::ids::{AlbumId, ArtistId, TrackId};

    fn track(artist: &str, title: &str, duration: Option<u32>) -> Track {
        Track {
            id: TrackId::of(artist, "", title),
            title: title.into(),
            artist_id: ArtistId::of(artist),
            album_id: AlbumId::of(artist, ""),
            artist: artist.into(),
            album: String::new(),
            year: None,
            no: None,
            disc: None,
            duration,
            tags: Vec::new(),
            saved: false,
            file: None,
        }
    }

    fn wanted() -> Wanted {
        Wanted::of(&track("Beyoncé", "Don't Stop the Paper Moon", Some(214)))
    }

    #[test]
    fn the_same_song_matches_through_tags_spelling_and_credits() {
        let w = wanted();
        for (artist, title, path) in [
            ("Beyonce", "Dont Stop the Paper Moon", "x"),
            ("Beyoncé", "Don't Stop the Paper Moon (2011 Remaster)", "x"),
            ("Beyoncé feat. Jay-Z", "Don't Stop the Paper Moon - Radio Edit", "x"),
            ("Beyoncé, Jay-Z", "Don't Stop the Paper Moon", "x"),
            (
                "Various Artists",
                "Don't Stop the Paper Moon",
                "Hits 2003\\Beyonce - Don't Stop.mp3",
            ),
        ] {
            assert!(
                w.matches(&track(artist, title, None), Some(212), path),
                "{artist} / {title}"
            );
        }
        assert!(
            w.matches(&track("Beyonce", "Don't Stop the Paper Moon", None), None, "x"),
            "no length: names decide"
        );
    }

    #[test]
    fn other_songs_and_other_artists_do_not_match() {
        let w = wanted();
        for (artist, title) in [
            ("Beyoncé", "Don't Stop the Paper Moon (Remix)"),
            ("Beyoncé", "Paper Moon"),
            ("Solange", "Don't Stop the Paper Moon"),
            ("Various Artists", "Don't Stop the Paper Moon"),
        ] {
            assert!(
                !w.matches(&track(artist, title, None), Some(214), "x"),
                "{artist} / {title}"
            );
        }
        let part = Wanted::of(&track("Ana Lua", "Symphony of Glass Part 1", None));
        assert!(
            !part.matches(&track("Ana Lua", "Symphony of Glass Part 2", None), None, "x"),
            "numbers must agree"
        );
        let short = Wanted::of(&track("Ana Lua", "Intro", None));
        assert!(!short.matches(&track("Ana Lua", "Outro", None), None, "x"));
    }

    #[test]
    fn a_file_of_another_length_is_another_recording() {
        let w = wanted();
        let found = track("Beyonce", "Don't Stop the Paper Moon", None);
        assert!(w.matches(&found, Some(219), "x"));
        assert!(!w.matches(&found, Some(245), "x"), "an edit or extended take");
        let live = track("Beyonce", "Don't Stop the Paper Moon (Live)", None);
        assert!(
            !w.matches(&live, Some(301), "x"),
            "the live tag is dropped, the length isn't"
        );
    }

    #[test]
    fn the_second_query_is_looser_than_the_first() {
        let t = track("Sigur Rós", "Hoppípolla (Remastered)", None);
        let first = Wanted::query(&t);
        assert_eq!(Wanted::of(&t).looser_query(&first).as_deref(), Some("sigur hoppipolla"));
    }
}
