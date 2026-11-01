//! `getLyricsBySongId` (OpenSubsonic `songLyrics`), through the `Lyrics` seam. Synced lyrics
//! become timed lines in milliseconds, plain ones untimed lines; instrumental songs and songs
//! without lyrics get an empty list.

use lull_core::model::LyricsKind;
use serde_json::{Value, json};

use crate::params::Params;
use crate::response::Fail;
use crate::{Reply, Service};

pub async fn by_song(svc: &Service, p: &Params) -> Result<Reply, Fail> {
    let track = svc.track(p.required("id")?).await?;
    let lyrics = svc.lyrics.lyrics(&track.id).await?;
    let (synced, line): (bool, Vec<Value>) = match lyrics.kind {
        LyricsKind::Synced => (
            true,
            lyrics
                .lines
                .iter()
                .map(|l| json!({ "start": (l.t * 1000.0).round() as i64, "value": l.text }))
                .collect(),
        ),
        LyricsKind::Plain => (
            false,
            lyrics
                .plain
                .as_deref()
                .unwrap_or_default()
                .lines()
                .map(|text| json!({ "value": text }))
                .collect(),
        ),
        LyricsKind::Instrumental | LyricsKind::None => (false, Vec::new()),
    };
    let structured = if line.is_empty() {
        Vec::new()
    } else {
        // LRCLIB doesn't say which language; `und` is the spec's "unknown".
        vec![json!({
            "displayArtist": track.artist,
            "displayTitle": track.title,
            "lang": "und",
            "offset": 0,
            "synced": synced,
            "line": line,
        })]
    };
    Ok(Reply::of("lyricsList", json!({ "structuredLyrics": structured })))
}
