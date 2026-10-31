//! The tray icon and its native menu (DESKTOP.md 4). Window items (Show lull, Mini player, Quit) are
//! handled here; player items are forwarded to the web UI as `tray` events, which it runs as its own
//! actions. Labels follow `set_now_playing`.

use crate::main_window::{self, report};
use crate::mini;
use crate::platform;
use crate::state::NowPlaying;
use serde::Serialize;
use tauri::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Wry};

pub struct Tray {
    icon: TrayIcon,
    now: MenuItem<Wry>,
    play: MenuItem<Wry>,
    next: MenuItem<Wry>,
    previous: MenuItem<Wry>,
    like: CheckMenuItem<Wry>,
    save: MenuItem<Wry>,
    mini: CheckMenuItem<Wry>,
}

/// Payload of the `tray` event the web UI listens to.
#[derive(Clone, Serialize)]
pub struct Action {
    pub action: &'static str,
}

pub fn create(app: &AppHandle) -> tauri::Result<Tray> {
    let item = |id: &str, text: &str| MenuItem::with_id(app, id, text, false, None::<&str>);
    let check = |id: &str, text: &str| CheckMenuItem::with_id(app, id, text, false, false, None::<&str>);
    let sep = || PredefinedMenuItem::separator(app);
    let now = item("now", "Not connected")?;
    let play = item("play-pause", "Play")?;
    let next = item("next", "Next")?;
    let previous = item("previous", "Previous")?;
    let like = check("like", "Like")?;
    let save = item("save", "Save to library")?;
    let show = MenuItem::with_id(app, "show", "Show lull", true, None::<&str>)?;
    let mini = CheckMenuItem::with_id(app, "mini", "Mini player", true, false, None::<&str>)?;
    let classic = MenuItem::with_id(app, "classic", "Classic mode", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit lull", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &now,
            &sep()?,
            &play,
            &next,
            &previous,
            &sep()?,
            &like,
            &save,
            &sep()?,
            &show,
            &mini,
            &classic,
            &sep()?,
            &quit,
        ],
    )?;
    let icon = TrayIconBuilder::with_id("lull")
        .icon(platform::tray_icon(app)?)
        .tooltip("lull: not connected")
        .menu(&menu)
        // Windows: left click shows or hides lull, right click opens the menu. Linux delivers no
        // clicks (AppIndicator), so the menu opens on any click there and holds every action.
        .show_menu_on_left_click(cfg!(not(windows)))
        .on_menu_event(on_menu)
        .on_tray_icon_event(on_click)
        .build(app)?;
    Ok(Tray {
        icon,
        now,
        play,
        next,
        previous,
        like,
        save,
        mini,
    })
}

impl Tray {
    /// Labels, check marks and tooltip for the current track. Player items are off while idle or
    /// while the server can't be reached.
    pub fn show(&self, now: &NowPlaying, on_server: bool) {
        let connected = on_server && now.connected;
        let idle = now.title.is_empty();
        let line = match () {
            _ if !connected => "Not connected".to_owned(),
            _ if idle => "Nothing playing".to_owned(),
            _ => format!("{}, {}", now.title, now.artist),
        };
        let usable = connected && !idle;
        // "lull: nothing playing", but "lull: Paper Moon Static, Velvet Orchard".
        let tooltip = if usable {
            format!("lull: {line}")
        } else {
            format!("lull: {}", line.to_lowercase())
        };
        let result = (|| -> tauri::Result<()> {
            self.now.set_text(&line)?;
            self.play.set_text(if now.playing { "Pause" } else { "Play" })?;
            self.like.set_checked(now.liked)?;
            self.save
                .set_text(if now.saved { "Saved" } else { "Save to library" })?;
            self.save.set_enabled(usable && !now.saved)?;
            for item in [&self.play, &self.next, &self.previous] {
                item.set_enabled(usable)?;
            }
            self.like.set_enabled(usable)?;
            self.icon.set_tooltip(Some(tooltip))
        })();
        report(result);
    }

    pub fn set_mini(&self, open: bool) {
        report(self.mini.set_checked(open));
    }

    pub fn set_icon(&self, image: tauri::image::Image<'static>) {
        report(self.icon.set_icon(Some(image)));
    }
}

fn on_menu(app: &AppHandle, event: MenuEvent) {
    let result = match event.id().as_ref() {
        "show" => mini::close(app),
        "mini" => mini::toggle(app),
        "quit" => {
            app.exit(0);
            Ok(())
        }
        // Classic mode is a screen of the web UI: show it first, then let the web UI open it.
        "classic" => mini::close(app).and_then(|()| forward(app, "classic")),
        "play-pause" => forward(app, "play-pause"),
        "next" => forward(app, "next"),
        "previous" => forward(app, "previous"),
        "like" => forward(app, "like"),
        "save" => forward(app, "save"),
        _ => Ok(()),
    };
    report(result);
}

fn on_click(icon: &TrayIcon, event: TrayIconEvent) {
    if let TrayIconEvent::Click {
        button: MouseButton::Left,
        button_state: MouseButtonState::Up,
        ..
    } = event
    {
        report(main_window::toggle(icon.app_handle()));
    }
}

/// Run a player action in the web UI.
pub fn forward(app: &AppHandle, action: &'static str) -> tauri::Result<()> {
    app.emit_to(main_window::LABEL, "tray", Action { action })
}
