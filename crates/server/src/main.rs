//! The `lull` binary: the headless hub (Docker, the Linux package) and the container healthcheck.

use std::path::PathBuf;
use std::process::ExitCode;

use lull_core::config::env_vars;
use lull_server::BoxError;

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let outcome = match args.first().map(String::as_str) {
        None | Some("serve") => serve().await,
        Some("healthcheck") => healthcheck().await.map(|()| ExitCode::SUCCESS),
        #[cfg(unix)]
        Some(_) => lull_server::cli::run(&args).await.map(|()| ExitCode::SUCCESS),
        #[cfg(not(unix))]
        Some(other) => Err(format!("unknown command {other:?}; use serve or healthcheck").into()),
    };
    match outcome {
        Ok(code) => code,
        Err(e) => {
            eprintln!("lull: {e}");
            ExitCode::FAILURE
        }
    }
}

async fn serve() -> Result<ExitCode, BoxError> {
    let env = env_vars()?;
    init_logs(env.get("LULL_LOG").map_or("info", String::as_str))?;
    // A kit the launcher started runs as what the launcher was installed as.
    if let Some(install) = env
        .get("LULL_INSTALL")
        .and_then(|i| lull_core::build::Install::parse(i))
    {
        lull_core::build::set_install(install);
    }
    // A newer hub kit in the data folder runs instead of this build, watched (ADR-0090).
    if let Some(code) = lull_server::launch::kits(&env).await? {
        return Ok(code);
    }
    // The Linux package names its slskd; the hub then runs it itself (ADR-0090).
    match env.get("LULL_SLSKD").filter(|p| !p.is_empty()).map(PathBuf::from) {
        Some(slskd) => lull_server::managed::serve(env, slskd, shutdown()).await?,
        None => lull_server::run(env, shutdown()).await?,
    }
    // Exiting to start again on the update it downloaded: systemd or Docker starts lull again, or the launcher
    // runs the new kit.
    Ok(if lull_server::updates::restarting() {
        ExitCode::from(lull_server::launch::UPDATE)
    } else {
        ExitCode::SUCCESS
    })
}

fn init_logs(filter: &str) -> Result<(), BoxError> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(tracing_subscriber::EnvFilter::try_new(filter)?)
        .init();
    Ok(())
}

/// Ctrl-C, SIGTERM, or an update ready to start.
async fn shutdown() {
    let ctrl_c = tokio::signal::ctrl_c();
    let update = lull_server::updates::restart_requested();
    #[cfg(unix)]
    {
        let mut term = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(s) => s,
            Err(_) => {
                tokio::select! { _ = ctrl_c => {}, () = update => {} }
                return;
            }
        };
        tokio::select! { _ = ctrl_c => {}, _ = term.recv() => {}, () = update => {} }
    }
    #[cfg(not(unix))]
    tokio::select! { _ = ctrl_c => {}, () = update => {} }
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
