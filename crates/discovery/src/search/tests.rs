//! Merging, ranking and the sources' answers, on recorded answers (`fixtures/`). Deezer's
//! search is closed where they were recorded, so its track and album pages are its `/track`
//! and `/album` answers for the same songs, cut to the fields a search answer has.

use std::time::Duration;

use lull_core::ids::{ArtistId, TrackId};
use lull_core::model::Track;
use serde_json::{Value, json};
use tokio::time::Instant;

use super::merge::{self, AlbumHit, ArtistHit, Fit, Found, Kind, Merged, Query, SongHit};
use crate::sources::{deezer, itunes, lastfm, musicbrainz};

fn fixture(name: &str) -> Value {
    let text = match name {
        "deezer-track" => include_str!("fixtures/deezer-track.json"),
        "deezer-album" => include_str!("fixtures/deezer-album.json"),
        "deezer-artist" => include_str!("fixtures/deezer-artist.json"),
        "deezer-quota" => include_str!("fixtures/deezer-quota.json"),
        "deezer-withheld" => include_str!("fixtures/deezer-withheld.json"),
        "deezer-infos" => include_str!("fixtures/deezer-infos.json"),
        "itunes-song" => include_str!("fixtures/itunes-song.json"),
        "musicbrainz-recording" => include_str!("fixtures/musicbrainz-recording.json"),
        _ => "null",
    };
    serde_json::from_str(text).unwrap_or(Value::Null)
}

fn song(artist: &str, title: &str, album: &str, duration: Option<u32>) -> SongHit {
    SongHit {
        title: title.into(),
        artist: artist.into(),
        album: album.into(),
        duration,
        ..SongHit::default()
    }
}

fn merged(songs: Vec<SongHit>) -> Merged {
    let mut m = Merged::default();
    m.add(Found {
        songs,
        ..Found::default()
    });
    m
}

#[test]
fn a_shared_isrc_is_one_song_whatever_the_titles_say() {
    let mut a = song("Radiohead", "Creep", "Pablo Honey", Some(238));
    a.isrcs = vec!["GBAYE9200070".into()];
    let mut b = song("Radiohead", "Creep (Album Version)", "The Best Of", Some(300));
    b.isrcs = vec!["GBAYE9200070".into()];
    assert_eq!(merged(vec![a, b]).songs.len(), 1);
}

#[test]
fn same_artist_and_title_merge_only_within_three_seconds() {
    let m = merged(vec![
        song("Velvet Orchard", "Paper Moon Static", "Satellite Porch", Some(201)),
        song("velvet orchard", "Paper Moon Static", "Satellite Porch", Some(204)),
        song("Velvet Orchard", "Paper Moon Static", "Satellite Porch", Some(260)),
    ]);
    assert_eq!(m.songs.len(), 2, "201 s and 204 s are one take; 260 s is another");
    // A different ISRC doesn't keep a remaster apart.
    let mut a = song("Radiohead", "Creep", "Pablo Honey", Some(238));
    a.isrcs = vec!["GBAYE9200070".into()];
    let mut b = song("Radiohead", "Creep - 2008 Remaster", "The Best Of", Some(237));
    b.isrcs = vec!["GBAYE0800001".into()];
    assert_eq!(merged(vec![a, b]).songs.len(), 1);
}

#[test]
fn feat_and_remaster_variants_are_one_row_that_keeps_the_best_facts() {
    let mut first = song(
        "Velvet Orchard",
        "Paper Moon Static (feat. Neon Wren)",
        "Satellite Porch",
        None,
    );
    first.popularity = 0.4;
    let mut later = song(
        "Velvet Orchard feat. Neon Wren",
        "Paper Moon Static - 2011 Remaster",
        "Satellite Porch",
        Some(201),
    );
    later.popularity = 0.7;
    later.art = Some("https://img.example/{size}.jpg".into());
    later.year = Some(2011);
    let m = merged(vec![first, later]);
    assert_eq!(m.songs.len(), 1);
    let row = &m.songs[0];
    assert_eq!(
        row.title, "Paper Moon Static (feat. Neon Wren)",
        "the first identity stays"
    );
    assert_eq!((row.duration, row.year, row.popularity), (Some(201), Some(2011), 0.7));
    assert!(row.art.is_some());
}

