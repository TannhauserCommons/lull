//! Finding a song another service names (a playlist link) in lull's catalog, for the `Catalog`
//! seam. The library's copy wins; then Deezer by ISRC, a lookup that answers in every country
//! and within Deezer's roomy limit; then the fast catalog by name: Deezer where it is open, else
//! iTunes, whose small budget is kept for the songs that come without an ISRC.
//! A hit counts when it is the same song by search's rule (`merge::same_take`: the same take, so
//! a line "Shelter (Them Jeans drum edit)" never finds the plain "Shelter"), with the whole
//! credit and each name in it tried (`radio::credited_names`, radio's own reading of credits), so
//! "Tyler, The Creator" and "A, B" both find theirs.
//! Names in different scripts can't be compared (ミドリ in the link, "Midori" in the store): then
//! the store's top hit for the query counts when its length is within search's tolerance and the
//! names in one script agree (`across_scripts`). A change to these rules bumps
//! `seams::FIND_SONG_RULE`.

use async_trait::async_trait;
use lull_core::Result;
use lull_core::model::{SongQuery, Track};
use lull_core::seams::Catalog;

use super::catalog::Choice;
use super::describe;
use super::merge::{Found, Merged, SongHit, same_song, same_take};
use crate::Service;
use crate::radio::credited_names;

/// Library hits looked at.
const SAVED: usize = 10;

#[async_trait]
impl Catalog for Service {
    async fn find_song(&self, song: &SongQuery) -> Result<Option<Track>> {
        let wanted = wanted(song);
        let names = credited_names(&song.artist);
        // The first name of the credit, not the whole: "Daft Punk, Pharrell Williams Get Lucky"
        // finds less than "Daft Punk Get Lucky".
        let query = format!("{} {}", names.get(1).unwrap_or(&song.artist), song.title);
        let saved = self.store.search_saved(&query, SAVED).await?;
        if let Some(t) = saved.into_iter().find(|t| is_wanted(&super::saved(t.clone()), &wanted)) {
            return Ok(Some(t));
        }
        let Some(hit) = self.catalog_hit(song, &query, &wanted).await? else {
            return Ok(None);
        };
        let id = hit.id();
        let mut merged = Merged::default();
        merged.add(Found {
            songs: vec![hit],
            ..Found::default()
        });
        let pictured = describe(self, &merged, &[0], &[], &[]).await?;
        Ok(pictured.tracks.get(&id).cloned())
    }

    fn same_take(&self, song: &SongQuery, track: &Track) -> bool {
        is_wanted(&super::saved(track.clone()), &wanted(song))
    }
}

impl Service {
    async fn catalog_hit(&self, song: &SongQuery, query: &str, wanted: &[SongHit]) -> Result<Option<SongHit>> {
        let by_isrc = || async {
            match &song.isrc {
                Some(isrc) => self.deezer.by_isrc(isrc).await,
                None => Ok(None),
            }
        };
        if let Some(hit) = by_isrc().await? {
            return Ok(Some(hit));
        }
        Ok(match self.catalog.choice() {
            Choice::Deezer => pick(self.deezer.tracks(query).await?, wanted),
            Choice::Itunes => pick(self.itunes.match_songs(query).await?, wanted),
        })
    }
}

/// The song as search rows, once per artist credited: the whole credit first, then each name.
fn wanted(song: &SongQuery) -> Vec<SongHit> {
    credited_names(&song.artist)
        .into_iter()
        .map(|artist| SongHit {
            title: song.title.clone(),
            artist,
            duration: song.duration,
            isrcs: song.isrc.iter().cloned().collect(),
            ..SongHit::default()
        })
        .collect()
}

fn is_wanted(hit: &SongHit, wanted: &[SongHit]) -> bool {
    wanted.iter().any(|w| same_take(hit, w))
}

/// The store's hit for the song: the first that is the same song by name, else the top hit when
/// it is named in another script.
fn pick(hits: Vec<SongHit>, wanted: &[SongHit]) -> Option<SongHit> {
    let top = hits.first().filter(|h| across_scripts(h, wanted)).cloned();
    hits.into_iter().find(|h| is_wanted(h, wanted)).or(top)
}

