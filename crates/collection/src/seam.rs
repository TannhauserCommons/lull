//! The `Collection` seam (ADR-0002) for crates that act on the collection outside the web UI's
//! endpoints (Subsonic). Each call runs the same code as the matching route, so the same events
//! go out.

use async_trait::async_trait;
use lull_core::ids::TrackId;
use lull_core::model::{PlayReport, Playlist, PlaylistEdit};
use lull_core::seams::Collection;
use lull_core::{Error, Result};

use crate::playlists::{self, seq_of};
use crate::plays::{self, EndBody};
use crate::{Service, likes};

#[async_trait]
impl Collection for Service {
    async fn liked(&self) -> Result<Vec<TrackId>> {
        self.ctx
            .db
            .call(|conn| {
                Ok(conn
                    .prepare("SELECT track_id FROM likes ORDER BY liked_at DESC, rowid DESC")?
                    .query_map([], |r| r.get(0).map(TrackId))?
                    .collect::<rusqlite::Result<Vec<_>>>()?)
            })
            .await
    }

    async fn set_liked(&self, track: &TrackId, liked: bool) -> Result<()> {
        let track = track.clone();
        self.write(move |tx, events| likes::set(tx, events, &track, liked, lull_core::now()))
            .await
    }

    async fn playlists(&self) -> Result<Vec<Playlist>> {
        self.ctx.db.call(|conn| playlists::all(conn)).await
    }

    async fn playlist(&self, id: &str) -> Result<Option<Playlist>> {
        let Ok(seq) = seq_of(id) else {
            return Ok(None);
        };
        self.ctx.db.call(move |conn| playlists::one(conn, seq)).await
    }

    async fn create_playlist(&self, name: &str, tracks: &[TrackId]) -> Result<Playlist> {
        let (name, tracks) = (name.to_owned(), tracks.to_vec());
        self.write(move |tx, events| {
            let seq = playlists::make(tx, events, &name, "", &tracks, lull_core::now())?;
            created(playlists::one(tx, seq)?)
        })
        .await
    }

    async fn edit_playlist(&self, id: &str, edit: PlaylistEdit) -> Result<Playlist> {
        let seq = seq_of(id)?;
        self.write(move |tx, events| {
            playlists::edit_positions(tx, events, seq, edit)?;
            created(playlists::one(tx, seq)?)
        })
        .await
    }

    async fn playlist_cover(&self, id: &str, size: Option<u32>) -> Result<Option<axum::response::Response>> {
        match seq_of(id) {
            Ok(seq) => crate::covers::response(self, seq, size).await,
            Err(_) => Ok(None),
        }
    }

    async fn delete_playlist(&self, id: &str) -> Result<()> {
        let seq = seq_of(id)?;
        self.write(move |tx, events| match playlists::delete(tx, events, seq)? {
            true => Ok(()),
            false => Err(Error::NotFound(format!("No playlist pl-{seq}."))),
        })
        .await
    }

    async fn record_play(&self, play: PlayReport) -> Result<()> {
        let end = EndBody {
            played_s: play.played_s,
            completed: play.completed,
            skipped: false,
        };
        self.write(move |tx, events| plays::record(tx, events, &play.device, &play.track_id, play.started_at, &end))
            .await
            .map(drop)
    }
}

/// A playlist just written in this transaction is there to read back.
fn created(p: Option<Playlist>) -> Result<Playlist> {
    p.ok_or_else(|| Error::internal("a playlist written in this transaction is missing"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing;

    #[tokio::test]
    async fn every_playlist_change_publishes_one_playlist_event() -> Result<()> {
        let (svc, mut rx) = testing::service().await?;
        let song = testing::known_song(&svc, "Velvet Orchard", "Paper Moon Static").await?;
        let playlist = svc.create_playlist("Road", &[]).await?;
        svc.edit_playlist(
            &playlist.id,
            PlaylistEdit {
                name: Some("Road trip".into()),
                add: vec![song],
                ..PlaylistEdit::default()
            },
        )
        .await?;
        svc.delete_playlist(&playlist.id).await?;
        let events: Vec<_> = testing::published(&mut rx)
            .into_iter()
            .map(|e| (e.topic, e.data["playlistId"].clone(), e.data["deleted"].clone()))
            .collect();
        let id = serde_json::json!(playlist.id);
        let event = |deleted| ("playlist".to_owned(), id.clone(), serde_json::json!(deleted));
        assert_eq!(events, [event(false), event(false), event(true)]);
        Ok(())
    }

    #[tokio::test]
    async fn a_reported_play_ends_at_its_real_start_without_now_playing() -> Result<()> {
        let (svc, mut rx) = testing::service().await?;
        let device = testing::device(&svc, false).await?;
        let track = testing::known_song(&svc, "Velvet Orchard", "Paper Moon Static").await?;
        let started_at = lull_core::now() - 3600;
        svc.record_play(PlayReport {
            device: device.id,
            track_id: track,
            started_at,
            played_s: 180,
            completed: true,
        })
        .await?;
        let events = testing::published(&mut rx);
        let topics: Vec<&str> = events.iter().map(|e| e.topic.as_str()).collect();
        assert_eq!(topics, ["play.ended"]);
        assert_eq!(events[0].data["at"], started_at);
        assert_eq!(events[0].data["playedS"], 180);
        Ok(())
    }

    #[tokio::test]
    async fn a_private_device_s_reported_play_goes_nowhere() -> Result<()> {
        let (svc, mut rx) = testing::service().await?;
        let private = testing::device(&svc, true).await?;
        let track = testing::known_song(&svc, "Velvet Orchard", "Paper Moon Static").await?;
        svc.record_play(PlayReport {
            device: private.id,
            track_id: track,
            started_at: lull_core::now() - 3600,
            played_s: 180,
            completed: true,
        })
        .await?;
        let rows = svc
            .ctx
            .db
            .call(|c| Ok(c.query_row("SELECT COUNT(*) FROM plays", [], |r| r.get::<_, i64>(0))?))
            .await?;
        assert_eq!(rows, 0, "no history, stats or taste");
        assert!(
            testing::published(&mut rx).is_empty(),
            "nothing for Last.fm or discovery"
        );
        Ok(())
    }
}
