//! The listener's source rules (ADR-0008): the streaming and saving profiles, the peer limits and
//! when YouTube plays, from Settings › Sources. Collection publishes them in the `settings` event (ADR-0050); until
//! the first one arrives, the defaults apply.
//!
//! Streaming is per device (ADR-0017): each request streams by the source preset of the device
//! that asks (`Rules::profile_for`), and a fetch keeps the profile it was started for
//! (`Rules::with_stream`). The hub's own `stream` is Desktop's, for what no device asked for.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use axum::http::HeaderMap;
use lull_core::events::Bus;
use lull_core::presets::{self, Preset};
use lull_core::quality::{Minimum, Pick, SaveProfile, StreamProfile};
use lull_core::youtube;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rules {
    /// The streaming profile these rules decide by: the asking device's preset's, once
    /// `with_stream` set it; Desktop's as published.
    #[serde(skip)]
    pub stream: StreamProfile,
    /// The source presets, Desktop and Phone first, and the preset each device chose.
    #[serde(default = "builtins")]
    pub presets: Vec<Preset>,
    #[serde(default)]
    pub devices: BTreeMap<String, String>,
    pub save: SaveProfile,
    /// Peers with no free slot and a longer upload queue are skipped.
    pub max_peer_queue: u32,
    /// Seconds without new bytes before falling back to the next copy.
    pub stall_timeout: u32,
    /// When YouTube plays instead of, or before, a Soulseek copy (docs/context/youtube.md).
    #[serde(default)]
    pub youtube: youtube::Settings,
    /// Switch the playing song to a better copy once one is ready (docs/context/streaming.md#better-copies).
    #[serde(default = "on")]
    pub switch_to_better: bool,
}

fn on() -> bool {
    true
}

fn builtins() -> Vec<Preset> {
    presets::builtins(StreamProfile::default())
}

impl Default for Rules {
    fn default() -> Self {
        Rules {
            stream: StreamProfile::default(),
            presets: builtins(),
            devices: BTreeMap::new(),
            save: SaveProfile::default(),
            max_peer_queue: 20,
            stall_timeout: 15,
            youtube: youtube::Settings::default(),
            switch_to_better: true,
        }
    }
}

/// What a copy is wanted for: each has its own profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    Stream,
    Save,
}

impl Rules {
    pub fn minimum(&self, purpose: Purpose) -> &Minimum {
        match purpose {
            Purpose::Stream => &self.stream.minimum,
            Purpose::Save => &self.save.minimum,
        }
    }

    pub fn pick(&self, purpose: Purpose) -> Pick {
        match purpose {
            Purpose::Stream => self.stream.pick,
            Purpose::Save => self.save.pick,
        }
    }

    pub fn stall_timeout(&self) -> Duration {
        Duration::from_secs(u64::from(self.stall_timeout))
    }

    /// The streaming profile of the preset `device` uses, its kind told by the request's
    /// `headers` (`lull_core::presets`): what everything fetched for that request streams by.
    pub fn profile_for(&self, device: Option<&str>, headers: &HeaderMap) -> StreamProfile {
        presets::used(&self.presets, &self.devices, device, presets::phone(headers))
            .map_or_else(|| self.stream.clone(), |u| u.preset.stream.clone())
    }

    /// These rules, streaming by `stream` (a device's preset's, a fetch's own).
    pub fn with_stream(mut self, stream: &StreamProfile) -> Rules {
        stream.clone_into(&mut self.stream);
        self
    }

    /// Desktop's profile streams for what no device asked for.
    fn desktop(mut self) -> Rules {
        if let Some(p) = self.presets.iter().find(|p| p.id == presets::DESKTOP) {
            p.stream.clone_into(&mut self.stream);
        }
        self
    }
}

/// The latest rules, shared by search, sources and fetches.
#[derive(Clone, Default)]
pub struct Current(Arc<RwLock<Rules>>);

#[cfg(test)]
impl From<Rules> for Current {
    fn from(rules: Rules) -> Self {
        Current(Arc::new(RwLock::new(rules)))
    }
}

impl Current {
    pub fn get(&self) -> Rules {
        self.0.read().map(|r| r.clone()).unwrap_or_default()
    }

    /// Subscribes now (collection's start-up copy comes after soulseek starts) and keeps the
    /// rules current until the bus closes.
    pub fn follow(&self, bus: &Bus) -> impl Future<Output = ()> + Send + 'static {
        let mut events = bus.subscribe();
        let current = self.clone();
        async move {
            loop {
                match events.recv().await {
                    Ok(e) if e.topic == "settings" => current.apply(&e.data),
                    Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                }
            }
        }
    }

    fn apply(&self, settings: &Value) {
        let Some(sources) = settings.get("sources") else { return };
        match serde_json::from_value::<Rules>(sources.clone()) {
            Ok(rules) => {
                if let Ok(mut r) = self.0.write() {
                    *r = rules.desktop();
                }
            }
            // Collection validates before publishing, so this is a contract break: say so.
            Err(e) => tracing::warn!(error = %e, "settings.sources can't be read; keeping the previous rules"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::header::USER_AGENT;
    use serde_json::json;

    #[test]
    fn each_device_streams_by_its_preset_and_desktop_stands_for_the_rest() {
        let current = Current::default();
        let mut phone = presets::phone_from(&StreamProfile::default());
        phone.max_kbps = 256;
        let car = json!({ "id": "p-1", "name": "Car", "stream": { "minimum": { "losslessOnly": false, "kbps": 128 }, "pick": "fastest", "below": "playBest", "maxKbps": 128 } });
        current.apply(&json!({ "sources": {
            "presets": [
                { "id": "desktop", "name": "Desktop", "stream": { "minimum": { "losslessOnly": true, "kbps": 0 }, "pick": "highest", "below": "playBest" } },
                { "id": "phone", "name": "Phone", "stream": phone },
                car,
            ],
            "devices": { "d-car": "p-1" },
            "save": SaveProfile::default(), "maxPeerQueue": 20, "stallTimeout": 15,
        }}));
        let rules = current.get();
        assert!(rules.stream.minimum.lossless_only, "Desktop's is the hub's own");
        let mut iphone = HeaderMap::new();
        iphone.insert(
            USER_AGENT,
            axum::http::HeaderValue::from_static("Mozilla/5.0 (iPhone; CPU iPhone OS 18_1 like Mac OS X)"),
        );
        assert_eq!(
            rules.profile_for(Some("d-1"), &iphone).max_kbps,
            256,
            "a phone streams by Phone"
        );
        assert!(rules.profile_for(Some("d-1"), &HeaderMap::new()).minimum.lossless_only);
        assert_eq!(
            rules.profile_for(Some("d-car"), &iphone).max_kbps,
            128,
            "a device's choice wins"
        );
        let car = rules.profile_for(Some("d-car"), &iphone);
        assert_eq!(rules.clone().with_stream(&car).minimum(Purpose::Stream).kbps, 128);
    }
}
