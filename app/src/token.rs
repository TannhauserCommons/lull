//! This app's device token on its hub (ADR-0002) once that hub keeps credentials in a vault (ADR-0091): in
//! the system keyring on Windows and Linux, in Android's Keystore-backed storage on a phone, with
//! `settings.json` only saying so (`tokenInKeyring`). Keyring calls happen outside the settings' lock, since a
//! keyring may ask to be unlocked.

use std::sync::{Arc, Mutex, PoisonError};

/// Where a token is kept outside `settings.json`. Errors are sentences that never hold the token.
pub trait Keyring: Send + Sync {
    fn read(&self, name: &str) -> Result<Option<String>, String>;
    fn write(&self, name: &str, value: &str) -> Result<(), String>;
    fn remove(&self, name: &str) -> Result<(), String>;
}

/// The keyring entry.
const TOKEN: &str = "app.token";

pub struct Tokens {
    keyring: Arc<dyn Keyring>,
    /// What the keyring answered, so it is asked once a run.
    cached: Mutex<Option<String>>,
}

impl Tokens {
    pub fn new(keyring: Arc<dyn Keyring>) -> Tokens {
        Tokens {
            keyring,
            cached: Mutex::new(None),
        }
    }

    /// The token, given what `settings.json` holds (`file`) and whether it says the keyring has it. One in
    /// the file wins: an older lull wrote it after this one moved the last one to the keyring.
    pub fn get(&self, file: Option<&str>, in_keyring: bool) -> Option<String> {
        if file.is_some() || !in_keyring {
            return file.map(str::to_owned);
        }
        let mut cached = self.cached.lock().unwrap_or_else(PoisonError::into_inner);
        if cached.is_none() {
            match self.keyring.read(TOKEN) {
                Ok(token) => *cached = token,
                Err(e) => eprintln!("lull: this app's token on its hub can't be read: {e}"),
            }
        }
        cached.clone()
    }

    /// Keeps `token` (or forgets it) in the keyring when tokens are kept there. False when they aren't, or the
    /// keyring refused: the caller keeps it in `settings.json`.
    pub fn keep(&self, in_keyring: bool, token: Option<&str>) -> bool {
        if !in_keyring {
            return false;
        }
        let kept = match token {
            Some(t) => self.confirmed(t),
            None => self.keyring.remove(TOKEN).is_ok(),
        };
        if kept {
            *self.cached.lock().unwrap_or_else(PoisonError::into_inner) = token.map(str::to_owned);
        }
        kept
    }

    /// Moves the token `settings.json` holds (`file`) into the keyring. True once the keyring has read it
    /// back; the caller then takes it out of the file.
    pub fn take_in(&self, file: Option<&str>) -> bool {
        let Some(token) = file else { return false };
        let kept = self.confirmed(token);
        if kept {
            *self.cached.lock().unwrap_or_else(PoisonError::into_inner) = Some(token.to_owned());
        }
        kept
    }

    fn confirmed(&self, token: &str) -> bool {
        match self.keyring.write(TOKEN, token).and_then(|()| self.keyring.read(TOKEN)) {
            Ok(back) => back.as_deref() == Some(token),
            Err(e) => {
                eprintln!("lull: this app's token stays in settings.json: {e}");
                false
            }
        }
    }
}

/// Whether a hub's `/health` says it keeps credentials in a vault (its `secrets` switch, ADR-0091): then this
/// app's token there goes to the keyring too.
pub fn hub_keeps_secrets(origin: &tauri::Url) -> bool {
    crate::hub::health(origin).is_ok_and(|h| h.flags.iter().any(|f| f == "secrets"))
}

#[cfg(all(test, not(any(target_os = "android", target_os = "ios"))))]
mod tests {
    use super::*;
    use crate::desktop::keyring::SystemKeyring;

    /// The token moves into the keyring and is read from there; a new one replaces it there, and forgetting it
    /// takes it out. Where a keyring works (keyring.rs's `expected_keyring`), under a test identifier.
    #[test]
    fn the_token_moves_into_the_keyring_and_is_read_and_replaced_there() -> Result<(), String> {
        if crate::desktop::keyring::tests::expected_keyring()?.as_str() != "present" {
            return Ok(()); // keyring.rs's test checks what a machine without one says
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        let keyring = SystemKeyring::new(&format!("io.github.tannhausercommons.lull.test-token-{now}"));
        let tokens = Tokens::new(keyring.clone());
        assert!(
            !tokens.keep(false, Some("x")),
            "not in the keyring yet: the file keeps it"
        );
        assert!(tokens.take_in(Some("token-one")));
        assert_eq!(Keyring::read(&*keyring, TOKEN)?.as_deref(), Some("token-one"));
        assert_eq!(
            Tokens::new(keyring.clone()).get(None, true).as_deref(),
            Some("token-one")
        );

        assert!(tokens.keep(true, Some("token-two")));
        assert_eq!(tokens.get(None, true).as_deref(), Some("token-two"));
        assert_eq!(
            tokens.get(Some("from-an-older-lull"), true).as_deref(),
            Some("from-an-older-lull"),
            "the file's wins"
        );
        assert!(tokens.keep(true, None));
        assert_eq!(Keyring::read(&*keyring, TOKEN)?, None);
        Ok(())
    }
}