#[test]
fn the_same_song_as_a_single_and_on_an_album_is_one_row() {
    // Deezer's answer holds "Creep" on two releases (235 s and 238 s) and the acoustic take.
    let songs = deezer::songs(fixture("deezer-track")).unwrap_or_default();
    assert_eq!(songs.len(), 3);
    let m = merged(songs);
    let titles: Vec<&str> = m.songs.iter().map(|s| s.title.as_str()).collect();
    assert_eq!(titles, ["Creep", "Creep (Acoustic)"]);
    // A single and the album it is also on.
    let mut single = song("Velvet Orchard", "Paper Moon Static", "Paper Moon Static", Some(201));
    single.album_kind = Some("single".into());
    let mut album = song("Velvet Orchard", "Paper Moon Static", "Satellite Porch", Some(202));
    album.album_kind = Some("album".into());
    assert_eq!(merged(vec![single, album]).songs.len(), 1);
}

#[test]
fn musicbrainz_rows_merge_into_deezer_rows_and_bring_isrcs() {
    let mut m = merged(deezer::songs(fixture("deezer-track")).unwrap_or_default());
    let mb = musicbrainz::recordings(fixture("musicbrainz-recording")).unwrap_or_default();
    let creep = mb.songs.iter().find(|s| s.isrcs == ["GBAYE9200070"]).cloned();
    assert!(creep.is_some(), "the fixture has Radiohead's Creep");
    let creep = creep.unwrap_or_default();
    assert_eq!(
        (creep.album.as_str(), creep.album_kind.as_deref()),
        ("Pablo Honey", Some("album")),
        "its studio album, not a compilation"
    );
    assert!(
        creep
            .art
            .as_deref()
            .is_some_and(|a| a.starts_with("https://coverartarchive.org/release-group/"))
    );
    m.add(Found {
        songs: vec![creep],
        ..Found::default()
    });
    assert_eq!(m.songs.len(), 2, "MusicBrainz's Creep is Deezer's Creep");
    assert!(m.songs[0].isrcs.contains(&"GBAYE9200070".to_owned()));
    assert!(
        m.songs[0].art.as_deref().is_some_and(|a| a.contains("dzcdn")),
        "Deezer's cover, which came first, stays"
    );
}

#[test]
fn musicbrainz_rows_that_dont_hold_the_words_are_loose() {
    let q = Query::new("radiohead creep");
    let mb = musicbrainz::recordings(fixture("musicbrainz-recording")).unwrap_or_default();
    let fits: Vec<(String, Fit)> = mb.songs.iter().map(|s| (s.artist.clone(), q.song(s))).collect();
    assert!(fits.iter().any(|(a, f)| a == "Radiohead" && *f == Fit::Exact));
    assert!(fits.iter().any(|(a, f)| a == "Creep" && *f == Fit::Loose), "{fits:?}");
}

#[test]
fn a_row_shown_once_keeps_its_id() {
    let mut m = merged(vec![song("Velvet Orchard", "Paper Moon Static", "", None)]);
    let shown = m.songs[0].id();
    m.freeze();
    m.add(Found {
        songs: vec![song(
            "Velvet Orchard",
            "Paper Moon Static",
            "Satellite Porch",
            Some(201),
        )],
        ..Found::default()
    });
    assert_eq!(m.songs[0].id(), shown);
    assert_eq!(m.songs[0].duration, Some(201));
    // Before it is shown, a named release replaces the empty one.
    let mut m = merged(vec![song("Velvet Orchard", "Paper Moon Static", "", None)]);
    m.add(Found {
        songs: vec![song(
            "Velvet Orchard",
            "Paper Moon Static",
            "Satellite Porch",
            Some(201),
        )],
        ..Found::default()
    });
    assert_eq!(m.songs[0].album, "Satellite Porch");
}

#[test]
fn a_saved_copy_lends_its_id_before_the_row_is_shown() {
    let saved = Track {
        id: TrackId("t-saved".into()),
        title: "Paper Moon Static".into(),
        artist_id: ArtistId::of("Velvet Orchard"),
        artist: "Velvet Orchard".into(),
        album_id: lull_core::ids::AlbumId::of("Velvet Orchard", "Satellite Porch"),
        album: "Satellite Porch".into(),
        year: None,
        no: None,
        disc: None,
        duration: Some(202),
        tags: vec![],
        saved: true,
        file: None,
    };
    let mut m = merged(vec![song(
        "Velvet Orchard",
        "Paper Moon Static",
        "Paper Moon Static",
        Some(201),
    )]);
    m.add(Found {
        songs: vec![super::saved(saved)],
        ..Found::default()
    });
    assert_eq!(m.songs.len(), 1);
    assert_eq!(m.songs[0].id(), TrackId("t-saved".into()));
}

