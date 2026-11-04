//! What a Soulseek file is, from what a search tells us: the song (artist, album, title) parsed
//! from the peer's path, and the format facts from its attributes. The song decides the
//! `TrackId`, so the same song shared by many peers merges into one row
//! (docs/context/soulseek.md#reading-paths).

use lull_core::ids::normalize_artist;
use lull_core::model::FileFacts;

/// A song as its path names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Song {
    pub artist: String,
    /// The folder's artist when it names one (compilations: "Various Artists"), else `artist`.
    pub album_artist: String,
    pub album: String,
    pub title: String,
    pub no: Option<u32>,
    pub disc: Option<u32>,
    pub year: Option<u16>,
}

/// What a file's attributes say about it, as slskd reports them.
#[derive(Debug, Clone, Copy, Default)]
pub struct Attributes {
    pub bit_rate: Option<u32>,
    pub bit_depth: Option<u32>,
    pub sample_rate: Option<u32>,
    pub variable: Option<bool>,
    /// Seconds.
    pub length: Option<u32>,
}

/// Lowercase extension of a peer path (`flac`), if it has one.
pub fn extension(path: &str) -> Option<String> {
    let name = path.rsplit(['\\', '/']).next()?;
    let (_, ext) = name.rsplit_once('.')?;
    Some(ext.to_ascii_lowercase())
}

/// The peer's folder for a path: everything before the file name.
pub fn folder(path: &str) -> &str {
    path.rfind(['\\', '/']).map_or("", |i| &path[..i])
}

pub fn is_cover(path: &str) -> bool {
    matches!(extension(path).as_deref(), Some("jpg" | "jpeg" | "png" | "webp"))
}

/// Reads artist, album and title from a peer path such as
/// `@@x\Music\Artist\Album (1998) [FLAC]\CD2\03 - Title.flac`. `None` when the path has no
/// file name to read a title from.
pub fn song(path: &str) -> Option<Song> {
    let mut dirs: Vec<&str> = path
        .split(['\\', '/'])
        .filter(|p| !p.trim().is_empty() && !p.starts_with("@@"))
        .collect();
    let file = dirs.pop()?;
    let stem = file.rsplit_once('.').map_or(file, |(stem, _)| stem);

    let disc_dir = dirs.last().and_then(|d| disc_number(d));
    if disc_dir.is_some() {
        dirs.pop();
    }
    let mut folder = dirs.pop().map(read_folder).unwrap_or_default();
    // `Artist\Albums\Album`: grouping folders between artist and album name nobody.
    let parent_artist = dirs.iter().rev().find(|d| !is_generic(d)).map(|d| clean(d));
    if folder.artist.is_none()
        && let (Some(album), Some(artist)) = (&folder.album, &parent_artist)
    {
        // `Artist [1992] Album`: the folder repeats the artist without a dash.
        folder.album = Some(strip_leading_artist(album, artist));
    }
    let folder_artist = folder.artist.or(parent_artist);

    let file = read_file(stem, folder_artist.as_deref());
    let artist = file
        .artist
        .or_else(|| folder_artist.clone())
        .unwrap_or_else(|| "Unknown artist".to_owned());
    let title = (!file.title.is_empty()).then_some(file.title)?;
    Some(Song {
        album_artist: folder_artist.unwrap_or_else(|| artist.clone()),
        artist,
        album: folder.album.unwrap_or_default(),
        title,
        no: file.no,
        disc: disc_dir.or(file.disc),
        year: folder.year,
    })
}