/// Whether `hit` is the song though its artist or title is in another script than the link's:
/// both lengths known and within search's tolerance, and the names in one script the same song
/// by search's rule. Each name in another script is taken as agreeing.
fn across_scripts(hit: &SongHit, wanted: &[SongHit]) -> bool {
    wanted.iter().any(|w| {
        let (artist, title) = (apart(&hit.artist, &w.artist), apart(&hit.title, &w.title));
        let read = SongHit {
            artist: if artist { w.artist.clone() } else { hit.artist.clone() },
            title: if title { w.title.clone() } else { hit.title.clone() },
            duration: hit.duration,
            ..SongHit::default()
        };
        (artist || title) && hit.duration.is_some() && w.duration.is_some() && same_song(&read, w)
    })
}

#[derive(PartialEq)]
enum Script {
    Latin,
    Other,
    /// No letters (digits, signs): comparable with either.
    None,
}

/// A name's script, as far as comparing names goes: any letter outside Latin makes it `Other`.
fn script(name: &str) -> Script {
    let latin = |c: char| {
        c.is_ascii_alphabetic() || ('\u{00AA}'..='\u{024F}').contains(&c) || ('\u{1E00}'..='\u{1EFF}').contains(&c)
    };
    let mut seen = Script::None;
    for c in name.chars().filter(|c| c.is_alphabetic()) {
        if !latin(c) {
            return Script::Other;
        }
        seen = Script::Latin;
    }
    seen
}

