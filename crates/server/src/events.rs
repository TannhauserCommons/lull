//! `GET /api/events`: the server-sent event stream the UI lives on (docs/context/api.md#events).

use std::convert::Infallible;
use std::time::Duration;

use axum::Router;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::get;
use lull_core::events::Bus;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;

pub fn router(bus: Bus) -> Router {
    Router::new().route(
        "/api/events",
        get(move || {
            let rx = bus.subscribe();
            async move {
                let stream = BroadcastStream::new(rx).map(|item| -> Result<Event, Infallible> {
                    Ok(match item {
                        Ok(e) => Event::default().event(e.topic).data(e.data.to_string()),
                        // The client fell behind and missed events: it refetches what it shows.
                        Err(BroadcastStreamRecvError::Lagged(n)) => {
                            Event::default().event("resync").data(n.to_string())
                        }
                    })
                });
                Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
            }
        }),
    )
}
