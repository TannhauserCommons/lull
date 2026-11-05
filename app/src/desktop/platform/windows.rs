//! Windows: the tray glyph follows the taskbar's theme, not the app's (DESKTOP.md 4), the sidecar
//! dies with the app, a hidden `main` tells WebView2 so, a see-through `main` shows the system's
//! glass, links open in the browser, and the UI sounds hear the sound scheme and which window is in
//! front (ADR-0081).
//! `SystemUsesLightTheme` is that theme; Tauri's ThemeChanged follows the app theme instead, so a
//! thread waits on the registry key and swaps the glyph when it changes.
#![allow(unsafe_code)]

use crate::desktop::glass::Material;
use std::ffi::c_void;
use std::ptr::null_mut;
use windows::Win32::Graphics::Dwm::DwmExtendFrameIntoClientArea;
use windows::Win32::UI::Controls::MARGINS;
use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_NOTIFY, KEY_READ, KEY_SET_VALUE, REG_NOTIFY_CHANGE_LAST_SET, REG_OPTION_NON_VOLATILE,
    REG_SZ, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegCloseKey, RegCreateKeyExW, RegGetValueW, RegNotifyChangeKeyValue,
    RegOpenKeyExW, RegSetValueExW,
};
use windows_sys::w;

const PERSONALIZE: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
/// Its default value names the sound scheme: `.None` is "No Sounds" (Control Panel › Sound › Sounds).
const SCHEMES: &str = r"AppEvents\Schemes";

/// A light taskbar. Without the value (Windows 10 before 1903) the taskbar is dark.
pub fn light_taskbar() -> bool {
    personalize(w!("SystemUsesLightTheme")) == Some(1)
}

/// "Transparency effects" (Settings › Personalisation › Colours). Without the value it is on.
pub fn transparency_effects() -> bool {
    personalize(w!("EnableTransparency")) != Some(0)
}

/// A DWORD under the Personalize key, if it is there.
fn personalize(name: *const u16) -> Option<u32> {
    let path = wide(PERSONALIZE);
    let mut value: u32 = 0;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: a DWORD read into a u32 of the size we pass; the strings are null-terminated and
    // outlive the call.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            path.as_ptr(),
            name,
            RRF_RT_REG_DWORD,
            null_mut(),
            (&raw mut value).cast::<c_void>(),
            &mut size,
        )
    };
    (status == ERROR_SUCCESS).then_some(value)
}

/// The sound scheme is "No Sounds": the listener asked Windows for quiet, and lull's UI sounds rest.
/// Without the value (no scheme chosen) it isn't.
pub fn no_sounds() -> bool {
    const NONE: &str = ".None";
    let path = wide(SCHEMES);
    // Room for ".None" and its null; a longer name doesn't fit, and isn't ".None" either.
    let mut name = [0u16; 8];
    let mut size = size_of_val(&name) as u32;
    // SAFETY: a REG_SZ read into a buffer of the size we pass; a null value name reads the key's
    // default value; the path is null-terminated and outlives the call.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            path.as_ptr(),
            std::ptr::null(),
            RRF_RT_REG_SZ,
            null_mut(),
            name.as_mut_ptr().cast::<c_void>(),
            &mut size,
        )
    };
    let read = (size as usize / 2).min(name.len());
    status == ERROR_SUCCESS && String::from_utf16_lossy(&name[..read]).trim_end_matches('\0') == NONE
}

/// Calls `changed` with `no_sounds` now, then again whenever the sound scheme changes, for the life
/// of the app.
pub fn follow_sound_scheme(changed: impl Fn(bool) + Send + 'static) {
    changed(no_sounds());
    watch_key(SCHEMES, "the sound scheme", move || changed(no_sounds()));
}

/// One of lull's own windows is in front: the foreground window belongs to this process and shows
/// (`main`, the mini player, the Classic window, native Classic's windows, a dialog of lull's). The
/// tray's hidden window, which Windows puts in front while the tray's menu is open, doesn't count:
/// it stays there after the menu closes, until another window comes forward. Any thread.
pub fn foreground_is_ours() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId, IsWindowVisible};
    let mut process = 0u32;
    // SAFETY: plain queries; the process id is written into a u32 that outlives the call. A null
    // window (none in front, or a desktop switch) leaves it 0, which is never ours.
    let visible = unsafe {
        let front = GetForegroundWindow();
        GetWindowThreadProcessId(front, Some(&raw mut process));
        IsWindowVisible(front).as_bool()
    };
    visible && process == std::process::id()
}