/// Whether two names can't be compared: one in Latin letters, the other not.
fn apart(a: &str, b: &str) -> bool {
    matches!(
        (script(a), script(b)),
        (Script::Latin, Script::Other) | (Script::Other, Script::Latin)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(artist: &str, title: &str, duration: Option<u32>) -> SongHit {
        SongHit {
            artist: artist.into(),
            title: title.into(),
            duration,
            ..SongHit::default()
        }
    }

    fn query(artist: &str, title: &str, duration: Option<u32>) -> SongQuery {
        SongQuery {
            artist: artist.into(),
            title: title.into(),
            duration,
            ..SongQuery::default()
        }
    }

    #[test]
    fn a_song_credited_to_several_artists_matches_the_catalogs_main_artist() {
        let w = wanted(&query("Daft Punk, Pharrell Williams", "Get Lucky", Some(369)));
        assert!(is_wanted(
            &hit("Daft Punk", "Get Lucky (feat. Pharrell Williams)", Some(369)),
            &w
        ));
    }

    #[test]
    fn an_artist_whose_name_holds_a_comma_still_matches_as_a_whole() {
        let w = wanted(&query("Tyler, The Creator", "EARFQUAKE", Some(190)));
        assert!(is_wanted(&hit("Tyler, The Creator", "EARFQUAKE", Some(190)), &w));
    }

    #[test]
    fn a_remaster_tag_and_a_close_length_still_match() {
        let w = wanted(&query("The Beatles", "Hey Jude - Remastered 2015", Some(429)));
        assert!(is_wanted(
            &hit("The Beatles", "Hey Jude (Remastered 2015)", Some(431)),
            &w
        ));
    }

    #[test]
    fn a_song_named_in_kana_matches_the_stores_romaji_top_hit_of_its_length() {
        let w = wanted(&query("ミドリ", "ゆきこさん", Some(188)));
        let found = pick(
            vec![hit("Midori", "Yukikosan", Some(188)), hit("Midori", "Swing", Some(201))],
            &w,
        );
        assert_eq!(found.map(|h| h.title), Some("Yukikosan".to_owned()));
    }

    #[test]
    fn across_scripts_only_the_top_hit_of_a_close_length_counts() {
        let w = wanted(&query("ミドリ", "ゆきこさん", Some(188)));
        assert!(pick(vec![hit("Midori", "Yukikosan", Some(240))], &w).is_none());
        assert!(
            pick(
                vec![hit("Midori", "Swing", Some(240)), hit("Midori", "Yukikosan", Some(188))],
                &w
            )
            .is_none()
        );
        assert!(pick(vec![hit("Midori", "Yukikosan", None)], &w).is_none());
    }

    #[test]
    fn across_scripts_a_name_in_the_same_script_must_still_agree() {
        let w = wanted(&query("YOASOBI", "夜に駆ける", Some(261)));
        assert!(pick(vec![hit("YOASOBI", "Yoru ni Kakeru", Some(261))], &w).is_some());
        assert!(pick(vec![hit("Ado", "Yoru ni Kakeru", Some(261))], &w).is_none());
    }

    #[test]
    fn a_latin_name_with_another_title_is_still_unmatched() {
        let w = wanted(&query("The Beatles", "Hey Jude", Some(429)));
        assert!(pick(vec![hit("The Beatles", "Let It Be", Some(429))], &w).is_none());
        // Accented Latin is Latin: compared by name, as before.
        let w = wanted(&query("Beyoncé", "Halo", Some(261)));
        assert!(pick(vec![hit("Beyonce", "Single Ladies", Some(261))], &w).is_none());
    }

    #[test]
    fn another_artist_or_another_length_does_not_match() {
        let w = wanted(&query("The Beatles", "Hey Jude", Some(429)));
        assert!(!is_wanted(&hit("The Beatles", "Hey Jude", Some(240)), &w));
        assert!(!is_wanted(&hit("Wilson Pickett", "Hey Jude", Some(429)), &w));
    }

    #[test]
    fn a_shared_isrc_matches_whatever_the_names() {
        let w = wanted(&SongQuery {
            isrc: Some("GBUM71505902".into()),
            ..query("Beatles", "Hey Jude", None)
        });
        let mut h = hit("The Beatles", "Hey Jude (2015 Mix)", Some(431));
        h.isrcs = vec!["GBUM71505902".into()];
        assert!(is_wanted(&h, &w));
    }

    #[test]
    fn without_a_length_names_alone_decide() {
        let w = wanted(&query("Olivia Rodrigo", "drop dead", None));
        assert!(is_wanted(&hit("Olivia Rodrigo", "drop dead", Some(251)), &w));
    }

    #[test]
    fn a_song_in_capitals_or_with_its_featured_artist_is_the_same_song() {
        let catalog = hit("Luude", "Down Under (feat. Colin Hay)", Some(222));
        assert!(is_wanted(
            &catalog,
            &wanted(&query("LUUDE", "DOWN UNDER (FEAT. COLIN HAY)", Some(222)))
        ));
        assert!(
            is_wanted(&catalog, &wanted(&query("Luude", "Down Under", None))),
            "feat. aside"
        );
        assert!(is_wanted(
            &catalog,
            &wanted(&query("Luude, Colin Hay", "Down Under", Some(223)))
        ));
    }

    #[test]
    fn an_edit_or_a_live_take_is_not_the_original_when_a_length_is_missing() {
        // A playlist line may come without a length, and search's own rule let any length pass.
        let edit = wanted(&query("The xx", "Shelter (Them Jeans Drum Edit)", None));
        assert!(!is_wanted(&hit("The xx", "Shelter", Some(270)), &edit));
        assert!(is_wanted(
            &hit("The xx", "Shelter (Them Jeans Drum Edit)", Some(301)),
            &edit
        ));
        let original = wanted(&query("The xx", "Shelter", None));
        assert!(!is_wanted(&hit("The xx", "Shelter (Live)", Some(284)), &original));
        assert!(!is_wanted(&hit("The xx", "Shelter - Radio Edit", None), &original));
    }

    #[test]
    fn a_remix_is_not_the_original() {
        let original = wanted(&query("Luude", "Down Under (feat. Colin Hay)", None));
        assert!(!is_wanted(
            &hit("Luude", "Down Under (feat. Colin Hay) [Majestic Remix]", None),
            &original
        ));
        assert!(!is_wanted(
            &hit("Luude", "Down Under - Majestic Remix", None),
            &original
        ));
    }
}
