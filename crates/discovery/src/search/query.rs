//! Word matching: how well a row answers what was typed, and the version words that make a title
//! another take of a song (ADR-0008). The one matcher search and the listener's own index share.
//!
//! - **Fit:** exact, then every word typed, then most of them (a soundtrack named without the
//!   "season" the listener typed), then one of them, then loose; accents folded and typos
//!   forgiven ("beyonse halo"). Words found only in a "(from …)" note count as most: such a song
//!   is from the work the query names, not that work.

use lull_core::ids::{loose_artist, loose_title, normalize};

use super::merge::{AlbumHit, ArtistHit, SongHit};

/// Words shorter than this (but not numbers) don't count toward "most of the words": "of" and
/// "the" make "Sign of the Times" no answer to "skronk of the times".
const SIGNIFICANT_LETTERS: usize = 4;
/// A typed word may hold one typo in this many letters (and is this long at least); the whole
/// query one in this many, so a short word never passes for another.
const MISSPELT_WORD_LETTERS: usize = 5;
const MISSPELT_PHRASE_LETTERS: usize = 8;
/// Words in a bracket or " - " suffix that make a title another performance or mix of the song
/// it names ("(Live)", "[Daft Punk Remix]", "(Acoustic)", "- Radio Edit", "[Midland Version]",
/// "(Originally Performed By Adele)", "(Lullaby Rendition of Queen)"). Remasters and "feat."
/// are the same recording (`ids::loose_title` drops them). Search and radio read this one list.
const VERSION_TAGS: &[&str] = &[
    "live",
    "remix",
    "remixes",
    "mashup",
    "remixed",
    "rmx",
    "mix",
    "dub",
    "edit",
    "acoustic",
    "unplugged",
    "instrumental",
    "demo",
    "karaoke",
    "cover",
    "performed",
    "tribute",
    "rendition",
    "lullaby",
    "acapella",
    "cappella",
    "reprise",
    "rework",
    "slowed",
    "sped",
    "lofi",
    "take",
    "rehearsal",
    "version",
];

/// How well a row answers the query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Fit {
    /// The source found it (a typo, an alias, a name in another script), but the words differ.
    Loose,
    /// One of the words that count (four letters or more, or a number) starts a word of it:
    /// "lemon" of "米津玄師 lemon", whose artist is written in another script.
    Partly,
    /// At least two of the typed words, and two thirds of them (not counting short ones like
    /// "of"), start a word of its artist, title or album; or every word does, but only thanks
    /// to a "(from …)" note.
    Most,
    /// Every word typed starts a word of its artist, title or album (or is one with a typo).
    Words,
    /// The title or name is the query (or it with a typo), or the artist is (an artist's songs
    /// and albums), or artist and title together are; or the title begins with the words typed
    /// ("smells like teen"), or they are an artist's name and words of the title ("nirvana
    /// unplugged").
    Exact,
}

/// The query, in the forms rows are compared with.
pub struct Query {
    /// `loose_title` of the query, and the same without a leading "the".
    loose: [String; 2],
    /// The words typed, accents folded.
    words: Vec<String>,
}

impl Query {
    pub fn new(q: &str) -> Self {
        let loose = loose_title(q);
        let bare = loose.strip_prefix("the ").unwrap_or(&loose).to_owned();
        Query {
            words: folded_words(q),
            loose: [loose, bare],
        }
    }

    /// The query is `s`, a leading "the" aside on either side ("dark side of the moon" is
    /// "The Dark Side of the Moon"), or a typo away from it ("radiohed creep"), `s` cut short
    /// included ("halleluja"): it ranks as `s`.
    fn is(&self, s: &str) -> bool {
        self.spells(s, true)
    }

    /// As `is`, but never `s` cut short: the letters may be a longer name still being typed, so
    /// they don't name `s` outright.
    fn names(&self, s: &str) -> bool {
        self.spells(s, false)
    }

    fn spells(&self, s: &str, cut_short: bool) -> bool {
        let bare = s.strip_prefix("the ").unwrap_or(s);
        self.loose.iter().any(|l| {
            [s, bare]
                .iter()
                .any(|f| l == f || (misspelt_phrase(l, f) && (cut_short || !f.starts_with(l.as_str()))))
        })
    }

