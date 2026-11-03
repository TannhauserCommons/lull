//! Windows: the tray glyph follows the taskbar's theme, not the app's (DESKTOP.md 4), the sidecar
//! dies with the app, a hidden `main` tells WebView2 so, and links open in the browser.
//! `SystemUsesLightTheme` is that theme; Tauri's ThemeChanged follows the app theme instead, so a
//! thread waits on the registry key and swaps the glyph when it changes.
#![allow(unsafe_code)]

use std::ffi::c_void;
use std::ptr::null_mut;
use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_NOTIFY, KEY_READ, REG_NOTIFY_CHANGE_LAST_SET, RRF_RT_REG_DWORD, RegCloseKey,
    RegGetValueW, RegNotifyChangeKeyValue, RegOpenKeyExW,
};
use windows_sys::w;

const PERSONALIZE: *const u16 = w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");

/// A light taskbar. Without the value (Windows 10 before 1903) the taskbar is dark.
pub fn light_taskbar() -> bool {
    let mut value: u32 = 0;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: a DWORD read into a u32 of the size we pass; the strings are static and null-terminated.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PERSONALIZE,
            w!("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            null_mut(),
            (&raw mut value).cast::<c_void>(),
            &mut size,
        )
    };
    status == ERROR_SUCCESS && value == 1
}

/// Calls `changed` after every write under the Personalize key, for the life of the app.
pub fn watch_taskbar(changed: impl Fn() + Send + 'static) {
    std::thread::spawn(move || {
        let mut key: HKEY = null_mut();
        // SAFETY: opens a key we close below; `key` is only used on this thread.
        let opened = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, PERSONALIZE, 0, KEY_NOTIFY | KEY_READ, &mut key) };
        if opened != ERROR_SUCCESS {
            eprintln!("lull: can't watch the taskbar theme (error {opened})");
            return;
        }
        // SAFETY: synchronous wait on the open key; returns once a value under it changes.
        while unsafe { RegNotifyChangeKeyValue(key, 0, REG_NOTIFY_CHANGE_LAST_SET, null_mut(), 0) } == ERROR_SUCCESS {
            changed();
        }
        // SAFETY: the key opened above.
        unsafe { RegCloseKey(key) };
    });
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
