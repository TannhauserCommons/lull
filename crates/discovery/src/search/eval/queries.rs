//! The words a listener types for each song or artist they want: the whole title, artist and
//! title in either order, and the first characters of them while still typing.

use std::collections::HashSet;

use super::listening::{Listening, first_credit};

/// The targets the eval measures: the listener's heaviest songs and artists. Each song is a dozen
/// queries and iTunes allows about 9 queries a minute, so recording more costs hours (record.rs).
const SONGS: usize = 60;
const ARTISTS: usize = 25;
/// Characters typed so far, for a title or for artist and title; for an artist's name.
const TITLE_PREFIXES: [usize; 4] = [2, 3, 5, 8];
const ARTIST_PREFIXES: [usize; 2] = [2, 3];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Form {
    Title,
    ArtistTitle,
    TitleArtist,
    TitlePrefix(usize),
    ArtistTitlePrefix(usize),
    ArtistName,
    ArtistPrefix(usize),
}

impl Form {
    pub fn label(self) -> String {
        match self {
            Form::Title => "title".into(),
            Form::ArtistTitle => "artist title".into(),
            Form::TitleArtist => "title artist".into(),
            Form::TitlePrefix(n) => format!("title, first {n}"),
            Form::ArtistTitlePrefix(n) => format!("artist title, first {n}"),
            Form::ArtistName => "name".into(),
            Form::ArtistPrefix(n) => format!("name, first {n}"),
        }
    }
}

/// Who is wanted: a position in `Listening::songs` or `Listening::artists`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Song(usize),
    Artist(usize),
}

#[derive(Debug, Clone)]
pub struct Case {
    pub query: String,
    pub form: Form,
    pub target: Target,
}

/// Every case, in the order they are recorded: the artists, then the songs, heaviest first, so a
/// recording stopped half way has whole targets.
pub fn plan(listening: &Listening) -> Vec<Case> {
    let artists = listening.artists.iter().take(ARTISTS).enumerate();
    let songs = listening.songs.iter().take(SONGS).enumerate();
    let mut cases = Vec::new();
    for (i, artist) in artists {
        let name = typed(&artist.name);
        let mut forms = vec![(Form::ArtistName, name.clone())];
        forms.extend(ARTIST_PREFIXES.map(|n| (Form::ArtistPrefix(n), first(&name, n))));
        cases.extend(distinct(Target::Artist(i), forms));
    }
    for (i, song) in songs {
        let (title, artist) = (sung(&song.title), typed(first_credit(&song.artist)));
        let both = format!("{artist} {title}");
        let mut forms = vec![
            (Form::Title, title.clone()),
            (Form::ArtistTitle, both.clone()),
            (Form::TitleArtist, format!("{title} {artist}")),
        ];
        forms.extend(TITLE_PREFIXES.map(|n| (Form::TitlePrefix(n), first(&title, n))));
        forms.extend(TITLE_PREFIXES.map(|n| (Form::ArtistTitlePrefix(n), first(&both, n))));
        cases.extend(distinct(Target::Song(i), forms));
    }
    cases
}

/// The queries of the cases, each once, in order.
pub fn queries(cases: &[Case]) -> Vec<String> {
    let mut seen = HashSet::new();
    cases
        .iter()
        .filter(|c| seen.insert(c.query.as_str()))
        .map(|c| c.query.clone())
        .collect()
}

/// A target's cases, each query once: a title of three letters is its own 3, 5 and 8 characters.
fn distinct(target: Target, forms: Vec<(Form, String)>) -> impl Iterator<Item = Case> {
    let mut seen = HashSet::new();
    forms
        .into_iter()
        .filter(move |(_, query)| !query.is_empty() && seen.insert(query.clone()))
        .map(move |(form, query)| Case { query, form, target })
}

/// Lowercase, single spaces: how search keys a query.
fn typed(s: &str) -> String {
    s.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The first `n` characters, as typed so far, when that is less than the whole.
fn first(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return String::new();
    }
    typed(&s.chars().take(n).collect::<String>())
}

/// The title as a listener says it: no "(Remastered 2015)", "[feat. X]" or " - Live" tail, which
/// they don't type for the song they have in mind.
fn sung(title: &str) -> String {
    let mut bare = String::new();
    let mut depth = 0usize;
    for c in title.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            _ if depth == 0 => bare.push(c),
            _ => {}
        }
    }
    let head = bare.split(" - ").next().unwrap_or(&bare);
    match typed(head) {
        s if s.is_empty() => typed(title),
        s => s,
    }
}

#[test]
fn prefixes_are_whole_characters_and_titles_are_said_without_their_tags() {
    assert_eq!(first("米津玄師 lemon", 3), "米津玄");
    assert_eq!(first("ab", 2), "", "the whole is the full query, not a prefix");
    assert_eq!(sung("Creep (Remastered 2009) - Live"), "creep");
    assert_eq!(sung("(Intro)"), "(intro)");
}
