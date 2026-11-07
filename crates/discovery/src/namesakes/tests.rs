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