/// `s` as a null-terminated UTF-16 string for the Win32 API.
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// What `main` can show here without lag (measured on Windows 10 22H2: docs/context/desktop.md):
/// - Mica from Windows 11 (build 22000);
/// - Acrylic from build 22523, where the system draws it as a backdrop. Before that (Windows 10,
///   Windows 11 21H2) it is the old accent policy, whose window trails the pointer when dragged;
/// - Blur, the accent policy's plain blur, on Windows 10 only: on Windows 11 it lags the same way;
/// - Transparent everywhere.
pub fn glass_materials() -> Vec<Material> {
    let build = windows_version::OsVersion::current().build;
    [
        (Material::Mica, build >= 22000),
        (Material::Acrylic, build >= 22523),
        (Material::Blur, (17763..22000).contains(&build)),
        (Material::Transparent, true),
    ]
    .into_iter()
    .filter_map(|(m, here)| here.then_some(m))
    .collect()
}

/// `main` shows `material` behind its page, or is solid again. `main` is made transparent (its own
/// surface clear, its alpha let through: main_window.rs), so this only turns the webview's background
/// clear or back to solid white, and has the system draw the material under it. Mica and Acrylic are
/// drawn in the window's frame, so the frame covers the whole window while one is on.
pub fn set_glass(window: &tauri::WebviewWindow, material: Option<Material>, dark: bool) -> tauri::Result<()> {
    let webview: &tauri::Webview = window.as_ref();
    let clear = tauri::window::Color(0, 0, 0, 0);
    let solid = tauri::window::Color(255, 255, 255, 255);
    webview.set_background_color(Some(if material.is_some() { clear } else { solid }))?;
    let target = window.clone();
    window.run_on_main_thread(move || {
        let Ok(hwnd) = target.hwnd() else {
            return;
        };
        // Only one material at a time: whichever was on goes first. Each clear is refused where its
        // material doesn't exist, which is fine.
        let _ = window_vibrancy::clear_blur(&target);
        let _ = window_vibrancy::clear_acrylic(&target);
        let _ = window_vibrancy::clear_mica(&target);
        let frame = if matches!(material, Some(Material::Mica | Material::Acrylic)) {
            -1
        } else {
            0
        };
        let margins = MARGINS {
            cxLeftWidth: frame,
            cxRightWidth: frame,
            cyTopHeight: frame,
            cyBottomHeight: frame,
        };
        // SAFETY: `hwnd` is `main`'s, alive on this (its UI) thread.
        let framed = unsafe { DwmExtendFrameIntoClientArea(hwnd, &margins) };
        let drawn = match material {
            None | Some(Material::Transparent) => Ok(()),
            Some(Material::Blur) => window_vibrancy::apply_blur(&target, Some((0, 0, 0, 0))),
            Some(Material::Acrylic) => window_vibrancy::apply_acrylic(&target, None),
            Some(Material::Mica) => window_vibrancy::apply_mica(&target, Some(dark)),
        };
        if let Err(e) = framed {
            eprintln!("lull: glass frame: {e}");
        }
        if let Err(e) = drawn {
            eprintln!("lull: glass {material:?}: {e}");
        }
    })
}

/// `<scheme>:` links start `exe` with the link as its argument: `HKCU\Software\Classes\<scheme>`,
/// which needs no admin and follows the app wherever it is installed.
pub fn register_scheme(scheme: &str, exe: &std::path::Path) -> Result<(), String> {
    let root = format!(r"Software\Classes\{scheme}");
    set_string(&root, "", &format!("URL:{scheme} link"))?;
    set_string(&root, "URL Protocol", "")?;
    set_string(
        &format!(r"{root}\shell\open\command"),
        "",
        &format!("\"{}\" \"%1\"", exe.display()),
    )
}

