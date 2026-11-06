//! The hub on the local network (ADR-0006): a hub on a computer answers `_lull._tcp` queries, so
//! "Connect to a hub" lists it. Pairing still needs a code, so finding a hub grants nothing.

use mdns_sd::{ServiceDaemon, ServiceInfo};

/// The service every lull hub answers as; the desktop app browses for it.
pub const SERVICE: &str = "_lull._tcp.local.";

/// Answers `_lull._tcp` queries for this computer's hub until dropped.
pub struct Advert(ServiceDaemon);

pub fn advertise(name: &str, port: u16) -> Result<Advert, String> {
    let mdns = ServiceDaemon::new().map_err(|e| e.to_string())?;
    let host = format!("{}.local.", name.replace(|c: char| !c.is_ascii_alphanumeric(), "-"));
    let version = lull_core::build::version();
    let info = ServiceInfo::new(SERVICE, name, &host, "", port, &[("version", version.as_str())][..])
        .map_err(|e| e.to_string())?
        .enable_addr_auto();
    mdns.register(info).map_err(|e| e.to_string())?;
    Ok(Advert(mdns))
}

impl Drop for Advert {
    fn drop(&mut self) {
        let _ = self.0.shutdown();
    }
}

/// This computer's name, as the hub's advert and the app's device name show it ("lull on <name>").
pub fn computer_name() -> Option<String> {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .or_else(|_| std::fs::read_to_string("/etc/hostname"))
        .map(|h| h.trim().to_owned())
        .ok()
        .filter(|h| !h.is_empty())
}
