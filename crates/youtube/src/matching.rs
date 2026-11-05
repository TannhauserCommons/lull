//! Is this YouTube upload the song asked for, and how good a copy is it? Official audio (YouTube
//! Music songs, Topic channels, the artist's own channel) ranks first; the length must agree; live
//! takes, covers, sped-up, slowed and nightcore versions are refused unless the song is one
//! (docs/context/youtube.md#matching).

use std::collections::HashMap;

use lull_core::ids::{loose_artist, loose_title, normalize};
use lull_core::model::Track;
use lull_core::youtube::{FoundSong, Match};

/// Words that make another recording when the upload has them and the song doesn't. Each entry
/// is a run of words in the upload's normalized title.
const OTHER_VERSION: &[&[&str]] = &[
    &["live"],
    &["cover"],
    &["covers"],
    &["covered"],
    &["nightcore"],
    &["sped", "up"],
    &["speed", "up"],
    &["spedup"],
    &["slowed"],
    &["reverb"],
    &["8d"],
    &["karaoke"],
    &["instrumental"],
    &["remix"],
    &["acoustic"],
    &["unplugged"],
    &["demo"],
    &["reaction"],
    &["tutorial"],
    &["lesson"],
    &["piano"],
    &["guitar"],
    &["drum"],
    &["bass", "boosted"],
    &["mashup"],
    &["parody"],
    &["loop"],
    &["hour"],
    &["hours"],
    &["concert"],
];

/// Bracketed parts of an upload's title that say nothing about the recording: "(Official
/// Audio)", "[HD]", "(Lyric Video)", a translation's "(Sub.Español)" or "(Letra)". A bracket
/// with any of these words is dropped before the title is compared.
const PACKAGING: &[&str] = &[
    "official",
    "audio",
    "video",
    "lyric",
    "lyrics",
    "visualizer",
    "visualiser",
    "hd",
    "hq",
    "4k",
    "mv",
    "clip",
    "explicit",
    "remastered",
    "remaster",
    "color",
    "coded",
    "full",
    "sub",
    "subs",
    "letra",
    "legendado",
];

/// One spelling slip allowed per this many letters, as soulseek's matching allows.
const LETTERS_PER_SLIP: usize = 8;

/// What is asked for.
pub struct Wanted {
    artist: String,
    title: String,
    /// Every word of the song's artist, title and album, so "Live Forever", a live album's songs
    /// or Nightcore's own songs may be "live" or "nightcore" uploads.
    words: Vec<String>,
    duration: Option<u32>,
}

impl Wanted {
    pub fn of(track: &Track) -> Self {
        let words = normalize(&format!("{} {} {}", track.artist, track.title, track.album))
            .split(' ')
            .map(str::to_owned)
            .collect();
        Wanted {
            artist: loose_artist(&track.artist),
            title: loose_title(&track.title),
            words,
            duration: track.duration.filter(|d| *d > 0),
        }
    }

    /// A song by its artist, title and length alone (an upload's reading). Its words are the
    /// title's without version tags, so a reading "georgia (instagram live)" is no live song.
    pub fn named(artist: &str, title: &str, duration: Option<u32>) -> Self {
        let title = loose_title(title);
        Wanted {
            artist: loose_artist(artist),
            words: normalize(&format!("{artist} {title}"))
                .split(' ')
                .map(str::to_owned)
                .collect(),
            title,
            duration: duration.filter(|d| *d > 0),
        }
    }

    /// The words to search YouTube's videos for.
    pub fn query(track: &Track) -> String {
        format!("{} {}", track.artist, track.title)
    }

    /// The words to search YouTube Music's songs for: the artist's name quoted, so it is read as
    /// one name. Unquoted, "Germany Germany LENA" is the country and the singer Lena, and the
    /// song is not among the songs playback looks at. Quoted, 46 of 47 songs were found where
    /// playback looks, against 45 (2026-11-05; the miss is on YouTube Music under no name).
    /// Videos keep the plain words: fans spell names their own way, and quoting there was no
    /// better.
    pub fn music_query(track: &Track) -> String {
        format!("\"{}\" {}", track.artist.replace('"', ""), track.title)
    }
}

