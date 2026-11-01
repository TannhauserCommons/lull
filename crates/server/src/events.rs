//! `GET /api/events`: the server-sent event stream the UI lives on (docs/context/api.md#events).
//! An open stream marks its device online (lull Connect). It ends when the crates restart (the
//! client reconnects to the new bus) or when its device is unpaired.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::{Extension, State};
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::get;
use lull_core::devices::Device;
use lull_core::events;
use tokio::sync::{broadcast, mpsc};
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::CancellationToken;

use crate::devices::Stream;
use crate::hub::Hub;

pub fn router() -> Router<Arc<Hub>> {
    Router::new().route("/api/events", get(stream))
}

async fn stream(State(hub): State<Arc<Hub>>, Extension(device): Extension<Device>) -> impl IntoResponse {
    let ctx = hub.ctx().await;
    let events = ctx.bus.subscribe();
    let presence = hub.online.join(&device.id, &ctx.bus);
    let (tx, rx) = mpsc::channel(64);
    tokio::spawn(forward(events, tx, presence, ctx.shutdown.clone()));
    Sse::new(ReceiverStream::new(rx)).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

/// Copies bus events to one client until it leaves, its device is unpaired or the crates stop.
/// Events already published are sent first, so `hub.restarting` reaches the client.
async fn forward(
    mut events: broadcast::Receiver<events::Event>,
    tx: mpsc::Sender<Result<Event, Infallible>>,
    presence: Stream,
    shutdown: CancellationToken,
) {
    loop {
        let event = tokio::select! {
            biased;
            item = events.recv() => match item {
                Ok(e) => Event::default().event(e.topic).data(e.data.to_string()),
                // The client fell behind and missed events: it refetches what it shows.
                Err(broadcast::error::RecvError::Lagged(n)) => Event::default().event("resync").data(n.to_string()),
                Err(broadcast::error::RecvError::Closed) => return,
            },
            () = shutdown.cancelled() => return,
            () = presence.unpaired.cancelled() => return,
            () = tx.closed() => return,
        };
        if tx.send(Ok(event)).await.is_err() {
            return;
        }
    }
}
