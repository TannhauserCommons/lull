//! WebKitGTK's graphics on Linux (graphics.rs): the switches each mode sets in the environment
//! before the first webview, and what the engine itself reports it ended up using (`webkit://gpu`).
//!
//! WebKitGTK reads its switches from the environment: the UI process when it decides how to
//! composite, the page's process (which inherits them) when it makes its GL context. Since 2.44 it
//! composites on the GPU only through its DMA-BUF renderer, on the render node of the UI process's
//! EGL display (or `WEBKIT_WEB_RENDER_DEVICE_FILE`). Its page scripts can't tell what draws them
//! (WebGL's renderer always reads "Apple GPU"), so lull asks the engine's own report.

use crate::desktop::graphics::{Finding, Mode};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tauri::{AppHandle, WebviewUrl, WebviewWindowBuilder};

/// The switches that turn the GPU off, the listener's or a desktop entry's: "Use the graphics card"
/// clears them.
const GPU_OFF: &[&str] = &[
    "WEBKIT_DISABLE_COMPOSITING_MODE",
    "WEBKIT_DISABLE_DMABUF_RENDERER",
    "WEBKIT_SKIA_ENABLE_CPU_RENDERING",
    "LIBGL_ALWAYS_SOFTWARE",
];

/// Mesa's EGL, which draws on the processor where it can't drive the GPU. "Processor only" loads it
/// alone where it is installed, so a vendor's driver that crashes is never loaded at all. Pointing
/// glvnd at a file that isn't there would leave no EGL, so only one that exists is named.
const MESA_EGL: &[&str] = &[
    "/usr/share/glvnd/egl_vendor.d/50_mesa.json",
    "/etc/glvnd/egl_vendor.d/50_mesa.json",
];

/// What lull changed in its own environment, and what each was before: programs lull starts for
/// the listener (their browser) get the environment they would have had without lull.
static CHANGED: OnceLock<Vec<(&'static str, Option<OsString>)>> = OnceLock::new();

/// A graphics card the kernel knows (`/sys/class/drm/cardN`).
#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    /// The kernel driver bound to it (`nvidia`, `amdgpu`, `i915`, `nouveau`…).
    pub driver: String,
    /// The firmware showed the boot screen on it: the card driving the display, the default one.
    pub boot_vga: bool,
    /// Its render node (`/dev/dri/renderD129`), if it has one.
    pub render_node: Option<PathBuf>,
}