/// One string value under `HKEY_CURRENT_USER\<path>` (the key made when missing); `""` names the
/// key's default value.
fn set_string(path: &str, name: &str, value: &str) -> Result<(), String> {
    let (path, name, value) = (wide(path), wide(name), wide(value));
    let mut key: HKEY = null_mut();
    // SAFETY: null-terminated strings that outlive the call; `key` is closed below.
    let made = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            path.as_ptr(),
            0,
            std::ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            std::ptr::null(),
            &mut key,
            null_mut(),
        )
    };
    if made != ERROR_SUCCESS {
        return Err(format!("the registry refused the key (error {made})"));
    }
    let bytes = u32::try_from(value.len() * 2).unwrap_or(u32::MAX);
    // SAFETY: a REG_SZ of `bytes` bytes, its terminating null included, from a live buffer.
    let set = unsafe { RegSetValueExW(key, name.as_ptr(), 0, REG_SZ, value.as_ptr().cast::<u8>(), bytes) };
    // SAFETY: the key opened above.
    unsafe { RegCloseKey(key) };
    if set == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(format!("the registry refused the value (error {set})"))
    }
}

/// Calls `changed` after every write under the Personalize key, for the life of the app.
pub fn watch_taskbar(changed: impl Fn() + Send + 'static) {
    watch_key(PERSONALIZE, "the taskbar theme", changed);
}

/// Calls `changed`, from a thread of its own, after every write to a value of the key at
/// `HKEY_CURRENT_USER\<path>` (not its subkeys), for the life of the app.
fn watch_key(path: &'static str, what: &'static str, changed: impl Fn() + Send + 'static) {
    let spawned = std::thread::Builder::new().name("registry".into()).spawn(move || {
        let path = wide(path);
        let mut key: HKEY = null_mut();
        // SAFETY: opens a key we close below; `key` is only used on this thread.
        let opened = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, path.as_ptr(), 0, KEY_NOTIFY | KEY_READ, &mut key) };
        if opened != ERROR_SUCCESS {
            eprintln!("lull: can't watch {what} (error {opened})");
            return;
        }
        // SAFETY: synchronous wait on the open key; returns once a value under it changes.
        while unsafe { RegNotifyChangeKeyValue(key, 0, REG_NOTIFY_CHANGE_LAST_SET, null_mut(), 0) } == ERROR_SUCCESS {
            changed();
        }
        // SAFETY: the key opened above.
        unsafe { RegCloseKey(key) };
    });
    if let Err(e) = spawned {
        eprintln!("lull: can't watch {what}: {e}");
    }
}

/// Puts `child` in a job object that Windows closes, killing the child, when lull exits for any
/// reason (a crash included). One job for the life of the app.
pub fn kill_with_app(child: &std::process::Child) -> Result<(), String> {
    use std::os::windows::io::AsRawHandle;
    use std::sync::OnceLock;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, SetInformationJobObject,
    };
    // The handle as an address: raw handles aren't Send, and this one is never closed on purpose.
    static JOB: OnceLock<Result<usize, String>> = OnceLock::new();
    let job = JOB.get_or_init(|| {
        // SAFETY: a new unnamed job with default security; the handle stays open for the app's life.
        let job = unsafe { CreateJobObjectW(null_mut(), null_mut()) };
        if job.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        // SAFETY: a zeroed limit struct is valid; we set only the kill-on-close flag.
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: the struct and its size match the information class.
        let set = unsafe {
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast::<c_void>(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if set == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(job as usize)
    });
    let job = job.clone()?;
    // SAFETY: both handles are open: the job for the app's life, the process while `child` lives.
    let assigned = unsafe { AssignProcessToJobObject(job as *mut c_void, child.as_raw_handle()) };
    if assigned == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(())
}

/// A hidden webview still renders and keeps its memory unless WebView2 is told: not visible means
/// no frames and throttled timers; the low memory target lets it trim caches. Shown again: normal.
pub fn set_webview_awake(window: &tauri::WebviewWindow, awake: bool) -> tauri::Result<()> {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL, ICoreWebView2_19,
    };
    use windows::core::Interface;
    window.with_webview(move |webview| {
        let controller = webview.controller();
        let level = if awake {
            COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL
        } else {
            COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW
        };
        // SAFETY: the controller and its webview are live; Tauri runs this on their UI thread.
        let result = unsafe {
            controller.SetIsVisible(awake).and_then(|()| {
                controller
                    .CoreWebView2()?
                    .cast::<ICoreWebView2_19>()?
                    .SetMemoryUsageTargetLevel(level)
            })
        };
        if let Err(e) = result {
            eprintln!("lull: webview {}: {e}", if awake { "wake" } else { "sleep" });
        }
    })
}

