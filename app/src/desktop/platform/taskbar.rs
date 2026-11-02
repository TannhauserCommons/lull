//! Windows: Previous, Play/Pause and Next under a window's taskbar thumbnail (ITaskbarList3), like
//! other music players. `main` and the Classic window each get them; they show the one now-playing
//! report (player.rs) and send their clicks to whichever window plays.
//! The buttons can be added only once Windows has made the window's taskbar button, which it says
//! with the registered "TaskbarButtonCreated" message (again after Explorer restarts), and a click
//! arrives as WM_COMMAND: so each window gets a subclass that hears both.
#![allow(unsafe_code)]

use super::ThumbButtons;
use super::glyphs::{self, Glyph};
use super::windows::light_taskbar;
use crate::desktop::player;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Manager, WebviewWindow};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{CreateBitmap, DeleteObject};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::Shell::{
    DefSubclassProc, ITaskbarList3, RemoveWindowSubclass, SetWindowSubclass, THB_FLAGS, THB_ICON, THB_TOOLTIP,
    THBF_DISABLED, THBF_ENABLED, THBN_CLICKED, THUMBBUTTON, TaskbarList,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateIconIndirect, HICON, ICONINFO, RegisterWindowMessageW, SM_CXSMICON, WM_COMMAND, WM_NCDESTROY,
};
use windows::core::w;

const PREVIOUS: u32 = 1;
const PLAY_PAUSE: u32 = 2;
const NEXT: u32 = 3;
const SUBCLASS_ID: usize = 0x6c75_6c6c;

static APP: OnceLock<AppHandle> = OnceLock::new();
/// What the buttons show now, for a window whose taskbar button appears later.
static SHOWN: Mutex<ThumbButtons> = Mutex::new(ThumbButtons {
    playing: false,
    usable: false,
});
/// Windows whose buttons are in the taskbar (handles as numbers: HWND isn't Send).
static WINDOWS: Mutex<Vec<isize>> = Mutex::new(Vec::new());

/// Called once the window exists; the buttons follow when its taskbar button does. A subclass
/// belongs to the window's own thread, the UI thread.
pub fn attach(window: &WebviewWindow) {
    let _ = APP.set(window.app_handle().clone());
    let hwnd = match window.hwnd() {
        Ok(hwnd) => hwnd.0 as isize,
        Err(e) => return eprintln!("lull: no taskbar buttons: {e}"),
    };
    let label = window.label().to_owned();
    let subclassed = window.run_on_main_thread(move || {
        // SAFETY: a live window of this thread; the subclass removes itself when the window goes.
        if !unsafe { SetWindowSubclass(HWND(hwnd as *mut _), Some(subclass), SUBCLASS_ID, 0) }.as_bool() {
            eprintln!("lull: no taskbar buttons for {label}");
        }
    });
    if let Err(e) = subclassed {
        eprintln!("lull: no taskbar buttons: {e}");
    }
}

/// New state for every window's buttons; Windows wants the update from the UI thread.
pub fn show(app: &AppHandle, state: ThumbButtons) {
    {
        let mut shown = lock(&SHOWN);
        if *shown == state {
            return;
        }
        *shown = state;
    }
    let update = move || {
        for hwnd in lock(&WINDOWS).clone() {
            if let Err(e) = put(HWND(hwnd as *mut _), state, Put::Update) {
                eprintln!("lull: taskbar buttons: {e}");
            }
        }
    };
    if let Err(e) = app.run_on_main_thread(update) {
        eprintln!("lull: taskbar buttons: {e}");
    }
}

