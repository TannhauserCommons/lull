//! Windows: the tray glyph follows the taskbar's theme, not the app's (DESKTOP.md 4).
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
