//! A hub that runs its own slskd and answers on the local network (ADR-0090): the desktop app's
//! ("This computer is my hub") and the Linux package's (`LULL_SLSKD`). Both use this module, so
//! they are one hub. In Docker, compose runs slskd beside the hub instead.

pub mod advert;
pub mod slskd;
pub mod tailscale;

use std::collections::BTreeMap;
use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, TcpListener, UdpSocket};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use lull_core::config::{Config, Share};
use lull_core::db::{self, Db};
use lull_core::settings;

pub use slskd::{Fixed, Launch, Sidecar, Tie};

use crate::BoxError;
use crate::hub::Hub;

/// How often the hub's own slskd is checked for being wedged, and how often at most it is
/// restarted for that (as the hub itself would with a web login, docs/context/soulseek.md).
const WEDGE_CHECK: Duration = Duration::from_secs(60);
const WEDGE_RESTART_EVERY: Duration = Duration::from_secs(30 * 60);

/// The Linux package's hub (`lull serve` with `LULL_SLSKD`): slskd from the package, its folders
/// in the data folder, the advert under this computer's name. Serves until `shutdown`, then stops
/// slskd.
pub async fn serve(
    mut env: BTreeMap<String, String>,
    exe: PathBuf,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), BoxError> {
    let data = PathBuf::from(env.get("LULL_DATA_DIR").map_or("/data", String::as_str));
    let fixed = Fixed {
        exe,
        app_dir: data.join("slskd"),
        transfers: data.join("transfers"),
        port: free_port()?,
        api_key: slskd::secret()?,
        web_password: slskd::secret()?,
        // systemd stops every process of the service with it.
        tie: Tie::NONE,
    };
    env.extend(vars(&fixed));
    let db = Db::open(&data)?;
    db.migrate(&db::CORE)?;
    let port = crate::hub::configure(&env, &db).await?.listen.port();
    follow_lan_address(&db, port).await?;
    let launch = launch(&env, &db).await?;
    let sidecar = tokio::task::spawn_blocking(move || slskd::start(fixed, launch)).await??;
    let name = advert::computer_name().map_or_else(|| "lull".to_owned(), |n| format!("lull on {n}"));
    let _advert = advert::advertise(&name, port)
        .map_err(|e| tracing::warn!("no mDNS on this network: {e}"))
        .ok();
    let served = crate::run_with(env, Some(sidecar.clone()), shutdown).await;
    tokio::task::spawn_blocking(move || sidecar.stop()).await?;
    served
}

/// The settings a hub with its own slskd runs with: where slskd answers, its key, its folders and
/// the `slskd.yml` the hub writes the Soulseek account into.
pub fn vars(fixed: &Fixed) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("LULL_SLSKD_URL".to_owned(), fixed.url()),
        ("LULL_SLSKD_API_KEY".to_owned(), fixed.api_key.clone()),
        ("LULL_TRANSFERS_DIR".to_owned(), path_string(&fixed.transfers)),
        ("LULL_SLSKD_CONFIG_FILE".to_owned(), path_string(&fixed.config_file())),
    ])
}

/// What slskd shares, from the hub's settings (env over stored).
pub async fn launch(env: &BTreeMap<String, String>, db: &Db) -> Result<Launch, BoxError> {
    Ok(launch_of(&crate::hub::configure(env, db).await?))
}

pub(crate) fn launch_of(config: &Config) -> Launch {
    Launch {
        share: match config.soulseek.share {
            Share::None => None,
            Share::Library => Some(config.library.music_dir.clone()),
        },
    }
}

/// Pair codes carry `LULL_PUBLIC_URL`; without one they'd carry whatever address the asking
/// device used, which may be loopback. So each start stores this computer's LAN address, unless
/// the listener set an https address (Tailscale, a reverse proxy, their own domain), which is
/// never touched.
pub async fn follow_lan_address(db: &Db, port: u16) -> Result<(), BoxError> {
    const KEY: &str = "LULL_PUBLIC_URL";
    let stored = settings::stored(db).await?;
    let ours = stored.get(KEY).is_none_or(|u| u.starts_with("http://"));
    let Some(lan) = lan_ip().filter(|_| ours) else {
        return Ok(());
    };
    let url = format!("http://{lan}:{port}");
    if stored.get(KEY) != Some(&url) {
        settings::store(db, BTreeMap::from([(KEY.to_owned(), Some(url))])).await?;
    }
    Ok(())
}

/// The address this computer uses on its default route, usually its LAN address. Connecting a UDP
/// socket only picks the route; nothing is sent.
pub fn lan_ip() -> Option<IpAddr> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).ok()?;
    socket
        .local_addr()
        .ok()
        .map(|a| a.ip())
        .filter(|ip| !ip.is_loopback() && !ip.is_unspecified())
}

/// A free port for slskd's API on loopback, chosen once per run (the hub keeps its URL).
pub fn free_port() -> Result<u16, BoxError> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    Ok(listener.local_addr()?.port())
}

/// The hub tells a wedged slskd that fresh logins didn't cure by its Soulseek status
/// (`extra.restartSlskd`). Its own slskd is headless and refuses the web login the hub would
/// restart it with, so the process is restarted, at most once per 30 minutes.
pub(crate) async fn restart_when_wedged(hub: std::sync::Arc<Hub>, sidecar: Sidecar) {
    let mut restarted: Option<Instant> = None;
    loop {
        tokio::time::sleep(WEDGE_CHECK).await;
        let wedged = hub
            .ctx()
            .await
            .status
            .snapshot()
            .get("soulseek")
            .is_some_and(|s| s.extra["restartSlskd"] == true);
        if wedged && restarted.is_none_or(|at| at.elapsed() >= WEDGE_RESTART_EVERY) {
            restarted = Some(Instant::now());
            sidecar.restart();
        }
    }
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
