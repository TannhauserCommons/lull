//! Hubs on the local network (ADR-0006): the desktop app advertises its own hub as `_lull._tcp`,
//! and "Connect to a hub" lists the ones that answer. Pairing still needs a code, so finding a hub
//! grants nothing.

use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use serde::Serialize;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

const SERVICE: &str = "_lull._tcp.local.";

#[derive(Debug, Clone, Serialize)]
pub struct Found {
    /// The instance name: the hub computer's name.
    pub name: String,
    /// `http://<address>:<port>`, ready for pairing.
    pub url: String,
}

/// Every hub that answers within `window`, by name.
pub fn browse(window: Duration) -> Result<Vec<Found>, String> {
    let mdns = ServiceDaemon::new().map_err(|e| e.to_string())?;
    let events = mdns.browse(SERVICE).map_err(|e| e.to_string())?;
    let deadline = Instant::now() + window;
    let mut found = BTreeMap::new();
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        let Ok(event) = events.recv_timeout(left) else { break };
        if let ServiceEvent::ServiceResolved(hub) = event {
            // IPv4: link-local IPv6 needs a zone id that a URL can't carry for the webview.
            let Some(ip) = hub.get_addresses_v4().into_iter().min_by_key(|ip| reach(*ip)) else {
                continue;
            };
            let name = hub.fullname.trim_end_matches(SERVICE).trim_end_matches('.').to_owned();
            let url = format!("http://{ip}:{}", hub.port);
            found.insert(name.clone(), Found { name, url });
        }
    }
    // Shutting down only stops the daemon thread; a failure there changes nothing we found.
    let _ = mdns.shutdown();
    Ok(found.into_values().collect())
}

/// Which of a hub's addresses a device on the same network most likely reaches: home LAN ranges
/// first, then Tailscale's (100.64/10, only for devices on the tailnet), link-local last.
fn reach(ip: std::net::Ipv4Addr) -> u8 {
    match ip.octets() {
        [192, 168, ..] => 0,
        [10, ..] => 1,
        [172, b, ..] if (16..32).contains(&b) => 2,
        [100, b, ..] if (64..128).contains(&b) => 4,
        [169, 254, ..] => 5,
        _ => 3,
    }
}

/// Answers `_lull._tcp` queries for this computer's hub until dropped.
#[cfg(desktop)]
pub struct Advert(ServiceDaemon);

#[cfg(desktop)]
pub fn advertise(name: &str, port: u16) -> Result<Advert, String> {
    let mdns = ServiceDaemon::new().map_err(|e| e.to_string())?;
    let host = format!("{}.local.", name.replace(|c: char| !c.is_ascii_alphanumeric(), "-"));
    let info = ServiceInfo::new(
        SERVICE,
        name,
        &host,
        "",
        port,
        &[("version", env!("CARGO_PKG_VERSION"))][..],
    )
    .map_err(|e| e.to_string())?
    .enable_addr_auto();
    mdns.register(info).map_err(|e| e.to_string())?;
    Ok(Advert(mdns))
}

#[cfg(desktop)]
impl Drop for Advert {
    fn drop(&mut self) {
        let _ = self.0.shutdown();
    }
}
