//! The query set: words the listener types and what they mean by them, ranked on the fast
//! sources' answers recorded for those words (`fixtures/query-set.json`: iTunes US songs and
//! albums, Deezer artists and playlists, 2026-11-04, cut to the fields search reads). Search's
//! first answer fixes the top result and the order, and these are the sources it comes from, so
//! this is what the listener sees first.
//!
//! Misspellings, partial titles, artist and title in either order, other scripts, live and
//! remastered versions, a common word that is also a band, an album named like a song.

use std::collections::HashSet;

use lull_core::ids::{loose_title, normalize};
use serde_json::Value;

use super::merge::{Found, Merged, Query};
use super::{Ranked, rank_rows};
use crate::sources::{deezer, itunes};

/// Each query, and the top results that are what the listener meant ("kind: artist - title").
const CASES: &[(&str, &[&str])] = &[
    // Misspelt.
    (
        "radiohed creep",
        &["track: Radiohead - Creep (Acoustic)", "album: Radiohead - Creep"],
    ),
    ("beyonse halo", &["track: Beyoncé - Halo"]),
    ("fleetwod mac dreams", &["track: Fleetwood Mac - Dreams"]),
    ("metalica enter sandman", &["track: Metallica - Enter Sandman"]),
    ("bohemian rapsody", &["track: Queen - Bohemian Rhapsody"]),
    // Partial titles.
    ("smells like teen", &["track: Nirvana - Smells Like Teen Spirit"]),
    ("stairway to", &["track: Led Zeppelin - Stairway to Heaven"]),
    ("bohemian rhap", &["track: Queen - Bohemian Rhapsody"]),
    // Artist and title, either order.
    (
        "radiohead creep",
        &["track: Radiohead - Creep (Acoustic)", "album: Radiohead - Creep"],
    ),
    (
        "daft punk get lucky",
        &["track: Daft Punk, Pharrell Williams & Nile Rodgers - Get Lucky"],
    ),
    ("the weeknd blinding lights", &["track: The Weeknd - Blinding Lights"]),
    ("bts dynamite", &["track: BTS - Dynamite"]),
    ("acdc thunderstruck", &["track: AC/DC - Thunderstruck"]),
    // Another script.
    ("米津玄師 lemon", &["track: Kenshi Yonezu - Lemon"]),
    // A title many songs share: the famous one.
    ("hello", &["track: Adele - Hello"]),
    ("blinding lights", &["track: The Weeknd - Blinding Lights"]),
    // Versions typed.
    (
        "hotel california live",
        &[
            "track: Eagles - Hotel California (Live On MTV, 1994)",
            "track: Eagles - Hotel California (Live)",
        ],
    ),
    (
        "let it be remastered",
        &[
            "track: The Beatles - Let It Be (Remastered 2015)",
            "track: The Beatles - Let It Be (Remastered 2009)",
        ],
    ),
    // Albums, some named like songs.
    (
        "the dark side of the moon",
        &["album: Pink Floyd - The Dark Side of the Moon"],
    ),
    (
        "abbey road",
        &[
            "album: The Beatles - Abbey Road (Remastered)",
            "album: The Beatles - Abbey Road (2019 Mix)",
        ],
    ),
    ("ok computer", &["album: Radiohead - OK Computer"]),
    (
        "thriller",
        &["album: Michael Jackson - Thriller", "track: Michael Jackson - Thriller"],
    ),
    (
        "rumours",
        &[
            "album: Fleetwood Mac - Rumours",
            "album: Fleetwood Mac - Rumours (Super Deluxe Edition)",
        ],
    ),
    (
        "nirvana unplugged",
        &["album: Nirvana - MTV Unplugged In New York (Live Acoustic)"],
    ),
    (
        "mozart requiem",
        &["album: Academy of St Martin in the Fields & Neville Marriner - Mozart: Requiem"],
    ),
    (
        "interstellar soundtrack",
        &["album: Hans Zimmer - Interstellar (Original Motion Picture Soundtrack) [Expanded Edition]"],
    ),
    // A common word that is also a band; artists.
    ("muse", &["artist: Muse"]),
    ("yes", &["artist: Yes"]),
    ("train", &["artist: Train"]),
    ("taylor swift", &["artist: Taylor Swift"]),
];

