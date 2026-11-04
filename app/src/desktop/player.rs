//! Where the music plays and who shows it. The web UI in `main` plays, or Webamp in the Classic
//! window while that is open (classic.rs), or Classic's native player (classic_native). What is playing reaches Rust through `set_now_playing`;
//! the tray, the taskbar's thumbnail buttons and Discord all show that one report. Their buttons
//! come back here and go to whichever window plays.

use crate::desktop::classic;
use crate::desktop::discord::Listening;
use crate::desktop::main_window;
use crate::desktop::platform::{self, ThumbButtons};
use crate::desktop::state::Desktop;
use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};

/// Payload of the `tray` event the playing window listens to.
#[derive(Clone, Serialize)]
pub struct Action {
    pub action: &'static str,
}

/// Run a player action (play-pause, next, previous, like, save, classic) in the window that plays.
pub fn forward(app: &AppHandle, action: &'static str) -> tauri::Result<()> {
    #[cfg(any(windows, target_os = "linux"))]
    if crate::desktop::classic_native::is_open() {
        return crate::desktop::classic_native::act(app, action);
    }
    let target = if classic::window(app).is_some() {
        classic::LABEL
    } else {
        main_window::LABEL
    };
    app.emit_to(target, "tray", Action { action })
}

/// Show the latest report everywhere: after `set_now_playing`, and when `main` moves between the
/// hub and a bundled page.
pub fn refresh(app: &AppHandle) {
    let desktop = app.state::<Desktop>();
    let now = desktop.now_playing();
    let connected = desktop.on_hub() && now.connected;
    let usable = connected && !now.title.is_empty();
    if let Some(tray) = desktop.tray.get() {
        tray.show(&now, connected);
    }
    platform::show_thumb_buttons(
        app,
        ThumbButtons {
            playing: now.playing,
            usable,
        },
    );
    let settings = desktop.settings();
    let listening = usable.then(|| {
        // Discord draws the progress bar from when the song started and when it ends.
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis()) as i64;
        let start = now_ms - (now.position * 1000.0) as i64;
        Listening {
            id: now.id.clone(),
            artist_id: now.artist_id.clone(),
            album_id: now.album_id.clone(),
            title: now.title.clone(),
            artist: now.artist.clone(),
            album: now.album.clone(),
            playing: now.playing,
            start_ms: start,
            end_ms: (now.duration > 0.0).then(|| start + (now.duration * 1000.0) as i64),
        }
    });
    desktop.discord.show(
        settings.discord_presence,
        settings.discord,
        listening,
        desktop.hub_access(),
    );
}