/// Format facts for the UI's quality chip ("FLAC 16/44.1", "MP3 320", "MP3 V0"), or `None` when
/// the file isn't audio lull can play.
pub fn facts(ext: &str, size: u64, a: Attributes) -> Option<FileFacts> {
    let (codec, lossless) = match ext {
        "flac" => ("FLAC", true),
        "wav" => ("WAV", true),
        "aif" | "aiff" => ("AIFF", true),
        "m4a" if a.bit_depth.is_some() => ("ALAC", true),
        "m4a" | "aac" => ("AAC", false),
        "mp3" => ("MP3", false),
        "ogg" => ("OGG", false),
        "opus" => ("OPUS", false),
        _ => return None,
    };
    let khz = a.sample_rate.filter(|&hz| hz > 0).map(|hz| hz as f32 / 1000.0);
    let bits = a.bit_depth.and_then(|b| u8::try_from(b).ok()).filter(|&b| b > 0);
    // Lossless files rarely carry a bitrate; derive it from size and length.
    let kbps = a.bit_rate.filter(|&k| k > 0).or_else(|| {
        let secs = u64::from(a.length.filter(|&l| l > 0)?);
        u32::try_from(size.saturating_mul(8) / secs / 1000).ok()
    });
    let format = match (codec, lossless) {
        (_, true) => match (bits, khz) {
            (Some(b), Some(k)) => format!("{codec} {b}/{}", trim_khz(k)),
            _ => codec.to_owned(),
        },
        ("MP3", _) if a.variable == Some(true) => match a.bit_rate {
            Some(k) if k >= 220 => "MP3 V0".to_owned(),
            Some(k) if k >= 170 => "MP3 V2".to_owned(),
            _ => "MP3 VBR".to_owned(),
        },
        _ => match a.bit_rate {
            Some(k) if k > 0 => format!("{codec} {k}"),
            _ => codec.to_owned(),
        },
    };
    Some(FileFacts {
        format,
        codec: codec.to_owned(),
        lossless,
        khz,
        bits,
        kbps,
        size_bytes: Some(size),
    })
}

fn trim_khz(k: f32) -> String {
    let s = format!("{k:.1}");
    s.strip_suffix(".0").map(str::to_owned).unwrap_or(s)
}

#[derive(Default)]
struct Folder {
    artist: Option<String>,
    album: Option<String>,
    year: Option<u16>,
}

/// `Artist - Album (1998) [FLAC 16-44]`, `1998 - Album`, `Album`, and what rips add to them:
/// `Artist - Album - Flac`, `Album [SLSK]`, `03. EPs - Album (TOCP 8129)`, `Artist - EP - Album`.
fn read_folder(name: &str) -> Folder {
    let (rest, bracket_year) = strip_tags(name);
    let mut year = bracket_year;
    let mut parts: Vec<String> = split_dash(&rest)
        .into_iter()
        .filter(|p| match as_year(p) {
            Some(y) => {
                year = year.or(Some(y));
                false
            }
            None => !p.is_empty() && !is_release_tag(p) && !is_catalogue(p) && !is_kind_label(p),
        })
        .map(|p| without_trailing_kind(&p))
        .collect();
    match parts.len() {
        0 => Folder {
            year,
            ..Folder::default()
        },
        1 => Folder {
            artist: None,
            album: parts.pop(),
            year,
        },
        _ => {
            let artist = parts.remove(0);
            Folder {
                artist: Some(artist),
                album: Some(parts.join(" - ")),
                year,
            }
        }
    }
}

struct File {
    artist: Option<String>,
    title: String,
    no: Option<u32>,
    disc: Option<u32>,
}

/// `03 - Title`, `1-03 Title`, `103 Title`, `Artist - Title`, `Artist - Album - 03 - Title`.
fn read_file(stem: &str, folder_artist: Option<&str>) -> File {
    let stem = if stem.contains(' ') {
        stem.to_owned()
    } else {
        stem.replace('_', " ")
    };
    let (disc, no, rest) = leading_number(&stem);
    let mut parts = split_dash(rest);
    let mut no = no;
    // `Artist - Album - 03 - Title`: the number sits in the middle.
    if no.is_none()
        && let Some(i) = parts.iter().position(|p| p.len() <= 2 && p.parse::<u32>().is_ok())
        && i + 1 < parts.len()
    {
        no = parts[i].parse().ok();
        let artist = (i > 0).then(|| parts[0].clone());
        return File {
            artist,
            title: parts[i + 1..].join(" - "),
            no,
            disc,
        };
    }
    if parts.len() < 2 {
        return File {
            artist: None,
            title: parts.pop().unwrap_or_default(),
            no,
            disc,
        };
    }
    let first = normalize_artist(&parts[0]);
    let various =
        folder_artist.is_some_and(|a| matches!(normalize_artist(a).as_str(), "various artists" | "va" | "various"));
    let (artist, mut title) = match folder_artist.map(normalize_artist) {
        // `Artist - Title` in an artist's own folder, or the file names its artist.
        Some(fa) if !various && first.starts_with(&fa) => (None, parts[1..].join(" - ")),
        None => (Some(parts[0].clone()), parts[1..].join(" - ")),
        _ if various => (Some(parts[0].clone()), parts[1..].join(" - ")),
        // A dash inside the title itself ("Song - Live").
        _ => (None, parts.join(" - ")),
    };
    // `Artist - 01 Title`: the number follows the artist.
    if no.is_none()
        && let (_, Some(n), rest) = leading_number(&title)
    {
        no = Some(n);
        title = rest.to_owned();
    }
    File {
        artist,
        title,
        no,
        disc,
    }
}