/// The more powerful card of a laptop with two, and what makes programs render on it.
#[derive(Debug, Clone, PartialEq)]
pub struct Offload {
    pub driver: String,
    pub env: Vec<(&'static str, String)>,
}

/// Before any thread: the mode's switches, set in this process's environment (the page's process
/// inherits them). Returns what was done, for the log.
pub fn prepare(mode: Mode) -> String {
    let mesa = MESA_EGL.iter().map(Path::new).find(|p| p.exists());
    let offload = if mode == Mode::Gpu { offload(&cards()) } else { None };
    let changes = switches(mode, offload.as_ref(), mesa);
    let mut changed = Vec::new();
    for (name, value) in &changes {
        changed.push((*name, std::env::var_os(name)));
        // SAFETY: called first thing in `desktop::run`, before Tauri, GTK or any thread has started,
        // so nothing can read the environment meanwhile.
        #[allow(unsafe_code)]
        unsafe {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
    }
    let _ = CHANGED.set(changed);
    let said = changes
        .iter()
        .map(|(name, value)| match value {
            Some(value) => format!("{name}={value}"),
            None => format!("-{name}"),
        })
        .collect::<Vec<_>>()
        .join(" ");
    match (mode, offload) {
        (Mode::Auto, _) => "Automatic: WebKitGTK's own choice".into(),
        (Mode::Gpu, Some(o)) => format!("the graphics card, rendering on {} ({said})", o.driver),
        (Mode::Gpu, None) => format!(
            "the graphics card ({})",
            if said.is_empty() { "no switch needed" } else { &said }
        ),
        (Mode::Cpu, _) => format!("the processor only ({said})"),
    }
}

/// The switches for a mode: a value to set, or None to remove the variable.
fn switches(mode: Mode, offload: Option<&Offload>, mesa: Option<&Path>) -> Vec<(&'static str, Option<String>)> {
    match mode {
        Mode::Auto => Vec::new(),
        Mode::Gpu => GPU_OFF
            .iter()
            .map(|name| (*name, None))
            .chain(
                offload
                    .into_iter()
                    .flat_map(|o| o.env.iter().map(|(n, v)| (*n, Some(v.clone())))),
            )
            .collect(),
        Mode::Cpu => [
            ("WEBKIT_DISABLE_COMPOSITING_MODE", "1"),
            ("WEBKIT_DISABLE_DMABUF_RENDERER", "1"),
            ("WEBKIT_SKIA_ENABLE_CPU_RENDERING", "1"),
            ("LIBGL_ALWAYS_SOFTWARE", "1"),
        ]
        .into_iter()
        .map(|(n, v)| (n, Some(v.to_owned())))
        .chain(mesa.map(|p| ("__EGL_VENDOR_LIBRARY_FILENAMES", Some(p.display().to_string()))))
        .collect(),
    }
}

/// The card to render on where there are two: the one that didn't show the boot screen. NVIDIA's
/// own driver takes its render offload switches (what `prime-run` and switcheroo-control set);
/// Mesa's drivers take `DRI_PRIME`. WebKit is pointed at that card's render node as well, since it
/// renders on the node of the display's card otherwise. One card, or no card the kernel drives:
/// nothing to offload.
pub fn offload(cards: &[Card]) -> Option<Offload> {
    if cards.len() < 2 || !cards.iter().any(|c| c.boot_vga) {
        return None;
    }
    let card = cards.iter().find(|c| !c.boot_vga && !c.driver.is_empty())?;
    let mut env = if card.driver == "nvidia" {
        vec![
            ("__NV_PRIME_RENDER_OFFLOAD", "1".to_owned()),
            ("__GLX_VENDOR_LIBRARY_NAME", "nvidia".to_owned()),
            ("__VK_LAYER_NV_optimus", "NVIDIA_only".to_owned()),
        ]
    } else {
        vec![("DRI_PRIME", "1".to_owned())]
    };
    if let Some(node) = &card.render_node {
        env.push(("WEBKIT_WEB_RENDER_DEVICE_FILE", node.display().to_string()));
    }
    Some(Offload {
        driver: card.driver.clone(),
        env,
    })
}

/// The graphics cards in `/sys/class/drm` (`card0`, not its connectors `card0-HDMI-A-1`).
fn cards() -> Vec<Card> {
    let Ok(entries) = std::fs::read_dir("/sys/class/drm") else {
        return Vec::new();
    };
    let mut cards: Vec<(String, Card)> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let number = name.strip_prefix("card")?;
            if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let device = entry.path().join("device");
            let driver = std::fs::read_link(device.join("driver"))
                .ok()
                .and_then(|d| d.file_name().map(|n| n.to_string_lossy().into_owned()))
                .unwrap_or_default();
            let boot_vga = std::fs::read_to_string(device.join("boot_vga")).is_ok_and(|s| s.trim() == "1");
            let render_node = std::fs::read_dir(device.join("drm")).ok().and_then(|nodes| {
                nodes
                    .flatten()
                    .map(|n| n.file_name().to_string_lossy().into_owned())
                    .find(|n| n.starts_with("renderD"))
                    .map(|n| Path::new("/dev/dri").join(n))
            });
            Some((
                name,
                Card {
                    driver,
                    boot_vga,
                    render_node,
                },
            ))
        })
        .collect();
    cards.sort_by(|a, b| a.0.cmp(&b.0));
    cards.into_iter().map(|(_, card)| card).collect()
}

/// A program lull starts for the listener gets the environment lull itself was started with.
pub fn as_started(command: &mut std::process::Command) {
    for (name, before) in CHANGED.get().into_iter().flatten() {
        match before {
            Some(value) => command.env(name, value),
            None => command.env_remove(name),
        };
    }
}

/// The page that reports the engine's graphics.
const REPORT: &str = "webkit://gpu";
const PROBE: &str = "graphics-probe";

