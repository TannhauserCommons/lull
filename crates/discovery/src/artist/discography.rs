//! An artist's discography from every catalog that knows it (iTunes, Deezer where it is open,
//! MusicBrainz), merged into one list and grouped as Spotify does: albums, singles and EPs,
//! compilations, and the releases it appears on. The same release from several sources, or in a
//! clean and an explicit edition, counts once.

use lull_core::ids::{loose_title, normalize_artist};
use serde::{Deserialize, Serialize};

use crate::radio::main_and_featured;
use crate::search::merge::AlbumHit;
use crate::sources::Release;

/// Where an entry came from; earlier is preferred as a merged release's identity, because its
/// tracklist and cover come from that catalog in one call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Origin {
    Itunes,
    Deezer,
    MusicBrainz,
}

/// One release as a source lists it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// Title without the store's " - Single" or " - EP"; `artist` is the release's credit, empty
    /// when the source names none (Deezer's artist albums: the page's artist).
    pub album: AlbumHit,
    /// Whether the artist leads the release; `None` when the source can't say, decided from
    /// the credit.
    pub own: Option<bool>,
    pub explicit: bool,
    pub origin: Origin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Albums,
    Singles,
    Compilations,
    AppearsOn,
}

/// A MusicBrainz release group as an entry. Its first credited artist decides whether it is
/// the artist's own.
pub fn musicbrainz(release: Release, mbid: Option<&str>) -> Entry {
    let own = match (&release.artist_mbid, mbid) {
        (Some(first), Some(ours)) => Some(first == ours),
        _ => None,
    };
    Entry {
        own,
        explicit: false,
        origin: Origin::MusicBrainz,
        album: AlbumHit {
            art: release.mbid.as_deref().map(crate::art::cover_archive),
            mbid: release.mbid,
            title: release.title,
            artist: release.artist,
            kind: release.kind,
            date: release.date,
            ..AlbumHit::default()
        },
    }
}

/// Every entry merged and grouped, newest first within each group. A release is identified by
/// its loose title (case, punctuation and version tags such as "(Explicit)" or "(Deluxe)"
/// aside), within the artist's own releases or those it appears on, and within a year: an
/// album and a later single of the same name stay two.
pub fn merge(artist: &str, mut entries: Vec<Entry>) -> Vec<(Group, AlbumHit)> {
    for e in &mut entries {
        if e.album.artist.is_empty() {
            e.album.artist = artist.to_owned();
        }
        if e.own.is_none() {
            e.own = Some(leads(&e.album.artist, artist));
        }
    }
    // The preferred source first; within one, the explicit edition (as Spotify shows it).
    entries.sort_by_key(|e| (e.origin, !e.explicit));
    let mut merged: Vec<(bool, String, AlbumHit)> = Vec::new();
    for e in entries {
        let own = e.own.unwrap_or(false);
        let key = loose_title(&e.album.title);
        let same = merged
            .iter_mut()
            .find(|(o, k, a)| *o == own && *k == key && near(a.date.as_deref(), e.album.date.as_deref()));
        match same {
            Some((_, _, kept)) => fill(kept, e.album),
            None => merged.push((own, key, e.album)),
        }
    }
    let mut out: Vec<(Group, AlbumHit)> = merged
        .into_iter()
        .map(|(own, _, a)| (group(own, a.kind.as_deref()), a))
        .collect();
    // Newest first; undated last.
    out.sort_by(|(_, a), (_, b)| {
        b.date
            .is_some()
            .cmp(&a.date.is_some())
            .then_with(|| b.date.cmp(&a.date))
    });
    out
}

/// Whether `artist` leads `credit`: the whole main part ("Simon & Garfunkel"), or its first
/// artist ("BIJI & JUGGLERZ", "BIJI x Conducta"). Not a featured one ("CHAMOS feat. BIJI").
pub fn leads(credit: &str, artist: &str) -> bool {
    let (main, _) = main_and_featured(credit);
    let ours = normalize_artist(artist);
    if normalize_artist(main) == ours {
        return true;
    }
    let first = main.replace(" & ", ",").replace(" x ", ",").replace(" X ", ",");
    first.split(',').next().is_some_and(|f| normalize_artist(f) == ours)
}

