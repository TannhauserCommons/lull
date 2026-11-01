//! The `lull` binary: the headless hub (Docker, servers) and the container healthcheck.

use std::process::ExitCode;

use lull_core::config::env_vars;
use lull_server::BoxError;

#[tokio::main]
async fn main() -> ExitCode {
    let outcome = match std::env::args().nth(1).as_deref() {
        None | Some("serve") => serve().await,
        Some("healthcheck") => healthcheck().await,
        Some(other) => Err(format!("unknown command {other:?}; use serve or healthcheck").into()),
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("lull: {e}");
            ExitCode::FAILURE
        }
    }
}

async fn serve() -> Result<(), BoxError> {
    let env = env_vars()?;
    init_logs(env.get("LULL_LOG").map_or("info", String::as_str))?;
    lull_server::run(env, shutdown()).await
}

fn init_logs(filter: &str) -> Result<(), BoxError> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(tracing_subscriber::EnvFilter::try_new(filter)?)
        .init();
    Ok(())
}

async fn shutdown() {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let mut term = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(s) => s,
            Err(_) => return ctrl_c.await.unwrap_or(()),
        };
        tokio::select! { _ = ctrl_c => {}, _ = term.recv() => {} }
    }
    #[cfg(not(unix))]
    ctrl_c.await.unwrap_or(());
}

/// For the container healthcheck (the runtime image has no curl): exit 0 when `/health` answers.
async fn healthcheck() -> Result<(), BoxError> {
    let listen = std::env::var("LULL_LISTEN").unwrap_or_else(|_| "0.0.0.0:8080".into());
    let port = listen.rsplit(':').next().unwrap_or("8080");
    let resp = reqwest::Client::new()
        .get(format!("http://127.0.0.1:{port}/health"))
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await?;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("health answered {}", resp.status()).into())
    }
}