#[test]
fn ranking_puts_exact_matches_first_then_popularity_then_the_listeners_own() {
    let rows = [
        (Fit::Words, 0.95), // 0: popular, but only holds the words
        (Fit::Exact, 0.31), // 1: exact, less popular
        (Fit::Exact, 0.82), // 2: exact, popular
        (Fit::Exact, 0.36), // 3: exact, as popular as 1 in tenths, and liked
        (Fit::Loose, 0.99), // 4: a fuzzy hit
    ];
    let order = merge::rank(&rows, |i| i == 3);
    assert_eq!(order, [2, 3, 1, 0, 4]);
}

#[test]
fn the_top_result_is_the_best_fit_leaning_toward_artists() {
    // "radiohed": nothing fits exactly; Radiohead beats their best known song.
    let q = Query::new("radiohed");
    let artists = deezer::artists_of(fixture("deezer-artist")).unwrap_or_default();
    let tracks = deezer::songs(fixture("deezer-track")).unwrap_or_default();
    assert_eq!(artists[0].name, "Radiohead");
    let best_track = tracks
        .iter()
        .map(|s| (q.song(s), s.popularity))
        .fold(None, |b: Option<(Fit, f64)>, t| {
            Some(b.map_or(t, |b| if t.1 > b.1 { t } else { b }))
        });
    assert_eq!(
        merge::top(Some((q.artist(&artists[0]), artists[0].popularity)), None, best_track),
        Some(Kind::Artist)
    );
    // "creep": a popular song called that beats a small band called that.
    let q = Query::new("creep");
    let band = ArtistHit {
        name: "Creep".into(),
        popularity: merge::popularity(900, 1e7),
        ..ArtistHit::default()
    };
    let song = &tracks[0];
    assert_eq!(
        merge::top(
            Some((q.artist(&band), band.popularity)),
            None,
            Some((q.song(song), song.popularity))
        ),
        Some(Kind::Track)
    );
    // An album named exactly what was typed beats a song that only holds the words.
    let q = Query::new("creep ep");
    let ep = AlbumHit {
        title: "Creep EP".into(),
        artist: "Radiohead".into(),
        ..AlbumHit::default()
    };
    assert_eq!(
        merge::top(None, Some((q.album(&ep), 0.1)), Some((Fit::Words, 0.99))),
        Some(Kind::Album)
    );
}

#[test]
fn deezer_answers_become_rows_with_pictures_and_kinds() {
    let albums = deezer::albums_of(fixture("deezer-album")).unwrap_or_default();
    assert_eq!(albums[0].kind.as_deref(), Some("EP"));
    assert_eq!(
        albums[0].art.as_deref(),
        Some(
            "https://cdn-images.dzcdn.net/images/cover/d06ea246e25c65b3004627ff75e87eb8/{size}x{size}-000000-80-0-0.jpg"
        )
    );
    let artists = deezer::artists_of(fixture("deezer-artist")).unwrap_or_default();
    assert!(
        artists[0]
            .image
            .as_deref()
            .is_some_and(|i| i.contains("/artist/0d58cfbc90f2e776608bcdc0c45a4711/{size}x{size}"))
    );
    assert!(artists[0].popularity > artists[1].popularity);
    assert_eq!(
        crate::art::sized(artists[0].image.as_deref().unwrap_or_default(), 256)
            .matches("256x256")
            .count(),
        1
    );
    assert!(crate::art::sized("https://coverartarchive.org/release-group/x/front-{size}", 96).ends_with("front-250"));
}

#[test]
fn a_deezer_quota_error_is_recognised_although_it_comes_as_http_200() {
    assert_eq!(deezer::answer(fixture("deezer-quota")), Err(deezer::Refusal::Quota));
    assert!(deezer::answer(json!({ "error": { "code": 800, "message": "no data" } })).is_ok());
    assert!(matches!(
        deezer::answer(json!({ "error": { "code": 100, "message": "Invalid query" } })),
        Err(deezer::Refusal::Other(_))
    ));
}