/// One upload, from a search result or a full extraction.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Candidate {
    pub id: String,
    /// As YouTube shows it: "Radiohead - Weird Fishes / Arpeggi (Official Audio)".
    pub title: String,
    pub channel: String,
    pub duration: Option<u32>,
    /// YouTube Music lists it among songs: an art track.
    pub music: bool,
    /// The music metadata YouTube attaches to official audio (from a full extraction).
    pub track: Option<String>,
    pub artists: Vec<String>,
    /// A recorded live stream.
    pub live: bool,
}

impl Candidate {
    /// From YouTube Music, a Topic channel, VEVO, music metadata, or the artist's own channel.
    pub fn official(&self, wanted: &Wanted) -> bool {
        let channel = self.channel.to_ascii_lowercase();
        self.music
            || (self.track.is_some() && !self.artists.is_empty())
            || channel.ends_with(" - topic")
            || channel.ends_with("vevo")
            || loose_artist(&self.channel) == wanted.artist
    }
}

/// The score of an upload that is the song (higher is better), or `None` when it isn't the song
/// or isn't allowed by `strictness`.
pub fn judge(wanted: &Wanted, c: &Candidate, strictness: Match) -> Option<i32> {
    if another_version(wanted, c) {
        return None;
    }
    let official = c.official(wanted);
    if strictness == Match::Official && !official {
        return None;
    }
    let (title, artist_part) = match &c.track {
        Some(track) => (loose_title(track), split(&c.title).0),
        None => reading(wanted, split(&c.title)),
    };
    if !same_title(&title, &wanted.title) {
        return None;
    }
    if !artist_agrees(wanted, c, artist_part.as_deref()) {
        return None;
    }
    let off = match (wanted.duration, c.duration) {
        (Some(want), Some(have)) => want.abs_diff(have),
        _ => 0,
    };
    if off > slack_s(strictness) {
        return None;
    }
    let raw = normalize(&c.title);
    let mut score = 100 - 3 * off as i32;
    if official {
        score += 40;
    }
    if c.music {
        score += 10;
    }
    if title == wanted.title {
        score += 5;
    }
    // A music video may carry an intro or a skit; official audio never does.
    if has_run(&raw, &["video"]) && !has_run(&raw, &["audio"]) {
        score -= 10;
    }
    Some(score)
}

/// The upload's title and artist part, read "Artist - Title"; or a fan's "Title - Artist"
/// ("mount airy, georgia - clairo") when only that reading names the song and its artist.
fn reading(wanted: &Wanted, (artist_part, title_part): (Option<String>, String)) -> (String, Option<String>) {
    let title = loose_title(&title_part);
    match artist_part {
        Some(first)
            if !same_title(&title, &wanted.title)
                && same_title(&loose_title(&first), &wanted.title)
                && loose_artist(&title_part) == wanted.artist =>
        {
            (loose_title(&first), Some(title_part))
        }
        artist_part => (title, artist_part),
    }
}

/// The title alone, for YouTube Music's song results, which name no artist or length until the
/// upload is extracted.
pub fn title_matches(wanted: &Wanted, raw_title: &str) -> bool {
    let c = Candidate {
        title: raw_title.to_owned(),
        ..Candidate::default()
    };
    !another_version(wanted, &c)
        && (same_title(&loose_title(&split(raw_title).1), &wanted.title)
            || same_title(&loose_title(raw_title), &wanted.title))
}

/// How far the upload's length may be from the song's.
fn slack_s(strictness: Match) -> u32 {
    match strictness {
        Match::Official | Match::Close => 5,
        Match::Loose => 15,
    }
}

fn another_version(wanted: &Wanted, c: &Candidate) -> bool {
    let raw: Vec<String> = normalize(&c.title).split(' ').map(str::to_owned).collect();
    let song_has = |run: &[&str]| run.iter().all(|w| wanted.words.iter().any(|x| x == w));
    let upload_has = |run: &[&str]| {
        raw.windows(run.len())
            .any(|win| win.iter().zip(run).all(|(a, b)| a == b))
    };
    (c.live && !song_has(&["live"])) || OTHER_VERSION.iter().any(|run| upload_has(run) && !song_has(run))
}