    /// `s` begins with the whole words typed: a title being typed ("smells like teen", "stairway
    /// to"). Only for two words or more, as one word starts too many titles.
    fn begins(&self, s: &str) -> bool {
        self.words.len() >= 2
            && self.loose.iter().any(|l| {
                s.strip_prefix(l.as_str())
                    .or_else(|| s.strip_prefix("the ").and_then(|b| b.strip_prefix(l.as_str())))
                    .is_some_and(|rest| rest.starts_with(' '))
            })
    }

    /// One word typed: the listener is still at the start of a name or a title. A second word is
    /// more than a start, and `begins` already reads two words as a title being typed.
    pub fn is_short(&self) -> bool {
        self.words.len() == 1
    }

    /// The words typed, accents folded.
    pub fn words(&self) -> &[String] {
        &self.words
    }

    /// Every word typed starts one of `have` (folded words), or is one with a typo: what "all the
    /// words" means for a row, and for any text that is searched with the same rule.
    pub fn holds_every_word(&self, have: &[String]) -> bool {
        !self.words.is_empty() && self.words.iter().all(|w| holds(have, w))
    }

    /// `exact`: the forms that fit exactly when they are the words typed; `titled`: the forms
    /// that also fit exactly when they begin with them.
    fn fit(&self, exact: &[String], titled: &[String], text: &[&str]) -> Fit {
        if exact.iter().any(|e| self.is(e)) || titled.iter().any(|t| self.is(t) || self.begins(t)) {
            return Fit::Exact;
        }
        let own: Vec<String> = text.iter().flat_map(|t| folded_words(&without_from_notes(t))).collect();
        let all: Vec<String> = text.iter().flat_map(|t| folded_words(t)).collect();
        if self.words.is_empty() {
            return Fit::Loose;
        }
        if self.holds_every_word(&own) {
            return Fit::Words;
        }
        let significant: Vec<&String> = self.words.iter().filter(|w| is_significant(w)).collect();
        let held = significant.iter().filter(|w| holds(&all, w)).count();
        if self.holds_every_word(&all) || (held >= 2 && held * 3 >= significant.len() * 2) {
            Fit::Most
        } else if held >= 1 {
            Fit::Partly
        } else {
            Fit::Loose
        }
    }

    /// One word typed names this artist outright (`names`), not just a start of the name.
    pub fn names_artist(&self, a: &ArtistHit) -> bool {
        self.names(&loose_artist(&a.name))
    }

    /// One word typed names this song outright (`names`): its title or its artist's name, not
    /// just a start of one.
    pub fn names_song(&self, s: &SongHit) -> bool {
        let (exact, titled) = forms(&credited(&s.artist), &s.title);
        exact.iter().chain(&titled).any(|f| self.names(f))
    }

    pub fn song(&self, s: &SongHit) -> Fit {
        self.named(&s.artist, &s.title, &[&s.artist, &s.title, &s.album])
    }

    pub fn album(&self, a: &AlbumHit) -> Fit {
        self.named(&a.artist, &a.title, &[&a.artist, &a.title])
    }

    /// A song's or album's fit: exact also when the words are a credited artist's name and words
    /// of the title ("nirvana unplugged" is Nirvana's "MTV Unplugged In New York").
    fn named(&self, artist: &str, title: &str, text: &[&str]) -> Fit {
        let names = credited(artist);
        let (exact, titled) = forms(&names, title);
        if self.artist_and_title_words(&names, title) {
            Fit::Exact
        } else {
            self.fit(&exact, &titled, text)
        }
    }

    /// The words are a song's credited artist and words of its title ("lose yourself eminem"):
    /// they name that song.
    pub fn names_artist_and_title(&self, s: &SongHit) -> bool {
        self.artist_and_title_words(&credited(&s.artist), &s.title)
    }