#[test]
fn deezer_requests_keep_to_fifty_per_five_seconds_and_wait_out_a_quota() {
    let mut w = deezer::Window::default();
    let t0 = Instant::now();
    for _ in 0..50 {
        assert_eq!(w.reserve(t0), t0);
    }
    assert_eq!(
        w.reserve(t0),
        t0 + Duration::from_secs(5),
        "the 51st waits for the first to age out"
    );
    let later = t0 + Duration::from_secs(20);
    assert_eq!(w.reserve(later), later);
    w.close(later);
    assert_eq!(
        w.reserve(later),
        later + Duration::from_secs(5),
        "after a quota error, a whole window"
    );
}

#[test]
fn a_catalog_deezer_withholds_is_reported_not_taken_as_nothing_found() {
    let err = deezer::songs(fixture("deezer-withheld"))
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default();
    assert!(err.contains("isn't open in this country"), "{err}");
}

#[test]
fn last_fm_answers_become_rows_without_pictures() {
    // Last.fm's documented shapes: a single match comes as an object, several as an array.
    let tracks = json!({ "results": { "trackmatches": { "track": [
        { "name": "Creep", "artist": "Radiohead", "listeners": "2311458", "mbid": "" },
        { "name": "Creep", "artist": "TLC", "listeners": "512004", "mbid": "1c9b3d5d-3b4c-4cd8-a4c5-3d2c6b1a9e10" }
    ] } } });
    let artists = json!({ "results": { "artistmatches": { "artist":
        { "name": "Radiohead", "listeners": "7350000", "mbid": "a74b1b7f-71a5-4011-9441-d0b5e4122711" }
    } } });
    let found = lastfm::found(&tracks, &artists);
    assert_eq!(found.songs.len(), 2);
    assert!(found.songs[0].popularity > found.songs[1].popularity);
    assert!(found.songs.iter().all(|s| s.art.is_none() && s.album.is_empty()));
    assert_eq!(found.artists[0].name, "Radiohead");
    assert!(found.artists[0].image.is_none());
}

#[test]
fn deezer_says_where_its_catalog_is_closed() {
    assert_eq!(deezer::open(&fixture("deezer-infos")).ok(), Some(false));
    assert!(deezer::open(&json!({})).is_err());
}

#[test]
fn itunes_songs_bring_their_albums_with_kinds_and_sized_art() {
    let found = itunes::songs(fixture("itunes-song")).unwrap_or_default();
    assert_eq!(found.songs.len(), 6);
    let acoustic = &found.songs[0];
    assert_eq!(
        (
            acoustic.title.as_str(),
            acoustic.album.as_str(),
            acoustic.album_kind.as_deref()
        ),
        ("Creep (Acoustic)", "Creep", Some("EP")),
        "\"Creep - EP\" is the EP called Creep"
    );
    assert_eq!(acoustic.duration, Some(258));
    assert!(
        acoustic
            .art
            .as_deref()
            .is_some_and(|a| a.ends_with("/{size}x{size}bb.jpg"))
    );
    assert!(
        found.songs[0].popularity > found.songs[5].popularity,
        "Apple's order stands in"
    );
    assert_eq!(found.albums.len(), 6);
    assert_eq!(found.albums[1].kind.as_deref(), Some("single"));
    // Deezer's "Creep (Acoustic)" (258 s) and iTunes's are one row.
    let mut m = Merged::default();
    m.add(Found {
        songs: deezer::songs(fixture("deezer-track")).unwrap_or_default(),
        ..Found::default()
    });
    let before = m.songs.len();
    m.add(Found {
        songs: vec![acoustic.clone()],
        ..Found::default()
    });
    assert_eq!(m.songs.len(), before);
}

#[test]
fn the_itunes_budget_stays_under_twenty_a_minute() {
    let mut b = itunes::Budget::default();
    let t0 = Instant::now();
    assert_eq!((0..25).filter(|_| b.take(t0)).count(), 18);
    assert!(!b.take(t0 + Duration::from_secs(59)));
    assert!(b.take(t0 + Duration::from_secs(60)));
}