/// The one reading of an upload's title, for matching here and for playlists lull lists
/// (collection's link imports and outside playlists): "Artist - Title (Official Audio)" as the
/// artist part (at the first dash, or SoundCloud's "•") and the title, with
/// - packaging brackets dropped ("(Official Video)", "[HD]"), and quality tags at the end
///   outside them ("HD 1080p");
/// - a leading "<show> Soundtrack - " dropped (fan playlists of a series' songs);
/// - whatever follows " | " dropped: a channel or a show ("Song | Band", "Song | Live at X"),
///   which comes after the title;
/// - the artist said again before the title dropped (`Dominic Fike - Dominic Fike "Babydoll"`),
///   and quotes around the whole title.
pub fn split(raw: &str) -> (Option<String>, String) {
    let cleaned = drop_packaging(raw);
    let cleaned = cleaned.split(" | ").next().unwrap_or(&cleaned);
    let cleaned = without_work(&without_quality_tags(cleaned));
    let parts = [" - ", " – ", " — ", " -- ", " • "]
        .iter()
        .filter_map(|sep| cleaned.split_once(sep))
        .min_by_key(|(a, _)| a.len());
    match parts.map(|(a, t)| (a.trim(), t.trim())) {
        Some((a, t)) if !a.is_empty() && !t.is_empty() => (Some(a.to_owned()), unquoted(without_artist(t, a))),
        _ => (None, unquoted(cleaned.trim())),
    }
}

/// An upload as a song: its artist and title (the site's music metadata when it has some, a
/// Topic channel's name, the title's artist part, else the channel's name), and whether its
/// length is the song's (official audio). `None` when it names no song.
pub fn read_upload(
    title: &str,
    channel: &str,
    music_artist: Option<&str>,
    music_track: Option<&str>,
) -> Option<(String, String, bool)> {
    let (artist, song, official) = match (music_artist, music_track) {
        (Some(a), Some(t)) if t != title && !a.trim().is_empty() => (a.trim().to_owned(), split(t).1, true),
        _ => match channel.trim().strip_suffix(" - Topic") {
            Some(artist) => (artist.to_owned(), split(title).1, true),
            None => match split(title) {
                (Some(a), t) => (a, t, false),
                (None, t) => (channel_name(channel), t, false),
            },
        },
    };
    (!artist.is_empty() && !song.is_empty()).then_some((artist, song, official))
}

/// The songs among a video search's uploads, for search's last gap-filler: each upload read as a
/// song (`read_upload`), or as a fan's "Title - Artist" when the other uploads put its title part
/// first more often ("mount airy, georgia - clairo" beside "Clairo - mount airy, georgia" and
/// "clairo - wine"; playback tells the two apart by the song it wants, a search by the other
/// uploads), and kept
/// only when `judge` takes the upload for the song it reads as: what search offers, playback
/// plays, and a live take, a cover or a sped-up upload is no song here as it is none there. One
/// song per reading, in YouTube's order.
pub fn songs_of(found: &[Candidate]) -> Vec<FoundSong> {
    let reads: Vec<Option<(String, String, bool)>> = found
        .iter()
        .map(|c| {
            read_upload(
                &c.title,
                &c.channel,
                c.artists.first().map(String::as_str),
                c.track.as_deref(),
            )
        })
        .collect();
    let key = |artist: &str, title: &str| (loose_artist(artist), loose_title(title));
    // How often each name stands as an upload's artist: the name more uploads put first is the
    // artist ("clairo" in seven, "mount airy, georgia" in two).
    let mut as_artist: HashMap<String, usize> = HashMap::new();
    for (artist, _, _) in reads.iter().flatten() {
        *as_artist.entry(loose_artist(artist)).or_default() += 1;
    }
    let artist_count = |name: &str| as_artist.get(&loose_artist(name)).copied().unwrap_or(0);
    let mut songs: Vec<FoundSong> = Vec::new();
    for (c, read) in found.iter().zip(reads) {
        let Some((artist, title, official)) = read else {
            continue;
        };
        let reversed = !official && artist_count(&title) > artist_count(&artist);
        let (artist, title) = if reversed { (title, artist) } else { (artist, title) };
        if judge(&Wanted::named(&artist, &title, c.duration), c, Match::Close).is_none() {
            continue;
        }
        if songs.iter().any(|s| key(&s.artist, &s.title) == key(&artist, &title)) {
            continue;
        }
        songs.push(FoundSong {
            artist,
            title,
            duration: c.duration,
            video: c.id.clone(),
            official,
        });
    }
    songs
}