/// Reads the engine's own report in a hidden webview of its own, once, and calls `found` with what
/// it says. The webview goes once it has answered.
pub fn probe(app: &AppHandle, found: impl FnOnce(Result<Finding, String>) + Send + 'static) {
    let url = match REPORT.parse() {
        Ok(url) => url,
        Err(e) => return found(Err(format!("{REPORT}: {e}"))),
    };
    let found = std::sync::Mutex::new(Some(found));
    let built = WebviewWindowBuilder::new(app, PROBE, WebviewUrl::External(url))
        .visible(false)
        .skip_taskbar(true)
        .inner_size(320.0, 240.0)
        .on_page_load(move |window, payload| {
            if payload.event() != tauri::webview::PageLoadEvent::Finished {
                return;
            }
            let Some(found) = crate::desktop::state::lock(&found).take() else {
                return;
            };
            let window = window.clone();
            let read = window.clone().with_webview(move |webview| {
                use webkit2gtk::WebViewExt;
                webview.inner().evaluate_javascript(
                    READ_TABLES,
                    None,
                    None,
                    None::<&gtk::gio::Cancellable>,
                    move |result| {
                        use javascriptcore::ValueExt;
                        let _ = window.destroy();
                        found(
                            result
                                .map_err(|e| e.to_string())
                                .and_then(|value| tables(&value.to_str()))
                                .map(|tables| finding(&tables)),
                        );
                    },
                );
            });
            if let Err(e) = read {
                eprintln!("lull: can't read {REPORT}: {e}");
            }
        })
        .build();
    if let Err(e) = built {
        eprintln!("lull: can't open {REPORT}: {e}");
    }
}

/// `webkit://gpu`'s tables as `[[heading, [[key, value]…]]…]`.
const READ_TABLES: &str = "JSON.stringify(Array.from(document.querySelectorAll('h1')).map(function (h) { \
     var t = h.nextElementSibling; return [h.textContent, t && t.rows ? Array.from(t.rows).map(function (r) { \
     return [r.cells[0].innerText.trim(), r.cells[1] ? r.cells[1].innerText.trim() : '']; }) : []]; }))";

type Tables = Vec<(String, Vec<(String, String)>)>;

fn tables(json: &str) -> Result<Tables, String> {
    serde_json::from_str(json).map_err(|e| format!("{REPORT} read as {json:.200}: {e}"))
}

/// What the report says draws the page. The processor, when:
/// - the policy is "never";
/// - WebKitGTK 2.44 or later has no DMA-BUF renderer (its only way to composite on the GPU since);
/// - the GL renderer the page's process gets (or the UI process's) is a software one (llvmpipe).
fn finding(tables: &Tables) -> Finding {
    let row = |table: &str, key: &str| {
        tables
            .iter()
            .filter(|(heading, _)| heading.eq_ignore_ascii_case(table))
            .flat_map(|(_, rows)| rows)
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    };
    const UI: &str = "Hardware Acceleration Information";
    const PAGE: &str = "Hardware Acceleration Information (Render Process)";
    let webkit = row("Version Information", "WebKit version").unwrap_or("WebKitGTK (version unknown)");
    let policy = row(UI, "Policy").unwrap_or("unknown");
    let renderer = row(UI, "Renderer");
    let gl = row(PAGE, "GL_RENDERER").or_else(|| row(UI, "GL_RENDERER"));
    let only_dmabuf = webkit_at_least(webkit, 2, 44);
    let software = policy == "never"
        || (renderer.is_none() && only_dmabuf)
        || gl.is_some_and(|gl| crate::desktop::graphics::software_renderer(Some(gl)));
    Finding {
        software,
        detail: format!(
            "{webkit}; policy {policy}; renderer {}; GL {}",
            renderer.unwrap_or("none"),
            gl.unwrap_or("none")
        ),
        renderer: gl.map(str::to_owned),
    }
}

