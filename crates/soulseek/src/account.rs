//! The Soulseek account of an slskd lull runs itself (the desktop app's sidecar, ADR-0002).
//! slskd can't take an account over its API with an API key, so lull owns that slskd's config
//! file (`LULL_SLSKD_CONFIG_FILE`): every start writes the account from the settings into it,
//! and when that changed the file, waits for slskd to reload it and logs in again.

use std::path::Path;
use std::time::Duration;

use lull_core::config::SoulseekAccount;
use lull_core::{Error, Result};

use crate::slskd::Slskd;

/// slskd may still be starting (the app starts both at once), so lull waits this long for it.
const RELOAD_WAIT: Duration = Duration::from_secs(60);
const POLL: Duration = Duration::from_millis(500);

/// Writes the file whole; true when its content changed. Written beside and renamed over, so
/// slskd's file watcher never reads half a file.
pub async fn write(path: &Path, account: Option<&SoulseekAccount>) -> Result<bool> {
    let next = render(account)?;
    if tokio::fs::read_to_string(path).await.is_ok_and(|now| now == next) {
        return Ok(false);
    }
    let fail = |e: std::io::Error| Error::internal(format!("writing {}: {e}", path.display()));
    let partial = path.with_extension("lull-partial");
    tokio::fs::write(&partial, next).await.map_err(fail)?;
    tokio::fs::rename(&partial, path).await.map_err(fail)?;
    Ok(true)
}

/// The file lull owns: only the account. Strings are JSON-quoted, which YAML reads as is.
fn render(account: Option<&SoulseekAccount>) -> Result<String> {
    let mut yaml = String::from("# Written by lull at every start from its settings (LULL_SLSKD_CONFIG_FILE).\n");
    if let Some(account) = account {
        let quote = |s: &str| serde_json::to_string(s).map_err(|e| Error::internal(e.to_string()));
        yaml.push_str(&format!(
            "soulseek:\n  username: {}\n  password: {}\n",
            quote(&account.username)?,
            quote(account.password.expose())?
        ));
    }
    Ok(yaml)
}

/// After the file changed: once slskd has reloaded it, logs in with the new account (or logs
/// out, when the account was removed).
pub async fn apply(slskd: Slskd, account: Option<SoulseekAccount>) {
    match reloaded(&slskd, account.as_ref()).await {
        Some(was_logged_in) => {
            let outcome = match (&account, was_logged_in) {
                (Some(_), true) => slskd.relogin("lull's Soulseek account changed").await,
                (Some(_), false) => slskd.connect().await,
                (None, true) => slskd.disconnect("lull's Soulseek account was removed").await,
                (None, false) => Ok(()),
            };
            match outcome {
                Ok(()) => tracing::info!("slskd uses the Soulseek account from lull's settings"),
                Err(e) => tracing::warn!(error = %e, "applying the Soulseek account to slskd"),
            }
        }
        None => tracing::warn!("slskd didn't reload its config file; its Soulseek account applies when slskd restarts"),
    }
}

/// Waits until slskd has the new file, and says whether it was logged in. Logged in, slskd flags
/// a changed account as waiting for a reconnect; logged out, its options show the new username.
async fn reloaded(slskd: &Slskd, account: Option<&SoulseekAccount>) -> Option<bool> {
    let deadline = tokio::time::Instant::now() + RELOAD_WAIT;
    while tokio::time::Instant::now() < deadline {
        if let Ok(server) = slskd.server().await {
            if server.is_logged_in {
                if slskd.pending_reconnect().await.unwrap_or(false) {
                    return Some(true);
                }
            } else if let Ok(Some(options)) = slskd.options().await
                && options.soulseek.username.as_deref() == account.map(|a| a.username.as_str())
            {
                return Some(false);
            }
        }
        tokio::time::sleep(POLL).await;
    }
    None
}

#[cfg(test)]
mod tests {
    use lull_core::config::Secret;

    use super::*;

    #[tokio::test]
    async fn the_file_holds_only_the_account_and_is_rewritten_only_when_it_changes() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("lull-slskd-yml-{}", std::process::id()));
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| Error::internal(e.to_string()))?;
        let file = dir.join("slskd.yml");
        let account = SoulseekAccount {
            username: "me".into(),
            password: Secret::new("p\"w: #1"),
        };

        assert!(write(&file, Some(&account)).await?, "a new file is a change");
        let text = tokio::fs::read_to_string(&file)
            .await
            .map_err(|e| Error::internal(e.to_string()))?;
        assert!(
            text.ends_with("soulseek:\n  username: \"me\"\n  password: \"p\\\"w: #1\"\n"),
            "{text}"
        );
        assert!(
            !write(&file, Some(&account)).await?,
            "the same account leaves the file alone"
        );
        assert!(write(&file, None).await?, "removing the account is a change");
        let text = tokio::fs::read_to_string(&file)
            .await
            .map_err(|e| Error::internal(e.to_string()))?;
        assert!(!text.contains("soulseek:"), "{text}");

        let _ = tokio::fs::remove_dir_all(dir).await;
        Ok(())
    }
}