/// The listener's default browser. Only http(s) addresses reach here (links.rs).
pub fn open_in_browser(url: &tauri::Url) -> Result<(), String> {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::HSTRING;
    let target = HSTRING::from(url.as_str());
    // SAFETY: null-terminated strings that outlive the call.
    let result = unsafe { ShellExecuteW(None, windows::core::w!("open"), &target, None, None, SW_SHOWNORMAL) };
    // ShellExecute returns a number above 32 on success.
    if result.0 as usize > 32 {
        Ok(())
    } else {
        Err(format!("Windows couldn't open it (error {})", result.0 as usize))
    }
}

/// One WebView2 environment for all of lull's windows, made on the UI thread before the first.
/// Wry's own options (its default browser arguments), plus custom crash reporting: a crashed
/// process's minidump then stays in the app's data folder (`EBWebView\Crashpad\reports`) instead of
/// going to Microsoft (docs/context/desktop.md, Crashes).
struct Environment(webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Environment);
// SAFETY: only ever cloned (AddRef, which is thread-safe) and handed to Tauri, which uses it on the
// UI thread that made it, as it does with the environments it passes between windows itself.
unsafe impl Send for Environment {}
unsafe impl Sync for Environment {}

static ENVIRONMENT: std::sync::OnceLock<Environment> = std::sync::OnceLock::new();

/// What wry passes when it makes the environment itself (wry's webview2 `create_environment`, with
/// autoplay on as Tauri leaves it): the same, so nothing else changes.
const BROWSER_ARGS: &str =
    "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --autoplay-policy=no-user-gesture-required";

/// Makes the shared environment; says which WebView2 runs and where its crash dumps go.
pub fn create_environment(data_dir: &std::path::Path) -> Result<String, String> {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        CreateCoreWebView2EnvironmentWithOptions, ICoreWebView2Environment11, ICoreWebView2EnvironmentOptions,
    };
    use webview2_com::{CoreWebView2EnvironmentOptions, CreateCoreWebView2EnvironmentCompletedHandler, take_pwstr};
    use windows::Win32::Foundation::{E_POINTER, E_UNEXPECTED};
    use windows::core::{HSTRING, Interface, PCWSTR, PWSTR};

    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};

    // This runs before the first window, so before wry has set up COM on the UI thread: do as it
    // does (an STA; a second call on the thread is harmless).
    // SAFETY: called on the UI thread, which stays an STA for the app's life.
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
        .ok()
        .map_err(|e| e.to_string())?;
    let options = CoreWebView2EnvironmentOptions::default();
    // SAFETY: setters on an options object this function owns.
    unsafe {
        options.set_additional_browser_arguments(BROWSER_ARGS.to_owned());
        options.set_is_custom_crash_reporting_enabled(true);
    }
    let options: ICoreWebView2EnvironmentOptions = options.into();
    let folder = HSTRING::from(data_dir);
    let (tx, rx) = std::sync::mpsc::channel();
    CreateCoreWebView2EnvironmentCompletedHandler::wait_for_async_operation(
        // SAFETY: the folder and options outlive the call; WebView2 calls the handler back on this thread.
        Box::new(move |handler| unsafe {
            CreateCoreWebView2EnvironmentWithOptions(PCWSTR::null(), &folder, &options, &handler)
                .map_err(webview2_com::Error::WindowsError)
        }),
        Box::new(move |error, environment| {
            error?;
            tx.send(environment.ok_or_else(|| windows::core::Error::from(E_POINTER)))
                .map_err(|_| windows::core::Error::from(E_UNEXPECTED))
        }),
    )
    .map_err(|e| format!("{e:?}"))?;
    let environment = rx.recv().map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;

    let mut version = PWSTR::null();
    // SAFETY: a live environment; WebView2 allocates the string and take_pwstr frees it.
    let version = unsafe { environment.BrowserVersionString(&mut version) }
        .map_or_else(|e| e.to_string(), |()| take_pwstr(version));
    let dumps = environment.cast::<ICoreWebView2Environment11>().and_then(|e| {
        let mut folder = PWSTR::null();
        // SAFETY: as above.
        unsafe { e.FailureReportFolderPath(&mut folder) }.map(|()| take_pwstr(folder))
    });
    let _ = ENVIRONMENT.set(Environment(environment));
    Ok(format!(
        "WebView2 {version}; crash dumps in {}",
        dumps.unwrap_or_else(|e| format!("(unknown: {e})"))
    ))
}

