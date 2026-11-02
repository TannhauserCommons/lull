//! Merging, ranking and the sources' answers, on recorded answers (`fixtures/`). Deezer's
//! search is closed where they were recorded, so its track and album pages are its `/track`
//! and `/album` answers for the same songs, cut to the fields a search answer has.

use std::time::Duration;

use lull_core::ids::{ArtistId, TrackId};
use lull_core::model::Track;
use serde_json::{Value, json};
use tokio::time::Instant;

use super::merge::{self, AlbumHit, ArtistHit, Fit, Found, Kind, Merged, Query, Score, SongHit};
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
        // "cyberpunk edgerunners season 2", recorded 2026-11-02 (US store).
        "itunes-song-soundtrack" => include_str!("fixtures/itunes-song-soundtrack.json"),
        "itunes-album-soundtrack" => include_str!("fixtures/itunes-album-soundtrack.json"),
        // Dawid Podsiadło's "Let You Down": a single, his album and the Edgerunners soundtrack.
        "musicbrainz-recording-soundtrack" => include_str!("fixtures/musicbrainz-recording-soundtrack.json"),
        "musicbrainz-recording" => include_str!("fixtures/musicbrainz-recording.json"),
        "musicbrainz-release-group" => include_str!("fixtures/musicbrainz-release-group.json"),
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

