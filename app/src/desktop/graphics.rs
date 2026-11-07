//! How the webview draws (Settings › App › Graphics; docs/context/desktop.md, Graphics): with the
//! graphics card, the processor only, or as the engine decides (Automatic). The choice is made
//! before the first webview exists, so it is read from `settings.json` at the very start of the
//! run, before Tauri or any thread starts (`prepare`), and takes a restart to change.
//!
//! Two safety nets keep the choice from ever locking the listener out:
//! - **A start that didn't come up.** A run in a chosen mode is on trial from the moment `main`
//!   shows (or its page dies) until the listener first uses the page. A run that ends still on
//!   trial (a blank window, a crash at start, the listener giving up) makes the next start fall
//!   back to Automatic, and Settings says so.
//! - **Automatic is smart.** The app finds out what draws the page: on Linux WebKitGTK's own report
//!   (`webkit://gpu`, since its page scripts can't tell), on Windows WebGL's renderer, which
//!   WebView2 tells the page truly. Where the processor draws, lull takes its lighter look (no blur
//!   behind glass, no movement), and Settings says so, with the full look a click away.

use crate::desktop::platform;
use crate::desktop::recovery;
use crate::desktop::settings::Settings;
use crate::desktop::state::{Desktop, lock};
use crate::store::Store;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter, Manager};

/// The listener's choice.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// The engine's own choice, with lull's lighter look when it ends up on the processor.
    #[default]
    Auto,
    /// The graphics card always, the more powerful one where there are two.
    Gpu,
    /// No graphics card at all: for drivers that crash or show nothing.
    Cpu,
}

/// This run's graphics: what it uses, and what drawing the page was found to be.
pub struct Graphics {
    /// The mode this run uses: the listener's, or Automatic after a start that didn't come up.
    pub mode: Mode,
    /// What `prepare` told the engine, for the log.
    pub engine: String,
    /// The listener has used the page this run: the mode works here.
    proven: AtomicBool,
    /// The engine's report has been asked for (Linux, once a run).
    asked: AtomicBool,
    found: Mutex<Option<Finding>>,
}

/// What draws the page.
#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    /// The processor: lull's lighter look.
    pub software: bool,
    /// The GL renderer's name, for Settings ("llvmpipe (LLVM 15…)"), if known.
    pub renderer: Option<String>,
    /// The whole answer, for the log.
    pub detail: String,
}

/// The page's own report (Windows): WebGL's renderer, or null when it got no WebGL at all.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Drawn {
    pub renderer: Option<String>,
}

/// What the pages and Settings hear (the `graphics` event, `graphics_state`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    /// The listener's choice, from the next start on.
    pub mode: Mode,
    /// The processor draws the page, whatever was chosen.
    pub software: bool,
    /// The page takes lull's lighter look.
    pub light: bool,
    /// The listener asked for the full look whatever draws the page.
    pub full_look: bool,
    /// The GL renderer's name, when known.
    pub renderer: Option<String>,
    /// The choice lull left for Automatic because its last start didn't come up.
    pub fell_back: Option<Mode>,
    /// The page should report WebGL's renderer (`graphics_report`): where the engine tells pages the
    /// truth (WebView2) and nothing was found yet.
    pub ask: bool,
}

/// Before anything else in the run (no other thread exists yet): the trial of the last start, then
/// the engine's switches for this run's mode.
pub fn prepare(store: &Store<Settings>) -> Graphics {
    let mut settings = store.load();
    if settle_trial(&mut settings) {
        store.save(&settings);
    }
    let mode = settings.graphics;
    let found = (mode == Mode::Cpu).then(|| Finding {
        software: true,
        renderer: None,
        detail: "the processor only, as chosen".into(),
    });
    Graphics {
        mode,
        engine: platform::prepare_graphics(mode),
        proven: AtomicBool::new(false),
        asked: AtomicBool::new(false),
        found: Mutex::new(found),
    }
}

/// The last start's trial, if it never ended: its mode goes back to Automatic (only a chosen mode
/// goes on trial). True when the settings changed.
fn settle_trial(settings: &mut Settings) -> bool {
    let Some(failed) = settings.graphics_trial.take() else {
        return false;
    };
    if failed == settings.graphics && failed != Mode::Auto {
        settings.graphics_fell_back = Some(failed);
        settings.graphics = Mode::Auto;
    }
    true
}

/// `main` showed: until the listener uses the page, this run's mode is on trial.
pub fn shown(app: &AppHandle) {
    on_trial(app);
}

/// `main`'s page died: a start whose page dies before the listener used it is on trial too, shown
/// or not (a hidden `main` in the tray).
pub fn page_lost(app: &AppHandle) {
    on_trial(app);
}

fn on_trial(app: &AppHandle) {
    let graphics = app.state::<Graphics>();
    if graphics.proven.load(Ordering::Relaxed) || graphics.mode == Mode::Auto {
        return;
    }
    let mode = graphics.mode;
    let desktop = app.state::<Desktop>();
    if desktop.settings().graphics_trial != Some(mode) {
        desktop.update(|s| s.graphics_trial = Some(mode));
    }
}

/// The listener used the page (a click or a key in `main`): this mode works here. Ends the trial.
pub fn proven(app: &AppHandle) {
    if app.state::<Graphics>().proven.swap(true, Ordering::Relaxed) {
        return;
    }
    let desktop = app.state::<Desktop>();
    if desktop.settings().graphics_trial.is_some() {
        desktop.update(|s| s.graphics_trial = None);
    }
}