/// "Angel HD 1080p": quality tags at the end, outside brackets, dropped (not the whole title).
fn without_quality_tags(s: &str) -> String {
    let mut words: Vec<&str> = s.split_whitespace().collect();
    while words.len() > 1
        && words.last().is_some_and(|w| {
            matches!(
                w.to_ascii_lowercase().as_str(),
                "hd" | "hq" | "4k" | "1080p" | "720p" | "480p" | "60fps"
            )
        })
    {
        words.pop();
    }
    words.join(" ")
}

/// "Person Of Interest Soundtrack - Massive Attack - Live With Me" without the work it is from.
fn without_work(s: &str) -> String {
    match s.split_once(" - ") {
        Some((work, rest)) if normalize(work).split(' ').any(|w| matches!(w, "soundtrack" | "ost")) => {
            rest.trim().to_owned()
        }
        _ => s.to_owned(),
    }
}

fn without_artist(title: &str, artist: &str) -> String {
    match title.get(..artist.len()) {
        Some(head) if head.eq_ignore_ascii_case(artist) && title[artist.len()..].starts_with(' ') => {
            title[artist.len()..].trim().to_owned()
        }
        _ => title.to_owned(),
    }
}

/// A title quoted whole ("\"Song\""), not an apostrophe at its end ("Breezin'").
fn unquoted(s: impl AsRef<str>) -> String {
    let s = s.as_ref().trim();
    let quotes = ['"', '“', '”'];
    match s.strip_prefix(quotes).and_then(|o| o.strip_suffix(quotes)) {
        Some(inner) if !inner.trim().is_empty() => inner.trim().to_owned(),
        _ => s.to_owned(),
    }
}

fn drop_packaging(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(open) = rest.find(['(', '[']) {
        let close = if rest.as_bytes()[open] == b'(' { ')' } else { ']' };
        let Some(len) = rest[open + 1..].find(close) else { break };
        let inner = normalize(&rest[open + 1..open + 1 + len]);
        out.push_str(&rest[..open]);
        if !inner.split(' ').any(|w| PACKAGING.contains(&w)) {
            out.push_str(&rest[open..open + len + 2]);
        }
        rest = &rest[open + len + 2..];
    }
    out.push_str(rest);
    out
}

fn artist_agrees(wanted: &Wanted, c: &Candidate, artist_part: Option<&str>) -> bool {
    if wanted.artist.is_empty() {
        return true;
    }
    let names = c
        .artists
        .iter()
        .map(String::as_str)
        .chain(artist_part)
        .chain([channel_name(&c.channel).as_str()])
        .map(loose_artist)
        .collect::<Vec<_>>();
    names.iter().any(|n| contains_words(n, &wanted.artist) || contains_words(&wanted.artist, n) && !n.is_empty())
        // A compilation or a lyric channel: the artist named in the title itself.
        || contains_words(&loose_title(&c.title), &wanted.artist)
}

/// "Radiohead - Topic", "RadioheadVEVO", "Radiohead Official", "Radiohead Music": the name alone.
fn channel_name(channel: &str) -> String {
    let mut c = channel.trim();
    c = c.strip_suffix(" - Topic").unwrap_or(c);
    for suffix in ["VEVO", "Vevo", " Official", " official", " Music"] {
        if let Some(rest) = c.strip_suffix(suffix).filter(|r| !r.trim().is_empty()) {
            c = rest.trim();
        }
    }
    c.to_owned()
}

/// Whether `needle`'s words appear in `hay` as a run.
fn contains_words(hay: &str, needle: &str) -> bool {
    let (h, n): (Vec<&str>, Vec<&str>) = (hay.split(' ').collect(), needle.split(' ').collect());
    !needle.is_empty() && h.windows(n.len()).any(|w| w == n.as_slice())
}

fn has_run(normalized: &str, run: &[&str]) -> bool {
    contains_words(normalized, &run.join(" "))
}

