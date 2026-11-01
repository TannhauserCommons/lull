//! What outlives a restart (the database, pairing, who is online) and the current wiring of the
//! crates, which a restart replaces in place (ADR-0002, docs/context/hub.md).

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Router;
use lull_core::config::Config;
use lull_core::db::Db;
use lull_core::devices::{self, Kind};
use lull_core::events::Bus;
use lull_core::http::Http;
use lull_core::status::Status;
use lull_core::{Ctx, Error, Result, settings};
use serde_json::json;
use tokio::sync::{Mutex, RwLock, broadcast, mpsc};

use crate::devices::Online;
use crate::pairing::Pairing;

pub struct Hub {
    /// The `LULL_*` settings from the environment; they win over the stored ones.
    pub env: BTreeMap<String, String>,
    pub db: Db,
    pub pairing: Pairing,
    pub online: Arc<Online>,
    /// Serialises `PATCH /api/config`, so two changes never validate against the same old state.
    pub settings: Mutex<()>,
    /// Held for writing while the crates restart, so requests wait for the new ones.
    wiring: RwLock<Wiring>,
    restarts: mpsc::Sender<String>,
    requests: Mutex<mpsc::Receiver<String>>,
}

/// One start of the crates: their context and their routes.
struct Wiring {
    ctx: Ctx,
    router: Router,
}

impl Hub {
    /// Starts the crates with `config` and arms the setup code if no device is paired yet.
    pub async fn open(env: BTreeMap<String, String>, db: Db, config: Config) -> Result<Arc<Hub>> {
        let (restarts, requests) = mpsc::channel(1);
        let setup_code = config.setup_code.clone();
        let wiring = Wiring::start(config, db.clone(), restarts.clone()).await?;
        let hub = Arc::new(Hub {
            env,
            db,
            pairing: Pairing::new(setup_code),
            online: Arc::default(),
            settings: Mutex::new(()),
            wiring: RwLock::new(wiring),
            restarts,
            requests: Mutex::new(requests),
        });
        if !hub.paired().await? {
            hub.pairing.arm_setup();
        }
        Ok(hub)
    }

    /// The running crates' context. Waits while they restart.
    pub async fn ctx(&self) -> Ctx {
        self.wiring.read().await.ctx.clone()
    }

    pub async fn router(&self) -> Router {
        self.wiring.read().await.router.clone()
    }

    /// Whether a browser or app is paired. Subsonic clients don't count: they can't add devices.
    pub async fn paired(&self) -> Result<bool> {
        Ok(devices::list(&self.db).await?.iter().any(|d| d.kind != Kind::Subsonic))
    }

    /// The reason of the next `hub.restart`.
    pub async fn restart_requested(&self) -> Option<String> {
        self.requests.lock().await.recv().await
    }

    /// Stops the crates and starts them again with freshly resolved settings. Requests wait
    /// meanwhile; the SSE streams end, and clients reconnect to the new bus.
    pub async fn restart(&self, reason: &str) -> Result<()> {
        let mut wiring = self.wiring.write().await;
        tracing::info!(reason, "restarting the crates");
        wiring.ctx.bus.publish("hub.restarting", json!({ "reason": reason }));
        wiring.ctx.shutdown.cancel();
        let config = configure(&self.env, &self.db).await?;
        *wiring = Wiring::start(config, self.db.clone(), self.restarts.clone()).await?;
        Ok(())
    }

    /// Stops the crates for good (the hub is exiting).
    pub async fn stop(&self) {
        self.wiring.read().await.ctx.shutdown.cancel();
    }
}

impl Wiring {
    async fn start(config: Config, db: Db, restarts: mpsc::Sender<String>) -> Result<Wiring> {
        let bus = Bus::new();
        let ctx = Ctx {
            db,
            http: Http::new()?,
            status: Status::new(bus.clone()),
            shutdown: Default::default(),
            config: Arc::new(config),
            bus,
        };
        // Subscribed before the crates start, so a crate asking for a restart while it starts
        // is still heard.
        ctx.spawn(forward_restarts(ctx.bus.subscribe(), restarts));
        let router = crate::wire(&ctx).await?;
        Ok(Wiring { ctx, router })
    }
}

/// The environment merged over the stored settings (ADR-0002).
pub async fn configure(env: &BTreeMap<String, String>, db: &Db) -> Result<Config> {
    let (vars, _) = settings::merge(env, &settings::stored(db).await?);
    Config::from_vars(vars).map_err(|e| Error::BadRequest(e.to_string()))
}

/// Hands `hub.restart` requests from this wiring's bus to the restart loop. A request made
/// while one is already waiting is covered by it.
async fn forward_restarts(mut events: broadcast::Receiver<lull_core::events::Event>, restarts: mpsc::Sender<String>) {
    loop {
        match events.recv().await {
            Ok(e) if e.topic == "hub.restart" => {
                let reason = e.data.get("reason").and_then(|r| r.as_str()).unwrap_or("asked for");
                let _ = restarts.try_send(reason.to_owned());
            }
            Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
            Err(broadcast::error::RecvError::Closed) => return,
        }
    }
}
