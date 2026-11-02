//! An artist page's popular songs: what the world plays (Last.fm's play counts), placed on the
//! catalog's songs so each row has its release, cover and length. Only songs a catalog lists for
//! this artist are shown: Last.fm pools every artist of a name, and the catalog is the check.

use std::collections::HashMap;

use lull_core::ids::loose_title;
use serde::Serialize;

use super::discography::leads;
use crate::search::merge::SongHit;
use crate::sources::lastfm::Counted;

/// Last.fm's counts for a row.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Counts {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plays: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub listeners: Option<u64>,
}

/// The `n` most popular songs. With Last.fm's counts: its most played songs that `catalog`
/// holds (counts of one song's takes summed: "Boya" and "Boya (from Cyberpunk: Edgerunners
/// 2)"), then the catalog's order fills the rest. Without them, the catalog's order (Deezer's
/// rank, Apple's order). With no catalog songs at all, Last.fm's titles alone.
pub fn rank(artist: &str, lastfm: &[Counted], catalog: &[SongHit], n: usize) -> Vec<(SongHit, Option<Counts>)> {
    let songs = best_takes(artist, catalog);
    let by_key: HashMap<&str, &SongHit> = songs.iter().map(|(k, s)| (k.as_str(), *s)).collect();
    let mut out: Vec<(SongHit, Option<Counts>)> = Vec::new();
    let mut used: Vec<String> = Vec::new();
    for (key, counts, title) in summed(lastfm) {
        if out.len() == n {
            break;
        }
        let song = match by_key.get(key.as_str()) {
            Some(s) => (*s).clone(),
            None if songs.is_empty() => SongHit {
                title,
                artist: artist.to_owned(),
                ..SongHit::default()
            },
            None => continue,
        };
        used.push(key);
        out.push((song, Some(counts)));
    }
    for (key, song) in &songs {
        if out.len() == n {
            break;
        }
        if !used.contains(key) {
            used.push(key.clone());
            out.push(((*song).clone(), None));
        }
    }
    out
}

/// A song and its takes as one: a "(from …)" note and version tags aside.
pub fn song_key(title: &str) -> String {
    let mut kept = String::with_capacity(title.len());
    let mut rest = title;
    while let Some(open) = rest.find(['(', '[']) {
        let close = if rest.as_bytes()[open] == b'(' { ')' } else { ']' };
        let Some(len) = rest[open + 1..].find(close) else { break };
        let inner = rest[open + 1..open + 1 + len].trim_start().to_lowercase();
        kept.push_str(&rest[..open]);
        if !inner.starts_with("from ") {
            kept.push_str(&rest[open..open + len + 2]);
        }
        rest = &rest[open + len + 2..];
    }
    kept.push_str(rest);
    loose_title(&kept)
}

/// Each song once, in the catalog's order, on the artist's own release when there is one (the
/// single "Boya (from Cyberpunk: Edgerunners 2)" over the series' soundtrack).
fn best_takes<'a>(artist: &str, catalog: &'a [SongHit]) -> Vec<(String, &'a SongHit)> {
    let own = |s: &SongHit| {
        s.album_kind.as_deref() != Some("compilation") && s.album_artist.as_deref().is_none_or(|a| leads(a, artist))
    };
    let mut out: Vec<(String, &SongHit)> = Vec::new();
    for s in catalog {
        let key = song_key(&s.title);
        match out.iter_mut().find(|(k, _)| *k == key) {
            Some((_, kept)) if !own(kept) && own(s) => *kept = s,
            Some(_) => {}
            None => out.push((key, s)),
        }
    }
    out
}

/// Last.fm's rows by song, counts summed, most played first.
fn summed(lastfm: &[Counted]) -> Vec<(String, Counts, String)> {
    let mut out: Vec<(String, Counts, String)> = Vec::new();
    for c in lastfm {
        let key = song_key(&c.title);
        let add = |a: Option<u64>, b: Option<u64>| match (a, b) {
            (None, None) => None,
            (a, b) => Some(a.unwrap_or(0) + b.unwrap_or(0)),
        };
        match out.iter_mut().find(|(k, _, _)| *k == key) {
            Some((_, counts, _)) => {
                counts.plays = add(counts.plays, c.plays);
                counts.listeners = counts.listeners.max(c.listeners);
            }
            None => out.push((
                key,
                Counts {
                    plays: c.plays,
                    listeners: c.listeners,
                },
                c.title.clone(),
            )),
        }
    }
    out.sort_by_key(|(_, c, _)| std::cmp::Reverse(c.plays.unwrap_or(0)));
    out
}