/// A window builder on the shared environment, once it exists (else Tauri makes its own).
pub fn shared_environment<'a>(
    builder: tauri::WebviewWindowBuilder<'a, tauri::Wry, tauri::AppHandle>,
) -> tauri::WebviewWindowBuilder<'a, tauri::Wry, tauri::AppHandle> {
    match ENVIRONMENT.get() {
        Some(environment) => builder.with_environment(environment.0.clone()),
        None => builder,
    }
}

/// Hears WebView2's ProcessFailed for this window: which process died, why, its exit code and the
/// module it crashed in. `failed` runs on the UI thread.
pub fn watch_webview(
    window: &tauri::WebviewWindow,
    failed: impl Fn(crate::desktop::recovery::Failure) + Send + 'static,
) -> tauri::Result<()> {
    use webview2_com::ProcessFailedEventHandler;
    window.with_webview(move |webview| {
        let handler = ProcessFailedEventHandler::create(Box::new(move |_, args| {
            if let Some(args) = args {
                failed(failure(&args));
            }
            Ok(())
        }));
        let mut token = 0;
        // SAFETY: the controller and its webview are live; Tauri runs this on their UI thread.
        let added = unsafe {
            webview
                .controller()
                .CoreWebView2()
                .and_then(|core| core.add_ProcessFailed(&handler, &mut token))
        };
        if let Err(e) = added {
            eprintln!("lull: can't hear this window's crashes: {e}");
        }
    })
}

fn failure(
    args: &webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2ProcessFailedEventArgs,
) -> crate::desktop::recovery::Failure {
    use crate::desktop::recovery::{Failure, Process};
    use webview2_com::Microsoft::Web::WebView2::Win32::*;
    use webview2_com::take_pwstr;
    use windows::core::{Interface, PWSTR};

    let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
    // SAFETY: event args WebView2 handed us, read during the event.
    let _ = unsafe { args.ProcessFailedKind(&mut kind) };
    let helper = |name: &str| Process::Helper(name.to_owned());
    let process = match kind {
        COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED => Process::Renderer,
        COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE => Process::Unresponsive,
        COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED => Process::Browser,
        COREWEBVIEW2_PROCESS_FAILED_KIND_GPU_PROCESS_EXITED => helper("GPU process"),
        COREWEBVIEW2_PROCESS_FAILED_KIND_FRAME_RENDER_PROCESS_EXITED => helper("a frame's renderer"),
        COREWEBVIEW2_PROCESS_FAILED_KIND_UTILITY_PROCESS_EXITED => helper("utility process"),
        COREWEBVIEW2_PROCESS_FAILED_KIND_SANDBOX_HELPER_PROCESS_EXITED => helper("sandbox helper"),
        COREWEBVIEW2_PROCESS_FAILED_KIND_PPAPI_PLUGIN_PROCESS_EXITED => helper("plugin process"),
        COREWEBVIEW2_PROCESS_FAILED_KIND_PPAPI_BROKER_PROCESS_EXITED => helper("plugin broker"),
        other => helper(&format!("process of kind {}", other.0)),
    };
    let mut failure = Failure {
        process,
        reason: String::new(),
        exit_code: None,
        description: String::new(),
        module: String::new(),
    };
    if let Ok(more) = args.cast::<ICoreWebView2ProcessFailedEventArgs2>() {
        let (mut reason, mut code, mut description) = (COREWEBVIEW2_PROCESS_FAILED_REASON::default(), 0, PWSTR::null());
        // SAFETY: as above; take_pwstr frees the string WebView2 allocated.
        unsafe {
            if more.Reason(&mut reason).is_ok() {
                failure.reason = match reason {
                    COREWEBVIEW2_PROCESS_FAILED_REASON_UNEXPECTED => "unexpectedly".into(),
                    COREWEBVIEW2_PROCESS_FAILED_REASON_UNRESPONSIVE => "unresponsive".into(),
                    COREWEBVIEW2_PROCESS_FAILED_REASON_TERMINATED => "terminated".into(),
                    COREWEBVIEW2_PROCESS_FAILED_REASON_CRASHED => "crashed".into(),
                    COREWEBVIEW2_PROCESS_FAILED_REASON_LAUNCH_FAILED => "failed to start".into(),
                    COREWEBVIEW2_PROCESS_FAILED_REASON_OUT_OF_MEMORY => "out of memory".into(),
                    COREWEBVIEW2_PROCESS_FAILED_REASON_PROFILE_DELETED => "its profile was deleted".into(),
                    other => format!("reason {}", other.0),
                };
            }
            if more.ExitCode(&mut code).is_ok() {
                failure.exit_code = Some(code);
            }
            if more.ProcessDescription(&mut description).is_ok() {
                failure.description = take_pwstr(description);
            }
        }
    }
    if let Ok(more) = args.cast::<ICoreWebView2ProcessFailedEventArgs3>() {
        let mut module = PWSTR::null();
        // SAFETY: as above.
        if unsafe { more.FailureSourceModulePath(&mut module) }.is_ok() {
            failure.module = take_pwstr(module);
        }
    }
    failure
}

