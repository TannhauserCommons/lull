//! ADR-0035 in search: namesakes are separate rows with their own ids, and nothing the listener
//! did on one reaches another.

use std::collections::HashSet;

use lull_core::identity::{self, Anchor, Came};
use lull_core::ids::{AlbumId, ArtistId};
use rusqlite::{Connection, params};

use super::*;
use crate::search::merge::{ArtistHit, Found, Merged, SongHit};
use crate::sources::ArtistSource;

type R = std::result::Result<(), Box<dyn std::error::Error>>;

fn hub() -> std::result::Result<Connection, Box<dyn std::error::Error>> {
    let conn = Connection::open_in_memory()?;
    for m in [
        &lull_core::db::CORE,
        &lull_core::catalog::MIGRATIONS,
        &crate::MIGRATIONS,
        &identity::MIGRATIONS,
    ] {
        for step in m.steps {
            conn.execute_batch(step)?;
        }
    }
    Ok(conn)
}

/// Deezer's two artists called exactly Nirvana (asked 2026-11-07).
fn deezers_nirvanas() -> Found {
    let row = |id| ArtistHit {
        name: "Nirvana".into(),
        sources: vec![ArtistSource::Deezer(id)],
        ..ArtistHit::default()
    };
    let song = |title: &str, album: &str, id| SongHit {
        title: title.into(),
        artist: "Nirvana".into(),
        album: album.into(),
        artist_source: Some(ArtistSource::Deezer(id)),
        ..SongHit::default()
    };
    Found {
        artists: vec![row(415), row(278_793_911)],
        songs: vec![
            song("Lithium", "Nevermind", 415),
            song("Tiny Goddess", "Local Anthems", 278_793_911),
        ],
        ..Found::default()
    }
}

#[test]
fn namesakes_are_one_row_with_the_switch_off_as_before() {
    let mut m = Merged::new(false);
    m.add(deezers_nirvanas());
    assert_eq!(m.artists.len(), 1);
}

#[test]
fn namesakes_are_separate_search_rows_with_their_own_ids() -> R {
    let mut c = hub()?;
    let mut m = Merged::new(true);
    m.add(deezers_nirvanas());
    assert_eq!(m.artists.len(), 2, "their Deezer ids tell them apart");
    Jobs::of(&m).run(&mut c)?.apply(&mut m);
    let (first, second) = (m.artists[0].id(), m.artists[1].id());
    assert_eq!(
        first,
        ArtistId::own("Nirvana", "dz:415"),
        "moved to its own id when the second came"
    );
    assert_eq!(second, ArtistId::own("Nirvana", "dz:278793911"));
    assert_eq!(m.songs[0].artist_id(), first, "a song lands on its catalogue artist");
    assert_eq!(m.songs[1].artist_id(), second);
    Ok(())
}

#[test]
fn a_song_without_its_artists_id_fits_both_and_is_left_to_the_names_page() -> R {
    let mut c = hub()?;
    let mut m = Merged::new(true);
    m.add(deezers_nirvanas());
    m.add(Found {
        songs: vec![SongHit {
            title: "Bootleg".into(),
            artist: "Nirvana".into(),
            album: "Live Somewhere".into(),
            ..SongHit::default()
        }],
        ..Found::default()
    });
    Jobs::of(&m).run(&mut c)?.apply(&mut m);
    assert_eq!(
        m.songs[2].artist_id(),
        ArtistId::of("Nirvana"),
        "the name's id opens the name's page"
    );
    let kept: i64 = c.query_row(
        "SELECT COUNT(*) FROM identity_anchors WHERE anchor = ?1",
        [Anchor::Track(m.songs[2].id()).key()],
        |r| r.get(0),
    )?;
    assert_eq!(kept, 0, "nothing is kept for a song that brings no id");
    Ok(())
}