/// Splits a leading track number: `03 - x`, `03. x`, `1-03 x`, `103 x` (disc 1, track 3).
fn leading_number(s: &str) -> (Option<u32>, Option<u32>, &str) {
    let digits = s.len() - s.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    let (num, rest) = s.split_at(digits);
    let follows = rest.chars().next();
    // A digit run must end in a separator, or it's part of the title ("1979", "4AM").
    if num.is_empty() || num.len() > 3 || !matches!(follows, Some(' ' | '-' | '.' | '_' | ')')) {
        return (None, None, s);
    }
    let after = rest.trim_start_matches([' ', '.', '_', ')']);
    // `1-03 Title`, `01-03 Title`: disc, dash, track.
    if num.len() <= 2
        && let Some(rest) = after.strip_prefix('-')
    {
        let d2 = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        let ends = matches!(rest[d2..].chars().next(), None | Some(' ' | '.' | '-' | '_'));
        if (1..=2).contains(&d2) && ends {
            let (track, title) = rest.split_at(d2);
            return (num.parse().ok(), track.parse().ok(), trim_sep(title));
        }
    }
    let n: u32 = num.parse().unwrap_or(0);
    let (disc, no) = if num.len() == 3 {
        (Some(n / 100), n % 100)
    } else {
        (None, n)
    };
    (disc, Some(no), trim_sep(after))
}

fn trim_sep(s: &str) -> &str {
    s.trim_start_matches([' ', '-', '.', '_', ')']).trim()
}

/// Splits on spaced dashes (`-`, en and em dash), trimming each part.
fn split_dash(s: &str) -> Vec<String> {
    s.replace(" \u{2013} ", " - ")
        .replace(" \u{2014} ", " - ")
        .split(" - ")
        .map(|p| p.trim().to_owned())
        .collect()
}

/// Removes bracketed release tags (`[FLAC]`, `(1998)`, `{WEB 24-96}`) and returns the year one
/// carried. Brackets that are part of a title, like `(What's the Story)`, stay.
fn strip_tags(name: &str) -> (String, Option<u16>) {
    let mut out = String::with_capacity(name.len());
    let mut year = None;
    let mut rest = name;
    while let Some(open) = rest.find(['(', '[', '{']) {
        let close_char = match rest.as_bytes()[open] {
            b'(' => ')',
            b'[' => ']',
            _ => '}',
        };
        let Some(len) = rest[open + 1..].find(close_char) else {
            break;
        };
        let inner = &rest[open + 1..open + 1 + len];
        out.push_str(&rest[..open]);
        if let Some(y) = as_year(inner) {
            year = year.or(Some(y));
        } else if is_release_tag(inner) && names_edition(inner) {
            // `(Deluxe Edition 320k)`: the edition is the album's, the bitrate the rip's.
            let edition = without_tag_words(inner);
            out.push_str(&format!("{}{edition}{}", &rest[open..=open], close_char));
        } else if !is_release_tag(inner) && !is_catalogue(inner) && !is_kind_label(inner) {
            out.push_str(&rest[open..open + len + 2]);
        }
        rest = &rest[open + len + 2..];
    }
    out.push_str(rest);
    (clean(&out), year)
}

fn as_year(s: &str) -> Option<u16> {
    let s = s.trim();
    let y: u16 = s.get(..4)?.parse().ok()?;
    let tail = &s[4..];
    // `1998`, or a span such as `1998-2003` (the first year counts).
    let ok = tail.is_empty() || (tail.starts_with('-') && tail[1..].chars().all(|c| c.is_ascii_digit()));
    (ok && (1900..=2100).contains(&y)).then_some(y)
}

