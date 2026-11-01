//! A ListenBrainz export: the listens file as a JSON array, as JSON Lines (the `.jsonl` files
//! of the export's zip), or as the API's `{ "payload": { "listens": [...] } }`.

use lull_core::{Error, Result};
use serde::Deserialize;

use crate::imported_plays::{BATCH, Listen, Progress};

/// Parses the upload into listens, so a wrong file is an answer to the request. Listens with no
/// time (the one playing now) are not listens yet and are left out.
pub(crate) fn parse(body: &[u8]) -> Result<Vec<Listen>> {
    if body.starts_with(b"PK") {
        return Err(Error::BadRequest(
            "This is a zip. Unzip it and upload the listens .json or .jsonl file inside.".into(),
        ));
    }
    let text = std::str::from_utf8(body).map_err(|_| Error::BadRequest("This file is not UTF-8 text.".into()))?;
    let items = match serde_json::from_str::<Document>(text) {
        Ok(Document::List(items)) => items,
        Ok(Document::Api { payload }) => payload.listens,
        // One listen per line.
        Err(_) => text
            .lines()
            .enumerate()
            .filter(|(_, line)| !line.trim().is_empty())
            .map(|(n, line)| {
                serde_json::from_str::<Item>(line)
                    .map_err(|_| Error::BadRequest(format!("Line {} is not a ListenBrainz listen.", n + 1)))
            })
            .collect::<Result<Vec<_>>>()?,
    };
    if items.is_empty() {
        return Err(Error::BadRequest("This file has no listens in it.".into()));
    }
    Ok(items.into_iter().filter_map(Item::into_listen).collect())
}

/// Stores the listens in batches, reporting as it goes.
pub(crate) async fn import(progress: &mut Progress, listens: Vec<Listen>) -> Result<()> {
    let total = listens.len();
    for (i, batch) in listens.chunks(BATCH).enumerate() {
        let done = ((i + 1) * BATCH).min(total);
        let pct = u8::try_from(done * 100 / total).ok();
        progress.add(batch.to_vec(), pct).await?;
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Document {
    List(Vec<Item>),
    Api { payload: Payload },
}

#[derive(Deserialize)]
struct Payload {
    listens: Vec<Item>,
}

#[derive(Deserialize)]
struct Item {
    listened_at: Option<i64>,
    track_metadata: Meta,
}

#[derive(Deserialize)]
struct Meta {
    #[serde(default)]
    artist_name: String,
    #[serde(default)]
    track_name: String,
    release_name: Option<String>,
    additional_info: Option<Info>,
}

#[derive(Deserialize)]
struct Info {
    duration_ms: Option<u64>,
    duration: Option<u64>,
}

impl Item {
    fn into_listen(self) -> Option<Listen> {
        let m = self.track_metadata;
        let duration = m.additional_info.and_then(|i| {
            i.duration
                .or_else(|| i.duration_ms.map(|ms| ms / 1000))
                .and_then(|s| u32::try_from(s).ok())
        });
        Some(Listen {
            artist: m.artist_name,
            album: m.release_name.unwrap_or_default(),
            title: m.track_name,
            started_at: self.listened_at?,
            duration,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LISTEN: &str = r#"{"listened_at":1700000000,"track_metadata":{"artist_name":"Velvet Orchard","track_name":"Paper Moon Static","release_name":"Satellite Porch","additional_info":{"duration_ms":201500}}}"#;

    #[test]
    fn the_array_the_api_payload_and_json_lines_all_read() -> Result<()> {
        for body in [
            format!("[{LISTEN}, {LISTEN}]"),
            format!(r#"{{"payload":{{"user_id":"moth","listens":[{LISTEN},{LISTEN}]}}}}"#),
            format!("{LISTEN}\n\n{LISTEN}\n"),
        ] {
            let listens = parse(body.as_bytes())?;
            assert_eq!(listens.len(), 2);
            assert_eq!(listens[0].artist, "Velvet Orchard");
            assert_eq!(listens[0].album, "Satellite Porch");
            assert_eq!((listens[0].started_at, listens[0].duration), (1_700_000_000, Some(201)));
        }
        Ok(())
    }

    #[test]
    fn what_is_not_an_export_says_so() {
        assert!(matches!(parse(b"PK\x03\x04..."), Err(Error::BadRequest(m)) if m.contains("zip")));
        assert!(matches!(parse(b"[]"), Err(Error::BadRequest(m)) if m.contains("no listens")));
        assert!(matches!(parse(b"hello"), Err(Error::BadRequest(m)) if m.contains("Line 1")));
    }

    #[test]
    fn the_listen_playing_now_has_no_time_and_is_left_out() -> Result<()> {
        let playing = r#"{"track_metadata":{"artist_name":"A","track_name":"T"}}"#;
        assert_eq!(parse(format!("[{playing},{LISTEN}]").as_bytes())?.len(), 1);
        Ok(())
    }
}