/// A hung page can't run a reload: its renderer is busy (tried: the page stayed frozen). This ends the
/// renderer process hosting the window's main frame, and WebView2 then reports that it exited, which
/// reloads the window.
pub fn end_renderer(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    use webview2_com::GetProcessExtendedInfosCompletedHandler;
    use webview2_com::Microsoft::Web::WebView2::Win32::{ICoreWebView2_20, ICoreWebView2Environment13};
    use windows::core::Interface;
    window.with_webview(|webview| {
        let found = (|| -> windows::core::Result<()> {
            let mut frame = 0;
            // SAFETY: the controller and its webview are live; Tauri runs this on their UI thread.
            unsafe {
                webview
                    .controller()
                    .CoreWebView2()?
                    .cast::<ICoreWebView2_20>()?
                    .FrameId(&mut frame)?
            };
            let handler = GetProcessExtendedInfosCompletedHandler::create(Box::new(move |result, infos| {
                result?;
                match infos.map(|infos| renderer_of(&infos, frame)).transpose()?.flatten() {
                    // After this callback has returned and WebView2 is done with its objects.
                    Some(pid) => drop(std::thread::spawn(move || terminate(pid))),
                    None => eprintln!("lull: no renderer hosts frame {frame}"),
                }
                Ok(())
            }));
            // SAFETY: the environment lives as long as the app; WebView2 calls back on this thread.
            unsafe {
                webview
                    .environment()
                    .cast::<ICoreWebView2Environment13>()?
                    .GetProcessExtendedInfos(&handler)
            }
        })();
        if let Err(e) = found {
            eprintln!("lull: can't find the hung renderer: {e}");
        }
    })
}

/// The renderer process whose frames include `frame`.
fn renderer_of(
    infos: &webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2ProcessExtendedInfoCollection,
    frame: u32,
) -> windows::core::Result<Option<u32>> {
    use webview2_com::Microsoft::Web::WebView2::Win32::{COREWEBVIEW2_PROCESS_KIND_RENDERER, ICoreWebView2FrameInfo2};
    use windows::core::{BOOL, Interface};
    let mut count = 0;
    // SAFETY: a collection WebView2 handed us, read during its callback.
    unsafe {
        infos.Count(&mut count)?;
        for i in 0..count {
            let info = infos.GetValueAtIndex(i)?;
            let process = info.ProcessInfo()?;
            let (mut kind, mut pid) = (Default::default(), 0);
            process.Kind(&mut kind)?;
            process.ProcessId(&mut pid)?;
            if kind != COREWEBVIEW2_PROCESS_KIND_RENDERER {
                continue;
            }
            // Held while iterating: WebView2's iterator doesn't keep its collection alive, and
            // walking it after the collection's release read freed memory and crashed lull.
            let collection = info.AssociatedFrameInfos()?;
            let frames = collection.GetIterator()?;
            let mut more = BOOL::default();
            frames.HasCurrent(&mut more)?;
            while more.as_bool() {
                let mut id = 0;
                if frames
                    .GetCurrent()?
                    .cast::<ICoreWebView2FrameInfo2>()?
                    .FrameId(&mut id)
                    .is_ok()
                    && id == frame
                {
                    return Ok(u32::try_from(pid).ok());
                }
                frames.MoveNext(&mut more)?;
            }
        }
    }
    Ok(None)
}