/// Words of a rip, not of an album: formats, bitrates, sources, the share it came from.
const RELEASE_TAGS: &[&str] = &[
    "flac", "mp3", "aac", "alac", "ogg", "opus", "wav", "lossless", "kbps", "320", "256", "192", "v0", "v2", "cbr",
    "vbr", "web", "cd", "vinyl", "hi res", "hires", "24bit", "16bit", "24 bit", "16 bit", "khz", "24 96", "24 48",
    "24 44", "24 88", "16 44", "24 192", "slsk", "cds", "cdm", "itunes",
];
/// Words of an edition with its own tracklist, which keep a bracket that holds a rip's words
/// too: `(Deluxe Edition 320k)` stays `(Deluxe Edition)`.
const EDITION_WORDS: &[&str] = &[
    "deluxe",
    "edition",
    "collector",
    "collectors",
    "anniversary",
    "expanded",
    "reissue",
    "remaster",
    "remastered",
    "live",
    "special",
    "bonus",
    "legacy",
];

fn is_release_tag(inner: &str) -> bool {
    let n = lull_core::ids::normalize(inner);
    let padded = format!(" {n} ");
    RELEASE_TAGS.iter().any(|w| padded.contains(&format!(" {w} "))) || n.split(' ').any(is_bitrate)
}

/// `320k`, `256kbps`.
fn is_bitrate(word: &str) -> bool {
    let digits = word.trim_end_matches("kbps").trim_end_matches('k');
    digits.len() < word.len() && (2..=3).contains(&digits.len()) && digits.bytes().all(|b| b.is_ascii_digit())
}

fn names_edition(inner: &str) -> bool {
    lull_core::ids::normalize(inner)
        .split(' ')
        .any(|w| EDITION_WORDS.contains(&w))
}

/// The bracket's words without the rip's: "Deluxe Edition 320k" is "Deluxe Edition".
fn without_tag_words(inner: &str) -> String {
    let words: Vec<&str> = inner
        .split_whitespace()
        .filter(|w| {
            let n = lull_core::ids::normalize(w);
            !(RELEASE_TAGS.contains(&n.as_str()) || is_bitrate(&n))
        })
        .collect();
    words.join(" ")
}

/// A label's catalogue number: `TOCP 8129`, `CDR 6078, EU`, `7243 8 80919 2 3`, `60258228`.
/// Every word a number or a short code in capitals, and one a number of three digits or more.
fn is_catalogue(s: &str) -> bool {
    let words: Vec<&str> = s.split([' ', ',']).filter(|w| !w.is_empty()).collect();
    let code = |w: &str| (2..=5).contains(&w.len()) && w.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit());
    !words.is_empty()
        && words.iter().all(|w| w.bytes().all(|b| b.is_ascii_digit()) || code(w))
        && words.iter().any(|w| w.bytes().filter(u8::is_ascii_digit).count() >= 3)
}

/// A folder that sorts releases by kind, not an artist or an album: `EPs`, `03. Singles`,
/// `Singles, EPs`, `Albums`, `Live Albums`. ("Live" alone may be the band.)
fn is_kind_label(s: &str) -> bool {
    const KINDS: &[&str] = &[
        "album",
        "albums",
        "ep",
        "eps",
        "single",
        "singles",
        "compilation",
        "compilations",
        "discography",
        "bootleg",
        "bootlegs",
        "demos",
        "lp",
        "lps",
    ];
    const ALSO: &[&str] = &["studio", "live", "and", "other", "misc", "official", "b", "sides"];
    let unnumbered = s
        .trim_start_matches(|c: char| c.is_ascii_digit())
        .trim_start_matches(['.', '-', '_', ')', ' ']);
    let n = lull_core::ids::normalize(unnumbered);
    let words: Vec<&str> = n.split(' ').filter(|w| !w.is_empty()).collect();
    !words.is_empty()
        && words.iter().all(|w| KINDS.contains(w) || ALSO.contains(w))
        && words.iter().any(|w| KINDS.contains(w))
}

/// "Creep EP" and "Creep" are one release: a trailing kind word names the release, not the album.
fn without_trailing_kind(part: &str) -> String {
    for kind in [" EP", " Single", " CDS"] {
        if let Some(rest) = part.strip_suffix(kind)
            && !rest.trim().is_empty()
        {
            return clean(rest);
        }
    }
    part.to_owned()
}

