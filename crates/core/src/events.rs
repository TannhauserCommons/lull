//! In-process event bus. Every event also reaches the browser through `GET /api/events` (SSE),
//! so topics and payloads are part of the API contract (docs/context/api.md#events).

use serde::Serialize;
use serde_json::Value;
use tokio::sync::broadcast;

#[derive(Debug, Clone, Serialize)]
pub struct Event {
    pub topic: String,
    pub data: Value,
}

#[derive(Clone)]
pub struct Bus(broadcast::Sender<Event>);

impl Bus {
    pub fn new() -> Self {
        Bus(broadcast::channel(1024).0)
    }

    /// Fire and forget: no subscribers is fine, and a slow subscriber skips ahead, never blocks.
    pub fn publish(&self, topic: &str, data: impl Serialize) {
        match serde_json::to_value(data) {
            Ok(data) => {
                let _ = self.0.send(Event {
                    topic: topic.to_owned(),
                    data,
                });
            }
            Err(e) => tracing::error!(topic, error = %e, "event payload does not serialize"),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.0.subscribe()
    }
}

impl Default for Bus {
    fn default() -> Self {
        Self::new()
    }
}