/// Equal, or a slip or two apart with the same numbers ("Part 1" isn't "Part 2").
fn same_title(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let digits = |s: &str| s.chars().filter(char::is_ascii_digit).collect::<String>();
    let (sa, sb) = (a.replace(' ', ""), b.replace(' ', ""));
    let longest = sa.chars().count().max(sb.chars().count());
    digits(a) == digits(b) && !sa.is_empty() && distance(&sa, &sb) <= longest / LETTERS_PER_SLIP
}

/// Levenshtein distance by characters.
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cur = row[j + 1];
            row[j + 1] = (prev + usize::from(ca != *cb)).min(row[j] + 1).min(cur + 1);
            prev = cur;
        }
    }
    row[b.len()]
}

/// The uploads that are the song, best first; of two equal ones, the one YouTube listed first.
pub fn ranked<'a>(wanted: &Wanted, found: &'a [Candidate], strictness: Match) -> Vec<&'a Candidate> {
    let mut scored: Vec<(i32, &Candidate)> = found
        .iter()
        .filter_map(|c| judge(wanted, c, strictness).map(|s| (s, c)))
        .collect();
    scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    scored.into_iter().map(|(_, c)| c).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lull_core::ids::{AlbumId, ArtistId, TrackId};

    fn song(artist: &str, title: &str, album: &str, duration: u32) -> Wanted {
        Wanted::of(&Track {
            id: TrackId("t".into()),
            title: title.into(),
            artist_id: ArtistId("a".into()),
            artist: artist.into(),
            album_id: AlbumId("al".into()),
            album: album.into(),
            year: None,
            no: None,
            disc: None,
            duration: Some(duration),
            tags: vec![],
            saved: false,
            file: None,
        })
    }

    fn upload(title: &str, channel: &str, duration: u32) -> Candidate {
        Candidate {
            id: format!("{title}|{channel}"),
            title: title.into(),
            channel: channel.into(),
            duration: Some(duration),
            ..Candidate::default()
        }
    }

    fn fishes() -> Wanted {
        song("Radiohead", "Weird Fishes / Arpeggi", "In Rainbows", 318)
    }

    #[test]
    fn official_audio_beats_a_fan_upload_and_a_music_video() {
        let found = [
            upload("Radiohead - Weird Fishes/Arpeggi [HD]", "ExoArchive", 320),
            upload("Radiohead - Weird Fishes (Official Music Video)", "Radiohead", 356),
            upload("Weird Fishes / Arpeggi", "Radiohead - Topic", 318),
            upload(
                "Radiohead - Weird Fishes / Arpeggi (Lyrics On Screen)",
                "TheLampard80",
                319,
            ),
        ];
        let order: Vec<&str> = ranked(&fishes(), &found, Match::Close)
            .iter()
            .map(|c| c.channel.as_str())
            .collect();
        assert_eq!(
            order,
            ["Radiohead - Topic", "TheLampard80", "ExoArchive"],
            "official first, then the closest length"
        );
        assert!(
            judge(&fishes(), &found[1], Match::Loose).is_none(),
            "a video 38 s longer is another cut, however official"
        );
        assert!(
            judge(&fishes(), &found[0], Match::Close).is_some(),
            "a fan upload of the song"
        );
    }

    #[test]
    fn a_fan_upload_titled_song_first_or_with_a_translation_is_the_song() {
        // YouTube's search for this song on 2026-11-04: YouTube Music lists none of its songs as it,
        // so these are all there is.
        let airy = song("Clairo", "mount airy, georgia", "mount airy, georgia", 251);
        let found = [
            upload("mount airy, georgia - october 18th, 2018", "Claire Cottrill", 252),
            upload("Clairo - mount airy, georgia (Sub.Español)", "Pololo96", 252),
            upload("20 years / Georgia - Clairo | Lyrics", "Burntrice", 203),
            upload("clairo - mount airy, georgia / LOVE GALORE", "internal heaven", 442),
            upload("mount airy, georgia - clairo", "707vegas", 252),
        ];
        let order: Vec<&str> = ranked(&airy, &found, Match::Close)
            .iter()
            .map(|c| c.channel.as_str())
            .collect();
        assert_eq!(order, ["Pololo96", "707vegas"]);
        assert!(
            judge(
                &fishes(),
                &upload("Weird Fishes / Arpeggi - Radiohead", "x", 318),
                Match::Close
            )
            .is_some(),
            "song first, then the artist"
        );
        assert!(
            judge(
                &fishes(),
                &upload("Weird Fishes / Arpeggi - Someone Else", "x", 318),
                Match::Close
            )
            .is_none(),
            "song first, then another artist"
        );
    }

    #[test]
    fn live_cover_sped_up_and_nightcore_versions_are_refused_unless_the_song_is_one() {
        let w = fishes();
        for title in [
            "Weird Fishes/Arpeggi | Radiohead | From The Basement (Live)",
            "Radiohead - Weird Fishes (cover by Someone)",
            "Weird Fishes / Arpeggi - sped up",
            "Nightcore - Weird Fishes Arpeggi",
            "Radiohead - Weird Fishes / Arpeggi (slowed + reverb)",
            "Weird Fishes / Arpeggi (Piano Cover)",
        ] {
            assert!(judge(&w, &upload(title, "x", 318), Match::Loose).is_none(), "{title}");
        }
        let mut stream = upload("Radiohead - Weird Fishes / Arpeggi", "Radiohead", 318);
        stream.live = true;
        assert!(judge(&w, &stream, Match::Loose).is_none(), "a recorded live stream");

        let live_album = song("Radiohead", "Weird Fishes / Arpeggi", "Live at the Basement", 318);
        assert!(
            judge(
                &live_album,
                &upload("Radiohead - Weird Fishes / Arpeggi (Live)", "x", 318),
                Match::Close
            )
            .is_some()
        );
        let oasis = song("Oasis", "Live Forever", "Definitely Maybe", 277);
        assert!(
            judge(
                &oasis,
                &upload("Oasis - Live Forever (Official Video)", "Oasis", 278),
                Match::Close
            )
            .is_some()
        );
        let nightcore = song("Nightcore", "Angel with a Shotgun", "x", 200);
        assert!(
            judge(
                &nightcore,
                &upload("Nightcore - Angel with a Shotgun", "Nightcore", 200),
                Match::Close
            )
            .is_some()
        );
    }

    #[test]
    fn strictness_sets_how_official_and_how_close_in_length_an_upload_must_be() {
        let w = fishes();
        let fan = upload("Radiohead - Weird Fishes / Arpeggi", "noah2413", 318);
        assert!(judge(&w, &fan, Match::Close).is_some());
        assert!(judge(&w, &fan, Match::Official).is_none(), "official only");
        let long = upload("Weird Fishes / Arpeggi", "Radiohead - Topic", 330);
        assert!(judge(&w, &long, Match::Close).is_none(), "12 s off");
        assert!(judge(&w, &long, Match::Loose).is_some());
        let art_track = Candidate {
            track: Some("Weird Fishes / Arpeggi".into()),
            artists: vec!["Radiohead".into()],
            ..upload("Weird Fishes / Arpeggi", "Radiohead", 318)
        };
        assert!(
            judge(&w, &art_track, Match::Official).is_some(),
            "music metadata is official"
        );
    }

    #[test]
    fn another_song_by_the_artist_or_another_part_is_not_the_song() {
        let w = fishes();
        assert!(
            judge(
                &w,
                &upload("Radiohead - Reckoner", "Radiohead - Topic", 318),
                Match::Loose
            )
            .is_none()
        );
        assert!(
            judge(
                &w,
                &upload("Weird Fishes / Arpeggi", "Someone Else - Topic", 318),
                Match::Loose
            )
            .is_none(),
            "same title, another artist"
        );
        let part1 = song("Artist", "Song Part 1", "x", 200);
        assert!(judge(&part1, &upload("Artist - Song Part 2", "Artist", 200), Match::Loose).is_none());
        let slip = song("Sigur Rós", "Hoppípolla", "Takk...", 268);
        assert!(judge(&slip, &upload("Sigur Ros - Hoppipola", "Sigur Rós", 268), Match::Close).is_some());
    }

    #[test]
    fn packaging_brackets_are_dropped_and_the_artist_split_off() {
        assert_eq!(
            split("Radiohead - Weird Fishes / Arpeggi (Official Audio) [HD]"),
            (Some("Radiohead".into()), "Weird Fishes / Arpeggi".into())
        );
        assert_eq!(split("Song (feat. X) (Lyric Video)"), (None, "Song (feat. X)".into()));
    }

    fn read(title: &str, channel: &str) -> Option<(String, String, bool)> {
        read_upload(title, channel, None, None)
    }

    fn named(artist: &str, title: &str, official: bool) -> Option<(String, String, bool)> {
        Some((artist.into(), title.into(), official))
    }

    #[test]
    fn an_upload_reads_as_artist_and_title_without_its_packaging() {
        assert_eq!(
            read("Olivia Rodrigo - drop dead (Official Music Video)", "Olivia Rodrigo"),
            named("Olivia Rodrigo", "drop dead", false)
        );
        assert_eq!(
            read("Patient Zero", "Taylor Swift - Topic"),
            named("Taylor Swift", "Patient Zero", true)
        );
        assert_eq!(read("Hello [Lyrics]", "AdeleVEVO"), named("Adele", "Hello", false));
        assert_eq!(
            read("Juan RIOS • praia", "Juan RIOS"),
            named("Juan RIOS", "praia", false)
        );
        assert_eq!(
            read("Song (Live at Wembley) (Official Video) | Band", "x").map(|r| r.1),
            Some("Song (Live at Wembley)".into())
        );
    }

    #[test]
    fn fan_playlist_titles_lose_the_series_their_quality_tags_and_repeated_artist() {
        assert_eq!(
            read(
                "Person Of Interest Soundtrack - Massive Attack - Live With Me",
                "AshesInTheWind1"
            ),
            named("Massive Attack", "Live With Me", false)
        );
        assert_eq!(
            read("Massive Attack - Angel  HD 1080p", "k l.s"),
            named("Massive Attack", "Angel", false)
        );
        assert_eq!(
            read(
                "Dominic Fike - Dominic Fike \"Babydoll\" (Official Video)",
                "Dominic Fike"
            ),
            named("Dominic Fike", "Babydoll", false)
        );
        assert_eq!(
            read("HD", "Someone").map(|r| r.1),
            Some("HD".into()),
            "a title that is only a tag stays"
        );
        assert_eq!(
            read("Sweet Medicine - Breezin'", "Label").map(|r| r.1),
            Some("Breezin'".into())
        );
    }

    #[test]
    fn music_metadata_wins_over_the_upload_title() {
        assert_eq!(
            read_upload(
                "Sweet Medicine w/ ODYSSEE - Breezin' [lofi]",
                "Label",
                Some("Sweet Medicine"),
                Some("Breezin'")
            ),
            named("Sweet Medicine", "Breezin'", true)
        );
    }

    #[test]
    fn a_song_only_fans_uploaded_is_found_by_a_video_search_and_plays_its_upload() {
        // Live, "clairo mount airy georgia": Apple and YouTube Music lack the song; YouTube's video
        // search for it, 2026-11-05 (first 12 results).
        let found = [
            upload("mount airy, georgia - october 18th, 2018", "Claire Cottrill", 252),
            upload("Clairo - mount airy, georgia (Sub.Español)", "Pololo96", 252),
            upload("20 years / Georgia - Clairo | Lyrics", "Burntrice", 203),
            upload("clairo - mount airy, georgia / LOVE GALORE", "internal heaven", 442),
            upload("mount airy, georgia - clairo", "707vegas", 252),
            upload("clairo - wine", "DECM", 150),
            upload("Clairo - For Now", "David Dean Burkhart", 206),
            upload(
                "clairo - georgia (22/03/2020 instagram live unreleased song)",
                "yee yee",
                156,
            ),
            upload("Clairo - Intern", "randomshitstuff", 129),
            upload(
                "clairo mount airy georgia - october 18th, 2018 filp (inspired by clay10).wav",
                "omos",
                251,
            ),
            upload("clairo - thumbs", "Matt Hall", 104),
            upload("Appleville Clairo (9/12/2020)", "Claire Cottrill", 428),
        ];
        let songs = songs_of(&found);
        let named: Vec<(&str, &str)> = songs.iter().map(|s| (s.artist.as_str(), s.title.as_str())).collect();
        // The translated upload reads as the song, and the fan's "Title - Artist" joins it.
        let song = songs
            .iter()
            .find(|s| loose_artist(&s.artist) == "clairo" && loose_title(&s.title) == "mount airy georgia")
            .ok_or_else(|| format!("{named:?}"));
        let Ok(song) = song else { unreachable!("{named:?}") };
        assert_eq!(
            named
                .iter()
                .filter(|(_, t)| loose_title(t) == "mount airy georgia")
                .count(),
            1,
            "one song for both spellings: {named:?}"
        );
        // Another take is no song here, as it is none for playback.
        assert!(!named.iter().any(|(_, t)| t.contains("georgia (")), "{named:?}");
        // Playback takes an upload of what search offers, by its own judging.
        let wanted = Wanted::named(&song.artist, &song.title, song.duration);
        let plays = ranked(&wanted, &found, Match::Close);
        assert!(
            plays
                .first()
                .is_some_and(|c| c.channel == "Pololo96" || c.channel == "707vegas"),
            "{:?}",
            plays.first()
        );
    }

    /// An upload as its full extraction reads: YouTube's music metadata on official audio.
    fn extracted(title: &str, channel: &str, artist: &str, duration: u32) -> Candidate {
        Candidate {
            track: Some(title.into()),
            artists: vec![artist.into()],
            ..upload(title, channel, duration)
        }
    }

    #[test]
    fn a_song_named_like_a_famous_namesake_plays_only_its_own_upload() {
        // Germany Germany's "LENA" (Electrolove, 2:41 at Apple). The Topic upload as yt-dlp
        // extracted it on 2026-11-05; the namesakes are the uploads YouTube's searches offered.
        let lena = song("Germany Germany", "LENA", "Electrolove", 161);
        let topic = extracted("LENA", "Germany Germany - Topic", "Germany Germany", 162);
        assert!(topic.official(&lena));
        assert!(judge(&lena, &topic, Match::Official).is_some(), "its own Topic upload");
        let namesakes = [
            // Same title, another artist, even at nearly the same length.
            extracted("Lena", "Malte Wilsen - Topic", "Malte Wilsen", 165),
            extracted("Lena", "Mike Rain - Topic", "Mike Rain", 170),
            upload(
                "Lena - Satellite - Germany 🇩🇪 Official Music Video - Eurovision 2010",
                "Eurovision Song Contest",
                182,
            ),
            upload("Lena - Happy  (The Voice Kids )", "LenaBig Fan", 100),
        ];
        for c in &namesakes {
            assert!(judge(&lena, c, Match::Loose).is_none(), "{c:?}");
        }
        // A fan's re-upload of the song itself, but another cut: refused at any strictness.
        let recut = upload("Germany Germany - LENA", "synthwave archive", 190);
        assert!(judge(&lena, &recut, Match::Loose).is_none());
        let found: Vec<Candidate> = namesakes.iter().cloned().chain([recut, topic]).collect();
        let plays = ranked(&lena, &found, Match::Close);
        assert_eq!(
            plays.iter().map(|c| c.channel.as_str()).collect::<Vec<_>>(),
            ["Germany Germany - Topic"]
        );
    }

    #[test]
    fn youtube_music_is_asked_for_the_artists_name_as_one_phrase() {
        // "Germany Germany LENA" reads to YouTube as the country and the singer Lena: on
        // 2026-11-05 neither its five YouTube Music songs nor its eight videos held the song.
        // With the name quoted, YouTube Music's first song is it. Videos keep the plain words.
        let track = Track {
            id: TrackId("t".into()),
            title: "LENA".into(),
            artist_id: ArtistId("a".into()),
            artist: "Germany Germany".into(),
            album_id: AlbumId("al".into()),
            album: "Electrolove".into(),
            year: None,
            no: None,
            disc: None,
            duration: Some(161),
            tags: vec![],
            saved: false,
            file: None,
        };
        assert_eq!(Wanted::music_query(&track), "\"Germany Germany\" LENA");
        assert_eq!(Wanted::query(&track), "Germany Germany LENA");
    }
}
