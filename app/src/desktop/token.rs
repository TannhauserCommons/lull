//! This app's device token on its hub (ADR-0002) once that hub keeps credentials in a keyring (ADR-0091): in
//! the system keyring, `settings.json` only saying so (`tokenInKeyring`). Keyring calls happen outside the
//! settings' lock, since a keyring may ask to be unlocked.

use std::sync::{Arc, Mutex, PoisonError};

use lull_core::secrets::Vault;

use crate::desktop::keyring::SystemKeyring;
use crate::desktop::settings::Settings;

/// The keyring entry.
const TOKEN: &str = "app.token";

pub struct Tokens {
    keyring: Arc<SystemKeyring>,
    /// What the keyring answered, so it is asked once a run.
    cached: Mutex<Option<String>>,
}

impl Tokens {
    pub fn new(keyring: Arc<SystemKeyring>) -> Tokens {
        Tokens {
            keyring,
            cached: Mutex::new(None),
        }
    }

    /// The token `settings` points at. One in `settings.json` wins: an older lull wrote it after this one
    /// moved the last one to the keyring.
    pub fn get(&self, settings: &Settings) -> Option<String> {
        if settings.token.is_some() || !settings.token_in_keyring {
            return settings.token.clone();
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
    pub fn keep(&self, settings: &Settings, token: Option<&str>) -> bool {
        if !settings.token_in_keyring {
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

    /// Moves the token `settings.json` holds into the keyring. True once the keyring has read it back; the
    /// caller then takes it out of the file.
    pub fn take_in(&self, settings: &Settings) -> bool {
        let Some(token) = settings.token.as_deref() else {
            return false;
        };
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The token moves into the keyring and is read from there; a new one replaces it there, and forgetting it
    /// takes it out. Where CI says a keyring works (`LULL_TEST_KEYRING=present`), under a test identifier.
    #[test]
    fn the_token_moves_into_the_keyring_and_is_read_and_replaced_there() -> Result<(), String> {
        if std::env::var("LULL_TEST_KEYRING").as_deref() != Ok("present") {
            return Ok(()); // keyring.rs's test checks what a machine without one says
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        let keyring = SystemKeyring::new(&format!("io.github.tannhausercommons.lull.test-token-{now}"));
        let tokens = Tokens::new(keyring.clone());
        let mut settings = Settings {
            token: Some("token-one".into()),
            ..Settings::default()
        };
        assert!(
            !tokens.keep(&settings, Some("x")),
            "not in the keyring yet: the file keeps it"
        );
        assert!(tokens.take_in(&settings));
        settings.token = None;
        settings.token_in_keyring = true;
        assert_eq!(
            keyring.read(TOKEN).map_err(|e| e.to_string())?.as_deref(),
            Some("token-one")
        );
        assert_eq!(
            Tokens::new(keyring.clone()).get(&settings).as_deref(),
            Some("token-one")
        );

        assert!(tokens.keep(&settings, Some("token-two")));
        assert_eq!(tokens.get(&settings).as_deref(), Some("token-two"));
        let older = Settings {
            token: Some("from-an-older-lull".into()),
            ..settings.clone()
        };
        assert_eq!(
            tokens.get(&older).as_deref(),
            Some("from-an-older-lull"),
            "the file's wins"
        );
        assert!(tokens.keep(&settings, None));
        assert_eq!(keyring.read(TOKEN).map_err(|e| e.to_string())?, None);
        Ok(())
    }
}
