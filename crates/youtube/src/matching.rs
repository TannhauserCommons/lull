//! Is this YouTube upload the song asked for, and how good a copy is it? Official audio (YouTube
//! Music songs, Topic channels, the artist's own channel) ranks first; the length must agree; live
//! takes, covers, sped-up, slowed and nightcore versions are refused unless the song is one
//! (docs/context/youtube.md#matching).

use lull_core::ids::{loose_artist, loose_title, normalize};
use lull_core::model::Track;
use lull_core::youtube::Match;

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
/// Audio)", "[HD]", "(Lyric Video)". A bracket with any of these words is dropped before the
/// title is compared.
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

    /// The words to search YouTube for.
    pub fn query(track: &Track) -> String {
        format!("{} {}", track.artist, track.title)
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
    let (artist_part, title_part) = split(&c.title);
    let title = loose_title(&c.track.clone().unwrap_or(title_part));
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

/// "Artist - Title (Official Audio)": the artist part (when there is a " - ") and the title with
/// its packaging brackets dropped.
pub fn split(raw: &str) -> (Option<String>, String) {
    let cleaned = drop_packaging(raw);
    for sep in [" - ", " – ", " — ", " | "] {
        if let Some((a, t)) = cleaned.split_once(sep) {
            return (Some(a.trim().to_owned()), t.trim().to_owned());
        }
    }
    (None, cleaned.trim().to_owned())
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

/// "Radiohead - Topic", "RadioheadVEVO", "Radiohead Official": the name alone.
fn channel_name(channel: &str) -> String {
    let c = channel.trim();
    let c = c.strip_suffix(" - Topic").unwrap_or(c);
    let c = c.strip_suffix("VEVO").unwrap_or(c);
    let c = c.strip_suffix(" Official").unwrap_or(c);
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
}
