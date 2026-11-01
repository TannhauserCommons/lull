//! lull desktop: a thin Tauri wrapper around the web UI the lull server serves (design/desktop/
//! DESKTOP.md). It owns windows, the tray and the first-run connect page; every player action runs
//! in the web UI.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod main_window;
mod mini;
mod pages;
mod platform;
mod server;
mod settings;
mod state;
mod tray;

use crate::main_window::report;
use crate::settings::{Appearance, Store};
use crate::state::Desktop;
use tauri::{App, Listener, Manager, Window, WindowEvent};
use tauri_plugin_window_state::StateFlags;

fn main() {
    let builder = tauri::Builder::default()
        // First: a second launch hands over to this one and exits before anything else starts.
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            report(main_window::reveal(app))
        }))
        // Placement only: visibility and decorations are the wrapper's call.
        .plugin(
            tauri_plugin_window_state::Builder::new()
                .with_state_flags(StateFlags::SIZE | StateFlags::POSITION | StateFlags::MAXIMIZED)
                .build(),
        )
        .plugin(tauri_plugin_autostart::Builder::new().build());
    #[cfg(target_os = "linux")]
    let builder = builder.plugin(tauri_plugin_global_shortcut::Builder::new().build());
    let app = builder
        .invoke_handler(tauri::generate_handler![
            commands::test_server,
            commands::window_action,
            commands::set_now_playing,
            commands::desktop_settings,
            commands::set_desktop_settings,
        ])
        .setup(setup)
        .on_window_event(on_window_event)
        .build(tauri::generate_context!());
    match app {
        Ok(app) => app.run(|_, _| {}),
        Err(e) => {
            eprintln!("lull: {e}");
            std::process::exit(1);
        }
    }
}

fn setup(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    let store = Store::new(&app.path().app_config_dir()?);
    let platform = platform::detect();
    let with_tray = platform.tray;
    app.manage(Desktop::new(store, platform));
    let handle = app.handle();
    // No tray is a supported state (Close then quits), so a tray that fails to appear isn't fatal.
    if with_tray {
        match tray::create(handle) {
            Ok(tray) => {
                let _ = app.state::<Desktop>().tray.set(tray);
                platform::follow_taskbar(handle);
            }
            Err(e) => eprintln!("lull: no tray: {e}"),
        }
    }
    remember_appearance(handle);
    main_window::launch(handle);
    Ok(())
}

/// The web UI reports its design and theme; the bundled pages open with them (DESKTOP.md 6.1).
fn remember_appearance(app: &tauri::AppHandle) {
    let app = app.clone();
    app.clone().listen_any("appearance", move |event| {
        match serde_json::from_str::<Appearance>(event.payload()) {
            Ok(a) if a != app.state::<Desktop>().settings().appearance => {
                app.state::<Desktop>().update(|s| s.appearance = a)
            }
            Ok(_) => {}
            Err(e) => eprintln!("lull: ignoring appearance {}: {e}", event.payload()),
        }
    });
}

fn on_window_event(window: &Window, event: &WindowEvent) {
    let WindowEvent::CloseRequested { api, .. } = event else {
        return;
    };
    if window.label() == main_window::LABEL {
        main_window::close_requested(window, api);
    } else if mini::is_label(window.label()) {
        // Alt+F4 on the mini player works like its own close button.
        api.prevent_close();
        report(mini::close(window.app_handle()));
    }
}
