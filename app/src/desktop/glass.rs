//! A see-through `main` (docs/themes.md section 12; docs/context/desktop.md). The theme in use
//! asks for materials in its order (designs.js resolves theme.json's `window` for the mode shown);
//! the first one this computer draws without lag is granted, unless the listener (Settings ›
//! Appearance) or the system turned transparency off. The page hears what it got and styles itself
//! for that (`:root[data-window]`), so it never draws for a material it doesn't have.

use crate::desktop::main_window;
use crate::desktop::platform;
use crate::desktop::state::Desktop;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

/// What a window can show behind the page. Solid is no material at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Material {
    /// Windows 11: the wallpaper, blurred and tinted by the system.
    Mica,
    /// Frosted glass over whatever is behind the window (Windows 11).
    Acrylic,
    /// Blur over whatever is behind the window (Windows 10; KDE on X11).
    Blur,
    /// Clear: the desktop as it is.
    Transparent,
}

/// What the page asks for: the theme's materials in order, already resolved for day or night.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    pub materials: Vec<Material>,
    /// The page shows its night mode: Mica's dark variant.
    pub dark: bool,
    /// The page's `prefers-reduced-transparency` matches (the system asked for less transparency).
    pub reduced: bool,
}

/// What `main` shows, as the page hears it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Grant {
    pub material: Option<Material>,
    /// Why a theme that asked got nothing.
    pub held: Option<Held>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Held {
    /// "See-through window" is off in Settings › Appearance.
    Off,
    /// Windows' "Transparency effects" are off, or the system asks for reduced transparency.
    System,
    /// This computer can't draw any material the theme lists (or not without lag).
    Unsupported,
    /// Linux: `main` was made solid when it opened; a restart opens it see-through.
    Restart,
}

/// What this computer and its listener allow right now.
pub struct Here<'a> {
    /// The materials this computer draws without lag, from `platform::glass_materials`.
    pub can: &'a [Material],
    pub listener_on: bool,
    pub system_on: bool,
    /// `main` can become see-through without being made again (always on Windows).
    pub window_alpha: bool,
}

/// The first material the theme lists that is allowed here, or why there is none.
pub fn grant(request: Option<&Request>, here: &Here) -> Grant {
    let held = |why| Grant {
        material: None,
        held: Some(why),
    };
    let Some(request) = request.filter(|r| !r.materials.is_empty()) else {
        return Grant {
            material: None,
            held: None,
        };
    };
    if !here.listener_on {
        return held(Held::Off);
    }
    if !here.system_on || request.reduced {
        return held(Held::System);
    }
    match request.materials.iter().find(|m| here.can.contains(m)) {
        None => held(Held::Unsupported),
        Some(_) if !here.window_alpha => held(Held::Restart),
        Some(&material) => Grant {
            material: Some(material),
            held: None,
        },
    }
}

/// The page's request (the theme or its mode changed): kept for the next launch, applied now.
pub fn ask(app: &AppHandle, request: Option<Request>) -> Grant {
    let desktop = app.state::<Desktop>();
    if desktop.settings().window != request {
        desktop.update(|s| s.window = request);
    }
    apply(app)
}

/// Something else changed (the listener's setting, Windows' transparency effects): `main` follows,
/// and its page hears the new grant.
pub fn refresh(app: &AppHandle) {
    let before = app.state::<Desktop>().glass();
    let now = apply(app);
    if now != before
        && let Err(e) = app.emit_to(main_window::LABEL, "glass", now)
    {
        eprintln!("lull: glass: {e}");
    }
}

/// Whether `main`, about to be made, should be able to show the desktop (Linux decides it then).
pub fn wants_alpha(app: &AppHandle) -> bool {
    let desktop = app.state::<Desktop>();
    let here = here(&desktop, true);
    grant(desktop.settings().window.as_ref(), &here).material.is_some()
}