unsafe extern "system" fn subclass(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    if msg == button_created() {
        let state = *lock(&SHOWN);
        match put(hwnd, state, Put::Add) {
            Ok(()) => {
                let mut windows = lock(&WINDOWS);
                if !windows.contains(&(hwnd.0 as isize)) {
                    windows.push(hwnd.0 as isize);
                }
            }
            Err(e) => eprintln!("lull: taskbar buttons: {e}"),
        }
    } else if msg == WM_COMMAND && (wparam.0 >> 16) as u32 & 0xffff == THBN_CLICKED {
        let action = match wparam.0 as u32 & 0xffff {
            PREVIOUS => "previous",
            PLAY_PAUSE => "play-pause",
            NEXT => "next",
            _ => return LRESULT(0),
        };
        if let Some(app) = APP.get()
            && let Err(e) = player::forward(app, action)
        {
            eprintln!("lull: {action}: {e}");
        }
        return LRESULT(0);
    } else if msg == WM_NCDESTROY {
        lock(&WINDOWS).retain(|w| *w != hwnd.0 as isize);
        // SAFETY: removes the subclass this module set on this window.
        unsafe {
            let _ = RemoveWindowSubclass(hwnd, Some(subclass), SUBCLASS_ID);
        }
    }
    // SAFETY: the message as Windows sent it, to the next handler in the chain.
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

fn button_created() -> u32 {
    static MESSAGE: OnceLock<u32> = OnceLock::new();
    // SAFETY: a static, null-terminated name.
    *MESSAGE.get_or_init(|| unsafe { RegisterWindowMessageW(w!("TaskbarButtonCreated")) })
}

#[derive(Clone, Copy)]
enum Put {
    Add,
    Update,
}

fn put(hwnd: HWND, state: ThumbButtons, how: Put) -> windows::core::Result<()> {
    // SAFETY: COM is initialised on the UI thread (Tauri's window loop), where this runs.
    let taskbar: ITaskbarList3 = unsafe { CoCreateInstance(&TaskbarList, None, CLSCTX_INPROC_SERVER)? };
    // SAFETY: a live window handle.
    let size = unsafe { GetSystemMetricsForDpi(SM_CXSMICON, GetDpiForWindow(hwnd)) }.max(16) as u32;
    let buttons = [
        button(PREVIOUS, Glyph::Previous, "Previous", size, state.usable),
        if state.playing {
            button(PLAY_PAUSE, Glyph::Pause, "Pause", size, state.usable)
        } else {
            button(PLAY_PAUSE, Glyph::Play, "Play", size, state.usable)
        },
        button(NEXT, Glyph::Next, "Next", size, state.usable),
    ];
    // SAFETY: an initialised taskbar object and a live window; the buttons are plain data.
    unsafe {
        taskbar.HrInit()?;
        match how {
            Put::Add => taskbar.ThumbBarAddButtons(hwnd, &buttons),
            Put::Update => taskbar.ThumbBarUpdateButtons(hwnd, &buttons),
        }
    }
}

fn button(id: u32, glyph: Glyph, tip: &str, size: u32, enabled: bool) -> THUMBBUTTON {
    let mut button = THUMBBUTTON {
        dwMask: THB_ICON | THB_TOOLTIP | THB_FLAGS,
        iId: id,
        hIcon: icon(glyph, size),
        dwFlags: if enabled { THBF_ENABLED } else { THBF_DISABLED },
        ..Default::default()
    };
    for (slot, unit) in button.szTip.iter_mut().zip(tip.encode_utf16()) {
        *slot = unit;
    }
    button
}

/// One icon per glyph, size and taskbar theme, made once and kept for the app's life. A light
/// taskbar shows its thumbnails on light grey: dark glyphs there, white ones on a dark taskbar.
fn icon(glyph: Glyph, size: u32) -> HICON {
    type Icons = HashMap<(Glyph, u32, bool), isize>;
    static ICONS: Mutex<Option<Icons>> = Mutex::new(None);
    let light = light_taskbar();
    let mut icons = lock(&ICONS);
    let handle = *icons
        .get_or_insert_with(HashMap::new)
        .entry((glyph, size, light))
        .or_insert_with(|| {
            let rgb = if light { [0x1f, 0x1f, 0x1f] } else { [0xff, 0xff, 0xff] };
            make_icon(&glyphs::render(glyph, size, rgb), size).0 as isize
        });
    HICON(handle as *mut _)
}

/// A 32-bit icon from BGRA pixels; its alpha channel is the shape, so the mask is empty.
fn make_icon(pixels: &[u8], size: u32) -> HICON {
    let side = size as i32;
    let mask_bytes = vec![0u8; size.div_ceil(16) as usize * 2 * size as usize];
    // SAFETY: both buffers hold exactly the bitmap's rows; the bitmaps are freed once the icon has
    // copied them.
    unsafe {
        let color = CreateBitmap(side, side, 1, 32, Some(pixels.as_ptr().cast()));
        let mask = CreateBitmap(side, side, 1, 1, Some(mask_bytes.as_ptr().cast()));
        let info = ICONINFO {
            fIcon: true.into(),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: color,
        };
        let icon = CreateIconIndirect(&info).unwrap_or_default();
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        icon
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}