    fn artist_and_title_words(&self, names: &[String], title: &str) -> bool {
        let title_words = folded_words(title);
        names.iter().filter(|n| !n.is_empty()).any(|n| {
            self.loose.iter().any(|l| {
                let rest = l
                    .strip_prefix(n.as_str())
                    .and_then(|r| r.strip_prefix(' '))
                    .or_else(|| l.strip_suffix(n.as_str()).and_then(|r| r.strip_suffix(' ')));
                rest.is_some_and(|r| {
                    folded_words(r).iter().all(|w| {
                        title_words
                            .iter()
                            .any(|h| h.starts_with(w.as_str()) || misspelt_word(w, h))
                    })
                })
            })
        })
    }

    pub fn artist(&self, a: &ArtistHit) -> Fit {
        self.fit(&[loose_artist(&a.name)], &[], &[&a.name])
    }

    /// The words (of a title or a release, `version_words`) make another version than the
    /// words typed name: a live take, a remix, an acoustic or instrumental one, a cover's
    /// "(Originally Performed By …)", unless the listener typed that word ("hotel california
    /// live").
    pub(super) fn other_version(&self, words: &[String]) -> bool {
        let typed = |w: &String| {
            self.words.contains(w)
                || (self.words.iter().any(|t| t == "unplugged") && matches!(w.as_str(), "live" | "acoustic"))
        };
        words.iter().any(|w| !typed(w))
    }
}

/// The forms of a song or an album that are what someone types for it: each credited artist
/// (`names`: "Daft Punk, Pharrell Williams & Nile Rodgers" is also Daft Punk's) with the title
/// either side, the artist alone (their songs and albums), the title alone; titles also without
/// the tags of another version, as the version is ranked apart (`Score::version`). The second
/// list holds the forms that start with the title, which fit when they begin with the words.
/// The credited artists' names (`radio::credited_names`), as names are compared.
fn credited(artist: &str) -> Vec<String> {
    crate::radio::credited_names(artist)
        .iter()
        .map(|n| loose_artist(n))
        .collect()
}

fn forms(names: &[String], title: &str) -> (Vec<String>, Vec<String>) {
    let full = loose_title(title);
    let plain = loose_title(&without_versions(title).0);
    let titles: Vec<&String> = if plain == full {
        vec![&full]
    } else {
        vec![&full, &plain]
    };
    let mut exact = Vec::new();
    let mut titled: Vec<String> = titles.iter().map(|t| (*t).clone()).collect();
    for name in names {
        for t in &titles {
            titled.push(format!("{name} {t}"));
            exact.push(format!("{t} {name}"));
        }
        exact.push(name.clone());
    }
    (exact, titled)
}

/// A typed word starts one of `have`, or is one with a typo.
fn holds(have: &[String], word: &str) -> bool {
    have.iter().any(|h| word_fits(word, h))
}

/// A typed word starts `have`, or is it with a typo: the one rule of "holds", for a single word.
pub fn word_fits(typed: &str, have: &str) -> bool {
    have.starts_with(typed) || misspelt_word(typed, have)
}

/// A word that counts toward "most of the words": a number, or four letters or more.
fn is_significant(word: &str) -> bool {
    word.chars().count() >= SIGNIFICANT_LETTERS || word.chars().all(|c| c.is_numeric())
}

/// The words of `s`, accents folded ("Beyoncé" holds "beyonce").
pub(super) fn folded_words(s: &str) -> Vec<String> {
    normalize(s)
        .split(' ')
        .map(loose_title)
        .filter(|w| !w.is_empty())
        .collect()
}

/// A word typed with a typo of `word` ("beyonse", "fleetwod", "elish"): one edit in five
/// letters, two at most, and never for words under five letters ("halo" isn't "hall").
fn misspelt_word(typed: &str, word: &str) -> bool {
    let len = typed.chars().count();
    len >= MISSPELT_WORD_LETTERS && {
        let d = edits(typed, word);
        d > 0 && d <= 2 && d * MISSPELT_WORD_LETTERS <= len.max(word.chars().count())
    }
}

/// What was typed is `s` with a typo ("bohemian rapsody", "radiohed creep"): one edit in eight
/// letters, two at most, so short words never pass for others ("creep" isn't "creed").
fn misspelt_phrase(typed: &str, s: &str) -> bool {
    let d = edits(typed, s);
    d > 0 && d <= 2 && d * MISSPELT_PHRASE_LETTERS <= typed.chars().count()
}

