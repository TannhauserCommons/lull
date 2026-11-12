//! Live state of each outside dependency, owned by the crate that talks to it. `/health`
//! reports it and every change is published as a `service` event (the UI's status tray).

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use serde::Serialize;
use serde_json::Value;

use crate::events::Bus;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Ok,
    /// Working but degraded (reconnecting, scrobbles queued, cache held over its size).
    Warn,
    Down,
    /// Not configured; the features that need it are hidden or explain why.
    Off,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Service {
    pub state: State,
    /// One plain sentence the UI can show as is.
    pub detail: String,
    /// Service-specific numbers (cache: usedBytes, capBytes; lastfm: queued).
    #[serde(skip_serializing_if = "Value::is_null")]
    pub extra: Value,
}

#[derive(Clone)]
pub struct Status {
    services: Arc<RwLock<BTreeMap<&'static str, Service>>>,
    bus: Bus,
}

impl Status {
    pub fn new(bus: Bus) -> Self {
        Status {
            services: Arc::default(),
            bus,
        }
    }

    /// Records a service's state; publishes `service` only when something changed.
    pub fn set(&self, name: &'static str, state: State, detail: impl Into<String>, extra: Value) {
        let next = Service {
            state,
            detail: detail.into(),
            extra,
        };
        let Ok(mut map) = self.services.write() else { return };
        if map.get(name) == Some(&next) {
            return;
        }
        map.insert(name, next.clone());
        drop(map);
        self.bus
            .publish("service", serde_json::json!({ "name": name, "service": next }));
    }

    pub fn snapshot(&self) -> BTreeMap<&'static str, Service> {
        self.services.read().map(|m| m.clone()).unwrap_or_default()
    }
}
