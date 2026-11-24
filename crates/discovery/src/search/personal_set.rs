//! The personal case set: what search leads with for a listener, on the fast sources' answers
//! recorded for the words (`fixtures/personal-set.json`: iTunes US songs and albums, Deezer
//! artists and playlists, YouTube Music's search, 2026-11-20, cut as `fixtures/query-set.json`
//! is, with iTunes's `artistId`; "creep", "hello" and "muse" are the query set's recordings).
//! The listeners are made up and play public songs.
//!
//! They pin what `merge::AFFINITY_WEIGHT` must do: lift the listener's own music above equally
//! good matches that are only a little better known (the low end), and never above matches far
//! better known from a stray play, or above a better match (the high end).

use std::collections::HashMap;

use lull_core::ids::{ArtistId, TrackId};
use serde::Deserialize;
use serde_json::Value;

use super::merge::Merged;
use super::personal::index::{Song, tests::track};
use super::personal::score::CatalogId;
use super::personal::snapshot::Snapshot;
use super::query::{Fit, Query};
use super::query_set::{self, CASES};
use super::{Ranked, rank_personal};
use crate::artist::identity::Source;
use crate::sources::{ArtistSource, deezer};
use crate::taste::{Mark, Play, Signals};

const DAY: i64 = 86_400;