/// The owner's rule: a follow made before stays with the artist the listener's songs point to.
#[test]
fn a_follow_from_before_counts_only_for_the_nirvana_the_listeners_songs_are_on() -> R {
    let c = hub()?;
    let lithium = lull_core::ids::TrackId::of("Nirvana", "Nevermind", "Lithium");
    c.execute(
        "INSERT INTO likes (track_id, artist, title, album, liked_at) VALUES (?1, 'Nirvana', 'Lithium', 'Nevermind', 1)",
        params![lithium.as_str()],
    )?;
    c.execute(
        "INSERT INTO catalog_tracks (id, json, updated_at) VALUES (?1, ?2, 1)",
        params![lithium.as_str(), r#"{"artist":"Nirvana","albumId":"al-nevermind"}"#],
    )?;
    let nevermind = Anchor::Album(AlbumId("al-nevermind".into()));
    identity::place(&c, "Nirvana", &[Anchor::Deezer(415), nevermind], Came::Credit)?;
    identity::place(&c, "Nirvana", &[Anchor::Deezer(278_793_911)], Came::Credit)?;
    let follows = followed_in(&c, HashSet::from([ArtistId::of("Nirvana")]))?;
    assert_eq!(follows, HashSet::from([ArtistId::own("Nirvana", "dz:415")]));
    assert!(
        !follows.contains(&ArtistId::own("Nirvana", "dz:278793911")),
        "the namesake gets nothing"
    );
    Ok(())
}

#[test]
fn a_follow_from_before_counts_for_nobody_when_nothing_decides() -> R {
    let c = hub()?;
    identity::place(&c, "Nirvana", &[Anchor::Deezer(415)], Came::Credit)?;
    identity::place(&c, "Nirvana", &[Anchor::Deezer(278_793_911)], Came::Credit)?;
    assert!(followed_in(&c, HashSet::from([ArtistId::of("Nirvana")]))?.is_empty());
    let picked = ArtistId::own("Nirvana", "dz:278793911");
    identity::listener_chooses(&c, &ArtistId::of("Nirvana"), &picked)?;
    assert_eq!(
        followed_in(&c, HashSet::from([ArtistId::of("Nirvana")]))?,
        HashSet::from([picked])
    );
    Ok(())
}

#[test]
fn musicbrainz_lists_every_artist_called_exactly_the_name_with_what_tells_them_apart() -> R {
    // MusicBrainz's answer of 2026-11-07, trimmed: seven artists named Nirvana, one with it as an
    // alias, and two other names its search also gave.
    let body = serde_json::from_str(include_str!("../search/fixtures/musicbrainz-named-nirvana.json"))?;
    let found = crate::sources::musicbrainz::namesakes(body, "nirvana");
    let names: Vec<&str> = found.iter().map(|n| n.name.as_str()).collect();
    assert_eq!(found.len(), 8, "{names:?}");
    assert!(!names.contains(&"Approaching Nirvana") && !names.contains(&"Nirvanna the Band"));
    assert!(names.contains(&"Nirvana 2002"), "called Nirvana by an alias");
    let about = |mbid: &str| found.iter().find(|n| n.mbid == mbid).and_then(|n| n.about.clone());
    assert_eq!(
        about("9282c8b4-ca0b-4c6b-b7e3-4f7762dfc4d6").as_deref(),
        Some("60s band from the UK")
    );
    let bare = serde_json::json!({ "artists": [
        { "id": "x", "name": "Nirvana", "type": "Group", "country": "GB", "life-span": { "begin": "1967" } }
    ] });
    let bare = crate::sources::musicbrainz::namesakes(bare, "nirvana");
    assert_eq!(
        bare[0].about.as_deref(),
        Some("Group · GB · since 1967"),
        "no comment: type, country, years"
    );
    Ok(())
}

/// The owner's rule in taste: plays, likes and a follow from before weigh only the artist they
/// were made on; the namesake starts with nothing, and a song nothing places weighs nobody.
#[test]
fn taste_weighs_only_the_nirvana_the_listener_played_and_never_its_namesake() -> R {
    use crate::taste::{Play, Signals, Taste};
    let c = hub()?;
    let now = 2_000_000_000;
    let lithium = lull_core::ids::TrackId::of("Nirvana", "Nevermind", "Lithium");
    let bootleg = lull_core::ids::TrackId::of("Nirvana", "Live Somewhere", "Bootleg");
    for (t, title, album, al) in [
        (&lithium, "Lithium", "Nevermind", "al-nevermind"),
        (&bootleg, "Bootleg", "Live Somewhere", "al-live"),
    ] {
        c.execute(
            "INSERT INTO plays (track_id, artist, title, album, duration_s, started_at, played_s, completed) VALUES (?1, 'Nirvana', ?2, ?3, 200, ?4, 200, 1)",
            params![t.as_str(), title, album, now - 100],
        )?;
        let song = lull_core::model::Track {
            id: (*t).clone(),
            title: title.into(),
            artist_id: ArtistId::of("Nirvana"),
            artist: "Nirvana".into(),
            album_id: AlbumId(al.into()),
            album: album.into(),
            year: None,
            no: None,
            disc: None,
            duration: Some(200),
            tags: vec!["grunge".into()],
            saved: false,
            file: None,
        };
        lull_core::catalog::remember(&c, &[song])?;
    }
    c.execute(
        "INSERT INTO follows (kind, id, name, followed_at) VALUES ('artist', ?1, 'Nirvana', 1)",
        [ArtistId::of("Nirvana").as_str()],
    )?;
    identity::place(
        &c,
        "Nirvana",
        &[Anchor::Deezer(415), Anchor::Album(AlbumId("al-nevermind".into()))],
        Came::Credit,
    )?;
    identity::place(&c, "Nirvana", &[Anchor::Deezer(278_793_911)], Came::Credit)?;
    let (us, other) = (
        ArtistId::own("Nirvana", "dz:415"),
        ArtistId::own("Nirvana", "dz:278793911"),
    );

    let play = |track: &lull_core::ids::TrackId| Play {
        track: track.clone(),
        artist: "Nirvana".into(),
        at: now - 100,
        played_s: 200,
        duration_s: 200,
        completed: true,
        skipped: false,
    };
    let mut signals = Signals {
        plays: vec![play(&lithium), play(&bootleg)],
        followed_artists: vec!["Nirvana".into()],
        ..Signals::default()
    };
    let (placed, hidden) = placed(&c, &signals)?;
    assert_eq!(placed.tracks.get(&lithium), Some(&Some(us.clone())));
    assert_eq!(
        placed.tracks.get(&bootleg),
        Some(&None),
        "the bootleg fits both: nobody's"
    );
    assert_eq!(
        placed.follows,
        [Some(us.clone())],
        "the follow from before is the listener's Nirvana"
    );
    signals.namesakes = Some(placed);
    signals.hidden = hidden;
    let taste = Taste::build(&signals, now);
    assert!(taste.artist_weight(&us) > 0.0);
    assert_eq!(taste.artist_weight(&other), 0.0, "the namesake inherits nothing");
    assert_eq!(taste.artist_weight(&ArtistId::of("Nirvana")), 0.0, "nor does the name");
    Ok(())
}

/// ADR-0035: an artist link with no id but one of the artist's albums opens the namesake holding
/// that album; with neither, the name's page.
#[test]
fn an_artist_link_opens_the_namesake_its_album_or_ids_say() -> R {
    let c = hub()?;
    let nevermind = Anchor::Album(AlbumId::of("Nirvana", "Nevermind"));
    identity::place(&c, "Nirvana", &[Anchor::Deezer(415), nevermind], Came::Credit)?;
    identity::place(&c, "Nirvana", &[Anchor::Deezer(278_793_911)], Came::Credit)?;
    let open = |ids: crate::open::ArtistIds| -> std::result::Result<ArtistId, Box<dyn std::error::Error>> {
        Ok(identity::artist_of(&c, "Nirvana", &link_anchors("Nirvana", None, &ids))?.id("Nirvana"))
    };
    let by_album = crate::open::ArtistIds {
        album: Some("Nevermind".into()),
        ..Default::default()
    };
    assert_eq!(open(by_album)?, ArtistId::own("Nirvana", "dz:415"));
    let by_id = crate::open::ArtistIds {
        deezer: Some(278_793_911),
        ..Default::default()
    };
    assert_eq!(open(by_id)?, ArtistId::own("Nirvana", "dz:278793911"));
    assert_eq!(
        open(crate::open::ArtistIds::default())?,
        ArtistId::of("Nirvana"),
        "the name's page"
    );
    Ok(())
}

/// ADR-0035, "Carried over": the ids lull already holds per artist become its anchors, so the
/// listener's Nirvana keeps its MBID, and a second Nirvana with another is told apart from it.
#[test]
fn the_ids_lull_already_holds_become_anchors() -> R {
    let c = hub()?;
    c.execute(
        "INSERT INTO discovery_artists (id, json, updated_at) VALUES (?1, ?2, 1)",
        params![ArtistId::of("Nirvana").as_str(), r#"{"name":"Nirvana"}"#],
    )?;
    for (source, id) in [
        ("musicbrainz", "5b11f4ce-a62d-471e-81fc-a69a8278c7da"),
        ("deezer", "415"),
    ] {
        c.execute(
            "INSERT INTO discovery_artist_ids (artist_id, source, id, evidence) VALUES (?1, ?2, ?3, '{}')",
            params![ArtistId::of("Nirvana").as_str(), source, id],
        )?;
    }
    let held = held_ids(&c)?;
    assert_eq!(held.len(), 2);
    for (name, anchors) in held {
        identity::place(&c, &name, &anchors, Came::Library)?;
    }
    let other = identity::place(
        &c,
        "Nirvana",
        &[Anchor::Mbid("9282c8b4-ca0b-4c6b-b7e3-4f7762dfc4d6".into())],
        Came::Credit,
    )?;
    assert_eq!(identity::artists_named(&c, "Nirvana")?.len(), 2);
    assert_eq!(
        identity::artist_of(&c, "Nirvana", &[Anchor::Deezer(415)])?,
        identity::Place::One(ArtistId::own("Nirvana", "mb:5b11f4ce-a62d-471e-81fc-a69a8278c7da"))
    );
    assert_ne!(
        other,
        identity::Place::One(ArtistId::own("Nirvana", "mb:5b11f4ce-a62d-471e-81fc-a69a8278c7da"))
    );
    Ok(())
}

/// ADR-0035 in radio: each Nirvana's radio asks by its own ids, never the other's.
#[test]
fn each_namesakes_radio_uses_its_own_ids() -> R {
    let c = hub()?;
    identity::place(
        &c,
        "Nirvana",
        &[
            Anchor::Mbid("5b11f4ce-a62d-471e-81fc-a69a8278c7da".into()),
            Anchor::Deezer(415),
        ],
        Came::Credit,
    )?;
    identity::place(
        &c,
        "Nirvana",
        &[Anchor::Mbid("9282c8b4-ca0b-4c6b-b7e3-4f7762dfc4d6".into())],
        Came::Credit,
    )?;
    let us = own_ids(&c, &ArtistId::own("Nirvana", "mb:5b11f4ce-a62d-471e-81fc-a69a8278c7da"))?;
    let uk = own_ids(&c, &ArtistId::own("Nirvana", "mb:9282c8b4-ca0b-4c6b-b7e3-4f7762dfc4d6"))?;
    assert_eq!(us.mbid(), Some("5b11f4ce-a62d-471e-81fc-a69a8278c7da"));
    assert_eq!(us.deezer(), Some(415));
    assert_eq!(uk.mbid(), Some("9282c8b4-ca0b-4c6b-b7e3-4f7762dfc4d6"));
    assert_eq!(uk.deezer(), None, "never the other Nirvana's Deezer id");
    Ok(())
}
