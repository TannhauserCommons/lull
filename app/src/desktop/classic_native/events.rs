//! The hub's `stream` events for Classic's own player (docs/context/api.md, Events): where a song
//! still arriving from Soulseek or YouTube stands (finding, queued, buffering, streaming, stalled,
//! a fallback to another peer, failed, waiting for the listener to choose), how much of it is here,
//! which attempt its response belongs to, and a better copy on its way. One thread follows
//! `GET /api/events` (which also keeps this device online for lull Connect), reconnecting after a
//! drop as the page does, and hands each event to the engine.

use crate::hub;
use serde::Deserialize;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::Duration;
use tauri::Url;

/// Waits between tries after the stream drops or can't connect.
const RECONNECT: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StreamEvent {
    pub track_id: String,
    #[serde(default)]
    pub job_id: Option<String>,
    pub status: String,
    #[serde(default)]
    pub peer: Option<String>,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub queue_pos: Option<u32>,
    #[serde(default)]
    pub received: Option<u64>,
    #[serde(default)]
    pub size: Option<u64>,
    /// The transfer's speed, bytes a second.
    #[serde(default)]
    pub speed: Option<f64>,
    #[serde(default)]
    pub attempt: Option<u32>,
    #[serde(default)]
    pub error: Option<String>,
    /// For `choose`: the copies found, each a Source the hub can stream (`peer`, `path`, ...).
    #[serde(default)]
    pub choices: Vec<serde_json::Value>,
    #[serde(default)]
    pub upgrade: Option<Upgrade>,
}

/// A better copy arriving beside the one playing.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Upgrade {
    pub peer: String,
    pub path: String,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub received: Option<u64>,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub ready: bool,
}

impl StreamEvent {
    /// How much of the song is here, as a fraction (the page's `received`).
    pub fn fraction(&self) -> Option<f64> {
        let (received, size) = (self.received?, self.size.filter(|s| *s > 0)?);
        Some((received as f64 / size as f64).clamp(0.0, 1.0))
    }
}

/// What the engine hears.
pub enum Heard {
    Stream(Box<StreamEvent>),
    /// The stream (re)connected: what was missed is asked for again.
    Connected,
}

/// Follows the hub's events until `stop` is set (checked at least every 15 s, the hub's
/// keep-alive).
pub fn follow(origin: Url, token: String, to: Sender<Heard>, stop: Arc<AtomicBool>) {
    let spawned = std::thread::Builder::new()
        .name("classic-events".into())
        .spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let mut announced = false;
                let followed = hub::events(&origin, &token, |topic, data| {
                    if !announced {
                        announced = true;
                        if to.send(Heard::Connected).is_err() {
                            return false;
                        }
                    }
                    if topic == "stream" {
                        match serde_json::from_str::<StreamEvent>(data) {
                            Ok(event) => {
                                if to.send(Heard::Stream(Box::new(event))).is_err() {
                                    return false;
                                }
                            }
                            Err(e) => eprintln!("lull: Classic: an unreadable stream event: {e}"),
                        }
                    }
                    !stop.load(Ordering::Relaxed)
                });
                if let Err(e) = followed {
                    eprintln!("lull: Classic: the hub's events: {e}");
                }
                if !stop.load(Ordering::Relaxed) {
                    std::thread::sleep(RECONNECT);
                }
            }
        });
    if let Err(e) = spawned {
        eprintln!("lull: Classic can't follow the hub's events: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stream_event_reads_as_the_hub_writes_it() {
        let event: StreamEvent = serde_json::from_str(
            r#"{"trackId":"t1","jobId":"j","status":"streaming","peer":"p","format":"FLAC","peersFound":3,"queuePos":0,
            "received":50,"size":200,"attempt":2,"failedPeers":[],"error":null,"belowMinimum":false,"youtube":false,
            "upgrade":{"peer":"q","path":"a/b.flac","format":"FLAC","status":"buffering","received":10,"size":20,"ready":true}}"#,
        )
        .unwrap_or_default();
        assert_eq!((event.attempt, event.fraction()), (Some(2), Some(0.25)));
        assert!(event.upgrade.is_some_and(|u| u.ready && u.path == "a/b.flac"));
    }
}
