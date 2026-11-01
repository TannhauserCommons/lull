//! Spotify playlists as Exportify writes them (https://exportify.net): a CSV with one row per
//! song. Each row becomes an import entry, so the preview and the matching are the M3U import's.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::routing::post;
use axum::{Json, Router};
use lull_core::{Error, Result};
use serde_json::Value;

use crate::m3u::{self, Entry};
use crate::{Service, import};

pub(crate) fn routes() -> Router<Arc<Service>> {
    Router::new()
        .route("/api/import/exportify", post(preview))
        // A few thousand rows with every Spotify column outgrow axum's 2 MB default.
        .layer(DefaultBodyLimit::max(16 * 1024 * 1024))
}

async fn preview(State(svc): State<Arc<Service>>, body: Bytes) -> Result<Json<Value>> {
    let entries = parse(&m3u::decode(&body))?;
    import::preview_entries(&svc, None, entries).await.map(Json)
}

/// The songs of an Exportify CSV. Columns are found by their header, so their order and any
/// extra columns (popularity, audio features) don't matter.
fn parse(text: &str) -> Result<Vec<Entry>> {
    let mut rows = rows(text).into_iter();
    let header: Vec<String> = rows
        .next()
        .unwrap_or_default()
        .into_iter()
        .map(|h| h.trim().to_lowercase())
        .collect();
    let column = |names: &[&str]| header.iter().position(|h| names.contains(&h.as_str()));
    let (Some(title), Some(artists)) = (column(&["track name"]), column(&["artist name(s)", "artist name"])) else {
        return Err(Error::BadRequest(
            "This isn't an Exportify file: it has no \"Track Name\" and \"Artist Name(s)\" columns.".into(),
        ));
    };
    let (album, uri, duration) = (
        column(&["album name"]),
        column(&["track uri"]),
        column(&["duration (ms)"]),
    );

    let field = |row: &[String], i: Option<usize>| {
        i.and_then(|i| row.get(i))
            .map(|f| f.trim())
            .filter(|f| !f.is_empty())
            .map(str::to_owned)
    };
    Ok(rows
        .filter_map(|row| {
            let title = field(&row, Some(title))?;
            // Spotify joins a song's artists with ";": the first is the main one, the id's.
            let artist = field(&row, Some(artists))?
                .split(';')
                .next()
                .map(|a| a.trim().to_owned())
                .filter(|a| !a.is_empty())?;
            Some(Entry {
                location: field(&row, uri).unwrap_or_default(),
                artist: Some(artist),
                album: field(&row, album),
                title: Some(title),
                duration: field(&row, duration)
                    .and_then(|ms| ms.parse::<u64>().ok())
                    .and_then(|ms| u32::try_from(ms / 1000).ok()),
                track_id: None,
            })
        })
        .collect())
}

/// RFC 4180 rows: fields may be quoted, hold commas and line breaks, and escape `"` as `""`.
fn rows(text: &str) -> Vec<Vec<String>> {
    let (mut rows, mut row, mut field) = (Vec::new(), Vec::new(), String::new());
    let (mut quoted, mut chars) = (false, text.chars().peekable());
    while let Some(c) = chars.next() {
        match (c, quoted) {
            ('"', true) if chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            ('"', _) => quoted = !quoted,
            (',', false) => row.push(std::mem::take(&mut field)),
            ('\r', false) => {}
            ('\n', false) => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            (c, _) => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    // A blank line is a row of one empty field.
    rows.retain(|r| r.iter().any(|f| !f.is_empty()));
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "\u{feff}\"Track URI\",\"Track Name\",\"Album Name\",\"Artist Name(s)\",\"Release Date\",\"Duration (ms)\",\"Popularity\"\r\n\
        \"spotify:track:1\",\"Paper Moon, Static\",\"Satellite Porch\",\"Velvet Orchard;Moth & Lantern\",\"2019-05-03\",\"201500\",\"41\"\r\n\
        \"spotify:track:2\",\"Say \"\"Hello\"\"\",\"\",\"Low Tide Radio\",\"2020\",\"abc\",\"7\"\r\n\
        \r\n\
        \"spotify:track:3\",\"\",\"No Title\",\"Nobody\",\"2020\",\"1000\",\"1\"\r\n";

    #[test]
    fn exportify_rows_become_entries_with_the_main_artist() -> Result<()> {
        let entries = parse(&m3u::decode(FILE.as_bytes()))?;
        assert_eq!(entries.len(), 2, "the row without a title is dropped");
        assert_eq!(entries[0].title.as_deref(), Some("Paper Moon, Static"));
        assert_eq!(entries[0].artist.as_deref(), Some("Velvet Orchard"));
        assert_eq!(entries[0].album.as_deref(), Some("Satellite Porch"));
        assert_eq!(entries[0].duration, Some(201));
        assert_eq!(entries[0].location, "spotify:track:1");
        assert_eq!(entries[1].title.as_deref(), Some("Say \"Hello\""));
        assert_eq!((entries[1].album.clone(), entries[1].duration), (None, None));
        Ok(())
    }

    #[test]
    fn a_csv_without_the_exportify_columns_is_refused() {
        assert!(matches!(parse("name,band\nx,y\n"), Err(Error::BadRequest(_))));
    }

    #[test]
    fn quoted_fields_keep_commas_and_line_breaks() {
        assert_eq!(
            rows("a,\"b,\nc\",\"d\"\"e\"\n"),
            [["a", "b,\nc", "d\"e"].map(String::from).to_vec()]
        );
    }
}
