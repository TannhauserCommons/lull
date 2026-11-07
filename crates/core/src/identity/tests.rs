//! ADR-0035's rule, case by case ("The rule against every case").

use rusqlite::{Connection, params};

use super::*;

type R = std::result::Result<(), Box<dyn std::error::Error>>;

fn hub() -> std::result::Result<Connection, Box<dyn std::error::Error>> {
    let conn = Connection::open_in_memory()?;
    for m in [&crate::db::CORE, &crate::catalog::MIGRATIONS, &MIGRATIONS] {
        for step in m.steps {
            conn.execute_batch(step)?;
        }
    }
    Ok(conn)
}

fn mb(id: &str) -> Anchor {
    Anchor::Mbid(id.into())
}

fn album(id: &str) -> Anchor {
    Anchor::Album(AlbumId(id.into()))
}

const US: &str = "5b11f4ce-a62d-471e-81fc-a69a8278c7da";
const UK: &str = "9282c8b4-ca0b-4c6b-b7e3-4f7762dfc4d6";

fn nirvana() -> ArtistId {
    ArtistId::of("Nirvana")
}

#[test]
fn a_name_with_one_artist_keeps_the_names_id() -> R {
    let c = hub()?;
    assert_eq!(
        place(&c, "Nirvana", &[], Came::Credit)?,
        Place::One(nirvana()),
        "a bare name"
    );
    assert_eq!(place(&c, "Nirvana", &[mb(US)], Came::Credit)?, Place::One(nirvana()));
    assert_eq!(
        place(&c, "Nirvana", &[album("al-nevermind")], Came::Lookup)?,
        Place::One(nirvana())
    );
    assert_eq!(resolve(&c, &nirvana())?, Place::One(nirvana()));
    let none: i64 = c.query_row(
        "SELECT COUNT(*) FROM identity_anchors WHERE anchor LIKE 'you:%'",
        [],
        |r| r.get(0),
    )?;
    assert_eq!(none, 0);
    Ok(())
}

#[test]
fn a_bare_name_writes_nothing() -> R {
    let c = hub()?;
    place(&c, "Nirvana", &[], Came::Credit)?;
    let rows: i64 = c.query_row(
        "SELECT (SELECT COUNT(*) FROM identity_artists) + (SELECT COUNT(*) FROM identity_called)",
        [],
        |r| r.get(0),
    )?;
    assert_eq!(rows, 0);
    Ok(())
}

#[test]
fn an_artist_with_no_ids_stays_whole() -> R {
    let c = hub()?;
    for al in ["al-one", "al-two", "al-three", "al-four", "al-five"] {
        assert_eq!(
            place(&c, "Velvet Orchard", &[album(al)], Came::Lookup)?,
            Place::One(ArtistId::of("Velvet Orchard"))
        );
    }
    assert_eq!(artists_named(&c, "Velvet Orchard")?, [ArtistId::of("Velvet Orchard")]);
    Ok(())
}

#[test]
fn two_mbids_make_two_artists_and_the_name_a_page() -> R {
    let c = hub()?;
    place(&c, "Nirvana", &[mb(US), album("al-nevermind")], Came::Credit)?;
    let uk = place(&c, "Nirvana", &[mb(UK)], Came::Credit)?;
    let us_own = ArtistId::own("Nirvana", &mb(US).key());
    let uk_own = ArtistId::own("Nirvana", &mb(UK).key());
    assert_eq!(uk, Place::One(uk_own.clone()));
    assert_eq!(
        artists_named(&c, "Nirvana")?,
        [us_own.clone(), uk_own.clone()],
        "oldest first"
    );
    assert_eq!(
        place(&c, "Nirvana", &[], Came::Credit)?,
        Place::Several(vec![us_own.clone(), uk_own.clone()])
    );
    assert_eq!(
        resolve(&c, &nirvana())?,
        Place::Several(vec![us_own.clone(), uk_own.clone()])
    );
    assert_eq!(resolve(&c, &uk_own)?, Place::One(uk_own.clone()));
    // The first one's anchors moved with it.
    assert_eq!(artist_of(&c, "Nirvana", &[album("al-nevermind")])?, Place::One(us_own));
    assert_eq!(
        Place::Several(vec![]).id("Nirvana"),
        nirvana(),
        "a song of either carries the name's id"
    );
    Ok(())
}

