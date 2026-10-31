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
}

impl AlbumId {
    pub fn of(artist: &str, album: &str) -> Self {
        AlbumId(format!(
            "al-{:016x}",
            hash(&[&normalize_artist(artist), &normalize(album)])
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
}
