//! Stable ids derived from what a song *is* (artist, album, title), not where it comes from.
//! A Soulseek result and the same song saved in Jellyfin share one id, so a finished save flips
//! the badge in place everywhere (DESIGN.md §1) and likes, plays and playlists survive it.

use serde::{Deserialize, Serialize};

macro_rules! id_type {
    ($name:ident, $prefix:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            pub const PREFIX: &'static str = $prefix;
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

id_type!(TrackId, "t-");
id_type!(AlbumId, "al-");
id_type!(ArtistId, "ar-");

impl ArtistId {
    pub fn of(artist: &str) -> Self {
        ArtistId(format!("ar-{:016x}", hash(&[&normalize_artist(artist)])))
    }

    /// An artist's own id, for one of several artists sharing a name (ADR-0035): the name and the
    /// first anchor it was known by ("mb:<MBID>"), so it never equals the name's id.
    pub fn own(artist: &str, anchor: &str) -> Self {
        ArtistId(format!("ar-{:016x}", hash(&[&normalize_artist(artist), anchor])))
    }
}

impl AlbumId {
    pub fn of(artist: &str, album: &str) -> Self {
        AlbumId(format!(
            "al-{:016x}",
            hash(&[&normalize_artist(artist), &normalize(album)])
        ))
    }

    /// An album's own id, for one of several albums of an artist sharing a title (ADR-0035: a
    /// band's several self-titled albums): the names and the album's anchor ("rg:<MBID>").
    pub fn own(artist: &str, album: &str, anchor: &str) -> Self {
        AlbumId(format!(
            "al-{:016x}",
            hash(&[&normalize_artist(artist), &normalize(album), anchor])
        ))
    }
}

impl TrackId {
    pub fn of(artist: &str, album: &str, title: &str) -> Self {
        TrackId(format!(
            "t-{:016x}",
            hash(&[&normalize_artist(artist), &normalize(album), &normalize(title)])
        ))
    }
}

/// Lowercase, punctuation to spaces, whitespace collapsed. Keeps words such as "live" or
/// "remix": those are different recordings.
pub fn normalize(s: &str) -> String {
    let mapped: String = s.chars().map(|c| if c.is_alphanumeric() { c } else { ' ' }).collect();
    mapped.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `normalize`, plus a leading "the " dropped ("The Cure" = "Cure").
pub fn normalize_artist(s: &str) -> String {
    let n = normalize(s);
    n.strip_prefix("the ").map(str::to_owned).unwrap_or(n)
}

/// Words that make a bracket or a " - " suffix name a version of the same song, not another
/// song: "(feat. X)", "- 2011 Remaster", "(Radio Edit)", "[Live]", "(Album Version)".
const VERSION_WORDS: &[&str] = &[
    "feat",
    "ft",
    "featuring",
    "remaster",
    "remastered",
    "version",
    "edit",
    "live",
    "mono",
    "stereo",
    "explicit",
    "clean",
    "bonus",
    "deluxe",
];

/// A title for telling whether two sources hold the same song (catalog merging, Soulseek files;
/// ADR-0008): version tags (feat., remaster, version, edit, live) dropped, accents folded, "&"
/// and "and" alike. Never for ids: those keep "live" a different recording.
pub fn loose_title(s: &str) -> String {
    loosen(&drop_version_tags(s))
}

/// `loose_title` for artists: featured artists dropped, and a leading "the".
pub fn loose_artist(s: &str) -> String {
    let n = loosen(&drop_version_tags(s));
    n.strip_prefix("the ").map(str::to_owned).unwrap_or(n)
}

/// Normalized, folded, with "and" and everything from a "feat." on dropped.
fn loosen(s: &str) -> String {
    let words: Vec<String> = normalize(s).split(' ').map(fold).collect();
    let featuring = words
        .iter()
        .skip(1)
        .position(|w| matches!(w.as_str(), "feat" | "ft" | "featuring"))
        .map_or(words.len(), |i| i + 1);
    let kept: Vec<&str> = words[..featuring]
        .iter()
        .map(String::as_str)
        .filter(|w| *w != "and")
        .collect();
    kept.join(" ")
}

/// Brackets and " - " suffixes whose words name a version (or are just a year) are removed.
fn drop_version_tags(s: &str) -> String {
    let names_version = |inner: &str| {
        let n = normalize(inner);
        let words: Vec<&str> = n.split(' ').collect();
        words.iter().any(|w| VERSION_WORDS.contains(w))
            || (words.len() == 1 && words[0].len() == 4 && words[0].bytes().all(|b| b.is_ascii_digit()))
    };
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
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
        if !names_version(inner) {
            out.push_str(&rest[open..open + len + 2]);
        }
        rest = &rest[open + len + 2..];
    }
    out.push_str(rest);
    let out = out.replace(" – ", " - ").replace(" — ", " - ");
    let mut parts = out.split(" - ");
    let mut kept = parts.next().unwrap_or_default().to_owned();
    for part in parts {
        if names_version(part) {
            break;
        }
        kept.push_str(" - ");
        kept.push_str(part);
    }
    kept
}

/// Common Latin accents folded, so "Beyoncé" and "Beyonce" match.
fn fold(word: &str) -> String {
    let mut out = String::with_capacity(word.len());
    for c in word.chars() {
        match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' => out.push('a'),
            'ç' | 'č' | 'ć' => out.push('c'),
            'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ę' | 'ě' => out.push('e'),
            'ì' | 'í' | 'î' | 'ï' | 'ī' => out.push('i'),
            'ñ' | 'ń' | 'ň' => out.push('n'),
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ő' => out.push('o'),
            'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ů' | 'ű' => out.push('u'),
            'ý' | 'ÿ' => out.push('y'),
            'š' | 'ś' => out.push('s'),
            'ž' | 'ź' | 'ż' => out.push('z'),
            'ł' => out.push('l'),
            'ř' => out.push('r'),
            'ß' => out.push_str("ss"),
            'æ' => out.push_str("ae"),
            'œ' => out.push_str("oe"),
            c => out.push(c),
        }
    }
    out
}

/// FNV-1a over the parts with a separator: stable across builds and platforms.
fn hash(parts: &[&str]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            h = (h ^ 0x1f).wrapping_mul(0x0100_0000_01b3);
        }
        for b in part.bytes() {
            h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
        }
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_song_same_id_across_sources() {
        let jellyfin = TrackId::of("The Velvet Orchard", "Satellite Porch", "Paper Moon Static");
        let soulseek = TrackId::of("velvet orchard", "Satellite Porch!", "paper  moon static");
        assert_eq!(jellyfin, soulseek);
        assert_ne!(
            jellyfin,
            TrackId::of("Velvet Orchard", "Satellite Porch", "Paper Moon Static (live)")
        );
    }

    #[test]
    fn loose_forms_drop_version_tags_but_keep_what_makes_another_song() {
        let plain = loose_title("Paper Moon Static");
        for same in [
            "Paper Moon Static (feat. Neon Wren)",
            "Paper Moon Static - 2011 Remaster",
            "Paper Moon Static [Radio Edit]",
            "Paper Moon Static (Live at the Roundhouse)",
            "Paper Moon Static (1998)",
            "paper moon static feat. Neon Wren",
        ] {
            assert_eq!(loose_title(same), plain, "{same}");
        }
        assert_ne!(loose_title("Paper Moon Static (Remix)"), plain);
        assert_eq!(loose_title("Rock & Roll"), loose_title("Rock and Roll"));
        assert_eq!(
            loose_title("Feat. of Clay"),
            "feat of clay",
            "a title may start with it"
        );
        assert_eq!(loose_title("Song - Part 2"), "song part 2");
        assert_eq!(loose_artist("Beyoncé feat. Jay-Z"), loose_artist("Beyonce"));
        assert_eq!(loose_artist("The Cure"), "cure");
    }
}