#[test]
fn a_work_found_nowhere_fits_every_artist_of_its_name() -> R {
    let c = hub()?;
    place(&c, "Nirvana", &[mb(US)], Came::Credit)?;
    place(&c, "Nirvana", &[mb(UK)], Came::Credit)?;
    let lost = place(&c, "Nirvana", &[album("al-bootleg")], Came::Lookup)?;
    assert!(matches!(lost, Place::Several(ref v) if v.len() == 2), "{lost:?}");
    assert_eq!(
        artist_of(&c, "Nirvana", &[album("al-bootleg")])?,
        lost,
        "and nothing was kept"
    );
    Ok(())
}

#[test]
fn a_work_with_its_artists_id_lands_on_that_artist() -> R {
    let c = hub()?;
    place(&c, "Nirvana", &[mb(US)], Came::Credit)?;
    let uk = place(&c, "Nirvana", &[mb(UK)], Came::Credit)?;
    assert_eq!(
        place(&c, "Nirvana", &[mb(UK), album("al-simon-simopath")], Came::Lookup)?,
        uk
    );
    assert_eq!(
        artist_of(
            &c,
            "Nirvana",
            &[Anchor::Track(TrackId("t-pentecost".into())), album("al-simon-simopath")]
        )?,
        uk
    );
    Ok(())
}

#[test]
fn one_band_under_two_spellings_is_one_artist_and_among_both_names() -> R {
    let c = hub()?;
    place(&c, "Nirvana", &[mb(US)], Came::Credit)?;
    let deezer = place(&c, "Nirvana (UK)", &[Anchor::Deezer(281_527_041)], Came::Credit)?;
    assert_eq!(deezer, Place::One(ArtistId::of("Nirvana (UK)")));
    // MusicBrainz links its MBID to that Deezer artist.
    let linked = place(&c, "Nirvana", &[mb(UK), Anchor::Deezer(281_527_041)], Came::Link)?;
    assert_eq!(linked, deezer, "the shared Deezer id joins them across names");
    let named = artists_named(&c, "Nirvana")?;
    assert_eq!(named.len(), 2, "the UK band is one of the Nirvanas now: {named:?}");
    assert!(named.contains(&ArtistId::of("Nirvana (UK)")));
    assert_eq!(
        resolve(&c, &ArtistId::of("Nirvana (UK)"))?,
        deezer,
        "its own name still opens it"
    );
    assert!(matches!(resolve(&c, &nirvana())?, Place::Several(_)));
    Ok(())
}

#[test]
fn a_musicbrainz_merge_makes_one_artist_and_the_old_id_an_alias() -> R {
    let c = hub()?;
    place(&c, "Nirvana", &[mb(US)], Came::Credit)?;
    let old = place(&c, "Nirvana", &[mb(UK)], Came::Credit)?;
    let new = place(
        &c,
        "Nirvana",
        &[mb("00000000-0000-0000-0000-000000000003")],
        Came::Credit,
    )?;
    merged_mbid(&c, UK, "00000000-0000-0000-0000-000000000003")?;
    let (Place::One(old), Place::One(new)) = (old, new) else {
        return Err("one artist each".into());
    };
    assert_eq!(alias(&c, &new)?, old, "the newer is an alias of the older");
    assert_eq!(resolve(&c, &new)?, Place::One(old));
    assert_eq!(artists_named(&c, "Nirvana")?.len(), 2);
    Ok(())
}

#[test]
fn anchors_that_name_two_different_artists_keep_nothing_and_the_weightiest_wins() -> R {
    let c = hub()?;
    place(&c, "Nirvana", &[mb(US), album("al-nevermind")], Came::Credit)?;
    let uk = place(&c, "Nirvana", &[mb(UK)], Came::Credit)?;
    // A wrong credit: the UK band's MBID on the US band's album.
    assert_eq!(
        place(&c, "Nirvana", &[mb(UK), album("al-nevermind")], Came::Credit)?,
        uk
    );
    assert_eq!(
        artist_of(&c, "Nirvana", &[album("al-nevermind")])?,
        Place::One(ArtistId::own("Nirvana", &mb(US).key())),
        "the album stays where it was"
    );
    Ok(())
}

#[test]
fn the_listeners_word_tells_apart_two_bands_no_service_knows() -> R {
    let c = hub()?;
    place(&c, "Velvet Orchard", &[album("al-one")], Came::Lookup)?;
    place(&c, "Velvet Orchard", &[album("al-two")], Came::Lookup)?;
    let other = listener_splits(&c, "Velvet Orchard", &[album("al-two")])?;
    let named = artists_named(&c, "Velvet Orchard")?;
    assert_eq!(named.len(), 2, "{named:?}");
    assert_eq!(
        artist_of(&c, "Velvet Orchard", &[album("al-two")])?,
        Place::One(other.clone())
    );
    assert_ne!(
        artist_of(&c, "Velvet Orchard", &[album("al-one")])?,
        Place::One(other.clone())
    );
    // A service's id later never joins either on its own.
    let found = place(&c, "Velvet Orchard", &[mb(US)], Came::Credit)?;
    assert!(
        !named.contains(&found.id("Velvet Orchard")),
        "a third, not one of the listener's"
    );
    Ok(())
}

