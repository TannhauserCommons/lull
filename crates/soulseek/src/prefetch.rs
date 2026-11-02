//! Preloading the songs queued next (`PUT /api/stream/prefetch`), so pressing next plays at
//! once instead of waiting for a search and a new peer. Each device lists up to 3 tracks in
//! queue order; lull fetches the ones it doesn't have, in order, one search at a time and
//! behind the playing song (docs/context/streaming.md#preloading).

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use lull_core::ids::TrackId;
use lull_core::quality::StreamBelow;
use lull_core::youtube::{Mode, is_youtube};
use lull_core::{Error, Result};
use tokio::sync::watch;

use crate::Service;
use crate::budget::Lane;
use crate::fetch::Status;
use crate::rules::Purpose;

/// The most tracks one device may list: the next few in its queue.
pub const MAX_TRACKS: usize = 3;
/// How long a preload may take to get its bytes flowing before the next one starts anyway.
const START_WAIT: Duration = Duration::from_secs(30);
/// How often a preload looks again while the player's own song is still starting.
const PLAYER_POLL: Duration = Duration::from_millis(500);

/// The devices' lists, most recently sent first, and a counter that moves on every change.
pub struct Lists {
    by_device: Mutex<Vec<(String, Vec<TrackId>)>>,
    changed: watch::Sender<u64>,
}

impl Default for Lists {
    fn default() -> Self {
        Lists {
            by_device: Mutex::default(),
            changed: watch::Sender::new(0),
        }
    }
}

impl Lists {
    /// Replaces one device's list (`""` is the shared list of requests without a device) and
    /// returns every listed track, the latest device's first, each once.
    pub fn set(&self, device: String, tracks: Vec<TrackId>) -> Result<Vec<TrackId>> {
        if tracks.len() > MAX_TRACKS {
            return Err(Error::BadRequest(format!(
                "at most {MAX_TRACKS} tracks are preloaded at a time"
            )));
        }
        let wanted = {
            let mut lists = self.by_device.lock().map_err(|_| Error::internal("preload lists"))?;
            lists.retain(|(d, _)| *d != device);
            if !tracks.is_empty() {
                lists.insert(0, (device, tracks));
            }
            union(&lists)
        };
        self.changed.send_modify(|n| *n += 1);
        Ok(wanted)
    }

    fn wanted(&self) -> Vec<TrackId> {
        self.by_device.lock().map(|l| union(&l)).unwrap_or_default()
    }
}

fn union(lists: &[(String, Vec<TrackId>)]) -> Vec<TrackId> {
    let mut seen = HashSet::new();
    lists
        .iter()
        .flat_map(|(_, tracks)| tracks)
        .filter(|t| seen.insert((*t).clone()))
        .cloned()
        .collect()
}

/// A device sent its list: preloads nobody lists any more are let go, and the worker plans again.
pub fn listed(svc: &Service, device: String, tracks: Vec<TrackId>) -> Result<()> {
    let wanted = svc.prefetch.set(device, tracks)?;
    svc.fetcher.unlist(&wanted.into_iter().collect());
    Ok(())
}

/// Works through the listed tracks in order, starting over whenever a list changes.
pub async fn run(svc: Arc<Service>) {
    let mut changed = svc.prefetch.changed.subscribe();
    loop {
        changed.mark_unchanged();
        let wanted = svc.prefetch.wanted();
        let work = async {
            for id in &wanted {
                if let Err(e) = preload(&svc, id).await {
                    tracing::debug!(error = %e, track = %id, "preloading a queued song");
                }
            }
        };
        tokio::select! {
            () = work => {
                if changed.changed().await.is_err() {
                    return;
                }
            }
            r = changed.changed() => {
                if r.is_err() {
                    return;
                }
            }
        }
    }
}

/// Fetches one listed song unless it is saved, in the cache or already fetching. Its search
/// waits behind the player's, and the fetch starts once the player's own song plays; then
/// this waits for the preload's bytes to flow before the next one starts, so one peer at a
/// time is asked.
async fn preload(svc: &Service, id: &TrackId) -> Result<()> {
    // With YouTube on, a song Soulseek can't give is preloaded from YouTube by the fetch itself.
    let mode = svc.rules.get().youtube.mode;
    if (!svc.online() && mode == Mode::Off) || svc.fetcher.list_live(id) {
        return Ok(());
    }
    let track = svc.track(id).await?;
    let cached = svc.cache.finished(id, None).await?;
    if svc.searches.saved(id).await
        || cached
            .as_ref()
            .is_some_and(|e| mode != Mode::Off || !is_youtube(&e.source))
    {
        return Ok(());
    }
    // Start fast: the YouTube match and its extraction are made ready, so the song starts at
    // once even when Soulseek's copy isn't here yet.
    if mode == Mode::StartFast {
        svc.fetcher.warm(track.clone());
    }
    if svc.online() && !svc.sources.known(id) {
        let searched = svc.searches.resolve_in(&track, Lane::Prefetch).await;
        if mode == Mode::Off {
            searched?;
        }
    }
    if mode == Mode::Off && !worth_starting(svc, &track) {
        return Ok(());
    }
    let started = tokio::time::Instant::now();
    while svc.fetcher.player_waiting() && started.elapsed() < START_WAIT {
        tokio::time::sleep(PLAYER_POLL).await;
    }
    let f = svc.fetcher.prefetch(track);
    let mut progress = f.progress();
    let flowing =
        progress.wait_for(|p| p.ended() || matches!(p.status, Status::Buffering | Status::Streaming | Status::Done));
    let _ = tokio::time::timeout(START_WAIT, flowing).await;
    Ok(())
}

/// Whether a copy could play: nobody sharing it, or only copies below the minimum while the
/// rule isn't `playBest`, is left to the player's own fetch, which searches again or applies
/// the rule when the song plays.
fn worth_starting(svc: &Service, track: &lull_core::model::Track) -> bool {
    let rules = svc.rules.get();
    let ordered = svc.sources.ordered(&track.id, track.duration, &rules, Purpose::Stream);
    let usable: Vec<_> = ordered.iter().filter(|r| !r.queue_too_long).collect();
    !usable.is_empty() && (rules.stream.below == StreamBelow::PlayBest || usable.iter().any(|r| r.meets))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(names: &[&str]) -> Vec<TrackId> {
        names.iter().map(|n| TrackId((*n).to_owned())).collect()
    }

    #[test]
    fn a_device_replaces_its_own_list_and_every_device_is_served() -> Result<()> {
        let lists = Lists::default();
        lists.set("phone".into(), ids(&["a", "b"]))?;
        let both = lists.set("desk".into(), ids(&["c", "b"]))?;
        assert_eq!(both, ids(&["c", "b", "a"]), "the latest list first, each song once");
        let replaced = lists.set("phone".into(), ids(&["d"]))?;
        assert_eq!(
            replaced,
            ids(&["d", "c", "b"]),
            "phone's old songs are no longer listed"
        );
        assert_eq!(
            lists.set("desk".into(), Vec::new())?,
            ids(&["d"]),
            "an empty list clears"
        );
        assert!(
            lists.set("phone".into(), ids(&["a", "b", "c", "d"])).is_err(),
            "at most 3"
        );
        Ok(())
    }
}