/// Whether `artist` is anywhere in `credit`, featured included.
pub fn credits(credit: &str, artist: &str) -> bool {
    let ours = normalize_artist(artist);
    crate::radio::credited_names(credit)
        .iter()
        .any(|n| normalize_artist(n) == ours)
}

fn group(own: bool, kind: Option<&str>) -> Group {
    match kind.map(str::to_lowercase).as_deref() {
        _ if !own => Group::AppearsOn,
        Some("compilation") => Group::Compilations,
        Some("single" | "ep") => Group::Singles,
        _ => Group::Albums,
    }
}

/// Release dates within a year of each other, or one unknown.
fn near(a: Option<&str>, b: Option<&str>) -> bool {
    match (a.and_then(crate::sources::year), b.and_then(crate::sources::year)) {
        (Some(a), Some(b)) => a.abs_diff(b) <= 1,
        _ => true,
    }
}

/// The kept release gains what another source adds: the MBID (tags, enrichment), a date, a
/// cover, a catalog to ask for the tracklist, a type.
fn fill(kept: &mut AlbumHit, other: AlbumHit) {
    kept.mbid = kept.mbid.take().or(other.mbid);
    kept.date = match (kept.date.take(), other.date) {
        // The more precise of the two ("2024-06-14" over "2024").
        (Some(a), Some(b)) => Some(if b.len() > a.len() { b } else { a }),
        (a, b) => a.or(b),
    };
    kept.art = kept.art.take().or(other.art);
    kept.source = kept.source.or(other.source);
    kept.kind = kept.kind.take().or(other.kind);
}

#[cfg(test)]
mod tests {
    use super::{Entry, Group, Origin, credits, leads, merge, musicbrainz};
    use crate::search::merge::AlbumHit;
    use crate::sources::{AlbumSource, Release};

    fn itunes(title: &str, artist: &str, kind: &str, date: &str, own: Option<bool>, id: u64) -> Entry {
        Entry {
            album: AlbumHit {
                title: title.into(),
                artist: artist.into(),
                kind: Some(kind.into()),
                date: Some(date.into()),
                art: Some(format!("https://is1.mzstatic.com/{id}/{{size}}x{{size}}bb.jpg")),
                source: Some(AlbumSource::Itunes(id)),
                ..AlbumHit::default()
            },
            own,
            explicit: false,
            origin: Origin::Itunes,
        }
    }

    fn mb(title: &str, artist_mbid: &str, kind: &str, date: &str) -> Entry {
        musicbrainz(
            Release {
                mbid: Some(format!("rg-{title}")),
                title: title.into(),
                artist: "Biji".into(),
                artist_mbid: Some(artist_mbid.into()),
                kind: Some(kind.into()),
                date: Some(date.into()),
                tags: vec![],
            },
            Some("biji-mbid"),
        )
    }

    fn titles(groups: &[(Group, AlbumHit)], g: Group) -> Vec<&str> {
        groups
            .iter()
            .filter(|(x, _)| *x == g)
            .map(|(_, a)| a.title.as_str())
            .collect()
    }

