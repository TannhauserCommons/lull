//! Stars are lull's likes, and scrobbles are plays, both through the `Collection` seam so the web
//! UI, history, taste and Last.fm see them as if the web player had made them.

use lull_core::ids::TrackId;
use lull_core::model::PlayReport;
use serde_json::{Value, json};

use crate::params::Params;
use crate::response::Fail;
use crate::{Reply, Service, shape};

/// `star` / `unstar`. lull likes songs only, so `albumId` and `artistId` are refused.
pub async fn star(svc: &Service, p: &Params, liked: bool) -> Result<Reply, Fail> {
    if p.has("albumId") || p.has("artistId") {
        return Err(Fail::generic("lull likes songs; albums and artists can't be starred"));
    }
    let ids: Vec<TrackId> = p.all("id").map(|id| TrackId(id.to_owned())).collect();
    if ids.is_empty() {
        return Err(Fail::missing("id"));
    }
    for id in &ids {
        svc.collection.set_liked(id, liked).await?;
    }
    Ok(Reply::empty())
}

/// `getStarred2`, or `getStarred` under `key`: with only songs starred, the two are the same.
pub async fn starred(svc: &Service, key: &str) -> Result<Reply, Fail> {
    let liked = svc.liked().await?;
    let ids: Vec<TrackId> = liked.iter().map(|(id, _)| id.clone()).collect();
    let starred = liked.into_iter().collect();
    let song: Vec<Value> = svc
        .tracks(&ids)
        .await?
        .iter()
        .map(|t| shape::song(t, &starred))
        .collect();
    Ok(Reply::of(key, json!({ "artist": [], "album": [], "song": song })))
}

/// A submission is a song the client played to its scrobble point: recorded as a completed
/// play that started at `time` (ms), or one song length ago. "Now playing" (`submission=false`)
/// has no seam yet and is accepted without effect. A device in private listening records nothing.
pub async fn scrobble(svc: &Service, p: &Params, device: &str) -> Result<Reply, Fail> {
    let ids: Vec<&str> = p.all("id").collect();
    if ids.is_empty() {
        return Err(Fail::missing("id"));
    }
    if p.get("submission") == Some("false") {
        return Ok(Reply::empty());
    }
    let times = p
        .all("time")
        .map(|t| {
            t.parse::<i64>()
                .map_err(|_| Fail::generic(format!("time must be a number, not {t:?}")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    for (i, id) in ids.into_iter().enumerate() {
        let track = svc.track(id).await?;
        let played_s = track.duration.unwrap_or(0);
        let started_at = times
            .get(i)
            .map_or_else(|| lull_core::now() - i64::from(played_s), |ms| ms / 1000);
        svc.collection
            .record_play(PlayReport {
                device: device.to_owned(),
                track_id: track.id,
                started_at,
                played_s,
                completed: true,
            })
            .await?;
    }
    Ok(Reply::empty())
}