#[cfg(test)]
mod tests {
    use super::{rank, song_key};
    use crate::search::merge::SongHit;
    use crate::sources::lastfm::Counted;

    fn song(title: &str, album: &str, album_artist: Option<&str>, kind: &str) -> SongHit {
        SongHit {
            title: title.into(),
            artist: "BIJI".into(),
            album: album.into(),
            album_artist: album_artist.map(str::to_owned),
            album_kind: Some(kind.into()),
            ..SongHit::default()
        }
    }

    fn counted(title: &str, plays: u64) -> Counted {
        Counted {
            title: title.into(),
            plays: Some(plays),
            listeners: Some(plays / 4),
        }
    }

    /// Apple's song order for BIJI, live 2026-11-02 (shortened).
    fn bijis_songs() -> Vec<SongHit> {
        vec![
            song(
                "Boya",
                "Cyberpunk: Edgerunners 2 (Original Series Soundtrack)",
                Some("Various Artists"),
                "compilation",
            ),
            song("Zigidi", "BIJI, Pt. 1", None, "album"),
            song("Zo", "Zo", None, "single"),
            song("ESMERE", "ESMERE", None, "single"),
            song(
                "Boya (from Cyberpunk: Edgerunners 2)",
                "Boya (from Cyberpunk: Edgerunners 2)",
                None,
                "single",
            ),
            song("ESMERE", "LEGENDS", None, "album"),
            song("CAN CAN CAN", "LEGENDS", None, "album"),
        ]
    }

    #[test]
    fn last_fm_plays_order_the_catalogs_songs_on_their_own_releases() {
        let lastfm = vec![
            counted("ESMERE", 40_000),
            counted("Boya", 30_000),
            counted("Boya (from Cyberpunk: Edgerunners 2)", 25_000),
            counted("Biji Song Another Biji Sang", 90_000),
            counted("CAN CAN CAN", 1_000),
        ];
        let rows = rank("BIJI", &lastfm, &bijis_songs(), 10);
        let titles: Vec<(&str, &str)> = rows.iter().map(|(s, _)| (s.title.as_str(), s.album.as_str())).collect();
        assert_eq!(
            titles,
            [
                (
                    "Boya (from Cyberpunk: Edgerunners 2)",
                    "Boya (from Cyberpunk: Edgerunners 2)"
                ),
                ("ESMERE", "ESMERE"),
                ("CAN CAN CAN", "LEGENDS"),
                ("Zigidi", "BIJI, Pt. 1"),
                ("Zo", "Zo"),
            ],
            "another Biji's song isn't listed; the rest fill in Apple's order"
        );
        assert_eq!(
            rows[0].1.and_then(|c| c.plays),
            Some(55_000),
            "both takes of Boya count"
        );
        assert_eq!(rows[3].1, None);
    }

    #[test]
    fn without_last_fm_the_catalog_order_stands() {
        let rows = rank("BIJI", &[], &bijis_songs(), 3);
        let titles: Vec<&str> = rows.iter().map(|(s, _)| s.title.as_str()).collect();
        assert_eq!(titles, ["Boya (from Cyberpunk: Edgerunners 2)", "Zigidi", "Zo"]);
    }

    #[test]
    fn without_catalog_songs_last_fm_titles_are_shown() {
        let rows = rank("BIJI", &[counted("Boya", 10)], &[], 10);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0.album, "");
    }

    #[test]
    fn a_from_note_and_version_tags_dont_make_another_song() {
        assert_eq!(song_key("Boya (from Cyberpunk: Edgerunners 2)"), song_key("boya"));
        assert_eq!(song_key("ESMERE (Explicit)"), song_key("ESMERE"));
        assert_ne!(song_key("Gold (Tim Reaper Remix)"), song_key("Gold"));
    }
}