/// A row another source than Last.fm confirms.
fn score(fit: Fit, popularity: f64) -> Score {
    Score {
        confirmed: true,
        fit,
        popularity,
        lean: 0.0,
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
        score(Fit::Words, 0.95), // 0: popular, but only holds the words
        score(Fit::Exact, 0.31), // 1: exact, less popular
        score(Fit::Exact, 0.82), // 2: exact, popular
        score(Fit::Exact, 0.36), // 3: exact, as popular as 1 in tenths, and liked
        score(Fit::Loose, 0.99), // 4: a fuzzy hit
        Score {
            confirmed: false,
            ..score(Fit::Exact, 0.9)
        }, // 5: only Last.fm knows it
    ];
    let order = merge::rank(&rows, |i| i == 3);
    assert_eq!(order, [2, 3, 1, 0, 4, 5]);
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
        .map(|s| Score::song(&q, s))
        .fold(None, |b: Option<Score>, t| {
            Some(b.map_or(t, |b| if t.popularity > b.popularity { t } else { b }))
        });
    assert_eq!(
        merge::top(Some(Score::artist(&q, &artists[0])), None, best_track),
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
        merge::top(Some(Score::artist(&q, &band)), None, Some(Score::song(&q, song))),
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
        merge::top(None, Some(score(q.album(&ep), 0.1)), Some(score(Fit::Words, 0.99))),
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
    // Album searches take only what songs leave spare: six calls stay for songs.
    let mut b = itunes::Budget::default();
    assert_eq!((0..25).filter(|_| b.take_spare(t0)).count(), 12);
    assert_eq!((0..25).filter(|_| b.take(t0)).count(), 6);
}

#[test]
fn a_soundtrack_named_without_the_season_typed_is_the_top_result() {
    // Live, "cyberpunk edgerunners season 2" led with a cover single "(from Cyberpunk:
    // Edgerunners Season 2)"; Spotify leads with the soundtrack, whose title has no "season".
    let q = Query::new("cyberpunk edgerunners season 2");
    let mut m = Merged::default();
    m.add(itunes::songs(fixture("itunes-song-soundtrack")).unwrap_or_default());
    m.add(itunes::albums(fixture("itunes-album-soundtrack")).unwrap_or_default());
    let ost = "Cyberpunk: Edgerunners 2 (Original Series Soundtrack)";

    let album_scores: Vec<Score> = m.albums.iter().map(|a| Score::album(&q, a)).collect();
    let albums = merge::rank(&album_scores, |_| false);
    let first = &m.albums[albums[0]];
    assert_eq!(
        (first.title.as_str(), first.artist.as_str(), first.kind.as_deref()),
        (ost, "Various Artists", Some("compilation"))
    );
    assert_eq!(album_scores[albums[0]].fit, Fit::Most);
    let song_scores: Vec<Score> = m.songs.iter().map(|s| Score::song(&q, s)).collect();
    let songs = merge::rank(&song_scores, |_| false);
    assert_eq!(
        merge::top(None, Some(album_scores[albums[0]]), Some(song_scores[songs[0]])),
        Some(Kind::Album)
    );
    // The first season's soundtrack holds only half the words: listed, but below.
    let first_season = m
        .albums
        .iter()
        .position(|a| a.title == "Cyberpunk: Edgerunners (Original Series Soundtrack)");
    assert_eq!(first_season.map(|i| album_scores[i].fit), Some(Fit::Loose));
}

#[test]
fn a_song_from_the_work_typed_fits_it_only_in_part() {
    let q = Query::new("cyberpunk edgerunners season 2");
    let from = song(
        "Rico Nasty",
        "You Can't Run From Me (from Cyberpunk: Edgerunners Season 2)",
        "You Can't Run From Me (from Cyberpunk: Edgerunners Season 2)",
        None,
    );
    assert_eq!(q.song(&from), Fit::Most, "it is from the work typed, not that work");
    let named = song("Nameless", "Cyberpunk Edgerunners Season 2 Medley", "", None);
    assert_eq!(q.song(&named), Fit::Words);
    // Short words don't make "most": "of the times" is not "skronk of the times".
    let q = Query::new("skronk of the times");
    assert_eq!(q.song(&song("Harry Styles", "Sign of the Times", "", None)), Fit::Loose);
    // Two words of three is most; one of two is not.
    assert_eq!(
        Query::new("paper moon static").song(&song("Velvet Orchard", "Paper Moon", "", None)),
        Fit::Most
    );
    assert_eq!(
        Query::new("paper static").song(&song("Velvet Orchard", "Paper Moon", "", None)),
        Fit::Loose
    );
}

#[test]
fn itunes_album_search_names_soundtracks_compilations_and_singles() {
    let found = itunes::albums(fixture("itunes-album-soundtrack")).unwrap_or_default();
    assert_eq!(found.albums.len(), 9);
    let ost = &found.albums[0];
    assert_eq!(ost.kind.as_deref(), Some("compilation"), "Various Artists");
    assert_eq!(ost.date.as_deref(), Some("2026-10-20"));
    assert!(ost.art.as_deref().is_some_and(|a| a.ends_with("/{size}x{size}bb.jpg")));
    assert_eq!(
        (found.albums[1].title.as_str(), found.albums[1].kind.as_deref()),
        ("You Can't Run From Me (from Cyberpunk: Edgerunners 2)", Some("single"))
    );
    assert_eq!(found.albums[8].kind.as_deref(), Some("album"));
    assert!(ost.popularity > found.albums[1].popularity, "Apple's order stands in");
}

#[test]
fn a_soundtrack_song_lives_on_its_soundtrack() {
    // Before, any secondary type ranked below singles: "Let You Down" landed on a reissue.
    let found = musicbrainz::recordings(fixture("musicbrainz-recording-soundtrack")).unwrap_or_default();
    assert_eq!(
        (found.songs[0].album.as_str(), found.songs[0].album_kind.as_deref()),
        ("Cyberpunk: Edgerunners (Original Series Soundtrack)", Some("album"))
    );
}

#[test]
fn an_artists_own_songs_match_their_name_as_well_as_a_song_called_that() {
    // Live, "the national" put Hannah Georgas's song "The National" above The National's own.
    let q = Query::new("the national");
    let called_that = SongHit {
        popularity: 0.7,
        ..song("Hannah Georgas", "The National", "Hannah Georgas", None)
    };
    let theirs = SongHit {
        popularity: 0.9,
        ..song("The National", "Fake Empire", "Boxer", None)
    };
    assert_eq!((q.song(&called_that), q.song(&theirs)), (Fit::Exact, Fit::Exact));
    let rows = [Score::song(&q, &called_that), Score::song(&q, &theirs)];
    assert_eq!(merge::rank(&rows, |_| false), [1, 0], "then popularity decides");
}

#[test]
fn last_fm_only_rows_never_lead_and_its_misspelt_artists_go() {
    // Live, warm "radiohed" put Last.fm's "Radiohed" above Radiohead.
    let q = Query::new("radiohed");
    let deezer = deezer::artists_of(fixture("deezer-artist")).unwrap_or_default();
    let lastfm = lastfm::found(
        &json!({ "results": { "trackmatches": { "track": [
            { "name": "Creep", "artist": "Radiohed", "listeners": "4100", "mbid": "" }
        ] } } }),
        &json!({ "results": { "artistmatches": { "artist": [
            { "name": "Radiohed", "listeners": "3900", "mbid": "" },
            { "name": "Radiohead", "listeners": "7350000", "mbid": "" },
            { "name": "Radiohedz", "listeners": "900000", "mbid": "" }
        ] } } }),
    );
    let mut m = Merged::default();
    m.add(Found {
        artists: deezer,
        ..Found::default()
    });
    m.add(lastfm.clone());
    let names = |is: &[usize]| is.iter().map(|&i| m.artists[i].name.clone()).collect::<Vec<_>>();
    let misspelt = merge::misspelt(&m.artists);
    let shown: Vec<usize> = (0..m.artists.len()).filter(|&i| !misspelt[i]).collect();
    assert!(
        !names(&shown).contains(&"Radiohed".to_owned()),
        "under 1% of Radiohead's fans, a letter away"
    );
    assert!(
        names(&shown).contains(&"Radiohedz".to_owned()),
        "a big audience of its own stays"
    );
    let scores: Vec<Score> = m.artists.iter().map(|a| Score::artist(&q, a)).collect();
    let order = merge::rank(&scores, |_| false);
    assert_eq!(
        m.artists[order[0]].name, "Radiohead",
        "confirmed by Deezer, though only a loose fit"
    );
    // A Last.fm-only song never takes the top from a confirmed artist.
    let only = Score::song(&q, &lastfm.songs[0]);
    assert!(!only.confirmed);
    assert_eq!(merge::top(Some(scores[order[0]]), None, Some(only)), Some(Kind::Artist));
}

#[test]
fn a_bandcamp_only_ep_is_found_by_its_title() {
    // Live, "skronk of the times" never showed Hairy Babies' EP: recordings don't match titles.
    let q = Query::new("skronk of the times");
    let found = musicbrainz::release_groups(fixture("musicbrainz-release-group")).unwrap_or_default();
    let kept: Vec<&AlbumHit> = found.albums.iter().filter(|a| q.album(a) >= Fit::Most).collect();
    assert_eq!(kept.len(), 1, "\"Sign of the Times\" and the like are someone else's");
    assert_eq!(
        (kept[0].title.as_str(), kept[0].artist.as_str(), kept[0].kind.as_deref()),
        ("Skronk of the Times", "HAIRY BABIES", Some("EP"))
    );
    assert!(
        kept[0]
            .art
            .as_deref()
            .is_some_and(|a| a.starts_with("https://coverartarchive.org/release-group/"))
    );
}

#[test]
fn last_fm_track_lists_become_songs_without_a_release() {
    // track.getSimilar gives an artist object and seconds as a number; tag.getTopTracks a string.
    let similar = json!([
        { "name": "Teardrop", "duration": 330, "mbid": "", "artist": { "name": "Massive Attack", "mbid": "10adbe5e-a2c0-4bf3-8249-2b4cbf6e6ca8" } },
        { "name": "Glory Box", "duration": "0", "artist": { "name": "Portishead" } }
    ]);
    let songs = lastfm::songs(&similar);
    assert_eq!(songs.len(), 2);
    assert_eq!((songs[0].duration_s, songs[0].album.as_str()), (Some(330), ""));
    assert!(songs[0].mbid.is_none() && songs[0].artist_mbid.is_some());
    assert_eq!(songs[1].duration_s, None, "0 is unknown");
    let one = json!({ "name": "Roads", "duration": "305", "artist": { "name": "Portishead" } });
    assert_eq!(
        lastfm::songs(&one)[0].duration_s,
        Some(305),
        "a single track comes as an object"
    );
}