/// Disc folders: `CD1`, `Disc 2`, `disk 01`.
fn disc_number(dir: &str) -> Option<u32> {
    let lower = dir.trim().to_ascii_lowercase();
    let rest = ["disc", "disk", "cd"].iter().find_map(|p| lower.strip_prefix(p))?;
    rest.trim_start_matches([' ', '-', '_', '.']).parse().ok()
}

/// Share roots that never name an artist.
fn is_generic(dir: &str) -> bool {
    const ROOTS: &[&str] = &[
        "music",
        "my music",
        "mp3",
        "mp3s",
        "flac",
        "albums",
        "downloads",
        "download",
        "complete",
        "soulseek",
        "soulseek downloads",
        "slsk",
        "shared",
        "share",
        "audio",
        "media",
        "library",
        "lossless",
        "incoming",
        "new",
        "users",
        "home",
        "various",
        "music library",
    ];
    let n = lull_core::ids::normalize(dir);
    dir.contains(':') || n.chars().count() <= 1 || ROOTS.contains(&n.as_str()) || is_kind_label(dir)
}

/// Drops the artist's words from the front of an album name, when they're there.
fn strip_leading_artist(album: &str, artist: &str) -> String {
    let artist_words = normalize_artist(artist).split(' ').count();
    let words: Vec<&str> = album.split_whitespace().collect();
    let lead = words.iter().take(artist_words).copied().collect::<Vec<_>>().join(" ");
    if words.len() > artist_words && normalize_artist(&lead) == normalize_artist(artist) {
        clean(&words[artist_words..].join(" "))
    } else {
        album.to_owned()
    }
}