    #[test]
    fn bijis_releases_are_grouped_like_spotify_newest_first() {
        // Live, 2026-11-02: iTunes's artist lookup listed 19 releases, MusicBrainz two singles,
        // and the artist page showed only MusicBrainz's.
        let entries = vec![
            itunes("BIJI, Pt. 2", "BIJI", "EP", "2024-11-29", Some(true), 1),
            itunes("LEGENDS", "BIJI", "album", "2026-09-04", Some(true), 2),
            itunes("Chopi", "BIJI & JUGGLERZ", "single", "2024-02-16", Some(true), 3),
            itunes(
                "Gold (Tim Reaper Remix)",
                "Conducta & BIJI",
                "single",
                "2023-09-13",
                Some(false),
                4,
            ),
            itunes(
                "Boya (from Cyberpunk: Edgerunners 2)",
                "BIJI",
                "single",
                "2026-10-20",
                Some(true),
                5,
            ),
            itunes("BIJI, Pt. 1", "BIJI", "album", "2024-06-14", Some(true), 6),
            itunes(
                "Cyberpunk: Edgerunners 2 (Original Series Soundtrack)",
                "Various Artists",
                "compilation",
                "2026-10-20",
                None,
                7,
            ),
            itunes(
                "Kurdish Flute (feat. BIJI)",
                "CHAMOS",
                "single",
                "2025-01-10",
                Some(false),
                8,
            ),
            mb("Chopi", "biji-mbid", "single", "2024-02-16"),
            mb("Gold (Tim Reaper remix)", "conducta-mbid", "single", "2023-09-13"),
        ];
        let groups = merge("BIJI", entries);
        assert_eq!(titles(&groups, Group::Albums), ["LEGENDS", "BIJI, Pt. 1"]);
        assert_eq!(
            titles(&groups, Group::Singles),
            ["Boya (from Cyberpunk: Edgerunners 2)", "BIJI, Pt. 2", "Chopi"]
        );
        assert_eq!(
            titles(&groups, Group::AppearsOn),
            [
                "Cyberpunk: Edgerunners 2 (Original Series Soundtrack)",
                "Kurdish Flute (feat. BIJI)",
                "Gold (Tim Reaper Remix)"
            ]
        );
        assert_eq!(groups.len(), 8, "MusicBrainz's Chopi and Gold are iTunes's");
        let chopi = groups.iter().find(|(_, a)| a.title == "Chopi").map(|(_, a)| a);
        assert_eq!(
            chopi.and_then(|a| a.mbid.as_deref()),
            Some("rg-Chopi"),
            "the MBID is kept"
        );
        assert_eq!(
            chopi.and_then(|a| a.source),
            Some(AlbumSource::Itunes(3)),
            "iTunes gives the tracklist"
        );
    }

    #[test]
    fn clean_and_explicit_editions_count_once_as_the_explicit_one() {
        let mut clean = itunes("LEGENDS", "BIJI", "album", "2026-09-04", Some(true), 10);
        let mut explicit = itunes("LEGENDS (Explicit)", "BIJI", "album", "2026-09-04", Some(true), 11);
        explicit.explicit = true;
        clean.explicit = false;
        let groups = merge("BIJI", vec![clean, explicit]);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].1.source, Some(AlbumSource::Itunes(11)));
    }

    #[test]
    fn an_album_and_a_much_later_single_of_the_same_name_stay_two() {
        let groups = merge(
            "Radiohead",
            vec![
                itunes("Creep", "Radiohead", "single", "1992-09-21", Some(true), 1),
                itunes("Creep", "Radiohead", "single", "2008-01-01", Some(true), 2),
            ],
        );
        assert_eq!(groups.len(), 2);
    }

    #[test]
    fn sources_without_a_credit_or_an_owner_take_the_pages_artist() {
        let deezer = Entry {
            album: AlbumHit {
                title: "ESMERE".into(),
                kind: Some("single".into()),
                ..AlbumHit::default()
            },
            own: Some(true),
            explicit: false,
            origin: Origin::Deezer,
        };
        let soundtrack = itunes(
            "Arcane (Soundtrack)",
            "Various Artists",
            "compilation",
            "2021-11-06",
            None,
            3,
        );
        let groups = merge("BIJI", vec![deezer, soundtrack]);
        assert_eq!(titles(&groups, Group::Singles), ["ESMERE"]);
        assert_eq!(
            groups
                .iter()
                .find(|(_, a)| a.title == "ESMERE")
                .map(|(_, a)| a.artist.as_str()),
            Some("BIJI")
        );
        assert_eq!(titles(&groups, Group::AppearsOn), ["Arcane (Soundtrack)"]);
    }

    #[test]
    fn the_artist_leads_a_credit_only_as_its_first_or_whole_main_artist() {
        assert!(leads("BIJI & JUGGLERZ", "Biji"));
        assert!(leads("BIJI x Conducta", "biji"));
        assert!(leads("Simon & Garfunkel", "Simon & Garfunkel"));
        assert!(leads("BIJI feat. MBALLA", "BIJI"));
        assert!(!leads("Conducta & BIJI", "BIJI"));
        assert!(!leads("CHAMOS feat. BIJI", "BIJI"));
        assert!(credits("CHAMOS feat. BIJI", "BIJI"));
        assert!(credits("Conducta x BIJI", "Biji"));
        assert!(!credits("Bijou", "Biji"));
    }
}
