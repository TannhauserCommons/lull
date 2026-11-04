//! Merging, ranking and the sources' answers, on recorded answers (`fixtures/`). Deezer's
//! search is closed where they were recorded, so its track and album pages are its `/track`
//! and `/album` answers for the same songs, cut to the fields a search answer has.

use std::collections::HashSet;
use std::time::Duration;

use lull_core::ids::{ArtistId, TrackId};
use lull_core::model::Track;
use serde_json::{Value, json};
use tokio::time::Instant;

use super::Board;
use super::merge::{self, AlbumHit, ArtistHit, Fit, Found, Kind, Merged, Query, Score, SongHit};
use crate::sources::{AlbumSource, ArtistSource, deezer, itunes, lastfm, musicbrainz};

fn fixture(name: &str) -> Value {
    let text = match name {
        "deezer-track" => include_str!("fixtures/deezer-track.json"),
        "deezer-album" => include_str!("fixtures/deezer-album.json"),
        "deezer-playlists" => include_str!("fixtures/deezer-playlists.json"),
        "deezer-artist" => include_str!("fixtures/deezer-artist.json"),
        "deezer-quota" => include_str!("fixtures/deezer-quota.json"),
        "deezer-withheld" => include_str!("fixtures/deezer-withheld.json"),
        "deezer-infos" => include_str!("fixtures/deezer-infos.json"),
        "itunes-song" => include_str!("fixtures/itunes-song.json"),
        // "cyberpunk edgerunners season 2", recorded 2026-11-02 (US store).
        "itunes-song-soundtrack" => include_str!("fixtures/itunes-song-soundtrack.json"),
        "itunes-album-soundtrack" => include_str!("fixtures/itunes-album-soundtrack.json"),
        "itunes-lookup-soundtrack" => include_str!("fixtures/itunes-lookup-soundtrack.json"),
        // Daft Punk's Discovery, `album/302127/tracks?limit=3`.
        "deezer-album-tracks" => include_str!("fixtures/deezer-album-tracks.json"),
        // Dawid Podsiadło's "Let You Down": a single, his album and the Edgerunners soundtrack.
        "musicbrainz-recording-soundtrack" => include_str!("fixtures/musicbrainz-recording-soundtrack.json"),
        // "radiohead", recorded 2026-11-02: interviews, bootleg sessions, others' songs.
        "musicbrainz-recording-radiohead" => include_str!("fixtures/musicbrainz-recording-radiohead.json"),
        "itunes-album-radiohead" => include_str!("fixtures/itunes-album-radiohead.json"),
        // Deezer's artist search for "mozart", "dark side of the moon", "blinding lights".
        "deezer-artist-near-matches" => include_str!("fixtures/deezer-artist-near-matches.json"),
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

/// A row's score.
fn score(fit: Fit, popularity: f64) -> Score {
    Score {
        fit,
        version: false,
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
    assert!(fits.iter().any(|(a, f)| a == "Creep" && *f == Fit::Partly), "{fits:?}");
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
    ];
    let order = merge::rank(&rows, |i| i == 3);
    assert_eq!(order, [2, 3, 1, 0, 4]);
}

#[test]
fn a_late_source_goes_after_the_rows_shown_and_everyone_gets_that_list() {
    // Live, two listeners searching "edgerunners 2" a minute apart got different top results,
    // one without "High Forever": iTunes's songs had been skipped for its budget.
    let q = Query::new("edgerunners 2");
    let (liked, followed) = (HashSet::new(), HashSet::new());
    let mut m = merged(vec![
        song(
            "Tsuneo Imahori",
            "Out of Bounds",
            "Cyberpunk: Edgerunners 2 (Original Series Score)",
            Some(120),
        ),
        song("Hurakion", "Cyberpunk: Edgerunners 2 Trailer Theme Music", "", None),
    ]);
    let mut board = Board::first(super::rank_rows(&q, &m, &liked, &followed));
    m.freeze();
    let first = board.clone();
    let high_forever = SongHit {
        popularity: 0.9,
        ..song(
            "Petite Meller",
            "High Forever",
            "Cyberpunk: Edgerunners 2 (Original Series Soundtrack)",
            Some(180),
        )
    };
    m.add(Found {
        songs: vec![high_forever],
        ..Found::default()
    });
    let ranked_anew = super::rank_rows(&q, &m, &liked, &followed);
    assert_eq!(ranked_anew.songs[0], 2, "ranked anew, the late match would lead");
    let added = board.add(ranked_anew);
    assert_eq!(added.songs, [2], "announced once");
    assert_eq!(
        board.songs[..first.songs.len()],
        first.songs[..],
        "the rows shown don't move"
    );
    assert_eq!(board.songs.last(), Some(&2), "and the strong match isn't dropped");
    assert_eq!(board.top, first.top, "the top result stays");
    assert!(board.add(super::rank_rows(&q, &m, &liked, &followed)).songs.is_empty());
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
        lull_core::pictures::sized(artists[0].image.as_deref().unwrap_or_default(), 256)
            .matches("256x256")
            .count(),
        1
    );
    assert!(
        lull_core::pictures::sized("https://coverartarchive.org/release-group/x/front-{size}", 96)
            .ends_with("front-250")
    );
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
fn a_catalog_deezer_withholds_is_reported_not_taken_as_nothing_found() {
    let err = deezer::songs(fixture("deezer-withheld"))
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default();
    assert!(err.contains("isn't open in this country"), "{err}");
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
    assert_eq!(first_season.map(|i| album_scores[i].fit), Some(Fit::Partly));
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
    assert_eq!(q.song(&named), Fit::Exact, "its title begins with every word typed");
    let named = song("Nameless", "Medley: Cyberpunk Edgerunners Season 2", "", None);
    assert_eq!(q.song(&named), Fit::Words);
    // Short words don't make "most": "of the times" is not "skronk of the times".
    let q = Query::new("skronk of the times");
    assert_eq!(
        q.song(&song("Harry Styles", "Sign of the Times", "", None)),
        Fit::Partly
    );
    // Two words of three is most; one of two is not.
    assert_eq!(
        Query::new("paper moon static").song(&song("Velvet Orchard", "Paper Moon", "", None)),
        Fit::Most
    );
    assert_eq!(
        Query::new("paper static").song(&song("Velvet Orchard", "Paper Moon", "", None)),
        Fit::Partly
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

#[test]
fn catalog_albums_keep_their_catalog_id_for_the_tracklist() {
    // Live, "Go to album" asked MusicBrainz twice behind its 1.1 s pace (up to 17 s a call),
    // then showed one track when it had no exact title. The catalog's own id is one fast call.
    let ost = Some(AlbumSource::Itunes(6823237344));
    let mut m = Merged::default();
    m.add(itunes::songs(fixture("itunes-song-soundtrack")).unwrap_or_default());
    let worth_it = m
        .songs
        .iter()
        .find(|s| s.title == "Worth It")
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        worth_it.album_hit().and_then(|a| a.source),
        ost,
        "an album known from its song"
    );
    m.add(itunes::albums(fixture("itunes-album-soundtrack")).unwrap_or_default());
    let row = m
        .albums
        .iter()
        .find(|a| a.title.starts_with("Cyberpunk: Edgerunners 2"));
    assert_eq!(row.and_then(|a| a.source), ost, "one row, the id kept");
    let deezer = deezer::songs(fixture("deezer-track")).unwrap_or_default();
    assert!(
        deezer
            .iter()
            .all(|s| matches!(s.album_source, Some(AlbumSource::Deezer(_))))
    );
    let deezer = deezer::albums_of(fixture("deezer-album")).unwrap_or_default();
    assert!(matches!(deezer[0].source, Some(AlbumSource::Deezer(_))));
}

#[test]
fn an_itunes_lookup_is_the_albums_tracklist_in_order() {
    let songs = itunes::tracklist(fixture("itunes-lookup-soundtrack")).unwrap_or_default();
    assert_eq!(songs.len(), 8, "the collection row itself is not a song");
    let first = &songs[0];
    assert_eq!(
        (
            first.title.as_str(),
            first.artist.as_str(),
            first.no,
            first.disc,
            first.duration_s
        ),
        ("Watson (feat. Oslo Green)", "Petite Meller", Some(1), None, Some(202))
    );
    assert_eq!(first.album_artist.as_deref(), Some("Various Artists"));
    assert_eq!(
        songs.iter().filter_map(|s| s.no).collect::<Vec<_>>(),
        (1..=8).collect::<Vec<_>>()
    );
    assert_eq!(songs[1].title, "You Can't Run From Me");
    assert_eq!(
        songs[2].artist_source,
        Some(ArtistSource::Itunes(1_689_784_894)),
        "each song names its artist by Apple's id, exact unlike the name (BIJI)"
    );
}

#[test]
fn a_deezer_album_answer_is_its_tracklist_in_order() {
    let songs = deezer::tracklist(fixture("deezer-album-tracks")).unwrap_or_default();
    let got: Vec<(&str, Option<u32>, Option<u32>)> =
        songs.iter().map(|s| (s.title.as_str(), s.no, s.duration_s)).collect();
    assert_eq!(
        got,
        [
            ("One More Time", Some(1), Some(320)),
            ("Aerodynamic", Some(2), Some(212)),
            ("Digital Love", Some(3), Some(301))
        ]
    );
    assert!(songs.iter().all(|s| s.artist == "Daft Punk" && s.disc.is_none()));
    assert!(songs.iter().all(|s| s.artist_source == Some(ArtistSource::Deezer(27))));
    assert_eq!(songs[0].isrc.as_deref(), Some("GBDUW0000053"));
}

/// Deezer's artists for one of the recorded near-match queries.
fn deezer_artists(query: &str) -> Vec<ArtistHit> {
    deezer::artists_of(fixture("deezer-artist-near-matches")[query].clone()).unwrap_or_default()
}

/// The top result once every fast source is in.
fn top_of(q: &Query, m: &Merged) -> Option<Kind> {
    let best = |scores: Vec<Score>| merge::rank(&scores, |_| false).first().map(|&i| scores[i]);
    let mut artist_scores: Vec<Score> = m.artists.iter().map(|a| Score::artist(q, a)).collect();
    merge::overshadow(&m.artists, &mut artist_scores);
    merge::top(
        best(artist_scores),
        best(m.albums.iter().map(|a| Score::album(q, a)).collect()),
        best(m.songs.iter().map(|s| Score::song(q, s)).collect()),
    )
}

#[test]
fn a_famous_near_match_beats_a_small_exact_namesake() {
    // Live, "mozart" led with a pop act called Mozart (22k fans) over Wolfgang Amadeus Mozart.
    let q = Query::new("mozart");
    let artists = deezer_artists("mozart");
    let mut scores: Vec<Score> = artists.iter().map(|a| Score::artist(&q, a)).collect();
    merge::overshadow(&artists, &mut scores);
    let order = merge::rank(&scores, |_| false);
    assert_eq!(artists[order[0]].name, "Wolfgang Amadeus Mozart");
    // "radiohed": a namesake MusicBrainz knows (no audience) ranks below Radiohead.
    let q = Query::new("radiohed");
    let artists = vec![
        ArtistHit {
            name: "Radiohed".into(),
            ..ArtistHit::default()
        },
        deezer::artists_of(fixture("deezer-artist"))
            .unwrap_or_default()
            .remove(0),
    ];
    let mut scores: Vec<Score> = artists.iter().map(|a| Score::artist(&q, a)).collect();
    merge::overshadow(&artists, &mut scores);
    assert_eq!(merge::rank(&scores, |_| false), [1, 0]);
}

#[test]
fn an_album_or_song_named_like_a_small_artist_is_the_top_result() {
    // Live, the tops were small artists named "The Dark Side of the Moon" and "Blinding Lights".
    let q = Query::new("dark side of the moon");
    let mut m = Merged::default();
    m.add(Found {
        artists: deezer_artists("dark_side_of_the_moon"),
        albums: vec![AlbumHit {
            title: "The Dark Side of the Moon".into(),
            artist: "Pink Floyd".into(),
            kind: Some("album".into()),
            popularity: 0.6,
            ..AlbumHit::default()
        }],
        ..Found::default()
    });
    assert_eq!(q.album(&m.albums[0]), Fit::Exact, "a leading \"the\" aside");
    assert_eq!(top_of(&q, &m), Some(Kind::Album));
    let q = Query::new("blinding lights");
    let mut m = Merged::default();
    m.add(Found {
        artists: deezer_artists("blinding_lights"),
        songs: vec![SongHit {
            popularity: 0.7,
            ..song("The Weeknd", "Blinding Lights", "After Hours", Some(200))
        }],
        ..Found::default()
    });
    assert_eq!(top_of(&q, &m), Some(Kind::Track));
}

#[test]
fn musicbrainz_interviews_and_bootlegs_are_not_search_results() {
    // Live, "radiohead" listed "Interview CD" and Maida Vale Q&A sessions among its albums.
    let found = musicbrainz::recordings(fixture("musicbrainz-recording-radiohead")).unwrap_or_default();
    let titles: Vec<&str> = found.songs.iter().map(|s| s.title.as_str()).collect();
    assert!(
        titles
            .iter()
            .all(|t| !t.contains("Interview") && !t.contains("Q&A") && !t.contains("Track #")),
        "{titles:?}"
    );
    assert!(titles.contains(&"Clawing at the Walls (Radiohead Beatstrumental)"));
    assert!(found.albums.iter().all(|a| a.title != "Interview CD"));
    // Apple's album search brings the studio albums the recordings never named.
    let albums = itunes::albums(fixture("itunes-album-radiohead"))
        .unwrap_or_default()
        .albums;
    for title in ["OK Computer", "Amnesiac", "The King of Limbs"] {
        assert!(
            albums.iter().any(|a| a.title == title && a.artist == "Radiohead"),
            "{title}"
        );
    }
}

#[test]
fn credit_strings_are_not_artists() {
    // Live, the artists row held "SZA with Kendrick Lamar", "Radiohead - Topic" and the like.
    let credits = json!({ "recordings": [{
        "id": "r1", "title": "Gentle Ghost", "artist-credit": [
            { "name": "Radiohead", "joinphrase": " & ", "artist": { "id": "a74b1b7f" } },
            { "name": "Portishead", "joinphrase": "", "artist": { "id": "8f6bd1e4" } }
        ], "releases": []
    }] });
    let found = musicbrainz::recordings(credits).unwrap_or_default();
    let names: Vec<&str> = found.artists.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["Radiohead", "Portishead"], "one row per credited artist");

    let fans = |name: &str, fans: u64| ArtistHit {
        name: name.into(),
        audience: Some(fans),
        ..ArtistHit::default()
    };
    let mut artists = deezer::artists_of(fixture("deezer-artist")).unwrap_or_default();
    artists.truncate(1);
    artists.extend([
        fans("Radiohead - Topic", 2_000),
        fans("Radiohead • Lossless", 900),
        ArtistHit {
            name: "Radiohead & Portishead".into(),
            ..ArtistHit::default()
        },
        fans("Radiodread", 50_000),
    ]);
    let hidden = merge::borrowed(&artists);
    let kept: Vec<&str> = artists
        .iter()
        .zip(&hidden)
        .filter(|(_, h)| !**h)
        .map(|(a, _)| a.name.as_str())
        .collect();
    assert_eq!(kept, ["Radiohead", "Radiodread"]);
}

#[test]
fn deezers_playlist_search_gives_title_owner_size_and_picture_in_its_order() {
    let found = deezer::playlists_of(fixture("deezer-playlists")).unwrap_or_default();
    assert_eq!(found.len(), 3);
    let first = &found[0];
    assert_eq!((first.site.as_str(), first.rank), ("deezer", 0));
    assert!(
        first.title.to_lowercase().contains("person of interest"),
        "{}",
        first.title
    );
    assert!(!first.owner.is_empty() && first.tracks.is_some_and(|t| t > 0));
    assert!(
        first
            .cover
            .as_deref()
            .is_some_and(|c| c.starts_with("https://cdn-images.dzcdn.net/"))
    );
    assert_eq!(first.link(), format!("deezer:playlist:{}", first.id));
    assert_eq!(found[2].rank, 2);
}