#[derive(Deserialize, Default)]
struct Fixture {
    now: i64,
    answers: Value,
    listeners: HashMap<String, Listener>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Listener {
    songs: Vec<Kept>,
    follows: Vec<String>,
    /// Each artist's catalog ids, as the listener's own songs gave them.
    artist_ids: HashMap<String, Vec<(String, String)>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Kept {
    artist: String,
    title: String,
    album: String,
    year: Option<u16>,
    tags: Vec<String>,
    saved: bool,
    played_days_ago: Vec<i64>,
    skipped_days_ago: Vec<i64>,
    liked_days_ago: Option<i64>,
    playlisted: bool,
}

fn fixture() -> Fixture {
    // A fixture that does not parse is an empty one: every case then fails on its missing listener.
    serde_json::from_str(include_str!("fixtures/personal-set.json")).unwrap_or_default()
}

/// The snapshot of a made-up listener, built as the hub builds the real one.
fn listener(name: &str) -> Snapshot {
    let f = fixture();
    let l = &f.listeners[name];
    let mut signals = Signals {
        followed_artists: l.follows.clone(),
        ..Signals::default()
    };
    let mut songs = Vec::new();
    let mut playlisted = std::collections::HashSet::new();
    for k in &l.songs {
        let mut t = track(
            &k.artist,
            &k.title,
            &k.album,
            k.year,
            &k.tags.iter().map(String::as_str).collect::<Vec<_>>(),
        );
        // The recordings give catalog lengths the made-up files don't share: the same song by name.
        t.duration = None;
        let play = |days: i64, completed: bool| Play {
            track: t.id.clone(),
            artist: k.artist.clone(),
            at: f.now - days * DAY,
            played_s: if completed { 200 } else { 10 },
            duration_s: 200,
            completed,
            skipped: !completed,
        };
        signals.plays.extend(k.played_days_ago.iter().map(|&d| play(d, true)));
        signals.plays.extend(k.skipped_days_ago.iter().map(|&d| play(d, false)));
        signals.likes.extend(k.liked_days_ago.map(|d| Mark {
            track: t.id.clone(),
            artist: k.artist.clone(),
            at: f.now - d * DAY,
        }));
        if k.playlisted {
            playlisted.insert(t.id.clone());
        }
        songs.push(Song {
            track: t,
            saved: k.saved,
            album_artist: None,
        });
    }
    let ids: HashMap<ArtistId, Vec<CatalogId>> = l
        .artist_ids
        .iter()
        .map(|(artist, ids)| {
            let ids = ids
                .iter()
                .map(|(source, id)| {
                    let source = match source.as_str() {
                        "deezer" => Source::Deezer,
                        "itunes" => Source::Itunes,
                        _ => Source::Musicbrainz,
                    };
                    (source, id.clone())
                })
                .collect();
            (ArtistId::of(artist), ids)
        })
        .collect();
    Snapshot::from_signals(signals, songs, &playlisted, &ids, f.now)
}

/// The query's rows as a hub with this snapshot merges them: the index's answer first (it is in
/// memory), then the catalogs'.
fn merged(q: &str, answers: &Value, snapshot: &Snapshot) -> Merged {
    let mut m = Merged::default();
    m.add(snapshot.found(&Query::new(q)));
    let catalogs = query_set::merged(q, answers);
    m.add(super::merge::Found {
        songs: catalogs.songs,
        albums: catalogs.albums,
        artists: catalogs.artists,
        playlists: catalogs.playlists,
        ..super::merge::Found::default()
    });
    m
}

fn song_line(m: &Merged, i: usize) -> String {
    format!("{} - {}", m.songs[i].artist, m.songs[i].title)
}

fn first_songs(m: &Merged, r: &Ranked, n: usize) -> Vec<String> {
    r.songs.iter().take(n).map(|&i| song_line(m, i)).collect()
}

fn lead(q: &str, answers: &Value, who: &str) -> (Merged, Ranked) {
    let snapshot = listener(who);
    let m = merged(q, answers, &snapshot);
    let r = rank_personal(&Query::new(q), &m, &snapshot);
    (m, r)
}

fn query_set_answers(q: &str) -> Value {
    query_set::recorded()[q].clone()
}

#[test]
fn creep_leads_with_the_take_a_listener_plays() {
    let answers = query_set_answers("creep");
    let (m, r) = lead("creep", &answers, "creep-acoustic");
    assert_eq!(first_songs(&m, &r, 1), ["Radiohead - Creep (Acoustic)"]);
    let (m, r) = lead("creep", &answers, "empty");
    assert_ne!(
        first_songs(&m, &r, 1),
        ["Radiohead - Creep (Acoustic)"],
        "any other listener: other versions go after"
    );
}

#[test]
fn hello_leads_with_lionel_richies_for_his_listener() {
    let answers = query_set_answers("hello");
    let (m, r) = lead("hello", &answers, "lionel-richie");
    assert_eq!(first_songs(&m, &r, 1), ["Lionel Richie - Hello"]);
    let (m, r) = lead("hello", &answers, "empty");
    assert_ne!(
        first_songs(&m, &r, 1),
        ["Lionel Richie - Hello"],
        "for anyone else he is further down"
    );
}

#[test]
fn one_stray_play_leads_a_short_search_but_not_a_longer_one_over_a_far_better_known_match() {
    // "hello" is one word: the listener's KES Hello, played once, is theirs and leads (the
    // short-search rule). "stairway to" is two: Mastodon's, played once, is as good a match as Led
    // Zeppelin's, and far less known, so it stays behind (`AFFINITY_WEIGHT`'s high end).
    let (m, r) = lead("hello", &query_set_answers("hello"), "stray-play");
    assert_eq!(first_songs(&m, &r, 1), ["KES - Hello"]);
    let answers = query_set_answers("stairway to");
    let (m, r) = lead("stairway to", &answers, "stray-play");
    let (m_empty, r_empty) = lead("stairway to", &answers, "empty");
    assert_eq!(first_songs(&m, &r, 1), first_songs(&m_empty, &r_empty, 1));
}

#[test]
fn radiohead_1997_finds_the_listeners_saved_ok_computer_songs_below_the_name_matches() {
    let f = fixture();
    let (m, r) = lead("radiohead 1997", &f.answers["radiohead 1997"], "radiohead-fan");
    let q = Query::new("radiohead 1997");
    let fits: Vec<Fit> = r.songs.iter().map(|&i| q.song(&m.songs[i])).collect();
    let saved: Vec<usize> = (0..r.songs.len())
        .filter(|&n| m.songs[r.songs[n]].saved.is_some())
        .collect();
    let titles = |at: &[usize]| at.iter().map(|&n| song_line(&m, r.songs[n])).collect::<Vec<_>>();
    // The year is only a fact of the song: "1997" is not in its names, so these fit as Radiohead's
    // songs (Partly) and go after the rows whose names hold both words ("OKNOTOK 1997 2017").
    assert_eq!(
        saved.len(),
        7,
        "the seven saved OK Computer songs, and not Creep (1993): {:?}",
        titles(&saved)
    );
    assert!(saved.iter().all(|&n| fits[n] == Fit::Partly), "{:?}", titles(&saved));
    let first_partly = fits.iter().position(|&f| f == Fit::Partly).unwrap_or(0);
    assert!(first_partly > 0 && fits[..first_partly].iter().all(|&f| f >= Fit::Words));
    // And of the rows that fit that way, they lead: the others are Radiohead's from other years.
    let from_other_years = |n: usize| m.songs[r.songs[n]].year.is_some_and(|y| y != 1997);
    let behind: Vec<usize> = (first_partly..r.songs.len())
        .filter(|&n| fits[n] == Fit::Partly && m.songs[r.songs[n]].saved.is_none() && from_other_years(n))
        .collect();
    let last_saved = saved.last().copied().unwrap_or(0);
    assert!(
        behind.iter().all(|&n| n > last_saved),
        "other years' songs go behind the saved 1997 ones: {:?} / {:?}",
        titles(&saved),
        titles(&behind)
    );
}

#[test]
fn the_listeners_radiohead_leads_every_other_song_for_ra_even_one_named_ra() {
    let f = fixture();
    let (m, r) = lead("ra", &f.answers["ra"], "radiohead-fan");
    // A short search: rows that are "ra" itself (a song named Ra, "Au/Ra") fit better, but the
    // listener's songs that "ra" starts ("Radiohead") lead them, and the top result is theirs (a
    // song or an album of the band: the albums lean toward the top).
    let theirs = |i: usize| m.songs[i].artist == "Radiohead";
    let lines: Vec<String> = r.songs.iter().map(|&i| song_line(&m, i)).collect();
    let last_theirs = (0..r.songs.len()).rev().find(|&n| theirs(r.songs[n]));
    let first_other = (0..r.songs.len()).find(|&n| !theirs(r.songs[n]));
    assert!(last_theirs.is_some(), "their songs are found from \"ra\": {lines:?}");
    assert!(
        last_theirs < first_other,
        "all eight of their Radiohead songs come before every other song: {lines:?}"
    );
    assert_eq!(r.songs.iter().filter(|&&i| theirs(i)).count(), 8, "{lines:?}");
    let top = r.top.as_ref().is_some_and(|t| {
        m.songs.iter().any(|s| s.id().0 == t.id && s.artist == "Radiohead")
            || m.albums.iter().any(|a| a.id().0 == t.id && a.artist == "Radiohead")
    });
    assert!(top, "the top result is theirs, a song or an album: {:?}", r.top);
}

#[test]
fn a_well_known_artist_named_exactly_leads_a_short_search_over_a_song_of_theirs_it_merely_starts() {
    // "muse" names the band for anyone; a listener's "Musetta's Waltz" starts it, and stays a song.
    let answers = query_set_answers("muse");
    let (m, r) = lead("muse", &answers, "museum-visitor");
    let (m_empty, r_empty) = lead("muse", &answers, "empty");
    assert_eq!(
        r.top, r_empty.top,
        "the same top result as for a listener with none of it"
    );
    assert_eq!(first_songs(&m, &r, 1), first_songs(&m_empty, &r_empty, 1));
    assert!(
        r.songs.iter().any(|&i| m.songs[i].title.starts_with("Musetta")),
        "their song is still found"
    );
}

#[test]
fn a_namesake_of_a_liked_artist_gets_no_lift_and_the_artist_is_found_by_its_id() {
    // Deezer's rows for "muse": the band (705, 5M fans) and namesakes, each with its own id.
    let rows = deezer::artists_of(query_set_answers("muse")["deezer-artist"].clone()).unwrap_or_default();
    let row = |id: u64| {
        rows.iter()
            .find(|a| a.sources == [ArtistSource::Deezer(id)])
            .cloned()
            .unwrap_or_default()
    };
    let (band, namesake) = (row(705), row(5_548_712));
    assert_eq!(
        (band.name.as_str(), namesake.name.as_str()),
        ("Muse", "Muse"),
        "the recording has both"
    );

    let fan_of_the_band = listener("big-muse-fan");
    assert!(
        fan_of_the_band.artist(&band).score > 0.0,
        "the band the listener follows"
    );
    assert_eq!(
        fan_of_the_band.artist(&namesake).score,
        0.0,
        "a namesake shares the name, not the id"
    );

    let fan_of_the_namesake = listener("small-muse-fan");
    assert!(fan_of_the_namesake.artist(&namesake).score > 0.0);
    assert_eq!(
        fan_of_the_namesake.artist(&band).score,
        0.0,
        "the famous band is the namesake of a small one"
    );
}

/// What leads "muse" for a listener of Muse's songs, whether lull holds Deezer's id for the band
/// (their songs were found through it) or not (a song with no catalog record to take it from).
fn muse_top(who: &str) -> Option<String> {
    let answers = query_set_answers("muse");
    let (m, r) = lead("muse", &answers, who);
    let top = r.top?;
    match top.kind {
        "artist" => m
            .artists
            .iter()
            .find(|a| a.id().0 == top.id)
            .map(|a| format!("artist: {}", a.name)),
        _ => Some(format!("{}: {}", top.kind, top.id)),
    }
}

#[test]
fn the_artist_leads_its_own_name_for_a_listener_of_its_songs_whether_or_not_lull_holds_its_id() {
    assert_eq!(muse_top("muse-songs-with-ids").as_deref(), Some("artist: Muse"));
    assert_eq!(
        muse_top("muse-songs-no-ids").as_deref(),
        Some("artist: Muse"),
        "the songs and their album are lifted by the listener's records; the band's row has no id to be reached by"
    );
    assert_eq!(muse_top("empty").as_deref(), Some("artist: Muse"));
}

#[test]
fn an_empty_listener_gets_the_order_the_query_set_checks() {
    let recorded = query_set::recorded();
    let empty = listener("empty");
    let mut changed = Vec::new();
    for &(q, _) in CASES {
        let m = query_set::merged(q, &recorded[q]);
        let today = query_set::ranked(q, &m);
        let with = rank_personal(&Query::new(q), &m, &empty);
        if today.songs != with.songs
            || today.albums != with.albums
            || today.artists != with.artists
            || today.top != with.top
        {
            changed.push(q);
        }
    }
    assert!(changed.is_empty(), "{changed:?}");
}

#[test]
fn a_listener_who_hid_nothing_and_played_nothing_here_finds_no_affinity_anywhere() {
    let empty = listener("empty");
    let m = query_set::merged("hello", &query_set_answers("hello"));
    assert!(m.songs.iter().all(|s| empty.song(s).score == 0.0));
    let _ = TrackId::of("a", "b", "c");
}
