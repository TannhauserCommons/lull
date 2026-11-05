//! The catalog's previews of a track, for the `Previews` seam: the reference a copy's sound is
//! checked against (ADR-0016). Only the catalog row that is the track itself counts: the one whose
//! artist, album and title make the track's id, as search made it. A row found by name alone can
//! be a re-recording under the same title (a band's later take of its own song), whose sound
//! would refuse the right copy. Apple first, then Deezer where Deezer answers; each lookup is
//! background work, kept a week.

use async_trait::async_trait;
use lull_core::Result;
use lull_core::model::Track;
use lull_core::seams::Previews;
use lull_core::youtube::Reference;

use crate::Service;
use crate::radio::credited_names;
use crate::sources::itunes::Previewed;

#[async_trait]
impl Previews for Service {
    async fn previews(&self, track: &Track) -> Result<Vec<Reference>> {
        let names = credited_names(&track.artist);
        let query = format!("{} {}", names.get(1).unwrap_or(&track.artist), track.title);
        let mut found = Vec::new();
        match self.itunes.previews(&query).await {
            Ok(rows) => found.extend(own(track, rows, "apple")),
            Err(e) => tracing::debug!(error = %e, track = %track.id, "Apple's previews"),
        }
        match self.deezer.previews(&query).await {
            Ok(rows) => found.extend(own(track, rows, "deezer")),
            Err(e) => tracing::debug!(error = %e, track = %track.id, "Deezer's previews"),
        }
        Ok(found)
    }
}

/// The rows that are `track` itself, as references.
pub fn own(track: &Track, rows: Vec<Previewed>, source: &str) -> Option<Reference> {
    rows.into_iter().find(|r| r.hit.id() == track.id).map(|r| Reference {
        source: source.to_owned(),
        id: r.id.to_string(),
        url: r.url,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::sources::itunes;

    #[test]
    fn only_the_catalog_row_that_is_the_track_is_its_reference() {
        // Apple holds the 2008 song and its re-recording under nearly the same name: only the
        // row whose artist, album and title make the track's id is its reference.
        let body = json!({ "results": [
            { "wrapperType": "track", "trackId": 2, "trackName": "Love Story (Taylor's Version)", "artistName": "Taylor Swift",
              "collectionName": "Fearless (Taylor's Version)", "trackTimeMillis": 235_000,
              "previewUrl": "https://audio-ssl.itunes.apple.com/tv.m4a" },
            { "wrapperType": "track", "trackId": 1, "trackName": "Love Story", "artistName": "Taylor Swift",
              "collectionName": "Fearless", "trackTimeMillis": 235_000,
              "previewUrl": "https://audio-ssl.itunes.apple.com/2008.m4a" },
            { "wrapperType": "track", "trackId": 3, "trackName": "Love Story", "artistName": "Taylor Swift",
              "collectionName": "Fearless (Platinum Edition)", "trackTimeMillis": 235_000 }
        ] });
        let rows = itunes::previewed(body).unwrap_or_default();
        assert_eq!(rows.len(), 2, "a row without a preview is none");
        let track = crate::store::track(rows[1].hit.song());
        let reference = own(&track, rows, "apple");
        assert_eq!(
            reference.map(|r| (r.id, r.url)),
            Some(("1".to_owned(), "https://audio-ssl.itunes.apple.com/2008.m4a".to_owned()))
        );
    }

    #[test]
    fn a_store_that_answers_nothing_gives_no_reference() {
        // Apple's search answers nothing for some storefronts (Korea's, 2026-11-05).
        let rows = itunes::previewed(json!({ "resultCount": 0, "results": [] })).unwrap_or_default();
        let hit = crate::search::merge::SongHit {
            title: "Good Day".into(),
            artist: "IU".into(),
            album: "Real".into(),
            ..crate::search::merge::SongHit::default()
        };
        let track = crate::store::track(hit.song());
        assert_eq!(own(&track, rows, "apple"), None);
    }
}
