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
/// extra columns (popularity, audio features) don't matter. "Added At" is when the song was added
/// to the Spotify playlist: it becomes the song's date in lull's playlist.
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
    let (album, uri, duration, added) = (
        column(&["album name"]),
        column(&["track uri"]),
        column(&["duration (ms)"]),
        column(&["added at"]),
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
                added: field(&row, added).and_then(|at| unix_time(&at)),
            })
        })
        .collect())
}

/// Unix seconds of an ISO 8601 time as Exportify writes it ("2021-03-04T18:39:20Z"), with an
/// offset ("+02:00") or fractions of a second, or a bare date (midnight UTC). `None` for anything
/// else: the song is then dated by its place in the file.
fn unix_time(s: &str) -> Option<i64> {
    let s = s.trim();
    let num = |a: usize, b: usize| {
        s.get(a..b)
            .filter(|t| t.bytes().all(|c| c.is_ascii_digit()))?
            .parse::<i64>()
            .ok()
    };
    let (year, month, day) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    if s.get(4..5) != Some("-") || s.get(7..8) != Some("-") || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let mut at = days_from_civil(year, month, day) * 86_400;
    if s.len() == 10 {
        return Some(at);
    }
    if !matches!(s.get(10..11), Some("T" | " ")) || s.get(13..14) != Some(":") || s.get(16..17) != Some(":") {
        return None;
    }
    let (hour, minute, second) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    at += hour * 3600 + minute * 60 + second;
    let mut rest = &s[19..];
    if let Some(frac) = rest.strip_prefix('.') {
        rest = frac.trim_start_matches(|c: char| c.is_ascii_digit());
    }
    let offset = match rest {
        "" | "Z" | "z" => 0,
        _ => {
            let sign = match rest.get(..1)? {
                "+" => 1,
                "-" => -1,
                _ => return None,
            };
            let hm = rest[1..].replace(':', "");
            if hm.len() != 4 || !hm.bytes().all(|c| c.is_ascii_digit()) {
                return None;
            }
            sign * (hm[..2].parse::<i64>().ok()? * 3600 + hm[2..].parse::<i64>().ok()? * 60)
        }
    };
    Some(at - offset)
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Hinnant's days-from-civil).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
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

    const FILE: &str = "\u{feff}\"Track URI\",\"Track Name\",\"Album Name\",\"Artist Name(s)\",\"Release Date\",\"Duration (ms)\",\"Popularity\",\"Added By\",\"Added At\"\r\n\
        \"spotify:track:1\",\"Paper Moon, Static\",\"Satellite Porch\",\"Velvet Orchard;Moth & Lantern\",\"2019-05-03\",\"201500\",\"41\",\"spotify:user:x\",\"2021-03-04T18:39:20Z\"\r\n\
        \"spotify:track:2\",\"Say \"\"Hello\"\"\",\"\",\"Low Tide Radio\",\"2020\",\"abc\",\"7\",\"\",\"yesterday\"\r\n\
        \r\n\
        \"spotify:track:3\",\"\",\"No Title\",\"Nobody\",\"2020\",\"1000\",\"1\",\"\",\"\"\r\n";

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
    fn added_at_becomes_the_songs_date_and_an_unreadable_one_none() -> Result<()> {
        let entries = parse(&m3u::decode(FILE.as_bytes()))?;
        assert_eq!(entries[0].added, Some(1_614_883_160), "2021-03-04T18:39:20Z");
        assert_eq!(
            entries[1].added, None,
            "\"yesterday\" is no date: the song is dated by its place"
        );
        let without = parse("Track Name,Artist Name(s)\nA,B\n")?;
        assert_eq!(without[0].added, None, "an older export has no Added At column");
        Ok(())
    }

    #[test]
    fn iso_times_are_read_with_offsets_fractions_or_as_a_bare_date() {
        assert_eq!(unix_time("2021-03-04T18:39:20Z"), Some(1_614_883_160));
        assert_eq!(unix_time("2021-03-04T18:39:20.123Z"), Some(1_614_883_160));
        assert_eq!(unix_time("2021-03-04T15:09:20+02:00"), Some(1_614_863_360));
        assert_eq!(unix_time("2021-03-04T13:09:20"), Some(1_614_863_360), "no zone is UTC");
        assert_eq!(unix_time("2020-02-29"), Some(1_582_934_400));
        assert_eq!(unix_time("1969-12-31T23:59:59Z"), Some(-1));
        for bad in [
            "",
            "2021-13-04T00:00:00Z",
            "2021-03-04T25:00:00Z",
            "04/03/2021",
            "2021-03-04T18:39:20+5",
            "2021-03-04X",
        ] {
            assert_eq!(unix_time(bad), None, "{bad}");
        }
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