fn clean(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use lull_core::ids::TrackId;

    use super::*;

    /// Merging across peers depends on reading the same song from differently organised shares.
    #[test]
    fn reads_the_same_song_from_differently_organised_shares() {
        let cases = [
            r"@@abcde\Music\Boards of Canada\Music Has the Right to Children (1998) [FLAC]\03 - Telephasic Workshop.flac",
            r"@@fghij\share\Boards of Canada - Music Has the Right to Children\03. Telephasic Workshop.mp3",
            r"@@klmno\mp3\Boards of Canada - 1998 - Music Has the Right to Children\Boards of Canada - 03 - Telephasic Workshop.mp3",
            "@@pqrst/Downloads/Boards of Canada/Music Has the Right to Children/03 Boards of Canada - Telephasic Workshop.flac",
        ];
        for path in cases {
            let s = song(path).unwrap_or_else(|| unreachable!("{path}"));
            assert_eq!(
                (
                    normalize_artist(&s.artist).as_str(),
                    s.album.as_str(),
                    s.title.as_str(),
                    s.no
                ),
                (
                    "boards of canada",
                    "Music Has the Right to Children",
                    "Telephasic Workshop",
                    Some(3)
                ),
                "{path}"
            );
        }
        // Seen live: disc-track prefixes, grouping folders, the artist repeated in the folder.
        for path in [
            r"Music\Aphex Twin\Selected Ambient Works 85–92 (2008)\01-01 Xtal.flac",
            r"music\Aphex Twin\Albums\Selected Ambient Works 85–92 (1992)\01 - Xtal.flac",
            r"@@xoezd\Music\Aphex Twin\Aphex Twin [1992] Selected Ambient Works 85-92\01 - Aphex Twin - Xtal.flac",
            r"music\mattia\Aphex Twin\aphex twin [1992] selected ambient works 85-92\01-xtal.flac",
            r"@@cheuh\complete\Aphex Twin\Albums\1992 (2008) - Selected Ambient Works 85-92 (CD, Album, RM, RE)\01 Xtal.mp3",
        ] {
            let s = song(path).unwrap_or_else(|| unreachable!("{path}"));
            assert_eq!(
                TrackId::of(&s.artist, &s.album, &s.title),
                TrackId::of("Aphex Twin", "Selected Ambient Works 85-92", "Xtal"),
                "{path}: {s:?}"
            );
            assert_eq!(s.no, Some(1), "{path}");
        }
        let disc = song(r"@@x\Music\Artist\Album\CD2\1-04 Song (Live).flac").unwrap_or_else(|| unreachable!());
        assert_eq!(
            (disc.disc, disc.no, disc.title.as_str()),
            (Some(2), Some(4), "Song (Live)")
        );
        let comp = song(r"@@x\Various Artists - Summer Mix (2004)\07 - Some Band - Their Song.mp3")
            .unwrap_or_else(|| unreachable!());
        assert_eq!(
            (comp.artist.as_str(), comp.title.as_str(), comp.year),
            ("Some Band", "Their Song", Some(2004))
        );
        let title_brackets = song(r"@@x\Oasis\(What's the Story) Morning Glory [FLAC]\02 Roll With It.flac")
            .unwrap_or_else(|| unreachable!());
        assert_eq!(title_brackets.album, "(What's the Story) Morning Glory");
    }

    #[test]
    fn a_rips_folder_name_is_its_album_without_the_rips_words() {
        // Live, "radiohead creep" listed Pablo Honey as about 13 albums, one per folder name.
        let album = |path: &str| {
            let s = song(path).unwrap_or_else(|| unreachable!("{path}"));
            (normalize_artist(&s.album_artist), s.album)
        };
        let radiohead = |album: &str| ("radiohead".to_owned(), album.to_owned());
        for path in [
            r"music\Radiohead\Pablo Honey\02 Creep.flac",
            r"Music\Radiohead - Pablo Honey - Flac\02 - Creep.flac",
            r"share\Radiohead - Pablo Honey [SLSK]\02 Creep.mp3",
            r"music\Radiohead\Radiohead - Album - Pablo Honey\02. Creep.flac",
            r"music\Radiohead\Album - Pablo Honey\02 Creep.flac",
        ] {
            assert_eq!(album(path), radiohead("Pablo Honey"), "{path}");
        }
        for path in [
            r"music\Radiohead\03. EPs - Creep (TOCP 8129)\01 Creep.flac",
            r"music\Radiohead\Singles, EPs\Creep\01 Creep.flac",
            r"music\Radiohead\Radiohead - EP - Creep\01 Creep.flac",
            r"music\Radiohead\Radiohead - Creep (Single)\01 Creep.flac",
            r"music\Radiohead\Creep EP\01 Creep.flac",
        ] {
            assert_eq!(album(path), radiohead("Creep"), "{path}");
        }
        // An edition with its own tracklist stays its own album; only the rip's words go.
        for path in [
            r"music\Radiohead - Pablo Honey (Deluxe Edition) - Flac 24-88\02 Creep.flac",
            r"music\Radiohead\Album - Pablo Honey (Deluxe Edition 320k)\02 Creep.mp3",
        ] {
            assert_eq!(album(path), radiohead("Pablo Honey (Deluxe Edition)"), "{path}");
        }
        assert_eq!(
            album(r"music\Fleetwood Mac\Rumours (Live)\01 Dreams.flac"),
            ("fleetwood mac".to_owned(), "Rumours (Live)".to_owned())
        );
        // "Live" alone may be the band.
        assert_eq!(
            album(r"music\Live - Throwing Copper\01 Lightning Crashes.flac"),
            ("live".to_owned(), "Throwing Copper".to_owned())
        );
    }

    #[test]
    fn labels_formats_the_way_the_quality_chip_reads() {
        let flac = Attributes {
            bit_depth: Some(16),
            sample_rate: Some(44_100),
            length: Some(200),
            ..Attributes::default()
        };
        let label = |ext, a| facts(ext, 26_000_000, a).map(|f| f.format);
        assert_eq!(label("flac", flac).as_deref(), Some("FLAC 16/44.1"));
        assert_eq!(facts("flac", 26_000_000, flac).and_then(|f| f.kbps), Some(1040));
        let mp3 = |rate, vbr| Attributes {
            bit_rate: Some(rate),
            variable: Some(vbr),
            ..Attributes::default()
        };
        assert_eq!(label("mp3", mp3(320, false)).as_deref(), Some("MP3 320"));
        assert_eq!(label("mp3", mp3(245, true)).as_deref(), Some("MP3 V0"));
        assert_eq!(label("jpg", Attributes::default()), None);
    }
}