/// The words of a title that make it another performance or mix of a song, not the song itself.
const VERSION_WORDS: &[&str] = &[
    "live",
    "remix",
    "mix",
    "acoustic",
    "instrumental",
    "demo",
    "karaoke",
    "edit",
];

fn recorded() -> Value {
    serde_json::from_str(include_str!("fixtures/query-set.json")).unwrap_or(Value::Null)
}

/// The fast sources' answers merged in the order they arrive from the homelab: Deezer's
/// artists (about 0.6 s), iTunes's songs and albums (0.6-1 s), Deezer's playlists.
fn merged(q: &str, answers: &Value) -> Merged {
    let mut m = Merged::default();
    m.add(Found {
        artists: deezer::artists_of(answers["deezer-artist"].clone()).unwrap_or_default(),
        ..Found::default()
    });
    m.add(itunes::answer(
        q,
        itunes::songs(answers["itunes-song"].clone()).unwrap_or_default(),
        itunes::albums(answers["itunes-album"].clone()),
    ));
    m.add(Found {
        playlists: deezer::playlists_of(answers["deezer-playlist"].clone()).unwrap_or_default(),
        ..Found::default()
    });
    m
}

fn ranked(q: &str, m: &Merged) -> Ranked {
    rank_rows(&Query::new(q), m, &HashSet::new(), &HashSet::new())
}

/// "kind: artist - title" of the top result.
fn top_line(r: &Ranked, m: &Merged) -> String {
    let Some(top) = &r.top else {
        return "none".into();
    };
    let named = match top.kind {
        "track" => m
            .songs
            .iter()
            .find(|s| s.id().0 == top.id)
            .map(|s| format!("{} - {}", s.artist, s.title)),
        "album" => m
            .albums
            .iter()
            .find(|a| a.id().0 == top.id)
            .map(|a| format!("{} - {}", a.artist, a.title)),
        "artist" => m.artists.iter().find(|a| a.id().0 == top.id).map(|a| a.name.clone()),
        _ => m.playlists.iter().find(|p| p.link() == top.id).map(|p| p.title.clone()),
    };
    format!("{}: {}", top.kind, named.unwrap_or_else(|| top.id.clone()))
}

fn is_version(title: &str) -> bool {
    normalize(title).split(' ').any(|w| VERSION_WORDS.contains(&w))
}

#[test]
fn the_query_set_finds_what_the_listener_meant_first() {
    let recorded = recorded();
    let mut wrong = Vec::new();
    for &(q, meant) in CASES {
        let m = merged(q, &recorded[q]);
        assert!(
            !m.songs.is_empty() || !m.artists.is_empty(),
            "no recorded answer for {q:?}"
        );
        let top = top_line(&ranked(q, &m), &m);
        if !meant.contains(&top.as_str()) {
            wrong.push(format!("{q:?}: {top}, meant {}", meant.join(" or ")));
        }
    }
    assert!(
        wrong.is_empty(),
        "{} of {} wrong:\n{}",
        wrong.len(),
        CASES.len(),
        wrong.join("\n")
    );
}

#[test]
fn a_song_comes_before_its_other_versions_unless_one_is_typed() {
    let recorded = recorded();
    let mut wrong = Vec::new();
    for &(q, _) in CASES {
        let typed: Vec<String> = normalize(q).split(' ').map(str::to_owned).collect();
        if typed.iter().any(|w| VERSION_WORDS.contains(&w.as_str())) {
            continue;
        }
        let m = merged(q, &recorded[q]);
        let order = ranked(q, &m).songs;
        for (at, &i) in order.iter().enumerate().take(10) {
            let s = &m.songs[i];
            let plain_later = order[at + 1..].iter().map(|&j| &m.songs[j]).find(|o| {
                !is_version(&o.title) && o.artist == s.artist && loose_title(&o.title) == loose_title(&s.title)
            });
            if is_version(&s.title) && plain_later.is_some() {
                wrong.push(format!(
                    "{q:?}: {} - {} before {}",
                    s.artist,
                    s.title,
                    plain_later.map_or("", |o| &o.title)
                ));
            }
        }
    }
    assert!(wrong.is_empty(), "{} wrong:\n{}", wrong.len(), wrong.join("\n"));
}
