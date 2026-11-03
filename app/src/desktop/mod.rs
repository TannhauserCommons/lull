//! lull on Windows and Linux (design/desktop/DESKTOP.md, ADR-0006): the hub's web UI in a window,
//! with a tray, a mini player and the first-run choice. When this computer is the hub, the app runs
//! it in-process with its bundled slskd. Every player action runs in the web UI.

mod classic;
#[cfg(windows)]
mod classic_native;
mod commands;
mod discord;
mod glass;
mod links;
mod local_hub;
mod main_window;
mod mini;
mod open_links;
mod platform;
mod player;
mod recovery;
mod settings;
mod signin;
mod slskd;
mod state;
mod tailscale;
mod theme;
mod tray;
mod updater;

use crate::desktop::main_window::report;
use crate::desktop::state::Desktop;
use crate::pages::Appearance;
use crate::pairing;
use crate::store::Store;
use tauri::{App, AppHandle, Listener, Manager, RunEvent, Window, WindowEvent};
use tauri_plugin_window_state::StateFlags;

pub fn run() {
    let builder = tauri::Builder::default()
        // First: a second launch hands over to this one and exits before anything else starts. A
        // `lull:` link it was started with opens here.
        .plugin(tauri_plugin_single_instance::init(
            |app, args, _| match open_links::from_args(&args) {
                Some(link) => open_links::open(app, link),
                None => report(main_window::reveal(app)),
            },
        ))
        // Placement only: visibility and decorations are the wrapper's call.
        .plugin(
            tauri_plugin_window_state::Builder::new()
                .with_state_flags(StateFlags::SIZE | StateFlags::POSITION | StateFlags::MAXIMIZED)
                .build(),
        )
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .plugin(tauri_plugin_updater::Builder::new().build());
    #[cfg(target_os = "linux")]
    let builder = builder.plugin(tauri_plugin_global_shortcut::Builder::new().build());
    let app = builder
        .invoke_handler(tauri::generate_handler![
            pairing::discover_hubs,
            pairing::find_hub,
            commands::window_action,
            commands::set_now_playing,
            commands::desktop_settings,
            commands::set_desktop_settings,
            commands::tailscale_status,
            commands::tailscale_serve,
            commands::classic_open,
            commands::classic_handoff,
            commands::classic_fit,
            commands::classic_back,
            commands::app_update,
            commands::app_update_check,
            commands::app_update_install,
            commands::resume_after_crash,
            commands::window_glass,
            commands::set_theme_package,
            commands::theme_package,
            crate::reconnect::reconnect_status,
        ])
        .setup(setup)
        .on_window_event(on_window_event)
        .build(tauri::generate_context!());
    match app {
        Ok(app) => app.run(on_run_event),
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
    app.manage(crate::reconnect::Reconnect::default());
    let handle = app.handle();
    let h = handle.clone();
    platform::follow_network(move || {
        crate::reconnect::wake(&h, crate::reconnect::Wake::Network);
    });
    // No tray is a supported state (Close then quits), so a tray that fails to appear isn't fatal.
    if with_tray {
        match tray::create(handle) {
            Ok(tray) => {
                let _ = app.state::<Desktop>().tray.set(tray);
            }
            Err(e) => eprintln!("lull: no tray: {e}"),
        }
    }
    platform::follow_personalize(handle);
    remember_appearance(handle);
    open_links::register(handle);
    if let Some(link) = open_links::from_args(&std::env::args().collect::<Vec<_>>()) {
        open_links::open(handle, link);
    }
    recovery::start(handle);
    classic::follow(handle);
    main_window::launch(handle);
    updater::start(handle);
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
    if window.label() == main_window::LABEL {
        main_window::on_window_event(window, event);
    }
    let WindowEvent::CloseRequested { api, .. } = event else {
        return;
    };
    if window.label() == main_window::LABEL {
        main_window::close_requested(window, api);
    } else if mini::is_label(window.label()) {
        // Alt+F4 on the mini player works like its own close button.
        api.prevent_close();
        report(mini::close(window.app_handle()));
    } else if window.label() == classic::LABEL {
        // Alt+F4 on Webamp works like its own close button: the queue goes back to lull first.
        api.prevent_close();
        report(classic::close(window.app_handle()));
    }
}

/// Quitting stops the hub on this computer and its slskd, which would otherwise keep the Soulseek login.
/// Classic's native window isn't a Tauri window: while it plays with `main` closed, the last Tauri
/// window closing doesn't end lull.
fn on_run_event(app: &AppHandle, event: RunEvent) {
    #[cfg(windows)]
    if let RunEvent::ExitRequested { code: None, api, .. } = &event
        && classic_native::is_open()
    {
        api.prevent_exit();
    }
    if let RunEvent::Exit = event
        && let Some(local) = app.state::<Desktop>().local_hub()
    {
        local.stop();
    }
}