/// "WebKitGTK 2.50.4 (…)" is at least major.minor.
fn webkit_at_least(version: &str, major: u32, minor: u32) -> bool {
    let numbers = version
        .split_whitespace()
        .find(|w| w.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .map(|v| v.split('.').map_while(|n| n.parse::<u32>().ok()).collect::<Vec<_>>())
        .unwrap_or_default();
    match numbers.as_slice() {
        [a, b, ..] => (*a, *b) >= (major, minor),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(driver: &str, boot_vga: bool, node: Option<&str>) -> Card {
        Card {
            driver: driver.into(),
            boot_vga,
            render_node: node.map(PathBuf::from),
        }
    }

    #[test]
    fn a_hybrid_nvidia_laptop_renders_on_the_nvidia_card() {
        let cards = [
            card("i915", true, Some("/dev/dri/renderD128")),
            card("nvidia", false, Some("/dev/dri/renderD129")),
        ];
        let o = offload(&cards);
        assert_eq!(o.as_ref().map(|o| o.driver.as_str()), Some("nvidia"));
        let env = o.map(|o| o.env).unwrap_or_default();
        assert!(env.contains(&("__NV_PRIME_RENDER_OFFLOAD", "1".into())));
        assert!(env.contains(&("__GLX_VENDOR_LIBRARY_NAME", "nvidia".into())));
        assert!(env.contains(&("WEBKIT_WEB_RENDER_DEVICE_FILE", "/dev/dri/renderD129".into())));
        assert!(!env.iter().any(|(n, _)| *n == "DRI_PRIME"));
    }

    #[test]
    fn a_mesa_discrete_card_takes_dri_prime() {
        let cards = [
            card("amdgpu", true, None),
            card("amdgpu", false, Some("/dev/dri/renderD129")),
        ];
        let env = offload(&cards).map(|o| o.env).unwrap_or_default();
        assert!(env.contains(&("DRI_PRIME", "1".into())));
    }

    #[test]
    fn one_card_or_none_driving_the_display_offloads_nothing() {
        assert_eq!(offload(&[card("nvidia", true, None)]), None);
        assert_eq!(offload(&[]), None);
        // a card without a driver (nouveau blacklisted, NVIDIA's not installed) is no card to render on
        assert_eq!(offload(&[card("i915", true, None), card("", false, None)]), None);
        // no card showed the boot screen (a VM, a server): no default to tell apart
        assert_eq!(
            offload(&[card("virtio_gpu", false, None), card("nvidia", false, None)]),
            None
        );
    }

    #[test]
    fn each_mode_sets_its_switches() {
        assert!(
            switches(Mode::Auto, None, None).is_empty(),
            "Automatic leaves WebKitGTK alone"
        );

        let gpu = switches(Mode::Gpu, None, None);
        for name in GPU_OFF {
            assert!(gpu.contains(&(name, None)), "the graphics card clears {name}");
        }
        let nvidia = Offload {
            driver: "nvidia".into(),
            env: vec![("__NV_PRIME_RENDER_OFFLOAD", "1".into())],
        };
        assert!(switches(Mode::Gpu, Some(&nvidia), None).contains(&("__NV_PRIME_RENDER_OFFLOAD", Some("1".into()))));

        let cpu = switches(Mode::Cpu, None, None);
        for name in [
            "WEBKIT_DISABLE_COMPOSITING_MODE",
            "WEBKIT_DISABLE_DMABUF_RENDERER",
            "WEBKIT_SKIA_ENABLE_CPU_RENDERING",
            "LIBGL_ALWAYS_SOFTWARE",
        ] {
            assert!(
                cpu.contains(&(name, Some("1".into()))),
                "the processor only sets {name}"
            );
        }
        assert!(
            !cpu.iter().any(|(n, _)| *n == "__EGL_VENDOR_LIBRARY_FILENAMES"),
            "no Mesa EGL file, no glvnd switch"
        );
        let mesa = Path::new("/usr/share/glvnd/egl_vendor.d/50_mesa.json");
        assert!(
            switches(Mode::Cpu, None, Some(mesa))
                .contains(&("__EGL_VENDOR_LIBRARY_FILENAMES", Some(mesa.display().to_string())))
        );
    }

    fn report(policy: &str, renderer: Option<&str>, gl: &str) -> Tables {
        let mut ui = vec![("Policy".to_owned(), policy.to_owned())];
        if let Some(r) = renderer {
            ui.push(("Renderer".into(), r.into()));
        }
        vec![
            (
                "Version Information".into(),
                vec![("WebKit version".into(), "WebKitGTK 2.50.4 (tarball)".into())],
            ),
            ("Hardware Acceleration Information".into(), ui),
            (
                "Hardware Acceleration Information (Render Process)".into(),
                vec![("GL_RENDERER".into(), gl.into())],
            ),
        ]
    }

    #[test]
    fn the_engine_report_tells_the_processor_from_the_card() {
        let dmabuf = Some("DMABuf (Supported buffers: Hardware, Shared Memory)");
        assert!(!finding(&report("always", dmabuf, "Mesa Intel(R) UHD Graphics 620 (KBL GT2)")).software);
        assert!(finding(&report("always", dmabuf, "llvmpipe (LLVM 15.0.7, 256 bits)")).software);
        assert!(finding(&report("never", None, "")).software, "policy never");
        assert!(
            finding(&report("always", None, "NVIDIA GeForce RTX 3060")).software,
            "no DMA-BUF renderer since 2.44"
        );
    }

    #[test]
    fn webkit_versions_compare() {
        assert!(webkit_at_least("WebKitGTK 2.50.4 (tarball)", 2, 44));
        assert!(webkit_at_least("WebKitGTK 2.44.0", 2, 44));
        assert!(!webkit_at_least("WebKitGTK 2.42.5 (tarball)", 2, 44));
        assert!(!webkit_at_least("unknown", 2, 44));
    }
}
