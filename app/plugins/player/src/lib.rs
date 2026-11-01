//! lull's native player for Android (ADR-0006). Playback and downloads are Kotlin (`android/`); the
//! web UI calls them as `plugin:lull-player|<command>` through `window.LullNative` (the app's
//! `src/mobile/native.js`). Rust only registers the Kotlin plugin and tells it which hub to use.
#![cfg(target_os = "android")]

use serde::Serialize;
use tauri::plugin::{Builder, PluginHandle, TauriPlugin};
use tauri::{Manager, Runtime};

pub struct Player<R: Runtime>(PluginHandle<R>);

#[derive(Serialize)]
struct Hub<'a> {
    origin: &'a str,
    token: &'a str,
}

impl<R: Runtime> Player<R> {
    /// The hub streams and downloads come from: Kotlin adds `Authorization: Bearer` to its requests
    /// and puts the token in the WebView's `lull_device` cookie (Tauri can't set cookies on Android).
    pub fn configure(&self, origin: &str, token: &str) -> Result<(), String> {
        self.0
            .run_mobile_plugin::<()>("configure", Hub { origin, token })
            .map_err(|e| e.to_string())
    }

    /// Android filters multicast unless an app holds a lock; mDNS discovery needs it while it runs.
    pub fn multicast(&self, on: bool) -> Result<(), String> {
        #[derive(Serialize)]
        struct On {
            on: bool,
        }
        self.0
            .run_mobile_plugin::<()>("multicast", On { on })
            .map_err(|e| e.to_string())
    }
}

pub trait PlayerExt<R: Runtime> {
    fn player(&self) -> &Player<R>;
}

impl<R: Runtime, T: Manager<R>> PlayerExt<R> for T {
    fn player(&self) -> &Player<R> {
        self.state::<Player<R>>().inner()
    }
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("lull-player")
        .setup(|app, api| {
            let handle = api.register_android_plugin("app.lull.player", "PlayerPlugin")?;
            app.manage(Player(handle));
            Ok(())
        })
        .build()
}
