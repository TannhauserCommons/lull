//! The listener's theme for the bundled pages (docs/themes.md section 5; docs/context/desktop.md).
//! Those pages come from the app, not the hub, so they can't fetch a theme stored on the hub. `main`
//! has it loaded: it hands the theme in use over as one package (manifest, CSS with its assets
//! inlined, slot templates), the app keeps it for this run, and the pages ask for it by id. Without
//! it a page shows the design the theme extends (`base` in its hash).

use crate::desktop::state::Desktop;
use crate::pages::Design;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};

/// The hub takes a theme of up to 16 MB; inlined as `data:` URLs its assets grow by a third.
const MAX_BYTES: usize = 24 << 20;

/// A listener's theme as `main` loaded it (designs.js, `Designs.packageOf`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Package {
    /// theme.json as the hub serves it; the pages read it as they read the hub's.
    pub manifest: Manifest,
    /// theme.css, every `url(assets/...)` a `data:` URL.
    pub css: String,
    /// slot name (`miniPlayer`, `brand.night`, ...) -> template, untrusted: the pages sanitize it.
    pub slots: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub id: Design,
    #[serde(flatten)]
    pub rest: serde_json::Map<String, serde_json::Value>,
}

impl Package {
    fn size(&self) -> usize {
        self.css.len() + self.slots.iter().map(|(k, v)| k.len() + v.len()).sum::<usize>()
    }
}

/// `main` shows this theme now (None: a built-in design). The bundled pages hear `theme-package`
/// with its id, and ask for it again.
pub fn hand_over(app: &AppHandle, package: Option<Package>) -> Result<(), String> {
    if let Some(p) = &package
        && p.size() > MAX_BYTES
    {
        return Err(format!(
            "theme {} is {} bytes, more than {MAX_BYTES}",
            p.manifest.id.as_str(),
            p.size()
        ));
    }
    let desktop = app.state::<Desktop>();
    let id = package.as_ref().map(|p| p.manifest.id.as_str().to_owned());
    if desktop.theme().as_deref() == package.as_ref() {
        return Ok(());
    }
    desktop.set_theme(package.map(Arc::new));
    if let Some(id) = id {
        app.emit("theme-package", serde_json::json!({ "id": id }))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// The theme `main` handed over, if it is the one asked for.
pub fn package(app: &AppHandle, id: &str) -> Option<Package> {
    app.state::<Desktop>()
        .theme()
        .filter(|p| p.manifest.id.as_str() == id)
        .map(|p| (*p).clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_package_is_what_designs_js_sends_and_its_id_is_a_design_name() -> Result<(), String> {
        let sent = r#"{"manifest":{"id":"paper-moon","name":"Paper moon","extends":"skydeck","api":1},
            "css":":root{--sky:#123}","slots":{"miniPlayer":"<div></div>"}}"#;
        let p: Package = serde_json::from_str(sent).map_err(|e| e.to_string())?;
        assert_eq!(p.manifest.id.as_str(), "paper-moon");
        assert_eq!(p.manifest.rest["extends"], "skydeck");
        let back = serde_json::to_value(&p).map_err(|e| e.to_string())?;
        assert_eq!(back["manifest"]["name"], "Paper moon");

        let odd = r#"{"manifest":{"id":"../x"},"css":"","slots":{}}"#;
        assert!(serde_json::from_str::<Package>(odd).is_err());
        let extra = r#"{"manifest":{"id":"x"},"css":"","slots":{},"script":"alert(1)"}"#;
        assert!(serde_json::from_str::<Package>(extra).is_err());
        Ok(())
    }
}
