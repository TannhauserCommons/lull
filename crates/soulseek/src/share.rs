//! Sharing per `LULL_SHARE`. slskd sets its shares when it starts (the deploy kit hands it the
//! library read-only unless `LULL_SHARE=none`); slskd 0.26 can't change them through an API
//! key. So lull checks what slskd really shares and says so when it doesn't match, and asks
//! slskd to rescan after saves so newly saved songs are shared too (docs/context/soulseek.md#sharing).

use std::time::Duration;

use lull_core::config::Share;
use lull_core::events::Bus;

use crate::slskd::Slskd;

/// Saves come in bursts (an album); one rescan after the burst is enough.
const RESCAN_AFTER: Duration = Duration::from_secs(60);

/// A sentence for the status tray when slskd's shares don't match `LULL_SHARE`.
pub async fn mismatch(slskd: &Slskd, want: Share) -> Option<String> {
    let shared = match slskd.shares().await {
        Ok(s) => s.into_iter().filter(|s| !s.is_excluded).count(),
        Err(e) => return Some(format!("cannot read slskd's shares: {e}")),
    };
    match (want, shared) {
        (Share::Library, 0) => {
            Some("slskd shares nothing although LULL_SHARE=library; many peers refuse users who share nothing".into())
        }
        (Share::None, n) if n > 0 => Some(format!("slskd shares {n} folders although LULL_SHARE=none")),
        _ => None,
    }
}

/// Rescans slskd's shares a while after the last finished save.
pub async fn rescan_after_saves(slskd: Slskd, bus: Bus) {
    let mut events = bus.subscribe();
    let mut due: Option<tokio::time::Instant> = None;
    loop {
        let wake = due.unwrap_or_else(|| tokio::time::Instant::now() + Duration::from_secs(3600));
        tokio::select! {
            e = events.recv() => match e {
                Ok(e) if e.topic == "save" && crate::save_finished(&e.data) => {
                    due = Some(tokio::time::Instant::now() + RESCAN_AFTER);
                }
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            },
            _ = tokio::time::sleep_until(wake), if due.is_some() => {
                due = None;
                match slskd.rescan_shares().await {
                    Ok(()) => tracing::info!("asked slskd to rescan its shares after saves"),
                    Err(e) => tracing::warn!(error = %e, "slskd share rescan"),
                }
            }
        }
    }
}