/// `main` draws what is granted now. Before `main` exists only the grant is kept.
pub fn apply(app: &AppHandle) -> Grant {
    let desktop = app.state::<Desktop>();
    let settings = desktop.settings();
    let now = grant(settings.window.as_ref(), &here(&desktop, desktop.main_alpha()));
    desktop.set_glass(now);
    if let Some(main) = app.get_webview_window(main_window::LABEL) {
        let dark = settings.window.as_ref().is_some_and(|r| r.dark);
        main_window::report(platform::set_glass(&main, now.material, dark));
    }
    now
}

fn here<'a>(desktop: &'a Desktop, window_alpha: bool) -> Here<'a> {
    Here {
        can: &desktop.platform.glass,
        listener_on: desktop.settings().glass,
        system_on: platform::transparency_effects(),
        window_alpha,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Material::*;

    fn asking(materials: &[Material]) -> Request {
        Request {
            materials: materials.to_vec(),
            dark: false,
            reduced: false,
        }
    }

    const WINDOWS_10: &[Material] = &[Blur, Transparent];
    const WINDOWS_11: &[Material] = &[Mica, Acrylic, Transparent];

    fn on(can: &[Material]) -> Here<'_> {
        Here {
            can,
            listener_on: true,
            system_on: true,
            window_alpha: true,
        }
    }

    #[test]
    fn the_first_material_this_computer_draws_without_lag_wins_in_the_themes_order() {
        let glass = asking(&[Mica, Acrylic, Blur]);
        assert_eq!(grant(Some(&glass), &on(WINDOWS_11)).material, Some(Mica));
        assert_eq!(grant(Some(&glass), &on(WINDOWS_10)).material, Some(Blur));
        let frosted = asking(&[Acrylic, Blur]);
        assert_eq!(grant(Some(&frosted), &on(WINDOWS_11)).material, Some(Acrylic));
        // Clear glass only when the theme lists it: blur never falls back to it by itself.
        let blur = asking(&[Blur]);
        let none = grant(Some(&blur), &on(WINDOWS_11));
        assert_eq!((none.material, none.held), (None, Some(Held::Unsupported)));
        assert_eq!(
            grant(Some(&asking(&[Blur, Transparent])), &on(WINDOWS_11)).material,
            Some(Transparent)
        );
        assert_eq!(grant(Some(&glass), &on(&[])).held, Some(Held::Unsupported));
    }

    #[test]
    fn a_solid_theme_asks_nothing_and_hears_no_reason() {
        for request in [None, Some(asking(&[]))] {
            assert_eq!(
                grant(request.as_ref(), &on(WINDOWS_11)),
                Grant {
                    material: None,
                    held: None
                }
            );
        }
    }

    #[test]
    fn the_listener_and_the_system_can_turn_glass_off() {
        let glass = asking(&[Mica, Acrylic, Blur]);
        let off = Here {
            listener_on: false,
            ..on(WINDOWS_11)
        };
        assert_eq!(grant(Some(&glass), &off).held, Some(Held::Off));
        let system_off = Here {
            system_on: false,
            ..on(WINDOWS_11)
        };
        assert_eq!(grant(Some(&glass), &system_off).held, Some(Held::System));
        let reduced = Request {
            reduced: true,
            ..glass.clone()
        };
        let got = grant(Some(&reduced), &on(WINDOWS_11));
        assert_eq!((got.material, got.held), (None, Some(Held::System)));
        let solid_window = Here {
            window_alpha: false,
            ..on(WINDOWS_11)
        };
        assert_eq!(grant(Some(&glass), &solid_window).held, Some(Held::Restart));
    }

    #[test]
    fn the_page_can_only_ask_for_materials_lull_knows() {
        let read = |json: &str| serde_json::from_str::<Request>(json);
        assert!(read(r#"{ "materials": ["mica", "blur"], "dark": true, "reduced": false }"#).is_ok());
        assert!(read(r#"{ "materials": ["frosty"], "dark": true, "reduced": false }"#).is_err());
        assert!(read(r#"{ "materials": ["none"], "dark": true, "reduced": false }"#).is_err());
        assert!(read(r##"{ "materials": [], "dark": true, "reduced": false, "tint": "#fff" }"##).is_err());
    }
}