/// The words that make a title another version of the song it names: the version tags of its
/// brackets and " - " suffix ("Talamak (Tom Lown Remix)", "So Many Details - Live at KEXP"),
/// never its own words ("Live and Let Die", "Dub Be Good to Me").
pub(super) fn version_words(title: &str) -> Vec<String> {
    without_versions(title).1
}

/// Whether a title names another take of a song (`version_words`).
pub fn names_a_version(title: &str) -> bool {
    !version_words(title).is_empty()
}

/// The words that make a release another version of its songs: its tags, and "live" for a
/// concert ("Live at Reading", "Live At SoFi Stadium").
pub(super) fn release_versions(release: &str) -> Vec<String> {
    let mut words = version_words(release);
    if normalize(release).starts_with("live ") {
        words.push("live".into());
    }
    words
}

/// The text without its brackets and " - " suffixes that name another version of the song, and
/// the version words they held: "Blinding Lights (Live)" is ("Blinding Lights", ["live"]).
pub(super) fn without_versions(s: &str) -> (String, Vec<String>) {
    let mut found = Vec::new();
    let mut names_version = |part: &str| {
        let before = found.len();
        found.extend(
            normalize(part)
                .split(' ')
                .filter(|w| VERSION_TAGS.contains(w))
                .map(str::to_owned),
        );
        found.len() > before
    };
    let s = s.replace(" – ", " - ").replace(" — ", " - ");
    let mut kept = String::with_capacity(s.len());
    let mut rest = s.as_str();
    while let Some(open) = rest.find(['(', '[', '{']) {
        let close = match rest.as_bytes()[open] {
            b'(' => ')',
            b'[' => ']',
            _ => '}',
        };
        let Some(len) = rest[open + 1..].find(close) else {
            break;
        };
        kept.push_str(&rest[..open]);
        if !names_version(&rest[open + 1..open + 1 + len]) {
            kept.push_str(&rest[open..open + len + 2]);
        }
        rest = &rest[open + len + 2..];
    }
    kept.push_str(rest);
    let mut parts = kept.split(" - ");
    let mut plain = parts.next().unwrap_or_default().to_owned();
    for part in parts {
        if names_version(part) {
            break;
        }
        plain.push_str(" - ");
        plain.push_str(part);
    }
    (plain, found)
}

/// The text without brackets that say where a song is from: "You Can't Run From Me (from
/// Cyberpunk: Edgerunners Season 2)" → "You Can't Run From Me ".
fn without_from_notes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(open) = rest.find(['(', '[']) {
        let close = if rest[open..].starts_with('(') { ')' } else { ']' };
        let Some(len) = rest[open + 1..].find(close) else { break };
        let inner = rest[open + 1..open + 1 + len].trim_start_matches(|c: char| !c.is_alphanumeric());
        let is_note = inner.get(..5).is_some_and(|w| w.eq_ignore_ascii_case("from "));
        out.push_str(&rest[..open]);
        if !is_note {
            out.push_str(&rest[open..open + len + 2]);
        }
        rest = &rest[open + len + 2..];
    }
    out.push_str(rest);
    out
}
/// Edits between two strings, a swap of neighbours counting as one ("indain", "indian").
/// Lengths more than two apart count as three, as no rule allows more.
pub(super) fn edits(a: &str, b: &str) -> usize {
    // Most pairs are far apart in length: say so before any allocation (an index of 50,000 songs asks).
    if a.chars().count().abs_diff(b.chars().count()) > 2 {
        return 3;
    }
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    // Rows i-2, i-1 and i of the table.
    let mut before: Vec<usize> = vec![0; b.len() + 1];
    let mut last: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut row = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            row[j] = (last[j - 1] + cost).min(last[j] + 1).min(row[j - 1] + 1);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                row[j] = row[j].min(before[j - 2] + 1);
            }
        }
        before = std::mem::replace(&mut last, row);
    }
    last[b.len()]
}
