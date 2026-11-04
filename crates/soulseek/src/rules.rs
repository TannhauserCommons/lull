//! The listener's source rules (ADR-0008): the streaming and saving profiles, the peer limits and
//! when YouTube plays, from Settings › Sources. Collection publishes them in the `settings` event (ADR-0050); until
//! the first one arrives, the defaults apply.

use std::sync::{Arc, RwLock};
use std::time::Duration;

use lull_core::events::Bus;
use lull_core::quality::{Minimum, Pick, SaveProfile, StreamProfile};
use lull_core::youtube;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rules {
    pub stream: StreamProfile,
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

impl Default for Rules {
    fn default() -> Self {
        Rules {
            stream: StreamProfile::default(),
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
                    *r = rules;
                }
            }
            // Collection validates before publishing, so this is a contract break: say so.
            Err(e) => tracing::warn!(error = %e, "settings.sources can't be read; keeping the previous rules"),
        }
    }
}