fn terminate(pid: u32) {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_TERMINATE, TerminateProcess};
    // SAFETY: a handle opened here for this one call and closed after it.
    unsafe {
        let process = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if process.is_null() {
            return eprintln!("lull: can't open renderer {pid}: {}", std::io::Error::last_os_error());
        }
        if TerminateProcess(process, 1) == 0 {
            eprintln!("lull: can't end renderer {pid}: {}", std::io::Error::last_os_error());
        }
        CloseHandle(process);
    }
}

/// Where lull's own crash dump goes, as a null-terminated wide path made before any crash.
static CRASH_DUMP: std::sync::OnceLock<Vec<u16>> = std::sync::OnceLock::new();

/// lull's own process crashing (not a page's, which WebView2 dumps): a minidump in `dump`, one file
/// overwritten by the next crash, then Windows ends the process as usual.
pub fn dump_on_crash(dump: &std::path::Path) {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::System::Diagnostics::Debug::SetUnhandledExceptionFilter;
    let wide = dump.as_os_str().encode_wide().chain(Some(0)).collect();
    if CRASH_DUMP.set(wide).is_ok() {
        // SAFETY: installs a filter that only reads the static path above.
        unsafe { SetUnhandledExceptionFilter(Some(write_dump)) };
    }
}

/// Runs on the crashing thread: no allocation, only the prepared path and dbghelp.
unsafe extern "system" fn write_dump(
    info: *const windows_sys::Win32::System::Diagnostics::Debug::EXCEPTION_POINTERS,
) -> i32 {
    use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_WRITE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{CREATE_ALWAYS, CreateFileW, FILE_ATTRIBUTE_NORMAL};
    use windows_sys::Win32::System::Diagnostics::Debug::{
        MINIDUMP_EXCEPTION_INFORMATION, MiniDumpWithIndirectlyReferencedMemory, MiniDumpWithThreadInfo,
        MiniDumpWithUnloadedModules, MiniDumpWriteDump,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetCurrentProcessId, GetCurrentThreadId};
    /// Windows' EXCEPTION_CONTINUE_SEARCH: the process then ends as it would have.
    const CONTINUE_SEARCH: i32 = 0;
    let Some(path) = CRASH_DUMP.get() else {
        return CONTINUE_SEARCH;
    };
    // SAFETY: the path is null-terminated; the handle is closed below; `info` is the exception
    // Windows passed this filter, valid while it runs.
    unsafe {
        let file = CreateFileW(
            path.as_ptr(),
            GENERIC_WRITE,
            0,
            std::ptr::null(),
            CREATE_ALWAYS,
            FILE_ATTRIBUTE_NORMAL,
            std::ptr::null_mut(),
        );
        if file == INVALID_HANDLE_VALUE {
            return CONTINUE_SEARCH;
        }
        let exception = MINIDUMP_EXCEPTION_INFORMATION {
            ThreadId: GetCurrentThreadId(),
            ExceptionPointers: info.cast_mut(),
            ClientPointers: 0,
        };
        MiniDumpWriteDump(
            GetCurrentProcess(),
            GetCurrentProcessId(),
            file,
            MiniDumpWithThreadInfo | MiniDumpWithUnloadedModules | MiniDumpWithIndirectlyReferencedMemory,
            &exception,
            std::ptr::null(),
            std::ptr::null(),
        );
        CloseHandle(file);
    }
    CONTINUE_SEARCH
}

/// Calls `on_change` each time this computer's IPv4 addresses change, from a thread of its own.
pub fn follow_network(on_change: impl Fn() + Send + 'static) {
    use windows_sys::Win32::NetworkManagement::IpHelper::NotifyAddrChange;
    let spawned = std::thread::Builder::new().name("network".into()).spawn(move || {
        loop {
            // SAFETY: with no handle and no OVERLAPPED the call blocks this thread until the IPv4
            // address table changes; it reads and writes nothing of ours.
            let status = unsafe { NotifyAddrChange(std::ptr::null_mut(), std::ptr::null()) };
            if status != 0 {
                eprintln!("lull: not following network changes (error {status})");
                return;
            }
            on_change();
        }
    });
    if let Err(e) = spawned {
        eprintln!("lull: not following network changes: {e}");
    }
}