#[test]
fn the_listeners_placing_replaces_a_services() -> R {
    let c = hub()?;
    place(&c, "Nirvana", &[mb(US), album("al-live")], Came::Credit)?;
    let uk = place(&c, "Nirvana", &[mb(UK)], Came::Credit)?;
    let Place::One(uk) = uk else {
        return Err("one artist".into());
    };
    listener_places(&c, &album("al-live"), &uk)?;
    assert_eq!(artist_of(&c, "Nirvana", &[album("al-live")])?, Place::One(uk.clone()));
    place(&c, "Nirvana", &[mb(US), album("al-live")], Came::Credit)?;
    assert_eq!(
        artist_of(&c, "Nirvana", &[album("al-live")])?,
        Place::One(uk),
        "a credit doesn't undo it"
    );
    Ok(())
}

/// The owner's rule: what the listener did before stays with the artist their songs point to.
#[test]
fn a_names_id_from_before_is_the_artist_the_listeners_songs_are_on() -> R {
    let c = hub()?;
    c.execute(
        "INSERT INTO plays (track_id, artist, title, album, duration_s, started_at) VALUES ('t-lithium', 'Nirvana', 'Lithium', 'Nevermind', 257, 1)",
        [],
    )?;
    c.execute(
        "INSERT INTO catalog_tracks (id, json, updated_at) VALUES ('t-lithium', ?1, 1)",
        params![r#"{"albumId":"al-nevermind","artist":"Nirvana"}"#],
    )?;
    place(&c, "Nirvana", &[mb(US), album("al-nevermind")], Came::Credit)?;
    place(&c, "Nirvana", &[mb(UK)], Came::Credit)?;
    assert_eq!(owner(&c, &nirvana())?, Some(ArtistId::own("Nirvana", &mb(US).key())));
    Ok(())
}

#[test]
fn a_names_id_is_nobodys_when_the_listeners_songs_are_on_two_or_none() -> R {
    let c = hub()?;
    for (t, al) in [("t-lithium", "al-nevermind"), ("t-pentecost", "al-simon")] {
        c.execute(
            "INSERT INTO likes (track_id, artist, title, album, liked_at) VALUES (?1, 'Nirvana', ?1, ?2, 1)",
            params![t, al],
        )?;
        c.execute(
            "INSERT INTO catalog_tracks (id, json, updated_at) VALUES (?1, ?2, 1)",
            params![t, format!(r#"{{"albumId":"{al}","artist":"Nirvana"}}"#)],
        )?;
    }
    place(&c, "Nirvana", &[mb(US), album("al-nevermind")], Came::Credit)?;
    place(&c, "Nirvana", &[mb(UK), album("al-simon")], Came::Credit)?;
    assert_eq!(owner(&c, &nirvana())?, None, "songs of both: which one?");
    let uk = ArtistId::own("Nirvana", &mb(UK).key());
    listener_chooses(&c, &nirvana(), &uk)?;
    assert_eq!(owner(&c, &nirvana())?, Some(uk));
    Ok(())
}

#[test]
fn a_list_of_namesakes_adds_each_as_its_own_artist_and_joins_none_by_name() -> R {
    let c = hub()?;
    // lull knows one Nirvana by a Deezer id only, and songs of the name with no id at all.
    place(&c, "Nirvana", &[Anchor::Deezer(415)], Came::Credit)?;
    place(&c, "Nirvana", &[Anchor::Deezer(278_793_911)], Came::Credit)?;
    // MusicBrainz's link proves which MBID the first is.
    place(&c, "Nirvana", &[Anchor::Deezer(415), mb(US)], Came::Link)?;
    let us = add_distinct(&c, "Nirvana", &[mb(US)], Came::Credit)?;
    let uk = add_distinct(&c, "Nirvana", &[mb(UK)], Came::Credit)?;
    assert_eq!(
        us,
        ArtistId::own("Nirvana", "dz:415"),
        "the linked one is the one lull had"
    );
    assert_ne!(uk, us);
    assert_ne!(uk, ArtistId::own("Nirvana", "dz:278793911"), "never joined by name");
    assert_eq!(artists_named(&c, "Nirvana")?.len(), 3);
    Ok(())
}
