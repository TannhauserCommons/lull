//! Windows: the tray glyph follows the taskbar's theme, not the app's (DESKTOP.md 4), and the
//! sidecar dies with the app.
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