/// `main` loaded a page: on Linux, the first time, ask WebKitGTK what draws it.
pub fn page_loaded(app: &AppHandle) {
    let graphics = app.state::<Graphics>();
    if graphics.mode == Mode::Cpu || graphics.asked.swap(true, Ordering::Relaxed) {
        return;
    }
    let app = app.clone();
    platform::probe_graphics(&app.clone(), move |result| match result {
        Ok(finding) => found(&app, finding),
        Err(e) => recovery::log(&app, &format!("graphics: can't tell what draws the page: {e}")),
    });
}

/// The page's own report (where the engine tells it the truth): WebGL's renderer.
pub fn report(app: &AppHandle, drawn: Drawn) -> State {
    if state(app).ask {
        let software = software_renderer(drawn.renderer.as_deref());
        let detail = format!("WebGL {}", drawn.renderer.as_deref().unwrap_or("unavailable"));
        found(
            app,
            Finding {
                software,
                renderer: drawn.renderer,
                detail,
            },
        );
    }
    state(app)
}

fn found(app: &AppHandle, finding: Finding) {
    let graphics = app.state::<Graphics>();
    recovery::log(
        app,
        &format!(
            "graphics: drawn by the {} ({})",
            if finding.software { "processor" } else { "graphics card" },
            finding.detail
        ),
    );
    *lock(&graphics.found) = Some(finding);
    if let Err(e) = app.emit("graphics", state(app)) {
        eprintln!("lull: graphics event: {e}");
    }
}

/// What Settings shows and the pages apply. Nothing found yet: not the processor.
pub fn state(app: &AppHandle) -> State {
    let graphics = app.state::<Graphics>();
    let settings = app.state::<Desktop>().settings();
    let found = lock(&graphics.found).clone();
    let software = found.as_ref().is_some_and(|f| f.software);
    State {
        mode: settings.graphics,
        software,
        light: software && !settings.full_look,
        full_look: settings.full_look,
        renderer: found.as_ref().and_then(|f| f.renderer.clone()),
        fell_back: settings.graphics_fell_back,
        ask: cfg!(windows) && found.is_none(),
    }
}

/// The listener picked a mode (from the next start on) or the look (at once, in every window).
pub fn choose(app: &AppHandle, mode: Option<Mode>, full_look: Option<bool>) {
    app.state::<Desktop>().update(|s| {
        if let Some(mode) = mode {
            s.graphics = mode;
            s.graphics_fell_back = None;
            s.graphics_trial = None;
        }
        if let Some(full) = full_look {
            s.full_look = full;
        }
    });
    if let Err(e) = app.emit("graphics", state(app)) {
        eprintln!("lull: graphics event: {e}");
    }
}

/// The renderers that are the processor drawing: Mesa's llvmpipe, softpipe and swrast, Chromium's
/// SwiftShader, and Windows' Basic Render Driver (WARP). No renderer at all (a page without WebGL)
/// means no usable graphics driver: the engine draws on the processor too.
pub fn software_renderer(renderer: Option<&str>) -> bool {
    const SOFTWARE: &[&str] = &["llvmpipe", "softpipe", "swrast", "swiftshader", "basic render driver"];
    renderer.is_none_or(|r| {
        let r = r.to_ascii_lowercase();
        SOFTWARE.iter().any(|s| r.contains(s))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn processor_renderers_count_as_software() {
        for r in [
            "llvmpipe (LLVM 15.0.7, 256 bits)",
            "softpipe",
            "ANGLE (Google, Vulkan 1.3.0 (SwiftShader Device (Subzero) (0x0000C0DE)), SwiftShader driver)",
            "ANGLE (Microsoft, Microsoft Basic Render Driver Direct3D11 vs_5_0 ps_5_0, D3D11)",
            "Mesa X11 swrast",
        ] {
            assert!(software_renderer(Some(r)), "{r}");
        }
        assert!(software_renderer(None), "no WebGL at all");
    }

    #[test]
    fn graphics_cards_do_not() {
        for r in [
            "NVIDIA GeForce RTX 3060 Laptop GPU/PCIe/SSE2",
            "Mesa Intel(R) UHD Graphics 620 (KBL GT2)",
            "AMD Radeon RX 6600 (radeonsi, navi23, LLVM 17.0.6, DRM 3.54, 6.6.10)",
            "ANGLE (NVIDIA, NVIDIA GeForce GTX 1650 Direct3D11 vs_5_0 ps_5_0, D3D11)",
        ] {
            assert!(!software_renderer(Some(r)), "{r}");
        }
    }

    #[test]
    fn a_chosen_mode_whose_start_did_not_come_up_falls_back_to_automatic() {
        for mode in [Mode::Gpu, Mode::Cpu] {
            let mut settings = Settings {
                graphics: mode,
                graphics_trial: Some(mode),
                ..Settings::default()
            };
            assert!(settle_trial(&mut settings));
            assert_eq!(settings.graphics, Mode::Auto);
            assert_eq!(settings.graphics_fell_back, Some(mode));
            assert_eq!(settings.graphics_trial, None);
        }
    }

    #[test]
    fn a_start_that_came_up_keeps_its_mode() {
        let mut settings = Settings {
            graphics: Mode::Gpu,
            ..Settings::default()
        };
        assert!(!settle_trial(&mut settings));
        assert_eq!(settings.graphics, Mode::Gpu);
        // a trial left from a mode the listener has since changed is just forgotten
        let mut settings = Settings {
            graphics: Mode::Cpu,
            graphics_trial: Some(Mode::Gpu),
            ..Settings::default()
        };
        assert!(settle_trial(&mut settings));
        assert_eq!((settings.graphics, settings.graphics_fell_back), (Mode::Cpu, None));
    }
}
