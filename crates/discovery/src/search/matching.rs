//! Finding a song another service names (a playlist link) in lull's catalog, for the `Catalog`
//! seam. The library's copy wins; then the fast catalog: Deezer by ISRC or by name where it is
//! open, else iTunes by name (Deezer by ISRC after that, as a lookup by id answers everywhere).
//! A hit counts when it is the same song by search's rule (`merge::same_song`), with each artist
//! the service credits tried, so "Tyler, The Creator" and "A, B" both find theirs.

use async_trait::async_trait;
use lull_core::Result;
use lull_core::ids::loose_artist;
use lull_core::model::{SongQuery, Track};
use lull_core::seams::Catalog;

use super::catalog::Choice;
use super::describe;
use super::merge::{Found, Merged, SongHit, same_song};
use crate::Service;

/// Library hits looked at.
const SAVED: usize = 10;

#[async_trait]
impl Catalog for Service {
    async fn find_song(&self, song: &SongQuery) -> Result<Option<Track>> {
        let wanted = wanted(song);
        let query = format!("{} {}", credits(&song.artist)[0], song.title);
        let saved = self.store.search_saved(&query, SAVED).await.unwrap_or_else(|e| {
            tracing::info!(error = %e, "matching a song without the library");
            Vec::new()
        });
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
}

impl Service {
    async fn catalog_hit(&self, song: &SongQuery, query: &str, wanted: &[SongHit]) -> Result<Option<SongHit>> {
        let by_isrc = || async {
            match &song.isrc {
                Some(isrc) => self.deezer.by_isrc(isrc).await,
                None => Ok(None),
            }
        };
        let pick = |hits: Vec<SongHit>| hits.into_iter().find(|h| is_wanted(h, wanted));
        Ok(match self.catalog.choice() {
            Choice::Deezer => match by_isrc().await? {
                Some(hit) => Some(hit),
                None => pick(self.deezer.tracks(query).await?),
            },
            Choice::Itunes => match pick(self.itunes.match_songs(query).await?) {
                Some(hit) => Some(hit),
                None => by_isrc().await?,
            },
        })
    }
}

/// The song as search rows, once per artist credited: the whole credit first, then each name.
fn wanted(song: &SongQuery) -> Vec<SongHit> {
    let mut artists = vec![song.artist.clone()];
    artists.extend(credits(&song.artist).into_iter().filter(|a| *a != song.artist));
    artists
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
    wanted.iter().any(|w| same_song(hit, w))
}

/// The names in a credit, main artist first: "A, B & C feat. D" is A, B, C and D.
pub fn credits(artist: &str) -> Vec<String> {
    let mut names = vec![artist.to_owned()];
    for sep in [
        ", ",
        " & ",
        " x ",
        " X ",
        " feat. ",
        " ft. ",
        " featuring ",
        " with ",
        "; ",
    ] {
        names = names
            .into_iter()
            .flat_map(|n| n.split(sep).map(str::to_owned).collect::<Vec<_>>())
            .collect();
    }
    let mut names: Vec<String> = names
        .into_iter()
        .map(|n| n.trim().to_owned())
        .filter(|n| !loose_artist(n).is_empty())
        .collect();
    if names.is_empty() {
        names.push(artist.trim().to_owned());
    }
    names
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
    fn a_credit_splits_into_its_artists_main_first() {
        assert_eq!(
            credits("Daft Punk, Pharrell Williams"),
            ["Daft Punk", "Pharrell Williams"]
        );
        assert_eq!(credits("A & B feat. C"), ["A", "B", "C"]);
        assert_eq!(credits("Adele"), ["Adele"]);
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
}
